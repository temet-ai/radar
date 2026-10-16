//! The machine radar runs on: how to open a link and where the clipboard is.
//!
//! People run radar on their own desktop (macOS, Linux on Wayland or X11,
//! Windows, WSL), over SSH, inside tmux, or with no display at all. Over SSH
//! the browser and clipboard that matter are on the machine they sit at, out
//! of reach of any command run here, so radar asks their terminal instead: OSC
//! 52 puts text on the clipboard of the machine the terminal runs on. The
//! same is the last resort everywhere else.
//!
//! [`Env::detect`] reads the situation; [`open_ways`] and [`copy_ways`] say
//! what to try, in order. Both are plain functions of an [`Env`], so every
//! situation is tested without being in it.
//!
//! The same goes for opening a folder: a terminal there, or an editor on it
//! ([`folder_ways`]). Inside tmux that is a new tmux window, which works over
//! SSH too; on a desktop it is a window of its own; where neither can reach
//! the user, the command line that would do it goes to their clipboard.
//!
//! With `RADAR_SANDBOX` set (`.cargo/config.toml` sets it for every cargo
//! test), no program is started from here: no browser, clipboard,
//! notification, terminal or editor. The tests' hosts and binaries then never
//! reach the desktop of the machine that runs them.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine;

/// The folder radar's state goes under, on every platform: `$XDG_STATE_HOME`
/// when set, else `~/.local/state` (the action journal and the cache database
/// have always lived there; macOS's `dirs` has no state folder).
pub fn state_home() -> Option<std::path::PathBuf> {
    absolute_var("XDG_STATE_HOME").or_else(|| dirs::home_dir().map(|h| h.join(".local").join("state")))
}

/// The folder radar's caches go under: `$XDG_CACHE_HOME` when set, else the
/// platform's (`~/.cache`, `~/Library/Caches` on macOS).
pub fn cache_home() -> Option<std::path::PathBuf> {
    absolute_var("XDG_CACHE_HOME").or_else(dirs::cache_dir)
}

fn absolute_var(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os(name).map(std::path::PathBuf::from).filter(|p| p.is_absolute())
}

/// Whether `RADAR_SANDBOX` is set: radar starts no desktop program and no
/// provider sign-in. Every cargo test runs this way.
pub fn sandboxed() -> bool {
    std::env::var_os("RADAR_SANDBOX").is_some_and(|v| !v.is_empty())
}

/// For a test elsewhere in the crate of how a program is run: harmless ones
/// (`true`, `cat`) run on this thread despite the sandbox.
#[cfg(test)]
pub(crate) fn real_programs_on_this_thread() {
    tests::REAL_PROGRAMS.with(|c| c.set(true));
}

/// Where radar is running, as far as opening links and copying go.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    /// Over SSH: the user's browser and clipboard are on another machine.
    pub ssh: bool,
    pub macos: bool,
    pub windows: bool,
    /// Linux under Windows: Windows opens links and holds the clipboard.
    pub wsl: bool,
    pub wayland: bool,
    pub x11: bool,
    /// Inside tmux: escape sequences to the terminal go through it.
    pub tmux: bool,
    /// `$BROWSER`, if the user set one.
    pub browser: Option<String>,
    /// `$TERMINAL`, if the user set one: their terminal program.
    pub terminal: Option<String>,
    /// `$VISUAL`, else `$EDITOR`: the editor they chose for the shell.
    pub editor: Option<String>,
}

impl Env {
    pub fn detect() -> Self {
        let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
        let wsl = set("WSL_DISTRO_NAME")
            || std::fs::read_to_string("/proc/sys/kernel/osrelease")
                .is_ok_and(|r| r.to_lowercase().contains("microsoft"));
        Self {
            ssh: set("SSH_CONNECTION") || set("SSH_CLIENT") || set("SSH_TTY"),
            macos: cfg!(target_os = "macos"),
            windows: cfg!(target_os = "windows"),
            wsl,
            wayland: set("WAYLAND_DISPLAY"),
            x11: set("DISPLAY"),
            tmux: set("TMUX"),
            browser: std::env::var("BROWSER")
                .ok()
                .filter(|b| !b.trim().is_empty()),
            terminal: std::env::var("TERMINAL")
                .ok()
                .filter(|b| !b.trim().is_empty()),
            editor: ["VISUAL", "EDITOR"]
                .iter()
                .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty())),
        }
    }

    /// Whether a program started here can show a window to the user.
    fn has_desktop(&self) -> bool {
        !self.ssh && (self.macos || self.windows || self.wsl || self.wayland || self.x11)
    }
}

/// One way to do it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Way {
    /// Run a program: for opening, with the link among its arguments; for
    /// copying, with the text on its stdin.
    Run(String, Vec<String>),
    /// Ask the terminal to put the text on its clipboard (OSC 52).
    Terminal,
}

fn run(cmd: &str, args: &[&str]) -> Way {
    Way::Run(cmd.into(), args.iter().map(|a| (*a).to_string()).collect())
}

