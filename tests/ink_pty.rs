//! The real radar binary in a pseudo-terminal, drawn by Ink (M11
//! handover item 4, C4(b)(d), C1(e), C5(e)): the top bar, the repositories
//! list, a repository's page, the actions menu and the Work screen appear at
//! 80x24 and 160x40; the renderer process is ended by the pid the
//! supervisor logged and comes back; after `q`, the terminal is as it was
//! (alternate screen left, mouse reporting off, the cursor shown, raw mode
//! off). With no Ink to start, radar says why in two lines, exits 1 and leaves
//! the terminal as it found it (the one test here that needs no node).
//!
//! Nothing reaches a service: a scratch config names GitLab and Jira with
//! tokens from variables that are not set, their caches are written in a
//! scratch state folder, `RADAR_MODE=read-only` and `RADAR_NO_WRITES=1` are set,
//! and every XDG folder and HOME are scratch folders.
//!
//! The Ink runs need node and the bundle (`cd ink && npm run bundle`), so
//! they are ignored by default: `cargo nextest run -p radar --test ink_pty
//! --run-ignored all`. `RADAR_PTY_BIN` runs another radar binary than this
//! build's, `RADAR_INK_BUNDLE` another bundle, `RADAR_PTY_NODE` names the node to
//! put first on PATH, and `RADAR_PTY_OUT` keeps the captures (raw bytes and the
//! screen at each step) in that folder.

#![cfg(all(unix, feature = "gitlab", feature = "jira", feature = "workbench"))]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

/// How long one step may take before the test says what it saw.
const PATIENCE: Duration = Duration::from_secs(12);

const CONFIG: &str = r#"
[gitlab]
url = "https://gitlab.example"
token = "env:RADAR_PTY_SURELY_UNSET"
base = "acme"

[[gitlab.source]]
group = "team-a"
code = "a"
color = "green"

[jira]

[[jira.site]]
name = "acme"
url = "https://acme.atlassian.net"
email = "alice@example.com"
token = "env:RADAR_PTY_SURELY_UNSET"
color = "blue"

[[jira.site.project]]
key = "ABC"

[[jira.view]]
name = "Our project"
jql = "project = ABC ORDER BY updated DESC"
"#;

fn gitlab_cache() -> String {
    let mr = |iid: u64, title: &str, branch: &str| {
        format!(
            r#"{{"iid":{iid},"title":"{title}","draft":false,
            "web_url":"https://gitlab.example/acme/team-a/api/-/merge_requests/{iid}",
            "updated_at":"2026-10-06T07:00:00Z","source_branch":"{branch}","target_branch":"main",
            "notes":0,"conflicts":false,"merge_status":"NOT_APPROVED","approved":false,
            "approvals_required":1,"approvals_left":1,
            "author":{{"username":"alice","name":"Alice Example"}},"reviewers":[],"approved_by":[],
            "pipeline":"passed","pipeline_url":null,"additions":10,"deletions":2,"files":1,
            "project_path":"acme/team-a/api","project_name":"api"}}"#
        )
    };
    format!(
        r#"{{"me":"me","data":{{"acme/team-a":{{"mrs":[{},{}],
        "projects":[{{"name":"api","full_path":"acme/team-a/api","default_branch":"main"}},
                    {{"name":"web","full_path":"acme/team-a/web","default_branch":"main"}}]}}}}}}"#,
        mr(263, "ABC-12: Add a stats endpoint", "feature/abc-12-stats"),
        mr(264, "Tidy the build", "abc-20-tidy"),
    )
}

const JIRA_CACHE: &str = r#"{"data":{"acme":{
  "me":{"account_id":"acc-alice","name":"Alice Example"},
  "tickets":{"ABC-12":{"key":"ABC-12","summary":"Add a stats endpoint","kind":"Story","status":"In Review",
    "category":"doing","assignee":"Alice Example","assignee_id":"acc-alice",
    "created":"2026-09-20T09:00:00.000+0000","updated":"2026-10-06T06:00:00.000+0000",
    "project":"ABC","project_name":"Acme ABC","description":[]}},
  "found":{"project = ABC ORDER BY updated DESC":["ABC-12"]}}}}"#;

// --- a terminal screen, enough of one to read what was drawn -----------------

