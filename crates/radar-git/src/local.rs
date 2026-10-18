//! Local checkouts: where on this machine a repo is checked out, and what
//! can be done there (an editor on it, a terminal in it, a branch checked
//! out).
//!
//! radar looks through the folders in `[git] roots` for git checkouts (a
//! bounded number of levels down, past build output and hidden folders),
//! reads each one's `origin` remote and branch from its `.git` without
//! running git, and keeps the list as an index in its cache. A checkout
//! belongs to the repo whose path its `origin` names, in https or ssh form;
//! the host is not compared, so ssh aliases still match.
//!
//! The project type (java, python, …) comes from marker files at the
//! checkout's top, and picks the editor (`[[git.type]]`). Opening
//! a terminal or an editor is radar-core's business ([`system::folder_ways`]):
//! a tmux window, a desktop window, or the command on the user's clipboard.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use radar_core::input::TextInput;
use radar_core::journal::{ActionDescriptor, Origin, Outcome as JournalOutcome};
use radar_core::plugin::{Effect, Menu, MenuItem, Msg};
use radar_core::system::{self, Env, FolderLaunch, InFolder};
use serde::{Deserialize, Serialize};

use crate::GitPlugin;

// --- config ------------------------------------------------------------------

/// The `[git]` table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Local {
    /// Folders to look for checkouts in; `~` is your home folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
    /// How many folders down from a root to look.
    #[serde(default = "default_depth")]
    pub depth: usize,
    /// Your terminal's command line; empty: radar finds one. `{dir}` is the
    /// folder, `{cmd}` where a command goes (else it follows `-e`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub terminal: String,
    /// Project types, told apart by marker files, each with its editor.
    #[serde(default = "default_types", rename = "type")]
    pub types: Vec<ProjectType>,
    /// Your name in branch names: `{me}`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub me: String,
    /// The branch name template for repos no convention names.
    #[serde(default = "default_branch", skip_serializing_if = "is_default_branch")]
    pub branch: String,
    /// Where a parked worktree goes: `{clone}` the clone's folder, `{topic}`.
    #[serde(
        default = "default_worktree",
        skip_serializing_if = "is_default_worktree"
    )]
    pub worktree: String,
    /// A command run in a new worktree (e.g. to install dependencies).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub after_worktree: String,
    /// Repos tied to the clone you work in.
    #[serde(default, rename = "link", skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    /// Branch templates and base branches per repo path.
    #[serde(default, rename = "convention", skip_serializing_if = "Vec::is_empty")]
    pub conventions: Vec<crate::branch::Convention>,
    /// A ticket's type as it is written in a branch name (`story = "feature"`).
    #[serde(default, rename = "types", skip_serializing_if = "BTreeMap::is_empty")]
    pub type_map: BTreeMap<String, String>,
}

/// One `[[git.link]]`: the repo `repo` is worked on in the clone at `dir`
/// (found by origin otherwise). `branch` and `base` override its convention;
/// `worktree` overrides the global worktree path template.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Link {
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub dir: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub worktree: String,
}

pub const DEFAULT_BRANCH: &str = "{me}/{key}";
pub const DEFAULT_WORKTREE: &str = "{clone}-{topic}";

fn default_branch() -> String {
    DEFAULT_BRANCH.into()
}

fn is_default_branch(s: &str) -> bool {
    s == DEFAULT_BRANCH
}

fn is_default_worktree(s: &str) -> bool {
    s == DEFAULT_WORKTREE
}

fn default_worktree() -> String {
    DEFAULT_WORKTREE.into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectType {
    pub name: String,
    /// File names at a checkout's top that make it this type; `*.ext`
    /// matches by ending.
    pub markers: Vec<String>,
    /// The editor's command line; empty: radar asks which installed one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub editor: String,
}

pub const DEPTH_MAX: usize = 6;

fn default_depth() -> usize {
    4
}

pub fn default_types() -> Vec<ProjectType> {
    let t = |name: &str, markers: &[&str]| ProjectType {
        name: name.into(),
        markers: markers.iter().map(|m| (*m).to_string()).collect(),
        editor: String::new(),
    };
    vec![
        t("java", &["build.gradle", "build.gradle.kts", "pom.xml"]),
        t("python", &["pyproject.toml", "setup.py"]),
        t("node", &["package.json"]),
        t("rust", &["Cargo.toml"]),
        t("go", &["go.mod"]),
    ]
}

impl Default for Local {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            depth: default_depth(),
            terminal: String::new(),
            types: default_types(),
            me: String::new(),
            branch: default_branch(),
            worktree: default_worktree(),
            after_worktree: String::new(),
            links: Vec::new(),
            conventions: Vec::new(),
            type_map: BTreeMap::new(),
        }
    }
}

impl Local {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The first type whose marker the checkout has at its top.
    pub fn type_of(&self, c: &Checkout) -> Option<&ProjectType> {
        self.types.iter().find(|t| {
            t.markers
                .iter()
                .any(|m| c.files.iter().any(|f| marker_matches(m, f)))
        })
    }
}

pub fn marker_matches(marker: &str, file: &str) -> bool {
    match marker.strip_prefix('*') {
        Some(end) => file.ends_with(end),
        None => file == marker,
    }
}

fn home() -> Option<PathBuf> {
    dirs::home_dir()
}

/// `~/code` → your home folder's `code`; anything else as it is.
pub fn expand(root: &str) -> PathBuf {
    let root = root.trim();
    let Some(relative) = root.strip_prefix("~/").or((root == "~").then_some("")) else {
        return PathBuf::from(root);
    };
    match (Some(relative), home()) {
        (Some(""), Some(h)) => h,
        (Some(rest), Some(h)) => h.join(rest),
        _ => PathBuf::from(root),
    }
}

/// A stable absolute local identity without inspecting the folder's contents.
pub(crate) fn journal_path(path: &str) -> Option<PathBuf> {
    if path.trim().is_empty() {
        return None;
    }
    std::path::absolute(expand(path)).ok()
}

/// A path under your home folder as `~/…`, for showing.
pub fn tilde(path: &str) -> String {
    match home() {
        Some(h) => {
            let h = h.display().to_string();
            match path.strip_prefix(&h) {
                Some("") => "~".into(),
                Some(rest) if rest.starts_with('/') || rest.starts_with('\\') => {
                    format!("~{rest}")
                }
                _ => path.to_string(),
            }
        }
        None => path.to_string(),
    }
}

/// Owner-local display policy. Explicit fixtures never discover an ambient
/// home directory merely to abbreviate a path in a view or contribution.
pub(crate) fn display_path(p: &GitPlugin, path: &str) -> String {
    #[cfg(any(test, feature = "test-support"))]
    if p.fixture.is_some() {
        return path.to_string();
    }
    let _ = p;
    tilde(path)
}

/// Clean what someone pastes as a folder: quotes, `file://`, spaces.
pub fn clean_folder(input: &str) -> String {
    let s = input.trim().trim_matches(|c| c == '"' || c == '\'');
    let s = s.strip_prefix("file://").unwrap_or(s);
    let s = s.trim_end_matches(['/', '\\']);
    if s.is_empty() && input.trim().starts_with('/') {
        "/".into()
    } else {
        s.to_string()
    }
}

// --- the index -----------------------------------------------------------------