/// How to open `url`, best first. Where no browser can be reached, the link
/// goes to the clipboard so the user can paste it into one.
pub fn open_ways(env: &Env, url: &str) -> Vec<Way> {
    let mut out = Vec::new();
    if env.has_desktop() {
        // `$BROWSER` is `:`-separated, as `man xdg-open` reads it.
        for b in env.browser.iter().flat_map(|b| b.split(':')) {
            let mut words = b.split_whitespace();
            if let Some(cmd) = words.next() {
                let mut args: Vec<&str> = words.collect();
                args.push(url);
                out.push(run(cmd, &args));
            }
        }
        if env.macos {
            out.push(run("open", &[url]));
        } else if env.windows {
            out.push(run("cmd", &["/C", "start", "", url]));
        } else if env.wsl {
            out.push(run("wslview", &[url]));
            out.push(run("explorer.exe", &[url]));
        } else {
            out.push(run("xdg-open", &[url]));
            out.push(run("gio", &["open", url]));
        }
    }
    out.push(Way::Terminal);
    out
}

/// How to open an app's own link (`app://…`): only the desktop's opener,
/// which hands it to the program kept for its scheme. Never a browser and
/// never the clipboard: with no desktop here (or over SSH) there is no way,
/// and the caller opens the web link instead.
pub fn app_ways(env: &Env, link: &str) -> Vec<Way> {
    if !env.has_desktop() {
        return vec![];
    }
    if env.macos {
        vec![run("open", &[link])]
    } else if env.windows {
        vec![run("cmd", &["/C", "start", "", link])]
    } else if env.wsl {
        vec![run("wslview", &[link])]
    } else {
        vec![run("xdg-open", &[link]), run("gio", &["open", link])]
    }
}

/// How to show a desktop notification, best first: the notification centre
/// on macOS (`osascript`), `notify-send` on a Linux desktop. None over SSH
/// or with no desktop: the user is not at this machine's screen.
pub fn notify_ways(env: &Env, title: &str, body: &str) -> Vec<Way> {
    if !env.has_desktop() {
        return vec![];
    }
    if env.macos {
        // AppleScript strings: a backslash and a quote are escaped.
        let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
        let script = format!("display notification {} with title {}", quote(body), quote(title));
        vec![run("osascript", &["-e", &script])]
    } else if env.windows || env.wsl {
        vec![]
    } else {
        vec![run("notify-send", &["--app-name=radar", "--", title, body])]
    }
}

/// How to put text on the user's clipboard, best first.
pub fn copy_ways(env: &Env) -> Vec<Way> {
    let mut out = Vec::new();
    if !env.ssh {
        if env.macos {
            out.push(run("pbcopy", &[]));
        } else if env.windows {
            out.push(run("clip", &[]));
        } else if env.wsl {
            out.push(run("clip.exe", &[]));
        }
        if env.wayland {
            out.push(run("wl-copy", &[]));
        }
        if env.x11 {
            out.push(run("xclip", &["-selection", "clipboard"]));
            out.push(run("xsel", &["--clipboard", "--input"]));
        }
    }
    out.push(Way::Terminal);
    out
}

/// The bytes that ask the terminal to put `text` on its clipboard. Inside
/// tmux they go twice: plain, which tmux takes with `set-clipboard on`, and
/// wrapped, which it passes on with `allow-passthrough on`.
pub fn osc52(text: &str, tmux: bool) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(text);
    let plain = format!("\x1b]52;c;{b64}\x07");
    if tmux {
        let wrapped = format!("\x1bPtmux;{}\x1b\\", plain.replace('\x1b', "\x1b\x1b"));
        format!("{plain}{wrapped}")
    } else {
        plain
    }
}

/// What happened, for the status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Done {
    /// A program did it: `way` is its command (`wl-copy`, `xdg-open`).
    Ran { way: String },
    /// No program could (`why` says what the last one said, if any was
    /// tried): the text must go to the terminal (OSC 52), which only the
    /// thread drawing the screen may write to.
    UseTerminal { why: String },
}

/// Run `ways` until one works. Programs are tried in order; the first
/// [`Way::Terminal`] stops the search. An opener still running after a few
/// seconds has handed the link to a browser it started and is waiting on it:
/// that counts as done.
pub fn try_ways(ways: &[Way], stdin: Option<&str>) -> Result<Done, String> {
    let mut why = String::new();
    for way in ways {
        let Way::Run(cmd, args) = way else {
            return Ok(Done::UseTerminal { why });
        };
        match try_run(cmd, args, stdin, None, Duration::from_secs(3)) {
            Ok(()) => return Ok(Done::Ran { way: cmd.clone() }),
            Err(e) => why = e,
        }
    }
    Err(if why.is_empty() {
        "nothing to try".into()
    } else {
        why
    })
}