/// The cells a terminal shows, fed the bytes a program writes, and the modes
/// the program left it in.
struct Screen {
    w: usize,
    h: usize,
    cells: Vec<Vec<char>>,
    row: usize,
    col: usize,
    saved: (usize, usize),
    /// A byte sequence not complete at the end of the last chunk.
    pending: Vec<u8>,
    alt: bool,
    /// Times the alternate screen was entered.
    alt_entered: usize,
    mouse: Vec<u16>,
    cursor_shown: bool,
    /// What a terminal answers to the queries it was sent (the device's
    /// attributes, its status, the cursor, the cell size): a program that
    /// asks reads its keys after the answer.
    replies: Vec<u8>,
}

impl Screen {
    fn new(w: u16, h: u16) -> Self {
        let (w, h) = (w as usize, h as usize);
        Self {
            w,
            h,
            cells: vec![vec![' '; w]; h],
            row: 0,
            col: 0,
            saved: (0, 0),
            pending: Vec::new(),
            alt: false,
            alt_entered: 0,
            mouse: Vec::new(),
            cursor_shown: true,
            replies: Vec::new(),
        }
    }

    fn text(&self) -> String {
        self.cells
            .iter()
            .map(|r| r.iter().collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn clear(&mut self) {
        self.cells = vec![vec![' '; self.w]; self.h];
    }

    fn newline(&mut self) {
        if self.row + 1 >= self.h {
            self.cells.remove(0);
            self.cells.push(vec![' '; self.w]);
        } else {
            self.row += 1;
        }
    }

    fn put(&mut self, c: char) {
        if self.col >= self.w {
            self.col = 0;
            self.newline();
        }
        self.cells[self.row][self.col] = c;
        self.col += 1;
    }

    fn feed(&mut self, bytes: &[u8]) {
        let mut data = std::mem::take(&mut self.pending);
        data.extend_from_slice(bytes);
        let mut i = 0;
        while i < data.len() {
            let b = data[i];
            match b {
                0x1b => match self.escape(&data[i..]) {
                    Some(n) => i += n,
                    None => {
                        self.pending = data[i..].to_vec();
                        return;
                    }
                },
                b'\r' => {
                    self.col = 0;
                    i += 1;
                }
                b'\n' => {
                    self.newline();
                    i += 1;
                }
                0x08 => {
                    self.col = self.col.saturating_sub(1);
                    i += 1;
                }
                b'\t' => {
                    self.col = ((self.col / 8) + 1) * 8;
                    i += 1;
                }
                0..=0x1f | 0x7f => i += 1,
                _ => {
                    let len = match b {
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        0xf0..=0xf7 => 4,
                        _ => 1,
                    };
                    if i + len > data.len() {
                        self.pending = data[i..].to_vec();
                        return;
                    }
                    let c = std::str::from_utf8(&data[i..i + len]).ok().and_then(|s| s.chars().next()).unwrap_or('?');
                    self.put(c);
                    i += len;
                }
            }
        }
    }

    /// One escape sequence at the start of `s`: its length, or `None` when it
    /// is not complete yet.
    fn escape(&mut self, s: &[u8]) -> Option<usize> {
        let next = *s.get(1)?;
        match next {
            b'[' => {
                let end = s.iter().skip(2).position(|b| (0x40..=0x7e).contains(b))? + 2;
                let body = std::str::from_utf8(&s[2..end]).unwrap_or("");
                self.csi(body, s[end] as char);
                Some(end + 1)
            }
            b']' | b'P' | b'_' | b'^' => {
                // A string to BEL or ESC \.
                let mut j = 2;
                loop {
                    match s.get(j)? {
                        0x07 => return Some(j + 1),
                        0x1b if s.get(j + 1) == Some(&b'\\') => return Some(j + 2),
                        0x1b if s.get(j + 1).is_some() => return Some(j),
                        _ => j += 1,
                    }
                }
            }
            b'(' | b')' | b'#' => s.get(2).map(|_| 3),
            b'7' => {
                self.saved = (self.row, self.col);
                Some(2)
            }
            b'8' => {
                (self.row, self.col) = self.saved;
                Some(2)
            }
            b'M' => {
                self.row = self.row.saturating_sub(1);
                Some(2)
            }
            _ => Some(2),
        }
    }

    fn csi(&mut self, body: &str, last: char) {
        let private = body.starts_with('?');
        let body = body.trim_start_matches(['?', '>', '=', '<']);
        let args: Vec<usize> = body.split(';').map(|a| a.parse().unwrap_or(0)).collect();
        let n = |i: usize| args.get(i).copied().filter(|v| *v > 0).unwrap_or(1);
        let (w, h) = (self.w, self.h);
        match (private, last) {
            (true, 'h') | (true, 'l') => {
                let on = last == 'h';
                for mode in body.split(';').filter_map(|a| a.parse::<u16>().ok()) {
                    match mode {
                        1049 | 1047 | 47 => {
                            if on && !self.alt {
                                self.alt_entered += 1;
                                self.saved = (self.row, self.col);
                            }
                            self.alt = on;
                            self.clear();
                            if !on {
                                (self.row, self.col) = self.saved;
                            }
                        }
                        25 => self.cursor_shown = on,
                        1000 | 1002 | 1003 | 1005 | 1006 | 1015 => {
                            self.mouse.retain(|m| *m != mode);
                            if on {
                                self.mouse.push(mode);
                            }
                        }
                        _ => {}
                    }
                }
            }
            (false, 'H') | (false, 'f') => {
                self.row = (n(0) - 1).min(h - 1);
                self.col = (n(1) - 1).min(w - 1);
            }
            (false, 'A') => self.row = self.row.saturating_sub(n(0)),
            (false, 'B') => self.row = (self.row + n(0)).min(h - 1),
            (false, 'C') => self.col = (self.col + n(0)).min(w - 1),
            (false, 'D') => self.col = self.col.saturating_sub(n(0)),
            (false, 'E') => {
                self.row = (self.row + n(0)).min(h - 1);
                self.col = 0;
            }
            (false, 'F') => {
                self.row = self.row.saturating_sub(n(0));
                self.col = 0;
            }
            (false, 'G') | (false, '`') => self.col = (n(0) - 1).min(w - 1),
            (false, 'd') => self.row = (n(0) - 1).min(h - 1),
            (false, 'J') => {
                let (r, c) = (self.row, self.col.min(w));
                match args.first().copied().unwrap_or(0) {
                    0 => {
                        self.cells[r][c..].fill(' ');
                        self.cells[r + 1..].iter_mut().for_each(|l| l.fill(' '));
                    }
                    1 => {
                        self.cells[..r].iter_mut().for_each(|l| l.fill(' '));
                        self.cells[r][..c.min(w - 1) + 1].fill(' ');
                    }
                    _ => self.clear(),
                }
            }
            (false, 'K') => {
                let (r, c) = (self.row, self.col.min(w));
                match args.first().copied().unwrap_or(0) {
                    0 => self.cells[r][c..].fill(' '),
                    1 => self.cells[r][..c.min(w - 1) + 1].fill(' '),
                    _ => self.cells[r].fill(' '),
                }
            }
            (false, 'X') => {
                let (r, c) = (self.row, self.col.min(w));
                let end = (c + n(0)).min(w);
                self.cells[r][c..end].fill(' ');
            }
            (false, 'P') => {
                let r = self.row;
                for _ in 0..n(0) {
                    if self.col < w {
                        self.cells[r].remove(self.col);
                        self.cells[r].push(' ');
                    }
                }
            }
            (false, '@') => {
                let r = self.row;
                for _ in 0..n(0) {
                    if self.col < w {
                        self.cells[r].insert(self.col, ' ');
                        self.cells[r].pop();
                    }
                }
            }
            // Queries: answered as an xterm-like terminal with no graphics.
            (false, 'c') if !body.starts_with('>') && args.iter().all(|a| *a == 0) => self.replies.extend(b"\x1b[?62;22c"),
            (false, 'n') if args.first() == Some(&5) => self.replies.extend(b"\x1b[0n"),
            (false, 'n') if args.first() == Some(&6) => {
                self.replies.extend(format!("\x1b[{};{}R", self.row + 1, self.col.min(w - 1) + 1).bytes())
            }
            (false, 't') if args.first() == Some(&16) => self.replies.extend(b"\x1b[6;16;8t"),
            (false, 's') => self.saved = (self.row, self.col),
            (false, 'u') => (self.row, self.col) = self.saved,
            _ => {}
        }
    }
}

// --- radar in a pseudo-terminal ---------------------------------------------------

fn checkout() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn binary() -> PathBuf {
    std::env::var_os("RADAR_PTY_BIN").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_radar")))
}

fn bundle() -> PathBuf {
    let path = std::env::var_os("RADAR_INK_BUNDLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| checkout().join("ink/dist/radar-ink.js"));
    assert!(path.is_file(), "no Ink bundle at {}: cd ink && npm run bundle", path.display());
    path
}

/// Where the captures go: `RADAR_PTY_OUT`, else a fresh temporary folder.
fn out_dir() -> PathBuf {
    let dir = std::env::var_os("RADAR_PTY_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("radar-pty-{}", std::process::id())));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Scratch HOME and XDG folders with the config and the parts' caches.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("radar-pty-home-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["home", "config/radar", "state/radar", "cache", "data"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("config/radar/config.toml"), CONFIG).unwrap();
        std::fs::write(root.join("state/radar/gitlab.json"), gitlab_cache()).unwrap();
        std::fs::write(root.join("state/radar/jira.json"), JIRA_CACHE).unwrap();
        Self { root }
    }

    fn log(&self) -> PathBuf {
        self.root.join("state/radar/radar.log")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Copy a folder's files (not its log), recursively.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).expect("the state folder") {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("radar.log") {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            copy_dir(&path, &to.join(&name));
        } else {
            std::fs::copy(&path, to.join(&name)).unwrap();
        }
    }
}

/// Keys of a script step: text, with `<esc>`, `<enter>`, `<down>`, `<up>`,
/// `<left>`, `<right>`, `<tab>` for those keys.
fn keys(spec: &str) -> String {
    spec.replace("<esc>", "\x1b")
        .replace("<enter>", "\r")
        .replace("<down>", "\x1b[B")
        .replace("<up>", "\x1b[A")
        .replace("<right>", "\x1b[C")
        .replace("<left>", "\x1b[D")
        .replace("<tab>", "\t")
}

struct Run {
    name: String,
    screen: Screen,
    raw: Vec<u8>,
    rx: Receiver<Vec<u8>>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pair: portable_pty::PtyPair,
    scratch: Scratch,
    out: PathBuf,
    steps: usize,
    /// How long radar took to end after `q`.
    quit_took: Duration,
    /// A survey: a step that does not show what it waits for is recorded
    /// (`missed`) and the walk goes on.
    survey: bool,
    missed: Vec<String>,
}

impl Run {
    fn start(name: &str, w: u16, h: u16) -> Self {
        Self::launch(name, true, w, h, None)
    }

    /// radar with no Ink to find: the bundle it is told to use is not a file.
    fn start_without_ink(name: &str, w: u16, h: u16) -> Self {
        Self::launch(name, false, w, h, None)
    }

    /// radar with a copy of `config` (the operator's), read only: the
    /// environment is kept (tokens come from it), every XDG folder is a
    /// scratch one, and no cache is laid down, so what shows is read live.
    fn live(name: &str, w: u16, h: u16, config: &Path) -> Self {
        Self::launch(name, true, w, h, Some(config))
    }

    fn launch(renderer: &str, with_ink: bool, w: u16, h: u16, live: Option<&Path>) -> Self {
        let name = format!("{}{renderer}-{w}x{h}", if live.is_some() { "live-" } else { "" });
        let scratch = Scratch::new(&name);
        if let Some(config) = live {
            std::fs::copy(config, scratch.root.join("config/radar/config.toml")).expect("a copy of the config");
            for cache in ["gitlab.json", "jira.json"] {
                std::fs::remove_file(scratch.root.join("state/radar").join(cache)).unwrap();
            }
            // RADAR_PTY_STATE: a copy of a radar state folder (caches, the journal
            // with its work map), so Work shows that user's items.
            if let Some(state) = std::env::var_os("RADAR_PTY_STATE") {
                copy_dir(Path::new(&state), &scratch.root.join("state/radar"));
            }
        }
        let pair = native_pty_system()
            .openpty(PtySize { rows: h, cols: w, pixel_width: 0, pixel_height: 0 })
            .expect("a pseudo-terminal");
        let mut cmd = CommandBuilder::new(binary());
        if live.is_none() {
            cmd.env_clear();
            cmd.env("RADAR_SANDBOX", "1");
        }
        let mut path = std::env::var("PATH").unwrap_or_default();
        if let Some(node) = std::env::var_os("RADAR_PTY_NODE") {
            let dir = Path::new(&node).parent().unwrap().to_string_lossy().into_owned();
            path = format!("{dir}:{path}");
        }
        let root = &scratch.root;
        let home = match live {
            Some(_) => std::env::var("HOME").unwrap_or_default(),
            None => root.join("home").display().to_string(),
        };
        for (k, v) in [
            ("PATH", path),
            ("HOME", home),
            ("XDG_CONFIG_HOME", root.join("config").display().to_string()),
            ("XDG_STATE_HOME", root.join("state").display().to_string()),
            ("XDG_CACHE_HOME", root.join("cache").display().to_string()),
            ("XDG_DATA_HOME", root.join("data").display().to_string()),
            ("TERM", "xterm-256color".into()),
            ("COLORTERM", "truecolor".into()),
            ("LANG", "C.UTF-8".into()),
            ("RADAR_MODE", "read-only".into()),
            ("RADAR_NO_WRITES", "1".into()),
        ] {
            cmd.env(k, v);
        }
        // A bundle that is named but missing is "no Ink": radar-js is skipped too.
        cmd.env("RADAR_INK_BUNDLE", if with_ink { bundle() } else { root.join("no-such-bundle/radar-ink.js") });
        cmd.cwd(root.join("home"));
        let child = pair.slave.spawn_command(cmd).expect("radar starts");
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 65536];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            name,
            screen: Screen::new(w, h),
            raw: Vec::new(),
            rx,
            writer,
            child,
            pair,
            scratch,
            out: out_dir(),
            steps: 0,
            quit_took: Duration::ZERO,
            survey: false,
            missed: Vec::new(),
        }
    }