/// A git checkout found on this machine.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Checkout {
    pub dir: String,
    /// Its `origin` remote, as configured.
    pub remote: String,
    /// The repo path the remote names (`acme/team-a/api`), or empty.
    pub project: String,
    /// The branch checked out when it was read, or `detached at …`.
    pub branch: String,
    /// What is at its top: the names marker files are looked for in.
    pub files: Vec<String>,
    /// For a worktree, the folder of the clone it belongs to; empty for a
    /// clone itself.
    #[serde(default)]
    pub main: String,
}

/// What a scan found.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub checkouts: Vec<Checkout>,
    /// When it was made.
    pub at: i64,
    /// How many folders were looked in.
    pub folders: usize,
    /// True when the scan stopped at its limit before looking everywhere.
    pub cut: bool,
    /// Roots that are not folders here.
    pub missing: Vec<String>,
}

/// The local side of the plugin while it runs.
#[derive(Debug, Default)]
pub struct State {
    pub index: Option<Index>,
    pub scanning: bool,
    /// Say how the running scan went (the user asked for it).
    pub say_scan: bool,
    /// The start-up scan has been started.
    pub started: bool,
    /// Editors found installed by the last scan or editor lookup.
    pub installed: Vec<(String, String)>,
    /// The selected row of the checkouts, editors, links and branches sections.
    pub rows: [usize; 5],
    pub edit: Option<(crate::settings::Field, TextInput)>,
    /// Checkouts whose branch is being switched, by folder.
    pub switching: BTreeSet<String>,
    /// The checkouts of `[[git.link]]` folders (and their worktrees), read
    /// from disk: they need not be under a root.
    pub linked: Vec<Checkout>,
    /// The links have been read since start-up.
    pub linked_read: bool,
    /// Per repo, the worktree (or clone) folder acted on, when not the first.
    pub use_worktree: BTreeMap<String, String>,
    /// Per repo, the clones a look for it found (linking a repo with none
    /// known), offered to link.
    pub looked: BTreeMap<String, Vec<Checkout>>,
    /// What to open (`terminal`, `editor`) in a folder once the checkout
    /// or worktree being made there is done.
    pub then_open: BTreeMap<String, String>,
    /// The folders under the home folder that hold checkouts, with how
    /// many, most first: what the walk of the home folder found
    /// ([`walk_home`]), offered in the checkouts section to look in.
    pub places: Vec<(String, usize)>,
    /// Where the walk of the home folder is. It runs only once the user
    /// said yes to setup's look, and never under `RADAR_SANDBOX`.
    pub home: HomeWalk,
    /// The terminals found on this machine, read once at start-up (never
    /// under `RADAR_SANDBOX`): the terminal section shows them.
    pub terminals: Option<crate::terminal::Terminals>,
}

/// The walk of the home folder for folders that hold checkouts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HomeWalk {
    /// Not asked: nothing is walked.
    #[default]
    Off,
    /// The user said yes; the next tick starts it.
    Wanted,
    Walking,
    Done,
}

/// How far down the home folder the walk goes, how many folders it opens,
/// and how long it may take.
const WALK_DEPTH: usize = 3;
const WALK_FOLDERS: usize = 20_000;
const WALK_TIME: std::time::Duration = std::time::Duration::from_secs(1);