/// Run `cmd` and see how it goes: `Ok` once it exits well, or once it has
/// run for `patience` (it started something long-lived, a browser or an
/// editor, and that counts as done); `Err` with its last words otherwise.
fn try_run(
    cmd: &str,
    args: &[String],
    stdin: Option<&str>,
    dir: Option<&Path>,
    patience: Duration,
) -> Result<(), String> {
    #[cfg(test)]
    let sandboxed = || sandboxed() && !tests::REAL_PROGRAMS.with(std::cell::Cell::get);
    if sandboxed() {
        return Err(format!("{cmd}: not started (RADAR_SANDBOX)"));
    }
    let mut c = Command::new(cmd);
    c.args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(dir) = dir {
        c.current_dir(dir);
    }
    // Its own process group: a browser it starts outlives radar and never
    // reads radar's terminal.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    let mut child = c.spawn().map_err(|e| format!("{cmd}: {e}"))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(text.as_bytes())
            .map_err(|e| format!("{cmd}: {e}"))?;
    }
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(s)) if s.success() => return Ok(()),
            Ok(Some(s)) => {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                let last = err.lines().rev().find(|l| !l.trim().is_empty());
                return Err(format!(
                    "{cmd}: {}",
                    last.map_or_else(|| s.to_string(), str::to_string)
                ));
            }
            Ok(None) if started.elapsed() > patience => {
                // It lives on: keep reading what it says, so a program that
                // writes to stderr is never killed by a closed pipe.
                if let Some(mut e) = child.stderr.take() {
                    std::thread::spawn(move || {
                        let _ = std::io::copy(&mut e, &mut std::io::sink());
                    });
                }
                return Ok(());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("{cmd}: {e}")),
        }
    }
}

// --- a folder: a terminal there, an editor on it ------------------------------

/// What to open in a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InFolder<'a> {
    /// A shell, in a terminal.
    Terminal,
    /// An editor, by its command line: `code`, `nvim`, `open -a "IntelliJ IDEA"`.
    Editor(&'a str),
}

/// One way to do it: a program started in the folder, and what the status
/// line calls it when it worked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    pub cmd: String,
    pub args: Vec<String>,
    pub what: String,
}

/// How to open something in a folder: the ways to try, best first, and
/// the command line that does the same by hand, for the user's clipboard
/// when no way works or none can reach them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderWays {
    pub starts: Vec<Start>,
    /// Typed into a shell, it gets there: `cd '/x/api' && nvim .`.
    pub by_hand: String,
    /// Why nothing can be started from here, when `starts` is empty.
    pub why_not: Option<String>,
}

/// What the folder-launch helper actually observed. This is dispatch evidence,
/// never proof that an application opened, a file changed, or the clipboard did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderLaunch {
    /// An opener exited successfully or stayed running for its observation window.
    Dispatched(String),
    /// The environment has no way to attempt this launch.
    Unavailable { why: String, by_hand: String },
    /// All attempts returned errors. A process may already have had side effects.
    Unconfirmed { why: String, by_hand: String },
}

/// Try the planned launch, preserving the distinction between no available route
/// and an attempted dispatch whose effect could not be confirmed. Display strings
/// and the by-hand command are for the UI only, never journal metadata.
pub fn start_folder(ways: FolderWays, dir: &Path) -> FolderLaunch {
    start_folder_with(ways, |starts| start_in(starts, dir))
}

fn start_folder_with(
    ways: FolderWays,
    start: impl FnOnce(&[Start]) -> Result<String, String>,
) -> FolderLaunch {
    if let Some(why) = ways.why_not {
        return FolderLaunch::Unavailable {
            why,
            by_hand: ways.by_hand,
        };
    }
    match start(&ways.starts) {
        Ok(started) => FolderLaunch::Dispatched(started),
        Err(why) => FolderLaunch::Unconfirmed {
            why,
            by_hand: ways.by_hand,
        },
    }
}

/// Editors radar knows how to find: command, name, and the macOS app that
/// runs it when the command is not on `PATH`.
pub const EDITORS: [(&str, &str, Option<&str>); 18] = [
    ("code", "VS Code", Some("Visual Studio Code")),
    ("codium", "VSCodium", Some("VSCodium")),
    ("cursor", "Cursor", Some("Cursor")),
    ("zed", "Zed", Some("Zed")),
    ("subl", "Sublime Text", Some("Sublime Text")),
    ("idea", "IntelliJ IDEA", Some("IntelliJ IDEA")),
    ("pycharm", "PyCharm", Some("PyCharm")),
    ("webstorm", "WebStorm", Some("WebStorm")),
    ("goland", "GoLand", Some("GoLand")),
    ("rustrover", "RustRover", Some("RustRover")),
    ("clion", "CLion", Some("CLion")),
    ("nvim", "Neovim", None),
    ("vim", "Vim", None),
    ("hx", "Helix", None),
    ("emacs", "Emacs", None),
    ("nano", "nano", None),
    ("micro", "micro", None),
    ("kak", "Kakoune", None),
];

/// Programs that run inside the terminal they are started from.
const TERMINAL_EDITORS: [&str; 12] = [
    "vi", "vim", "nvim", "hx", "helix", "nano", "micro", "kak", "ne", "joe", "mg", "jed",
];