    fn take(&mut self, chunk: Vec<u8>) {
        self.screen.feed(&chunk);
        self.raw.extend(chunk);
        if !self.screen.replies.is_empty() {
            let replies = std::mem::take(&mut self.screen.replies);
            self.writer.write_all(&replies).unwrap();
            self.writer.flush().unwrap();
        }
    }

    /// Read what radar draws until `done` holds, then save the screen as a step.
    fn until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + if self.survey { Duration::from_secs(5) } else { PATIENCE };
        // Draws come in bursts: once it holds, take what is still arriving.
        loop {
            if done(self) {
                self.settle();
                if done(self) {
                    break;
                }
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left.min(Duration::from_millis(200))) {
                Ok(chunk) => self.take(chunk),
                Err(RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                Err(_) if self.survey => {
                    // A survey records what it did not see and goes on.
                    self.steps += 1;
                    self.missed.push(what.to_string());
                    self.save(&format!("{:02}-{}-MISSED", self.steps, what.replace(' ', "-")));
                    return;
                }
                Err(_) => {
                    self.save("timeout");
                    panic!("{}: waited for {what}; the screen is:\n{}\nlog:\n{}", self.name, self.screen.text(), self.log());
                }
            }
        }
        self.steps += 1;
        self.save(&format!("{:02}-{}", self.steps, what.replace(' ', "-")));
    }