/// The folders under `home` that hold git checkouts (a folder with a
/// `.git`), checkouts at most `depth` folders down, grouped by the folder
/// that holds them, with how many each holds, most first. A checkout's own
/// folders are looked in too (workspaces hold checkouts inside checkouts).
/// Hidden folders, `Library` and build output are not opened; a symbolic
/// link to a folder is followed (`~/projects` is often one), each folder
/// opened once; it stops after `most` folders or `time`. Paths are kept as
/// walked, links and all.
pub fn walk_home(home: &Path, depth: usize, most: usize, time: std::time::Duration) -> Vec<(String, usize)> {
    let started = std::time::Instant::now();
    let mut counts: BTreeMap<PathBuf, usize> = BTreeMap::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut level: Vec<PathBuf> = vec![home.to_path_buf()];
    let mut opened = 0usize;
    'walk: for _ in 0..depth {
        let mut next = Vec::new();
        for dir in level {
            if !seen.insert(dir.canonicalize().unwrap_or_else(|_| dir.clone())) {
                continue;
            }
            opened += 1;
            if opened > most || started.elapsed() > time {
                break 'walk;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            let mut subs: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() || (t.is_symlink() && e.path().is_dir())))
                .filter(|e| {
                    let n = e.file_name();
                    let n = n.to_string_lossy();
                    !n.starts_with('.') && n != "Library" && !SKIP.contains(&n.as_ref())
                })
                .map(|e| e.path())
                .collect();
            subs.sort();
            for sub in subs {
                if sub.join(".git").exists() {
                    *counts.entry(dir.clone()).or_default() += 1;
                }
                next.push(sub);
            }
        }
        level = next;
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().map(|(d, n)| (d.display().to_string(), n)).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// The walk was asked for: start it at the next tick, off the screen
/// thread. Never under `RADAR_SANDBOX`, where a test gives the answer.
pub fn want_home_walk(p: &mut GitPlugin) {
    if !radar_core::system::sandboxed() && p.local.home == HomeWalk::Off {
        p.local.home = HomeWalk::Wanted;
    }
}

/// What the walk found came back.
pub fn walked(p: &mut GitPlugin, places: Vec<(String, usize)>) {
    p.local.home = HomeWalk::Done;
    p.local.places = places.into_iter().map(|(d, n)| (tilde(&d), n)).collect();
}

/// The repo path a remote URL names, in any of git's forms:
/// `https://host/a/b.git`, `ssh://git@host:2222/a/b`, `git@host:a/b.git`.
pub fn remote_project(url: &str) -> Option<String> {
    let url = url.trim();
    let path = if let Some((_, rest)) = url.split_once("://") {
        rest.split_once('/')?.1
    } else {
        let (host, path) = url.split_once(':')?;
        // A local path or a Windows drive is not a remote.
        if host.contains('/') || host.contains('\\') || host.len() <= 1 {
            return None;
        }
        path
    };
    let path = path.trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    (path.contains('/') && !path.contains("//")).then(|| path.to_string())
}

/// The `url` of `[remote "origin"]` in a git config file.
pub fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines() {
        let l = line.trim();
        if let Some(head) = l.strip_prefix('[') {
            let head = head.trim_end_matches(']').trim();
            in_origin = head
                .split_once(char::is_whitespace)
                .is_some_and(|(kind, name)| {
                    kind.eq_ignore_ascii_case("remote") && name.trim().trim_matches('"') == "origin"
                });
            continue;
        }
        if in_origin
            && let Some((k, v)) = l.split_once('=')
            && k.trim().eq_ignore_ascii_case("url")
        {
            return Some(v.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// The branch a `HEAD` file names, or `detached at <sha>`.
pub fn head_branch(head: &str) -> String {
    let head = head.trim();
    match head.strip_prefix("ref: refs/heads/") {
        Some(b) => b.to_string(),
        None => format!("detached at {}", head.chars().take(8).collect::<String>()),
    }
}

/// A checkout's own git folder and the one its config lives in (they
/// differ for a `git worktree`).
fn git_dirs(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let dot = dir.join(".git");
    if dot.is_dir() {
        return Some((dot.clone(), dot));
    }
    let text = std::fs::read_to_string(&dot).ok()?;
    let rel = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    let git = if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        dir.join(rel)
    };
    let common = match std::fs::read_to_string(git.join("commondir")) {
        Ok(c) if Path::new(c.trim()).is_absolute() => PathBuf::from(c.trim()),
        Ok(c) => git.join(c.trim()),
        Err(_) => git.clone(),
    };
    Some((git, normal(&common)))
}

/// A path with its `.` and `..` folded away, without touching the disk.
fn normal(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// Read one checkout without running git; `None` without an `origin`.
pub fn read_checkout(dir: &Path) -> Option<Checkout> {
    let (git, common) = git_dirs(dir)?;
    let remote = origin_url(&std::fs::read_to_string(common.join("config")).ok()?)?;
    let branch = std::fs::read_to_string(git.join("HEAD"))
        .map(|h| head_branch(&h))
        .unwrap_or_default();
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n != ".git")
                .take(400)
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    // A worktree's git folder is not its common one: the clone is above that.
    let main = if git != common && !dir.join(".git").is_dir() {
        common
            .parent()
            .map(|m| m.display().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };
    Some(Checkout {
        dir: dir.display().to_string(),
        project: remote_project(&remote).unwrap_or_default(),
        remote,
        branch,
        files,
        main,
    })
}

/// The checkouts the links name, each followed by its worktrees (read from
/// the clone's `worktrees/*/gitdir` files, without running git). A link
/// whose folder is not a clone has none.
pub fn read_links(links: &[Link]) -> Vec<Checkout> {
    let mut out: Vec<Checkout> = Vec::new();
    for link in links {
        let Some(c) = read_checkout(&expand(&link.dir)) else {
            continue;
        };
        let admin = Path::new(&c.dir).join(".git/worktrees");
        let mut trees: Vec<Checkout> = std::fs::read_dir(admin)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path().join("gitdir")).ok())
            .filter_map(|g| Path::new(g.trim()).parent().and_then(read_checkout))
            .filter(|w| w.dir != c.dir)
            .collect();
        trees.sort_by(|a, b| a.dir.cmp(&b.dir));
        for c in std::iter::once(c).chain(trees) {
            if !out.iter().any(|o| o.dir == c.dir) {
                out.push(c);
            }
        }
    }
    out
}

/// Read the links again (they changed, or this is the first look).
pub fn refresh_links(p: &mut GitPlugin) {
    p.local.linked = read_links(&p.cfg.links);
    p.local.linked_read = true;
}

/// Folders a scan never goes into: build output and dependencies. Hidden
/// folders (`.git`, `.venv`, …) are skipped as well.
const SKIP: [&str; 8] = [
    "node_modules",
    "target",
    "build",
    "dist",
    "vendor",
    "venv",
    "__pycache__",
    "out",
];

/// Most folders one scan looks in, so a root like `/` cannot run for ever.
const MOST_FOLDERS: usize = 50_000;

/// Where to look for a repo's clone: the folders radar looks in, then the
/// ones the walk of the home folder found holding checkouts.
pub fn usual_places(roots: &[String], places: &[(String, usize)]) -> Vec<String> {
    let mut out: Vec<String> = roots.to_vec();
    for (place, _) in places {
        if !out.iter().any(|r| clean_folder(r) == clean_folder(place)) {
            out.push(place.clone());
        }
    }
    out
}

/// The clones of `project` in `places`, `depth` folders down: those whose
/// origin names it, then those whose folder has its name (a fork, a mirror),
/// nearest first. Bounded as a scan is.
pub fn look_for(project: &str, places: &[String], depth: usize) -> Vec<Checkout> {
    let name = project.rsplit('/').next().unwrap_or(project).to_ascii_lowercase();
    let mut by_origin = Vec::new();
    let mut by_name = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut folders = 0usize;
    for place in places {
        let base = expand(place);
        let mut level: Vec<PathBuf> = vec![base];
        for _ in 0..=depth {
            let mut next = Vec::new();
            for dir in level {
                folders += 1;
                if folders > MOST_FOLDERS || !seen.insert(dir.clone()) {
                    continue;
                }
                if dir.join(".git").exists()
                    && let Some(c) = read_checkout(&dir)
                {
                    if c.project.eq_ignore_ascii_case(project) {
                        by_origin.push(c);
                    } else if dir.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase() == name) {
                        by_name.push(c);
                    }
                }
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                let mut subs: Vec<PathBuf> = entries
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .filter(|e| {
                        let n = e.file_name();
                        let n = n.to_string_lossy();
                        !n.starts_with('.') && !SKIP.contains(&n.as_ref())
                    })
                    .map(|e| e.path())
                    .collect();
                subs.sort();
                next.extend(subs);
            }
            level = next;
        }
    }
    by_origin.extend(by_name);
    by_origin
}

/// Look through `roots` for git checkouts, `depth` folders down at most.
/// Symbolic links are not followed. A checkout's own folders are looked in
/// too: workspaces hold checkouts inside checkouts.
pub fn scan(roots: &[String], depth: usize) -> Index {
    let mut idx = Index::default();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for root in roots {
        let base = expand(root);
        if !base.is_dir() {
            idx.missing.push(root.clone());
            continue;
        }
        let mut stack = vec![(base, 0usize)];
        while let Some((dir, level)) = stack.pop() {
            if idx.folders >= MOST_FOLDERS {
                idx.cut = true;
                break;
            }
            if !seen.insert(dir.clone()) {
                continue;
            }
            idx.folders += 1;
            if dir.join(".git").exists()
                && let Some(c) = read_checkout(&dir)
            {
                idx.checkouts.push(c);
            }
            if level >= depth {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut subs: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .filter(|e| {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    !name.starts_with('.') && !SKIP.contains(&name.as_ref())
                })
                .map(|e| e.path())
                .collect();
            subs.sort();
            stack.extend(subs.into_iter().rev().map(|s| (s, level + 1)));
        }
    }
    idx.checkouts.sort_by(|a, b| a.dir.cmp(&b.dir));
    idx
}

/// The link for a repo, by its full path.
pub fn link_for<'a>(p: &'a GitPlugin, project: &str) -> Option<&'a Link> {
    p.cfg
        .links
        .iter()
        .find(|l| !l.repo.is_empty() && l.repo.eq_ignore_ascii_case(project))
}

/// The checkouts of a repo, by its full path: the linked clone first (and
/// its worktrees after it), then the clones found by their origin, then the
/// found worktrees.
pub fn checkouts_for<'a>(p: &'a GitPlugin, project: &str) -> Vec<&'a Checkout> {
    if crate::scoped::is_scoped(project) {
        return crate::scoped::checkouts(p, project);
    }
    let mut out: Vec<&Checkout> = Vec::new();
    if let Some(link) = link_for(p, project) {
        let dir = expand(&link.dir).display().to_string();
        let linked = p.local.linked.iter();
        out.extend(linked.clone().filter(|c| c.dir == dir));
        out.extend(linked.filter(|c| c.main == dir));
    }
    let found = p
        .local
        .index
        .iter()
        .flat_map(|i| i.checkouts.iter())
        .filter(|c| !c.project.is_empty() && c.project.eq_ignore_ascii_case(project));
    let (trees, clones): (Vec<&Checkout>, Vec<&Checkout>) = found.partition(|c| !c.main.is_empty());
    for c in clones.into_iter().chain(trees) {
        if !out.iter().any(|o| o.dir == c.dir) {
            out.push(c);
        }
    }
    out
}