/// A command line as words, honouring '' and "" quotes and `\` escapes.
/// The folders a typed path could go on to, for offering under a field:
/// `~/co` gives `~/code/`, `~/config/`; `/work/` gives what is in it. Kept in
/// the form typed (`~` stays), each ending in `/`, sorted, at most `most`.
/// Hidden folders only when what is typed after the last `/` starts with
/// `.`. Nothing until there is a `/` (or `~`).
pub fn folders_like(text: &str, most: usize) -> Vec<String> {
    let text = text.trim();
    let text = if text == "~" { "~/" } else { text };
    let Some(cut) = text.rfind('/') else { return vec![] };
    let (parent, stem) = (&text[..=cut], &text[cut + 1..]);
    let dir = match parent.strip_prefix("~/") {
        Some(rest) => match dirs::home_dir() {
            Some(h) => h.join(rest),
            None => return vec![],
        },
        None if parent.starts_with('/') => Path::new(parent).to_path_buf(),
        None => return vec![],
    };
    let Ok(entries) = std::fs::read_dir(&dir) else { return vec![] };
    let stem_l = stem.to_lowercase();
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.to_lowercase().starts_with(&stem_l))
        .filter(|n| !n.starts_with('.') || stem.starts_with('.'))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.into_iter().take(most).map(|n| format!("{parent}{n}/")).collect()
}

pub fn split_command(line: &str) -> Vec<String> {
    let (mut words, mut word, mut any) = (Vec::new(), String::new(), false);
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') | (None, '\\') => {
                if let Some(n) = chars.next() {
                    word.push(n);
                }
            }
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                any = false;
            }
            (None, c) => word.push(c),
        }
    }
    if any || !word.is_empty() {
        words.push(word);
    }
    words
}

/// Whether an editor's command line runs it inside a terminal (`nvim`,
/// `emacs -nw`) rather than in a window of its own (`code`, `idea`).
pub fn runs_in_terminal(editor: &str) -> bool {
    let words = split_command(editor);
    let Some(first) = words.first() else {
        return false;
    };
    let prog = Path::new(first)
        .file_name()
        .map_or(first.as_str(), |n| n.to_str().unwrap_or(first))
        .trim_end_matches(".exe");
    TERMINAL_EDITORS.contains(&prog)
        || (matches!(prog, "emacs" | "emacsclient")
            && words
                .iter()
                .any(|w| matches!(w.as_str(), "-nw" | "-t" | "--tty")))
}