    /// Take what is still arriving of a draw: until 150 ms pass quiet, or a
    /// second at most (a renderer that animates never goes quiet).
    fn settle(&mut self) {
        let end = Instant::now() + Duration::from_secs(1);
        while Instant::now() < end {
            match self.rx.recv_timeout(Duration::from_millis(150)) {
                Ok(chunk) => self.take(chunk),
                Err(_) => break,
            }
        }
    }

    /// No row of the list behind shows through a menu's rows (a merge
    /// request's words on a line of the menu).
    fn menu_is_clear(&self) -> bool {
        let rows = ["open its page", "hide its repos", "open the group in GitLab", "refresh now", "copy the group's link"];
        self.screen
            .text()
            .lines()
            .filter(|l| rows.iter().any(|r| l.contains(r)))
            .all(|l| !l.contains("Tidy") && !l.contains("ABC-12") && !l.contains("no recent run"))
    }

    fn shows(&self, words: &[&str]) -> bool {
        let text = self.screen.text();
        words.iter().all(|w| text.contains(w))
    }

    fn press(&mut self, bytes: &str) {
        self.writer.write_all(bytes.as_bytes()).unwrap();
        self.writer.flush().unwrap();
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.scratch.log()).unwrap_or_default()
    }

    /// The renderer processes the supervisor recorded, in order.
    fn renderer_pids(&self) -> Vec<u32> {
        self.log()
            .lines()
            .filter_map(|l| l.split("Ink renderer started, pid ").nth(1))
            .filter_map(|p| p.trim().parse().ok())
            .collect()
    }

    fn save(&self, step: &str) {
        std::fs::write(self.out.join(format!("{}-{step}.txt", self.name)), self.screen.text()).unwrap();
        std::fs::write(self.out.join(format!("{}.bin", self.name)), &self.raw).unwrap();
        std::fs::write(self.out.join(format!("{}-radar.log", self.name)), self.log()).unwrap();
    }

    /// Quit with `q` and wait for radar to end; its exit status.
    fn quit_within(&mut self, patience: Duration) -> portable_pty::ExitStatus {
        self.press("q");
        let started = Instant::now();
        let deadline = started + patience;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                while let Ok(chunk) = self.rx.recv_timeout(Duration::from_millis(150)) {
                    self.take(chunk);
                }
                self.save("quit");
                self.quit_took = started.elapsed();
                return status;
            }
            assert!(Instant::now() < deadline, "{}: radar did not quit:\n{}", self.name, self.screen.text());
            if let Ok(chunk) = self.rx.recv_timeout(Duration::from_millis(100)) {
                self.take(chunk);
            }
        }
    }

    /// The terminal changes size, as a window resized does.
    fn resize(&mut self, w: u16, h: u16) {
        self.pair.master.resize(PtySize { rows: h, cols: w, pixel_width: 0, pixel_height: 0 }).unwrap();
        let mut screen = Screen::new(w, h);
        (screen.alt, screen.alt_entered, screen.mouse, screen.cursor_shown) =
            (self.screen.alt, self.screen.alt_entered, self.screen.mouse.clone(), self.screen.cursor_shown);
        self.screen = screen;
    }

    /// The terminal's line settings now, as `stty -a` reads them on it.
    fn stty(&mut self) -> String {
        let mut cmd = CommandBuilder::new("stty");
        cmd.arg("-a");
        let mut child = self.pair.slave.spawn_command(cmd).expect("stty runs");
        let mut out = Vec::new();
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Ok(chunk) = self.rx.recv_timeout(Duration::from_millis(100)) {
                out.extend(chunk);
            }
            if child.try_wait().unwrap().is_some() {
                while let Ok(chunk) = self.rx.recv_timeout(Duration::from_millis(150)) {
                    out.extend(chunk);
                }
                break;
            }
            assert!(Instant::now() < deadline, "stty did not end");
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}