/// The checkout acted on for a repo: the one picked with `use_worktree`
/// when it is still there, else the first.
// The actions that run git in it (the next slice) are its callers.
#[allow(dead_code)]
pub fn chosen<'a>(p: &'a GitPlugin, project: &str) -> Option<&'a Checkout> {
    let all = checkouts_for(p, project);
    let picked = p
        .local
        .use_worktree
        .iter()
        .find(|(repo, _)| repo.eq_ignore_ascii_case(project))
        .and_then(|(_, dir)| all.iter().find(|c| c.dir == *dir));
    picked.or(all.first()).copied()
}

// --- git -------------------------------------------------------------------------

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        // Never ask for a password: the terminal is radar's.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("SSH_ASKPASS", "false")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("git failed")
            .trim()
            .trim_start_matches("fatal: ")
            .trim_start_matches("error: ")
            .to_string())
    }
}

/// Fetch `branch` from `origin` and check it out in `dir`: the local
/// branch if there is one (fast-forwarded when it can be), else a new one
/// tracking origin's. Refuses, changing nothing, while tracked files have
/// uncommitted changes; untracked files stay (git refuses itself if the
/// branch would overwrite one).
pub fn check_out(dir: &Path, branch: &str) -> Result<String, String> {
    if branch.is_empty() || branch.starts_with('-') {
        return Err(format!("\"{branch}\" is not a branch radar will check out"));
    }
    let dirty = git(dir, &["status", "--porcelain", "--untracked-files=no"])?;
    if !dirty.is_empty() {
        let n = dirty.lines().count();
        return Err(format!(
            "{n} uncommitted change{} in {}: commit or stash first; nothing was changed",
            if n == 1 { "" } else { "s" },
            tilde(&dir.display().to_string())
        ));
    }
    let remote = format!("origin/{branch}");
    git(
        dir,
        &[
            "fetch",
            "origin",
            &format!("+refs/heads/{branch}:refs/remotes/{remote}"),
        ],
    )
    .map_err(|e| format!("could not fetch {branch}: {e}"))?;
    if git(
        dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok()
    {
        git(dir, &["checkout", branch])?;
        Ok(match git(dir, &["merge", "--ff-only", &remote]) {
            Ok(_) => format!("on {branch}, up to date with origin"),
            Err(_) => format!("on {branch}; it has commits origin has not, so it was not updated"),
        })
    } else {
        git(dir, &["checkout", "-b", branch, "--track", &remote])?;
        Ok(format!("on {branch}, new, tracking origin's"))
    }
}

// --- tasks -----------------------------------------------------------------------

/// What a local task found.
pub enum LocalMsg {
    Scanned {
        index: Index,
        installed: Vec<(String, String)>,
    },
    /// Typed launch-helper evidence, with UI-only display context.
    Launched {
        result: FolderLaunch,
        what: String,
        also: String,
    },
    /// A blocking task did not return an observation.
    Failed(String),
    /// No editor set, or it is not installed: ask which.
    PickEditor {
        dir: String,
        kind: Option<String>,
        why: String,
        installed: Vec<(String, String)>,
    },
    SwitchedBranch {
        dir: String,
        branch: String,
        result: Result<String, String>,
    },
    /// A look for a repo's clone, to link it, found these.
    LookedFor { project: String, found: Vec<Checkout> },
    /// The walk of the home folder found these folders holding checkouts.
    Walked { places: Vec<(String, usize)> },
    /// A worktree was made to open a branch in (or why not).
    Worktree {
        dir: String,
        result: Result<String, String>,
        parked: Option<crate::act::WorktreeResult>,
    },
}

/// Look for `project`'s clone in the usual places, off the event loop.
pub fn look(p: &GitPlugin, project: &str) -> Vec<Effect> {
    let places = usual_places(&p.cfg.roots, &p.local.places);
    let project = project.to_string();
    let name = project.rsplit('/').next().unwrap_or(&project).to_string();
    vec![
        Effect::Notify(format!("looking for {name} on this machine…")),
        blocking(move || LocalMsg::LookedFor {
            found: look_for(&project, &places, 7),
            project,
        }),
    ]
}

/// Blocking work off the event loop, its answer back as a [`LocalMsg`].
fn blocking(f: impl FnOnce() -> LocalMsg + Send + 'static) -> Effect {
    Effect::Spawn(Box::pin(async move {
        let msg = tokio::task::spawn_blocking(f)
            .await
            .unwrap_or_else(|e| LocalMsg::Failed(e.to_string()));
        Box::new(msg) as Msg
    }))
}

/// Start a scan of the roots, unless one is running. `say` reports it.
pub fn scan_now(p: &mut GitPlugin, say: bool) -> Vec<Effect> {
    if p.local.scanning {
        return vec![Effect::Notify("already looking for checkouts".into())];
    }
    if p.cfg.roots.is_empty() {
        return vec![Effect::Notify(
            "no folders to look in yet: add one in settings, git › checkouts".into(),
        )];
    }
    p.local.started = true;
    p.local.scanning = true;
    p.local.say_scan = say;
    let (roots, depth) = (p.cfg.roots.clone(), p.cfg.depth);
    #[cfg(any(test, feature = "test-support"))]
    let fixture = p.fixture.clone();
    #[cfg(any(test, feature = "test-support"))]
    let pending = fixture.as_ref().map(|f| f.pending());
    let mut fx = vec![
        Effect::Journal(ActionDescriptor::new(
            "git.checkouts.scan",
            Origin::Plugin("git".into()),
        )),
        blocking(move || {
            #[cfg(any(test, feature = "test-support"))]
            if let Some(fixture) = fixture {
                let _pending = pending;
                return match fixture.scan(&roots, depth) {
                    Ok(index) => LocalMsg::Scanned { index, installed: vec![] },
                    Err(why) => LocalMsg::Failed(why),
                };
            }
            LocalMsg::Scanned {
                index: scan(&roots, depth),
                installed: system::installed_editors(&Env::detect()),
            }
        }),
    ];
    if say {
        fx.push(Effect::Notify("looking for checkouts…".into()));
    }
    fx
}

/// Once at start-up: look for checkouts in the background.
pub fn tick(p: &mut GitPlugin) -> Vec<Effect> {
    #[cfg(any(test, feature = "test-support"))]
    if p.fixture.is_some() {
        // The fixture's only discovery is the explicit checkout scan below.
        // Never consult the ambient terminal, editor installation or home.
        p.local.terminals = Some(crate::terminal::Terminals::default());
        p.local.home = HomeWalk::Off;
    }
    if !p.local.linked_read {
        refresh_links(p);
    }
    if p.local.terminals.is_none() && !radar_core::system::sandboxed() {
        p.local.terminals = Some(crate::terminal::here());
    }
    let mut fx = vec![];
    if p.local.home == HomeWalk::Wanted {
        p.local.home = HomeWalk::Walking;
        if let Some(h) = home() {
            fx.push(blocking(move || LocalMsg::Walked { places: walk_home(&h, WALK_DEPTH, WALK_FOLDERS, WALK_TIME) }));
        } else {
            p.local.home = HomeWalk::Done;
        }
    }
    if p.local.started || p.cfg.roots.is_empty() {
        return fx;
    }
    fx.extend(scan_now(p, false));
    fx
}

/// A freshly read checkout may not be in the index yet, notably just after
/// making a worktree. Capture its actual identity before launching in it.
pub(crate) fn open_terminal(p: &GitPlugin, c: &Checkout) -> Vec<Effect> {
    vec![
        crate::act::journal_at("local.terminal.launch", &c.dir, &c.project, &c.branch),
        launch_task(p, &c.dir, None, String::new()),
    ]
}

/// A menu continuation reuses the launch's captured identity and semantic span.
fn launch_task(p: &GitPlugin, dir: &str, editor: Option<String>, also: String) -> Effect {
    let terminal = p.cfg.terminal.clone();
    let dir = dir.to_string();
    let name = Path::new(&dir)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    blocking(move || {
        let env = Env::detect();
        let what = editor
            .as_deref()
            .map_or(InFolder::Terminal, InFolder::Editor);
        let ways = system::folder_ways(&env, &dir, what, &terminal, &name);
        LocalMsg::Launched {
            result: system::start_folder(ways, Path::new(&dir)),
            what: "it".into(),
            also,
        }
    })
}

/// Start the first way that works, or preserve the typed fallback observation.
fn start_or_hand_over(ways: system::FolderWays, dir: &str, what: &str) -> LocalMsg {
    LocalMsg::Launched {
        result: system::start_folder(ways, Path::new(dir)),
        what: what.into(),
        also: String::new(),
    }
}

/// The editor for this checkout's type, if it is installed; else ask.
pub fn open_editor(p: &GitPlugin, c: &Checkout) -> Vec<Effect> {
    let kind = p.cfg.type_of(c);
    let editor = kind.map(|k| k.editor.clone()).unwrap_or_default();
    let kind = kind.map(|k| k.name.clone());
    let (dir, terminal) = (c.dir.clone(), p.cfg.terminal.clone());
    let name = Path::new(&dir)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    vec![
        crate::act::journal_at("local.editor.launch", &c.dir, &c.project, &c.branch),
        blocking(move || {
            let env = Env::detect();
            let prog = system::split_command(&editor)
                .into_iter()
                .next()
                .unwrap_or_default();
            if prog.is_empty() || !system::on_path(&prog) {
                let why = match (&kind, editor.is_empty()) {
                    (None, _) => "radar cannot tell what kind of project this is".to_string(),
                    (Some(k), true) => format!("no editor is set for {k}"),
                    (Some(_), false) => format!("{prog} is not installed here"),
                };
                return LocalMsg::PickEditor {
                    dir,
                    kind,
                    why,
                    installed: system::installed_editors(&env),
                };
            }
            let ways = system::folder_ways(&env, &dir, InFolder::Editor(&editor), &terminal, &name);
            start_or_hand_over(ways, &dir, &prog)
        }),
    ]
}

/// Fetch and check out `branch` in `dir` (asked first by the caller).
pub fn switch_branch(p: &mut GitPlugin, dir: &str, branch: &str) -> Vec<Effect> {
    if !p.local.switching.insert(dir.to_string()) {
        return vec![
            crate::act::journal_at("git.branch.checkout", dir, "", branch),
            Effect::Report(JournalOutcome::Blocked),
            Effect::Notify("already checking out there".into()),
        ];
    }
    let event = crate::act::journal_at("git.branch.checkout", dir, "", branch);
    let (dir, branch) = (dir.to_string(), branch.to_string());
    vec![
        event,
        Effect::Report(JournalOutcome::Confirmed),
        Effect::Notify(format!("checking out {branch}…")),
        blocking(move || LocalMsg::SwitchedBranch {
            result: check_out(Path::new(&dir), &branch),
            dir,
            branch,
        }),
    ]
}

/// The menu id that asks which editor to open `dir` with.
pub const PICK_EDITOR: &str = "local-editor";

pub fn on_msg(p: &mut GitPlugin, msg: LocalMsg) -> Vec<Effect> {
    match msg {
        LocalMsg::Walked { places } => {
            walked(p, places);
            vec![]
        }
        LocalMsg::Scanned {
            mut index,
            installed,
        } => {
            index.at = p.now;
            p.local.scanning = false;
            p.local.installed = installed;
            let mut fx = vec![
                Effect::Report(if index.cut || !index.missing.is_empty() {
                    JournalOutcome::Partial
                } else {
                    JournalOutcome::Succeeded
                }),
                Effect::PersistCache,
            ];
            if std::mem::take(&mut p.local.say_scan) {
                let mut said = format!(
                    "found {} checkout{} in {} folders",
                    index.checkouts.len(),
                    if index.checkouts.len() == 1 { "" } else { "s" },
                    index.folders
                );
                if !index.missing.is_empty() {
                    said.push_str(&format!(
                        " · not a folder here: {}",
                        index.missing.join(", ")
                    ));
                }
                fx.push(Effect::Notify(said));
            }
            p.local.index = Some(index);
            fx
        }
        LocalMsg::Launched { result, what, also } => {
            let (outcome, mut said, copy) = match result {
                FolderLaunch::Dispatched(started) => {
                    (JournalOutcome::Succeeded, format!("opened {started}"), None)
                }
                FolderLaunch::Unavailable { why, by_hand } => (
                    JournalOutcome::Blocked,
                    format!("{why}: `{by_hand}` is on your clipboard"),
                    Some(by_hand),
                ),
                FolderLaunch::Unconfirmed { why, by_hand } => (
                    JournalOutcome::Unknown,
                    format!("could not open {what} ({why}): `{by_hand}` is on your clipboard"),
                    Some(by_hand),
                ),
            };
            if !also.is_empty() {
                said.push_str(&format!(" · {also}"));
            }
            let mut fx = vec![Effect::Report(outcome)];
            fx.extend(copy.map(Effect::Copy));
            fx.push(if outcome == JournalOutcome::Succeeded { Effect::Notify(said) } else { Effect::error(None, said) });
            fx
        }
        LocalMsg::Failed(why) => vec![
            Effect::Report(JournalOutcome::Unknown),
            Effect::error(None, radar_core::writes::failed_line(&why)),
        ],
        LocalMsg::PickEditor {
            dir,
            kind,
            why,
            installed,
        } => {
            p.local.installed = installed.clone();
            if installed.is_empty() {
                return vec![
                    Effect::Report(JournalOutcome::Blocked),
                    Effect::error(None, format!(
                        "{why}, and radar found no editor installed: set one in settings, git › editors"
                    )),
                ];
            }
            let remember = kind.as_ref().is_some_and(|k| {
                p.cfg
                    .types
                    .iter()
                    .any(|t| t.name == *k && t.editor.is_empty())
            });
            let items = installed
                .iter()
                .map(|(cmd, name)| {
                    let hint = match (&kind, remember) {
                        (Some(k), true) => format!("{cmd} · and for {k} from now on"),
                        _ => cmd.clone(),
                    };
                    MenuItem::pick(name.clone(), hint, cmd.clone())
                })
                .collect();
            let repo = Path::new(&dir)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            vec![
                Effect::Notify(why),
                Effect::Menu(Menu::new(
                    format!("{PICK_EDITOR}:{}\t{dir}", kind.unwrap_or_default()),
                    format!("open {repo} with…"),
                    items,
                )),
            ]
        }
        LocalMsg::LookedFor { project, found } => {
            p.local.looked.insert(project.clone(), found);
            crate::act::open_link(p, &project)
        }
        LocalMsg::Worktree { dir, result, parked } => {
            let what = p.local.then_open.remove(&dir);
            let outcome = parked.as_ref().map_or_else(
                || if result.is_ok() { JournalOutcome::Succeeded } else { JournalOutcome::Unknown },
                crate::act::WorktreeResult::outcome,
            );
            let mut fx = vec![Effect::Report(outcome)];
            let mut launch = None;
            match result {
                Ok(said) => {
                    fx.push(Effect::Notify(radar_core::writes::done_line(&said)));
                    fx.extend(scan_now(p, false));
                    if let (Some(what), Some(c)) = (what, read_checkout(Path::new(&dir))) {
                        launch = Some((what, c));
                    }
                }
                Err(e) => fx.push(Effect::error(None, radar_core::writes::failed_line(&format!("no worktree made: {e}")))),
            }
            if let Some(parked) = parked {
                fx.extend(crate::act::journal_worktree(&parked));
            }
            // Scan and step observations must not inherit the launch's span.
            if let Some((what, c)) = launch {
                fx.extend(crate::act::open_in(p, &what, &c));
            }
            fx
        }
        LocalMsg::SwitchedBranch {
            dir,
            branch,
            result,
        } => {
            p.local.switching.remove(&dir);
            // A terminal or editor waiting for this checkout.
            let then = p.local.then_open.remove(&dir);
            match result {
                Ok(said) => {
                    if let Some(c) = p
                        .local
                        .index
                        .as_mut()
                        .and_then(|i| i.checkouts.iter_mut().find(|c| c.dir == dir))
                    {
                        c.branch = branch;
                    }
                    let mut fx = vec![
                        Effect::Report(JournalOutcome::Succeeded),
                        Effect::PersistCache,
                        Effect::Notify(radar_core::writes::done_line(&said)),
                    ];
                    if let (Some(what), Some(c)) = (then, read_checkout(Path::new(&dir))) {
                        fx.extend(crate::act::open_in(p, &what, &c));
                    }
                    fx
                }
                Err(e) => {
                    let also = if then.is_some() { " · w in its question opens it in a worktree instead" } else { "" };
                    vec![
                        Effect::Report(JournalOutcome::Unknown),
                        Effect::error(None, radar_core::writes::failed_line(&format!("not checked out: {e}{also}"))),
                    ]
                }
            }
        }
    }
}

/// The user picked an editor from the menu: open it, and keep it for the
/// project type when that type had none.
pub fn on_pick(p: &mut GitPlugin, arg: &str, editor: &str) -> Vec<Effect> {
    let (kind, dir) = arg.split_once('\t').unwrap_or(("", arg));
    let mut also = String::new();
    if let Some(t) = p
        .cfg
        .types
        .iter_mut()
        .find(|t| t.name == kind && t.editor.is_empty())
    {
        t.editor = editor.to_string();
        also = format!("now the editor for {kind}");
    }
    vec![launch_task(p, dir, Some(editor.to_string()), also)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A home folder of throwaway folders, each `.git` a checkout.
    fn fake_home(name: &str, checkouts: &[&str]) -> PathBuf {
        let home = std::env::temp_dir().join(format!("radar-walk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for c in checkouts {
            std::fs::create_dir_all(home.join(c).join(".git")).unwrap();
        }
        home
    }

    #[test]
    fn the_walk_finds_the_folders_that_hold_checkouts_most_first() {
        let home = fake_home("found", &[
            "projects/api",
            "projects/web",
            "projects/web/vendored",
            "work/watch/pseudopolis",
            "dotfiles",
            ".hidden/secret",
            "node_modules/dep",
            "Library/caches",
            "deep/one/two/three",
        ]);
        // Code kept elsewhere, linked into the home folder; the link back
        // up is opened once.
        let elsewhere = fake_home("elsewhere", &["ankh", "morpork"]);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&elsewhere, home.join("linked")).unwrap();
            std::os::unix::fs::symlink(&home, elsewhere.join("ankh/back")).unwrap();
        }
        let at = |rel: &str| if rel.is_empty() { home.display().to_string() } else { home.join(rel).display().to_string() };
        let found = walk_home(&home, 3, 1000, std::time::Duration::from_secs(10));
        // A checkout's own folders are looked in; hidden folders,
        // node_modules and Library are skipped; three folders down at most.
        let mut want = vec![(at("projects"), 2), (at(""), 1), (at("projects/web"), 1), (at("work/watch"), 1)];
        if cfg!(unix) {
            want.insert(0, (at("linked"), 2));
        }
        assert_eq!(found, want);
        // The count cap stops it: only the home folder itself is opened.
        assert_eq!(walk_home(&home, 3, 1, std::time::Duration::from_secs(10)), [(at(""), 1)]);
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&elsewhere);
    }

    #[test]
    fn nothing_is_walked_under_the_sandbox_and_a_given_answer_is_offered() {
        let mut p = GitPlugin::default();
        want_home_walk(&mut p);
        assert_eq!(p.local.home, HomeWalk::Off, "the sandbox never walks");
        assert!(tick(&mut p).is_empty());
        on_msg(&mut p, LocalMsg::Walked { places: vec![("/home/vimes/projects".into(), 5)] });
        assert_eq!(p.local.home, HomeWalk::Done);
        assert_eq!(usual_places(&["~/watch".into()], &p.local.places).len(), 2);
    }

    #[test]
    fn launch_observations_keep_dispatch_and_clipboard_facts_separate() {
        let mut p = GitPlugin::default();
        for (result, expected, copy) in [
            (
                FolderLaunch::Dispatched("SECRET_LABEL".into()),
                JournalOutcome::Succeeded,
                false,
            ),
            (
                FolderLaunch::Unavailable {
                    why: "SECRET_ENVIRONMENT".into(),
                    by_hand: "SECRET_COMMAND".into(),
                },
                JournalOutcome::Blocked,
                true,
            ),
            (
                FolderLaunch::Unconfirmed {
                    why: "SECRET_ERROR".into(),
                    by_hand: "SECRET_COMMAND".into(),
                },
                JournalOutcome::Unknown,
                true,
            ),
        ] {
            let fx = on_msg(
                &mut p,
                LocalMsg::Launched {
                    result,
                    what: "SECRET_EDITOR".into(),
                    also: "SECRET_CONFIG".into(),
                },
            );
            assert!(matches!(fx[0], Effect::Report(outcome) if outcome == expected));
            assert_eq!(
                fx.iter().filter(|e| matches!(e, Effect::Copy(_))).count(),
                usize::from(copy)
            );
            if expected == JournalOutcome::Succeeded {
                assert!(matches!(fx.last(), Some(Effect::Notify(said)) if said.ends_with(" · SECRET_CONFIG")));
            } else {
                assert!(matches!(fx.last(), Some(Effect::Notice(message))
                    if message.tone == radar_core::view::chrome::NoticeTone::Error && message.ends_with(" · SECRET_CONFIG")));
            }
            assert!(!fx.iter().any(|e| matches!(e, Effect::Journal(_))));
        }
        assert!(matches!(
            on_msg(&mut p, LocalMsg::Failed("SECRET_ERROR".into()))[0],
            Effect::Report(JournalOutcome::Unknown)
        ));
    }

    #[test]
    fn editor_choices_keep_the_launch_open_and_reuse_its_context() {
        let mut p = GitPlugin::default();
        let choice = |installed| LocalMsg::PickEditor {
            dir: "/work/api".into(),
            kind: None,
            why: "SECRET_REASON".into(),
            installed,
        };
        let fx = on_msg(&mut p, choice(vec![("editor".into(), "Editor".into())]));
        assert!(fx.iter().any(|e| matches!(e, Effect::Menu(_))));
        assert!(!fx.iter().any(|e| matches!(e, Effect::Report(_))));
        let picked = on_pick(&mut p, "\t/work/api", "editor");
        assert!(matches!(&picked[..], [Effect::Spawn(_)]));
        let blocked = on_msg(&mut p, choice(vec![]));
        assert!(matches!(
            blocked[0],
            Effect::Report(JournalOutcome::Blocked)
        ));
    }

    #[test]
    fn launch_subjects_capture_the_checkout_and_branch_before_work() {
        let mut p = crate::demo::sample();
        let checkout = p.local.index.as_ref().unwrap().checkouts[0].clone();
        let fx = open_terminal(&p, &checkout);
        p.local.index.as_mut().unwrap().checkouts[0].branch = "changed-later".into();
        let Effect::Journal(action) = &fx[0] else {
            panic!("launch descriptor")
        };
        assert_eq!(action.kind.as_str(), "local.terminal.launch");
        assert_eq!(action.subjects[0].key.instance, checkout.dir);
        assert!(
            action
                .subjects
                .iter()
                .any(|s| s.key.kind == "branch" && s.key.id == checkout.branch)
        );
        assert!(!format!("{action:?}").contains("SECRET_CONFIG"));
        assert!(!format!("{action:?}").contains("changed-later"));
    }

    #[test]
    fn remotes_name_their_repo_in_every_form() {
        for url in [
            "https://gitlab.com/acme/team-a/api.git",
            "https://alice:secret@gitlab.com/acme/team-a/api",
            "https://gitlab.com/acme/team-a/api/",
            "ssh://git@gitlab.com:2222/acme/team-a/api.git",
            "git@gitlab.com:acme/team-a/api.git",
            "gitlab-work:acme/team-a/api",
        ] {
            assert_eq!(
                remote_project(url).as_deref(),
                Some("acme/team-a/api"),
                "{url}"
            );
        }
        for url in [
            "/srv/git/api.git",
            "C:\\repos\\api",
            "api",
            "https://gitlab.com/api",
        ] {
            assert_eq!(remote_project(url), None, "{url}");
        }
    }

    #[test]
    fn origin_and_head_are_read_from_gits_files() {
        let config = "[core]\n\tbare = false\n[remote \"upstream\"]\n\turl = git@x:other/y.git\n\
                      [remote \"origin\"]\n\turl = git@gitlab.com:acme/team-a/api.git\n\
                      \tfetch = +refs/heads/*:refs/remotes/origin/*\n";
        assert_eq!(
            origin_url(config).as_deref(),
            Some("git@gitlab.com:acme/team-a/api.git")
        );
        assert_eq!(origin_url("[core]\n"), None);
        assert_eq!(
            head_branch("ref: refs/heads/feature/ABC-1\n"),
            "feature/ABC-1"
        );
        assert_eq!(head_branch("0123456789abcdef\n"), "detached at 01234567");
    }

    #[test]
    fn the_project_type_comes_from_marker_files() {
        let cfg = Local::default();
        let c = |files: &[&str]| Checkout {
            files: files.iter().map(|f| (*f).to_string()).collect(),
            ..Checkout::default()
        };
        assert_eq!(
            cfg.type_of(&c(&["README.md", "pom.xml"]))
                .map(|t| t.name.as_str()),
            Some("java")
        );
        assert_eq!(
            cfg.type_of(&c(&["Cargo.toml"])).map(|t| t.name.as_str()),
            Some("rust")
        );
        assert!(cfg.type_of(&c(&["README.md"])).is_none());
        assert!(marker_matches("*.csproj", "App.csproj"));
        assert!(!marker_matches("*.csproj", "csproj"));
    }

    #[test]
    fn pasted_folders_are_cleaned_and_home_is_tilde() {
        assert_eq!(clean_folder(" '/w/code/' "), "/w/code");
        assert_eq!(clean_folder("file:///w/code"), "/w/code");
        assert_eq!(clean_folder("/"), "/");
        if let Some(h) = dirs::home_dir() {
            let h = h.display().to_string();
            assert_eq!(expand("~/code"), PathBuf::from(format!("{h}/code")));
            assert_eq!(tilde(&format!("{h}/code")), "~/code");
            assert_eq!(tilde(&format!("{h}x/code")), format!("{h}x/code"));
        }
    }

    #[test]
    fn the_table_round_trips_and_a_default_one_is_left_out() {
        let mut p = crate::GitPlugin::default();
        assert!(
            radar_core::Plugin::config(&p).is_none(),
            "a default table is not written"
        );
        p.cfg.roots = vec!["~/code".into()];
        p.cfg.types[0].editor = "idea".into();
        let table = radar_core::Plugin::config(&p).unwrap();
        let mut back = crate::GitPlugin::default();
        radar_core::Plugin::configure(&mut back, Some(&table)).unwrap();
        assert_eq!(back.cfg, p.cfg, "{table}");
    }

    #[test]
    fn links_conventions_and_the_rest_round_trip_and_stay_out_of_a_default_table() {
        let mut p = crate::GitPlugin::default();
        p.cfg.me = "alice".into();
        p.cfg.links.push(Link {
            repo: "acme/team-a/api".into(),
            dir: "~/code/api".into(),
            branch: "{type}/{key}".into(),
            base: "develop".into(),
            worktree: "/work/watch/{topic}".into(),
        });
        p.cfg.conventions.push(crate::branch::Convention {
            under: "acme/team-b".into(),
            branch: "{key}-{slug}".into(),
            base: String::new(),
        });
        p.cfg.type_map.insert("story".into(), "feature".into());
        p.cfg.worktree = "{clone}-wt-{topic}".into();
        p.cfg.after_worktree = "make deps".into();
        let table = radar_core::Plugin::config(&p).unwrap();
        let text = table.to_string();
        assert!(
            text.contains("link = [") && text.contains("convention = ["),
            "{text}"
        );
        let mut back = crate::GitPlugin::default();
        radar_core::Plugin::configure(&mut back, Some(&table)).unwrap();
        assert_eq!(back.cfg, p.cfg, "{text}");
        // A table of old files has none of it, and says nothing new.
        let mut old = crate::GitPlugin::default();
        radar_core::Plugin::configure(
            &mut old,
            Some(&toml::from_str("roots = [\"~/code\"]").unwrap()),
        )
        .unwrap();
        assert_eq!(
            (old.cfg.branch.as_str(), old.cfg.worktree.as_str()),
            (DEFAULT_BRANCH, DEFAULT_WORKTREE)
        );
        assert!(
            !radar_core::Plugin::config(&old)
                .unwrap()
                .to_string()
                .contains("branch")
        );
    }

    #[test]
    fn a_link_wins_over_the_clone_found_and_worktrees_group_under_their_clone() {
        let s = Scratch::new("link");
        let found = repos(&s);
        let url = "git@example.com:acme/team-a/api.git";
        sh(&found, &["remote", "set-url", "origin", url]);
        // A worktree of the found clone, beside it.
        let wt = found.with_file_name("api-topic");
        sh(
            &found,
            &["worktree", "add", "-b", "topic", wt.to_str().unwrap()],
        );
        // Another clone of the same repo, outside the folder radar looks in.
        let mine = s.0.join("elsewhere").join("api");
        std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
        sh(
            &s.0,
            &[
                "clone",
                s.0.join("origin.git").to_str().unwrap(),
                mine.to_str().unwrap(),
            ],
        );
        sh(&mine, &["remote", "set-url", "origin", url]);
        let mine_wt = s.0.join("elsewhere").join("api-mine");
        sh(
            &mine,
            &["worktree", "add", "-b", "mine", mine_wt.to_str().unwrap()],
        );

        let mut p = crate::GitPlugin::default();
        let root = s.0.join("code").display().to_string();
        p.cfg.roots = vec![root.clone()];
        p.local.index = Some(scan(&[root], 4));
        let dirs = |p: &crate::GitPlugin| -> Vec<String> {
            checkouts_for(p, "acme/team-a/api")
                .iter()
                .map(|c| c.dir.clone())
                .collect()
        };
        let (clone, tree) = (found.display().to_string(), wt.display().to_string());
        assert_eq!(
            dirs(&p),
            [clone.clone(), tree.clone()],
            "the clone, then its worktree"
        );
        let found_tree = checkouts_for(&p, "acme/team-a/api")[1];
        assert_eq!(found_tree.main, clone, "a worktree knows its clone");
        assert_eq!(chosen(&p, "acme/team-a/api").unwrap().dir, clone);
        p.local
            .use_worktree
            .insert("acme/team-a/api".into(), tree.clone());
        assert_eq!(
            chosen(&p, "acme/team-a/api").unwrap().dir,
            tree,
            "the picked worktree"
        );
        p.local
            .use_worktree
            .insert("acme/team-a/api".into(), "/gone".into());
        assert_eq!(
            chosen(&p, "acme/team-a/api").unwrap().dir,
            clone,
            "a vanished pick is ignored"
        );

        // The link: its clone first, then its worktree, then what was found.
        p.cfg.links.push(Link {
            repo: "Acme/Team-A/API".into(),
            dir: mine.display().to_string(),
            ..Link::default()
        });
        refresh_links(&mut p);
        assert_eq!(
            dirs(&p),
            [
                mine.display().to_string(),
                mine_wt.display().to_string(),
                clone.clone(),
                tree
            ]
        );
        assert_eq!(
            chosen(&p, "acme/team-a/api").unwrap().dir,
            mine.display().to_string()
        );
        // A link that is not a clone offers nothing, and the found ones remain.
        p.cfg.links[0].dir = s.0.display().to_string();
        refresh_links(&mut p);
        assert_eq!(dirs(&p)[0], clone);
    }

    // --- on real, throwaway repositories --------------------------------------------

    /// A folder under the system's temp folder, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "radar-local-{name}-{}-{}",
                std::process::id(),
                radar_core::time::now_secs()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            // Resolved: macOS's temporary folder is a link (/var -> /private/var).
            Self(std::fs::canonicalize(&dir).unwrap())
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// git in `dir`, as a throwaway identity, never the user's config.
    fn sh(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// An "origin" bare repo with `main` and `feature/x`, and a clone of it
    /// whose origin URL is GitLab-shaped.
    fn repos(s: &Scratch) -> PathBuf {
        let origin = s.0.join("origin.git");
        let work = s.0.join("seed");
        std::fs::create_dir_all(&work).unwrap();
        sh(&s.0, &["init", "--bare", "origin.git"]);
        sh(&work, &["init"]);
        std::fs::write(work.join("pom.xml"), "<project/>").unwrap();
        sh(&work, &["add", "."]);
        sh(&work, &["commit", "-m", "first"]);
        sh(
            &work,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        sh(&work, &["push", "origin", "main"]);
        sh(&work, &["checkout", "-b", "feature/x"]);
        std::fs::write(work.join("a.txt"), "a").unwrap();
        sh(&work, &["add", "."]);
        sh(&work, &["commit", "-m", "feature"]);
        sh(&work, &["push", "origin", "feature/x"]);
        let clone = s.0.join("code").join("team-a").join("api");
        std::fs::create_dir_all(clone.parent().unwrap()).unwrap();
        sh(
            &s.0,
            &["clone", origin.to_str().unwrap(), clone.to_str().unwrap()],
        );
        clone
    }

    #[test]
    fn a_scan_finds_checkouts_reads_their_remote_and_skips_build_output() {
        let s = Scratch::new("scan");
        let api = s.0.join("code/team-a/api");
        let web = s.0.join("code/team-a/web");
        let deep = s.0.join("code/a/b/c/d/e/deep");
        let hidden = s.0.join("code/node_modules/dep");
        for (dir, url) in [
            (&api, "git@gitlab.com:acme/team-a/api.git"),
            (&web, "https://gitlab.com/acme/team-a/web.git"),
            (&deep, "git@gitlab.com:acme/deep.git"),
            (&hidden, "git@gitlab.com:acme/dep.git"),
        ] {
            std::fs::create_dir_all(dir.join(".git")).unwrap();
            std::fs::write(
                dir.join(".git/config"),
                format!("[remote \"origin\"]\n\turl = {url}\n"),
            )
            .unwrap();
            std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        }
        std::fs::write(api.join("build.gradle"), "").unwrap();
        // A worktree: `.git` is a file pointing at the main checkout's.
        let wt = s.0.join("code/api-wt");
        let wt_git = api.join(".git/worktrees/api-wt");
        std::fs::create_dir_all(&wt_git).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
        std::fs::write(wt_git.join("commondir"), "../..\n").unwrap();
        std::fs::write(wt_git.join("HEAD"), "ref: refs/heads/feature/x\n").unwrap();

        let root = s.0.join("code").display().to_string();
        let idx = scan(&[root, "/radar/no/such/folder".into()], 4);
        let found: Vec<(&str, &str)> = idx
            .checkouts
            .iter()
            .map(|c| (c.project.as_str(), c.branch.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                ("acme/team-a/api", "feature/x"),
                ("acme/team-a/api", "main"),
                ("acme/team-a/web", "main"),
            ],
            "the worktree, the checkouts; not too deep, not in node_modules"
        );
        assert_eq!(idx.missing, ["/radar/no/such/folder"]);
        let api_c = idx
            .checkouts
            .iter()
            .find(|c| c.dir.ends_with("team-a/api"))
            .unwrap();
        assert_eq!(Local::default().type_of(api_c).unwrap().name, "java");
    }

    #[test]
    fn checking_out_a_branch_fetches_it_and_refuses_a_dirty_tree() {
        let s = Scratch::new("checkout");
        let clone = repos(&s);
        // Uncommitted work: refused, nothing changed.
        std::fs::write(clone.join("pom.xml"), "<changed/>").unwrap();
        let err = check_out(&clone, "feature/x").unwrap_err();
        assert!(err.contains("1 uncommitted change"), "{err}");
        assert_eq!(read_checkout(&clone).unwrap().branch, "main");
        sh(&clone, &["checkout", "--", "pom.xml"]);
        // Untracked files do not count.
        std::fs::write(clone.join("notes.txt"), "mine").unwrap();
        let said = check_out(&clone, "feature/x").unwrap();
        assert!(said.contains("new, tracking"), "{said}");
        assert_eq!(read_checkout(&clone).unwrap().branch, "feature/x");
        // Again: the existing branch, fast-forwarded.
        sh(&clone, &["checkout", "main"]);
        let said = check_out(&clone, "feature/x").unwrap();
        assert!(said.contains("up to date"), "{said}");
        // A branch origin does not have.
        let err = check_out(&clone, "no-such-branch").unwrap_err();
        assert!(err.starts_with("could not fetch no-such-branch"), "{err}");
        assert!(check_out(&clone, "--force").is_err());
    }
}