/// A path as a shell word: bare when it is plain, quoted when not.
fn shell_word(s: &str, windows: bool) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-~:\\+@%,=".contains(c));
    if plain {
        s.to_string()
    } else if windows {
        format!("\"{s}\"")
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

fn start(cmd: &str, args: Vec<String>, what: impl Into<String>) -> Start {
    Start {
        cmd: cmd.into(),
        args,
        what: what.into(),
    }
}

/// The words of a terminal command line from the user's settings: `{dir}`
/// becomes the folder; a command to run goes where `{cmd}` is, or after
/// `-e` when there is no `{cmd}`.
fn template(line: &str, dir: &str, run: &[String]) -> Option<(String, Vec<String>)> {
    let mut words = Vec::new();
    let mut placed = false;
    for w in split_command(line) {
        if w == "{cmd}" {
            words.extend(run.iter().cloned());
            placed = true;
        } else {
            words.push(w.replace("{dir}", dir).replace("{cmd}", &run.join(" ")));
        }
    }
    if !placed && !run.is_empty() {
        words.push("-e".into());
        words.extend(run.iter().cloned());
    }
    let mut words = words.into_iter();
    Some((words.next()?, words.collect()))
}

/// How to open a terminal in `dir`, or an editor on it, here; best first.
/// `terminal` is the user's own terminal command line (empty: find one);
/// `name` names a new tmux window.
pub fn folder_ways(
    env: &Env,
    dir: &str,
    what: InFolder<'_>,
    terminal: &str,
    name: &str,
) -> FolderWays {
    let win = env.windows;
    let cd = if win {
        format!("cd {}", shell_word(dir, true))
    } else {
        format!("cd {}", shell_word(dir, false))
    };
    // What runs in a terminal: nothing (a shell), or the editor on `.`.
    let (in_terminal, run): (bool, Vec<String>) = match what {
        InFolder::Terminal => (true, vec![]),
        InFolder::Editor(e) if runs_in_terminal(e) => {
            let mut w = split_command(e);
            w.push(".".into());
            (true, w)
        }
        InFolder::Editor(e) => (false, split_command(e)),
    };
    let by_hand = if in_terminal && !run.is_empty() {
        format!("{cd} && {}", run.join(" "))
    } else {
        cd.clone()
    };
    let label = |thing: &str| match what {
        InFolder::Terminal => format!("a terminal in {thing}"),
        InFolder::Editor(_) => format!("{} in {thing}", run.first().map_or("it", |s| s.as_str())),
    };
    let mut starts = Vec::new();
    if in_terminal {
        // Inside tmux, a window of the same session: where the user already
        // is, and it works over SSH as well as at the desk.
        if env.tmux {
            let mut args = vec!["new-window".into(), "-c".into(), dir.into()];
            if !name.is_empty() {
                args.extend(["-n".into(), name.into()]);
            }
            args.extend(run.iter().cloned());
            starts.push(start("tmux", args, label("a new tmux window")));
        }
        if env.has_desktop() {
            if !terminal.trim().is_empty()
                && let Some((cmd, args)) = template(terminal, dir, &run)
            {
                let what = label(&format!("a {cmd} window"));
                starts.push(start(&cmd, args, what));
            }
            if env.macos {
                if run.is_empty() {
                    starts.push(start(
                        "open",
                        vec!["-a".into(), "Terminal".into(), dir.into()],
                        label("a Terminal window"),
                    ));
                } else {
                    let line = by_hand.replace('\\', "\\\\").replace('"', "\\\"");
                    let script = format!(
                        "tell application \"Terminal\"\nactivate\ndo script \"{line}\"\nend tell"
                    );
                    starts.push(start(
                        "osascript",
                        vec!["-e".into(), script],
                        label("a Terminal window"),
                    ));
                }
            } else if env.windows {
                let mut args = vec!["-d".into(), dir.into()];
                args.extend(run.iter().cloned());
                starts.push(start("wt", args, label("Windows Terminal")));
                let mut args: Vec<String> = ["/C", "start", "", "/D", dir, "cmd", "/K"]
                    .map(String::from)
                    .into();
                args.extend(run.iter().cloned());
                starts.push(start("cmd", args, label("a console window")));
            } else if env.wsl {
                let mut args: Vec<String> = ["wsl.exe", "--cd", dir].map(String::from).into();
                if !run.is_empty() {
                    args.push("--exec".into());
                    args.extend(run.iter().cloned());
                }
                starts.push(start("wt.exe", args, label("Windows Terminal")));
            } else {
                if let Some(t) = &env.terminal
                    && let Some((cmd, args)) = template(t, dir, &run)
                {
                    let what = label(&format!("a {cmd} window"));
                    starts.push(start(&cmd, args, what));
                }
                starts.push(start(
                    "xdg-terminal-exec",
                    run.clone(),
                    label("a terminal window"),
                ));
                let mut args = vec![];
                if !run.is_empty() {
                    args.push("-e".into());
                    args.extend(run.iter().cloned());
                }
                starts.push(start(
                    "x-terminal-emulator",
                    args,
                    label("a terminal window"),
                ));
            }
        }
    } else if env.has_desktop() {
        // An editor with a window of its own, on the folder.
        let mut words = run.clone();
        words.push(dir.into());
        let what = format!("{} on {dir}", run.first().map_or("it", |s| s.as_str()));
        if env.windows {
            let mut args = vec!["/C".to_string()];
            args.extend(words);
            starts.push(start("cmd", args, what));
        } else if let Some((cmd, args)) = words.split_first() {
            starts.push(start(cmd, args.to_vec(), what));
        }
    }
    let why_not = starts.is_empty().then(|| {
        if !in_terminal && (env.ssh || env.tmux) && !env.has_desktop() {
            "a windowed editor cannot open over ssh".to_string()
        } else if env.ssh {
            "over ssh radar cannot open a window on your machine".to_string()
        } else {
            "there is no desktop or tmux here to open it in".to_string()
        }
    });
    FolderWays {
        starts,
        by_hand,
        why_not,
    }
}

/// Try `starts` in order, each started in `dir`, until one works. Returns
/// what worked, for the status line, or why the last one failed.
pub fn start_in(starts: &[Start], dir: &Path) -> Result<String, String> {
    let mut why = String::from("nothing to try");
    for s in starts {
        match try_run(
            &s.cmd,
            &s.args,
            None,
            Some(dir),
            Duration::from_millis(1500),
        ) {
            Ok(()) => return Ok(s.what.clone()),
            Err(e) => why = e,
        }
    }
    Err(why)
}

/// Whether `cmd` (a program name, or a path to one) can be run here.
pub fn on_path(cmd: &str) -> bool {
    if cmd.is_empty() {
        return false;
    }
    if cmd.contains('/') || cmd.contains('\\') {
        return Path::new(cmd).is_file();
    }
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .map(str::to_string)
            .chain(std::iter::once(String::new()))
            .collect()
    } else {
        vec![String::new()]
    };
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths)
            .any(|d| exts.iter().any(|e| d.join(format!("{cmd}{e}")).is_file()))
    })
}