/// The journey both renderers take, waiting each time for what the step
/// must show: the top bar and the repositories list, the actions menu, a
/// group's page, a merge request's page, the Work screen; then the terminal
/// grows to 160x40 and the Work screen and the repositories list are drawn
/// at that size.
fn journey(run: &mut Run) {
    let top_bar = |r: &Run| r.screen.text().lines().next().is_some_and(|l| l.trim_start().starts_with("radar") && l.contains("Settings"));
    run.until("top bar and repositories", |r| top_bar(r) && r.shows(&["REPOSITORIES", "team-a", "api", "!263"]));
    run.press("x");
    run.until("actions menu", |r| r.shows(&["open its page", "open the group in GitLab"]) && r.menu_is_clear());
    run.press("\x1b");
    run.until("menu closed", |r| !r.shows(&["open the group in GitLab"]) && r.shows(&["OPENED BY"]));
    run.press("\r");
    run.until("group page", |r| r.shows(&["Repositories / a", "acme/team-a"]));
    run.press("\x1b");
    run.until("back on the list", |r| r.shows(&["OPENED BY"]) && !r.shows(&["Repositories / a"]));
    // Three keys in one write, as fast typing or a key held down sends them.
    run.press("\x1b[B\x1b[B\r");
    run.until("merge request page", |r| r.shows(&["Repositories / a / api / !263", "Pipeline", "Commits"]));
    run.press("\x1b");
    run.until("back from the page", |r| r.shows(&["OPENED BY"]) && !r.shows(&["/ !263"]));
    run.press("I");
    run.until("work screen", |r| top_bar(r) && r.shows(&["WORK", "work items"]));
    run.resize(160, 40);
    run.until("work screen at 160x40", |r| top_bar(r) && r.shows(&["WORK", "work items"]));
    run.press("R");
    run.until("repositories at 160x40", |r| top_bar(r) && r.shows(&["REPOSITORIES", "!264 Tidy the build"]));
    run.press("x");
    run.until("actions menu at 160x40", |r| r.shows(&["refresh now"]) && r.menu_is_clear());
    run.press("\x1b");
    run.until("menu closed at 160x40", |r| !r.shows(&["refresh now"]));
    run.press("\r");
    run.until("merge request page at 160x40", |r| r.shows(&["Repositories / a / api / !263", "Pipeline", "Commits"]));
    run.press("\x1b");
    run.until("back on the list at 160x40", |r| r.shows(&["OPENED BY"]) && !r.shows(&["/ !263"]));
    run.press(",");
    run.until("settings at 160x40", |r| r.shows(&["SETTINGS", "GitLab", "Jira"]));
    run.press("R");
    run.until("repositories again", |r| r.shows(&["REPOSITORIES", "!264 Tidy the build"]));
}