/// The editors installed here, as (command line, name): the user's
/// `$VISUAL`/`$EDITOR` first, then the known ones on `PATH`, then (macOS)
/// known apps that are installed without a command.
pub fn installed_editors(env: &Env) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(e) = &env.editor
        && split_command(e).first().is_some_and(|c| on_path(c))
    {
        out.push((e.clone(), format!("your $EDITOR, {e}")));
    }
    for (cmd, name, app) in EDITORS {
        if out.iter().any(|(c, _)| c == cmd) {
            continue;
        }
        if on_path(cmd) {
            out.push((cmd.into(), name.into()));
        } else if env.macos
            && let Some(app) = app
            && Path::new(&format!("/Applications/{app}.app")).is_dir()
        {
            out.push((format!("open -a \"{app}\""), name.into()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// Set by a test of how programs are run: it runs harmless ones
        /// (`true`, `sh`) on this thread despite the sandbox.
        pub(super) static REAL_PROGRAMS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    fn cmds(ways: &[Way]) -> Vec<String> {
        ways.iter()
            .map(|w| match w {
                Way::Run(c, _) => c.clone(),
                Way::Terminal => "terminal".into(),
            })
            .collect()
    }

    /// Every cargo test runs sandboxed, so no test opens a browser, the
    /// clipboard or a window on the machine running it.
    #[test]
    fn tests_are_sandboxed_and_a_sandbox_starts_nothing() {
        assert!(sandboxed(), "RADAR_SANDBOX comes from .cargo/config.toml");
        let mac = Env { macos: true, ..Env::default() };
        let opened = try_ways(&open_ways(&mac, "https://example.com/never-opened"), None);
        assert_eq!(opened, Ok(Done::UseTerminal { why: "open: not started (RADAR_SANDBOX)".into() }));
        let notified = try_ways(&notify_ways(&mac, "t", "b"), None);
        assert_eq!(notified, Err("osascript: not started (RADAR_SANDBOX)".into()));
    }

    #[test]
    fn a_desktop_notification_is_notify_send_on_linux_the_notification_centre_on_macos_and_none_over_ssh() {
        let linux = Env { wayland: true, ..Env::default() };
        assert_eq!(
            notify_ways(&linux, "radar · review", "-n done"),
            vec![Way::Run("notify-send".into(), ["--app-name=radar", "--", "radar · review", "-n done"].map(String::from).into())]
        );
        let mac = Env { macos: true, ..Env::default() };
        assert_eq!(
            notify_ways(&mac, "radar · \"x\"", "needs a\\b"),
            vec![Way::Run(
                "osascript".into(),
                vec!["-e".into(), r#"display notification "needs a\\b" with title "radar · \"x\"""#.into()]
            )]
        );
        assert!(notify_ways(&Env { ssh: true, wayland: true, ..Env::default() }, "t", "b").is_empty());
        assert!(notify_ways(&Env::default(), "t", "b").is_empty(), "no desktop");
    }

    #[test]
    fn over_ssh_links_and_copies_go_through_the_terminal_whatever_else_is_set() {
        // A desktop session is often still running on the far machine.
        let env = Env {
            ssh: true,
            wayland: true,
            x11: true,
            browser: Some("firefox".into()),
            ..Env::default()
        };
        assert_eq!(cmds(&open_ways(&env, "u")), ["terminal"]);
        assert_eq!(cmds(&copy_ways(&env)), ["terminal"]);
    }

    #[test]
    fn an_app_link_goes_only_to_the_desktops_opener_never_a_browser_or_the_clipboard() {
        let link = "app://open?id=1";
        let linux = Env {
            wayland: true,
            browser: Some("firefox".into()),
            ..Env::default()
        };
        assert_eq!(cmds(&app_ways(&linux, link)), ["xdg-open", "gio"]);
        let mac = Env {
            macos: true,
            ..Env::default()
        };
        assert_eq!(app_ways(&mac, link), [run("open", &[link])]);
        let windows = Env {
            windows: true,
            ..Env::default()
        };
        assert_eq!(cmds(&app_ways(&windows, link)), ["cmd"]);
        let wsl = Env {
            wsl: true,
            ..Env::default()
        };
        assert_eq!(cmds(&app_ways(&wsl, link)), ["wslview"]);
        // Over SSH, or with no display, there is no way: the web link opens.
        let ssh = Env {
            ssh: true,
            wayland: true,
            ..Env::default()
        };
        assert!(app_ways(&ssh, link).is_empty());
        assert!(app_ways(&Env::default(), link).is_empty());
    }

    #[test]
    fn on_each_desktop_its_own_tools_come_first_and_the_terminal_last() {
        let mac = Env {
            macos: true,
            ..Env::default()
        };
        assert_eq!(cmds(&open_ways(&mac, "u")), ["open", "terminal"]);
        assert_eq!(cmds(&copy_ways(&mac)), ["pbcopy", "terminal"]);

        let wayland = Env {
            wayland: true,
            ..Env::default()
        };
        assert_eq!(
            cmds(&open_ways(&wayland, "u")),
            ["xdg-open", "gio", "terminal"]
        );
        assert_eq!(cmds(&copy_ways(&wayland)), ["wl-copy", "terminal"]);

        let x11 = Env {
            x11: true,
            ..Env::default()
        };
        assert_eq!(cmds(&copy_ways(&x11)), ["xclip", "xsel", "terminal"]);

        let wsl = Env {
            wsl: true,
            ..Env::default()
        };
        assert_eq!(
            cmds(&open_ways(&wsl, "u")),
            ["wslview", "explorer.exe", "terminal"]
        );
        assert_eq!(cmds(&copy_ways(&wsl)), ["clip.exe", "terminal"]);

        let windows = Env {
            windows: true,
            ..Env::default()
        };
        assert_eq!(
            open_ways(&windows, "u")[0],
            run("cmd", &["/C", "start", "", "u"])
        );
    }

    #[test]
    fn with_no_display_the_link_goes_to_the_clipboard() {
        assert_eq!(cmds(&open_ways(&Env::default(), "u")), ["terminal"]);
        assert_eq!(cmds(&copy_ways(&Env::default())), ["terminal"]);
    }

    #[test]
    fn browser_comes_first_and_keeps_its_own_arguments() {
        let env = Env {
            wayland: true,
            browser: Some("firefox --new-tab:chromium".into()),
            ..Env::default()
        };
        let ways = open_ways(&env, "https://x");
        assert_eq!(ways[0], run("firefox", &["--new-tab", "https://x"]));
        assert_eq!(ways[1], run("chromium", &["https://x"]));
        assert_eq!(cmds(&ways)[2..], ["xdg-open", "gio", "terminal"]);
    }

    #[test]
    fn osc52_is_base64_and_wrapped_for_tmux() {
        assert_eq!(osc52("hi", false), "\x1b]52;c;aGk=\x07");
        let t = osc52("hi", true);
        assert!(t.starts_with("\x1b]52;c;aGk=\x07\x1bPtmux;\x1b\x1b]52;c;aGk=\x07"));
        assert!(t.ends_with("\x1b\\"));
    }

    #[test]
    fn a_missing_program_falls_through_to_the_next_way() {
        REAL_PROGRAMS.with(|c| c.set(true));
        let ways = [run("radar-no-such-program", &[]), run("true", &[])];
        assert_eq!(try_ways(&ways, None), Ok(Done::Ran { way: "true".into() }));
        let ways = [run("radar-no-such-program", &[]), Way::Terminal];
        assert!(matches!(
            try_ways(&ways, Some("x")),
            Ok(Done::UseTerminal { why }) if why.starts_with("radar-no-such-program")
        ));
        let ways = [run("radar-no-such-program", &[]), run("radar-no-such-program-either", &[])];
        assert!(try_ways(&ways, None).is_err());
        // A program that fails is an error. Asked without patience: with the
        // three seconds `try_ways` gives, a `false` the loaded machine had not
        // yet let exit would count as an opener still running.
        assert!(try_run("false", &[], None, None, Duration::MAX).is_err());
    }

    // --- a folder ---------------------------------------------------------------

    fn starts(env: &Env, what: InFolder<'_>, terminal: &str) -> Vec<String> {
        folder_ways(env, "/w/api", what, terminal, "api")
            .starts
            .iter()
            .map(|s| format!("{} {}", s.cmd, s.args.join(" ")).trim().to_string())
            .collect()
    }

    #[test]
    fn folder_launch_preserves_unavailable_success_and_uncertain_observations() {
        let unavailable = folder_ways(&Env::default(), "/w/api", InFolder::Terminal, "", "api");
        let by_hand = unavailable.by_hand.clone();
        assert!(matches!(
            start_folder_with(unavailable, |_| panic!("unavailable routes must not execute")),
            FolderLaunch::Unavailable { by_hand: command, .. } if command == by_hand
        ));
        let ways = folder_ways(
            &Env {
                wayland: true,
                ..Env::default()
            },
            "/w/api",
            InFolder::Editor("SECRET_COMMAND"),
            "",
            "api",
        );
        assert_eq!(
            start_folder_with(ways.clone(), |starts| {
                assert_eq!(starts[0].cmd, "SECRET_COMMAND");
                Ok("SECRET_LABEL".into())
            }),
            FolderLaunch::Dispatched("SECRET_LABEL".into())
        );
        assert_eq!(
            start_folder_with(ways.clone(), |_| Err("SECRET_PROCESS_ERROR".into())),
            FolderLaunch::Unconfirmed {
                why: "SECRET_PROCESS_ERROR".into(),
                by_hand: ways.by_hand,
            }
        );
    }

    #[test]
    fn inside_tmux_a_terminal_or_terminal_editor_is_a_new_tmux_window_even_over_ssh() {
        let env = Env {
            tmux: true,
            ssh: true,
            ..Env::default()
        };
        assert_eq!(
            starts(&env, InFolder::Terminal, ""),
            ["tmux new-window -c /w/api -n api"]
        );
        assert_eq!(
            starts(&env, InFolder::Editor("nvim"), ""),
            ["tmux new-window -c /w/api -n api nvim ."]
        );
        // A windowed editor cannot reach the user over ssh.
        let w = folder_ways(&env, "/w/api", InFolder::Editor("code"), "", "api");
        assert!(w.starts.is_empty());
        assert_eq!(
            w.why_not.as_deref(),
            Some("a windowed editor cannot open over ssh")
        );
        assert_eq!(w.by_hand, "cd /w/api");
    }

    #[test]
    fn inside_tmux_at_the_desk_the_tmux_window_comes_first_and_a_windowed_editor_opens() {
        let env = Env {
            tmux: true,
            wayland: true,
            ..Env::default()
        };
        assert_eq!(
            starts(&env, InFolder::Terminal, "")[0],
            "tmux new-window -c /w/api -n api"
        );
        assert_eq!(starts(&env, InFolder::Editor("code"), ""), ["code /w/api"]);
    }

    #[test]
    fn over_ssh_without_tmux_nothing_starts_and_the_command_goes_to_the_clipboard() {
        let env = Env {
            ssh: true,
            x11: true,
            ..Env::default()
        };
        let w = folder_ways(&env, "/w/my api", InFolder::Editor("nvim"), "", "api");
        assert!(w.starts.is_empty());
        assert_eq!(w.by_hand, "cd '/w/my api' && nvim .");
        assert_eq!(
            w.why_not.as_deref(),
            Some("over ssh radar cannot open a window on your machine")
        );
    }

    #[test]
    fn with_no_display_and_no_tmux_nothing_starts() {
        let w = folder_ways(&Env::default(), "/w/api", InFolder::Terminal, "", "api");
        assert!(w.starts.is_empty());
        assert_eq!(w.by_hand, "cd /w/api");
        assert!(w.why_not.unwrap().contains("no desktop or tmux"));
    }

    #[test]
    fn on_a_linux_desktop_the_users_terminal_then_the_standard_launchers() {
        let env = Env {
            wayland: true,
            terminal: Some("alacritty".into()),
            ..Env::default()
        };
        assert_eq!(
            starts(&env, InFolder::Terminal, ""),
            ["alacritty", "xdg-terminal-exec", "x-terminal-emulator"]
        );
        assert_eq!(
            starts(&env, InFolder::Editor("hx"), ""),
            [
                "alacritty -e hx .",
                "xdg-terminal-exec hx .",
                "x-terminal-emulator -e hx ."
            ]
        );
        // The user's own setting comes first, with the folder and command placed.
        assert_eq!(
            starts(
                &env,
                InFolder::Editor("nvim"),
                "foot --working-directory={dir} {cmd}"
            )[0],
            "foot --working-directory=/w/api nvim ."
        );
        assert_eq!(
            starts(&env, InFolder::Editor("idea"), "foot")[0],
            "idea /w/api",
            "a windowed editor needs no terminal"
        );
    }

    #[test]
    fn on_macos_terminal_app_and_windowed_apps() {
        let env = Env {
            macos: true,
            ..Env::default()
        };
        assert_eq!(
            starts(&env, InFolder::Terminal, ""),
            ["open -a Terminal /w/api"]
        );
        let s = starts(&env, InFolder::Editor("nvim"), "");
        assert!(
            s[0].starts_with("osascript -e") && s[0].contains("do script \"cd /w/api && nvim .\""),
            "{s:?}"
        );
        assert_eq!(
            starts(&env, InFolder::Editor("open -a \"IntelliJ IDEA\""), ""),
            ["open -a IntelliJ IDEA /w/api"]
        );
    }

    #[test]
    fn on_windows_and_wsl_windows_terminal() {
        let win = Env {
            windows: true,
            ..Env::default()
        };
        assert_eq!(
            starts(&win, InFolder::Terminal, ""),
            ["wt -d /w/api", "cmd /C start  /D /w/api cmd /K"]
        );
        assert_eq!(
            starts(&win, InFolder::Editor("code"), ""),
            ["cmd /C code /w/api"]
        );
        let wsl = Env {
            wsl: true,
            ..Env::default()
        };
        assert_eq!(
            starts(&wsl, InFolder::Editor("nvim"), ""),
            ["wt.exe wsl.exe --cd /w/api --exec nvim ."]
        );
        assert_eq!(starts(&wsl, InFolder::Editor("code"), ""), ["code /w/api"]);
    }

    #[test]
    fn command_lines_split_like_a_shell_and_terminal_editors_are_known() {
        assert_eq!(
            split_command(r#"open -a "IntelliJ IDEA" 'a b' c\ d"#),
            ["open", "-a", "IntelliJ IDEA", "a b", "c d"]
        );
        assert_eq!(split_command("  "), Vec::<String>::new());
        assert!(runs_in_terminal("/usr/bin/nvim"));
        assert!(runs_in_terminal("emacs -nw"));
        assert!(!runs_in_terminal("emacs"));
        assert!(!runs_in_terminal("code --new-window"));
        assert!(!runs_in_terminal(""));
    }

    #[test]
    fn a_start_runs_in_the_folder_and_a_failure_falls_through() {
        REAL_PROGRAMS.with(|c| c.set(true));
        let dir = std::env::temp_dir();
        let ok = [
            start("radar-no-such-program", vec![], "never"),
            start("true", vec![], "the second"),
        ];
        assert_eq!(start_in(&ok, &dir), Ok("the second".into()));
        // Not `false`: with the 1.5 s `start_in` gives, one the loaded machine
        // had not yet let exit would count as started.
        let bad = [start("radar-no-such-program", vec![], "never")];
        assert!(start_in(&bad, &dir).is_err());
        assert!(on_path("sh"));
        assert!(!on_path("radar-no-such-program"));
    }
}