/// The history overlay (`H`) over the repositories list: its events with
/// one selected, and esc back.
fn history(run: &mut Run) {
    run.press("R");
    run.until("repositories before history", |r| r.shows(&["REPOSITORIES", "OPENED BY"]));
    run.press("H");
    run.until("history", |r| r.shows(&["History · read only", "events · page 1", "OBSERVED UTC"]));
    run.press("\x1b");
    run.until("history closed", |r| r.shows(&["REPOSITORIES", "OPENED BY"]) && !r.shows(&["events · page 1"]));
}

/// After `q`: radar ended well and left the terminal as it found it.
fn restored(run: &mut Run) {
    restored_within(run, PATIENCE)
}

fn restored_within(run: &mut Run, patience: Duration) {
    let status = run.quit_within(patience);
    assert!(status.success(), "{}: radar exited {status:?}", run.name);
    terminal_as_found(run);
}

/// The alternate screen is left, mouse reporting off, the cursor shown, raw mode off.
fn terminal_as_found(run: &mut Run) {
    let s = &run.screen;
    assert!(!s.alt, "{}: the alternate screen is left", run.name);
    assert!(s.mouse.is_empty(), "{}: mouse reporting is off: {:?}", run.name, s.mouse);
    assert!(s.cursor_shown, "{}: the cursor is shown", run.name);
    let stty = run.stty();
    for flag in ["icanon", "echo", "isig"] {
        let off = format!("-{flag}");
        assert!(stty.split_whitespace().any(|w| w == flag) && !stty.split_whitespace().any(|w| w == off), "{}: raw mode is off ({flag}):\n{stty}", run.name);
    }
    std::fs::write(run.out.join(format!("{}-stty.txt", run.name)), &stty).unwrap();
}

/// Ink draws the journey; its process, ended by the pid the supervisor
/// logged, is started again and draws where radar was; `q` restores the
/// terminal.
#[test]
#[ignore = "needs node and the Ink bundle (cd ink && npm run bundle); run with --run-ignored all"]
fn ink_draws_the_journey_restarts_and_restores_the_terminal() {
    let mut run = Run::start("ink", 80, 24);
    run.until("the renderer started", |r| r.renderer_pids().len() == 1);
    journey(&mut run);

    let first = run.renderer_pids()[0];
    let entered = run.screen.alt_entered;
    let killed = std::process::Command::new("kill").args(["-KILL", &first.to_string()]).status().unwrap();
    assert!(killed.success(), "kill {first}");
    run.until("the renderer restarted", |r| {
        let pids = r.renderer_pids();
        pids.len() == 2 && pids[1] != first && r.screen.alt_entered > entered && r.shows(&["REPOSITORIES", "!264 Tidy the build"])
    });
    assert!(run.log().contains("Ink renderer stopped"), "{}", run.log());
    run.press("I");
    run.until("work after the restart", |r| r.shows(&["WORK", "work items"]));
    history(&mut run);
    restored(&mut run);
    println!("{}: renderer pids {:?}; captures in {}", run.name, run.renderer_pids(), run.out.display());
}

/// No Ink to start (no radar-js, no bundle): radar says why in two lines, exits 1,
/// starts no renderer and leaves the terminal as it found it. Needs no node.
#[test]
fn with_no_ink_radar_says_why_exits_1_and_leaves_the_terminal_as_found() {
    let mut run = Run::start_without_ink("no-ink", 80, 24);
    let deadline = Instant::now() + PATIENCE;
    let status = loop {
        if let Some(status) = run.child.try_wait().unwrap() {
            while let Ok(chunk) = run.rx.recv_timeout(Duration::from_millis(150)) {
                run.take(chunk);
            }
            break status;
        }
        assert!(Instant::now() < deadline, "{}: radar did not end:\n{}", run.name, run.screen.text());
        if let Ok(chunk) = run.rx.recv_timeout(Duration::from_millis(100)) {
            run.take(chunk);
        }
    };
    run.save("exit");
    let text = run.screen.text();
    assert_eq!(status.exit_code(), 1, "{}: exit code; the screen is:\n{text}", run.name);
    assert!(text.contains("radar: the renderer did not start: "), "{}: the reason line:\n{text}", run.name);
    assert!(text.contains("run scripts/install to rebuild it"), "{}: the fix line:\n{text}", run.name);
    assert!(run.renderer_pids().is_empty(), "no Ink was started: {}", run.log());
    terminal_as_found(&mut run);
}

/// One live run, read only, through Ink at 160x40: a copy of the
/// operator's config (`RADAR_PTY_CONFIG`), repositories and Work as they are
/// now. Nothing is asked, so nothing is answered; RADAR_MODE=read-only and
/// RADAR_NO_WRITES=1 refuse any write.
#[test]
#[ignore = "the operator's config, read live: RADAR_PTY_CONFIG=<config.toml>, run with --run-ignored all"]
fn live_read_only_through_ink_at_160x40() {
    let Some(config) = std::env::var_os("RADAR_PTY_CONFIG") else {
        eprintln!("RADAR_PTY_CONFIG is not set: no live run");
        return;
    };
    // RADAR_PTY_SIZE=WxH starts at another size than 160x40.
    let size = std::env::var("RADAR_PTY_SIZE").unwrap_or_else(|_| "160x40".into());
    let (w, h) = size.split_once('x').map(|(w, h)| (w.parse().unwrap(), h.parse().unwrap())).expect("WxH");
    let mut run = Run::live("ink", w, h, Path::new(&config));
    run.until("the renderer started", |r| r.renderer_pids().len() == 1);
    // RADAR_PTY_SCRIPT: steps `keys=>words` separated by ` | `, each waiting
    // until the screen shows every `&`-separated word (and none written `!word`) (live data differs
    // per user, so a walk past the first screens is given, not built in).
    let script = std::env::var("RADAR_PTY_SCRIPT")
        .unwrap_or_else(|_| "R=>REPOSITORIES&OPENED BY | I=>WORK&work items".into());
    // RADAR_PTY_SURVEY=1: a step that does not show its words is recorded as
    // MISSED and the walk goes on. A step `resize:WxH` resizes the terminal;
    // `kill` ends the Ink renderer by its logged pid.
    run.survey = std::env::var_os("RADAR_PTY_SURVEY").is_some();
    for (i, step) in script.split(" | ").enumerate() {
        let (spec, want) = step.split_once("=>").unwrap_or((step, ""));
        if let Some(size) = spec.strip_prefix("resize:") {
            let (w, h) = size.split_once('x').expect("resize:WxH");
            run.resize(w.parse().unwrap(), h.parse().unwrap());
        } else if spec == "kill"
            && let Some(pid) = run.renderer_pids().last()
        {
            std::process::Command::new("kill").args(["-KILL", &pid.to_string()]).status().unwrap();
        }
        let words: Vec<String> = want.split('&').filter(|w| !w.is_empty()).map(str::to_string).collect();
        if !spec.starts_with("resize:") && spec != "kill" {
            run.press(&keys(spec));
        }
        let what = format!("step {} {}", i + 1, spec.replace(['<', '>', '/', ':'], ""));
        // A word written `!word` must be gone.
        let (gone, shown): (Vec<&String>, Vec<&String>) = words.iter().partition(|w| w.starts_with('!'));
        run.until(&what, |r| {
            r.shows(&shown.iter().map(|w| w.as_str()).collect::<Vec<_>>())
                && !gone.iter().any(|w| r.screen.text().contains(&w[1..]))
        });
    }
    if !run.missed.is_empty() {
        println!("{}: MISSED {:?}", run.name, run.missed);
    }
    // Live reads may be in flight when q is pressed.
    restored_within(&mut run, Duration::from_secs(90));
    println!("{}: quit took {:?}; captures in {}", run.name, run.quit_took, run.out.display());
}
