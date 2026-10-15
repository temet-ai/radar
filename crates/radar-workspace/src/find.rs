//! Finding clones and workspaces on this machine, without running git: a
//! folder with a `.git` is a clone, and its `origin` is read from the
//! file. A workspace is found as a git repo whose folder holds other clones.

use std::path::{Path, PathBuf};

/// Bounded evidence from one read pass. Never cached or copied from file content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Observation {
    pub errors: u32,
    pub cutoffs: u32,
}

impl Observation {
    pub fn error(&mut self) {
        self.errors = self.errors.saturating_add(1);
    }

    pub fn cutoff(&mut self) {
        self.cutoffs = self.cutoffs.saturating_add(1);
    }

    pub fn incomplete(self) -> bool {
        self.errors != 0 || self.cutoffs != 0
    }

    fn optional_error(&mut self, error: &std::io::Error) {
        if error.kind() != std::io::ErrorKind::NotFound {
            self.error();
        }
    }
}

/// Missing optional configuration is expected; unreadable configuration is not.
pub(crate) fn optional_text(path: &Path, observed: &mut Observation) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(error) => {
            observed.optional_error(&error);
            None
        }
    }
}

pub(crate) fn scan_dir(
    path: &Path,
    required: bool,
    observed: &mut Observation,
) -> Option<std::fs::ReadDir> {
    match std::fs::read_dir(path) {
        Ok(entries) => Some(entries),
        Err(error) => {
            if required {
                observed.error();
            } else {
                observed.optional_error(&error);
            }
            None
        }
    }
}

fn is_dir(path: &Path, required: bool, observed: &mut Observation) -> bool {
    match std::fs::metadata(path) {
        Ok(metadata) => metadata.is_dir(),
        Err(error) => {
            if required {
                observed.error();
            } else {
                observed.optional_error(&error);
            }
            false
        }
    }
}

fn clone_observed(path: &Path, observed: &mut Observation) -> bool {
    match std::fs::metadata(path.join(".git")) {
        Ok(_) => true,
        Err(error) => {
            observed.optional_error(&error);
            false
        }
    }
}

/// Folders the look never goes into: build output and dependencies. Hidden
/// folders are skipped as well.
const SKIP: [&str; 8] = [
    "node_modules",
    "target",
    "build",
    "dist",
    "out",
    "vendor",
    "venv",
    "__pycache__",
];

/// The most folders one look reads.
const MOST_FOLDERS: usize = 20_000;

/// How far down clones are looked for.
pub const DEPTH: usize = 6;

/// A clone found under a workspace.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Repo {
    pub dir: String,
    /// The repo path its `origin` names (`acme/team-a/api`), or empty.
    pub project: String,
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

/// The repo path a remote URL names, in any of git's forms.
pub fn remote_project(url: &str) -> Option<String> {
    let url = url.trim();
    let path = if let Some((_, rest)) = url.split_once("://") {
        rest.split_once('/')?.1
    } else {
        let (host, path) = url.split_once(':')?;
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

/// The folder that holds a clone's `config`: its own `.git`, or for a
/// worktree the main clone's.
fn config_of(dir: &Path, observed: &mut Observation) -> Option<PathBuf> {
    let dot = dir.join(".git");
    if is_dir(&dot, true, observed) {
        return Some(dot.join("config"));
    }
    let text = optional_text(&dot, observed)?;
    let Some(rel) = text.lines().find_map(|l| l.strip_prefix("gitdir:")) else {
        observed.error();
        return None;
    };
    let rel = rel.trim();
    if rel.is_empty() {
        // Keep the existing fallback path, but this pointer cannot establish
        // a complete read even when that fallback happens to find a config.
        observed.error();
    }
    let git = if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        dir.join(rel)
    };
    let common = match optional_text(&git.join("commondir"), observed) {
        Some(c) if Path::new(c.trim()).is_absolute() => PathBuf::from(c.trim()),
        Some(c) => git.join(c.trim()),
        None => git,
    };
    Some(common.join("config"))
}

/// Whether `dir` is a git checkout (or a worktree of one).
pub fn is_clone(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// A clone read from its `.git`; `None` without an `origin`.
fn read_repo(dir: &Path, observed: &mut Observation) -> Option<Repo> {
    let remote = origin_url(&optional_text(&config_of(dir, observed)?, observed)?)?;
    Some(Repo {
        dir: dir.display().to_string(),
        project: remote_project(&remote).unwrap_or_default(),
    })
}

/// The clones under `top` (not `top` itself), `depth` folders down, in path
/// order. A clone is not looked into for more.
pub fn clones(top: &Path, depth: usize) -> Vec<Repo> {
    clones_observed(top, depth, &mut Observation::default())
}

pub(crate) fn clones_observed(top: &Path, depth: usize, observed: &mut Observation) -> Vec<Repo> {
    clones_limited(top, depth, MOST_FOLDERS, observed)
}

fn clones_limited(top: &Path, depth: usize, most: usize, observed: &mut Observation) -> Vec<Repo> {
    let mut found = Vec::new();
    let mut seen = 0usize;
    let mut todo = vec![(top.to_path_buf(), 0usize)];
    while let Some((dir, level)) = todo.pop() {
        let Some(entries) = scan_dir(&dir, level != 0, observed) else {
            continue;
        };
        for entry in entries {
            let e = match entry {
                Ok(e) => e,
                Err(_) => {
                    observed.error();
                    continue;
                }
            };
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.')
                || SKIP.contains(&name.as_str())
                || !is_dir(&path, true, observed)
            {
                continue;
            }
            seen += 1;
            if seen > most {
                observed.cutoff();
                found.sort_by(|a: &Repo, b| a.dir.cmp(&b.dir));
                return found;
            }
            if clone_observed(&path, observed) {
                if let Some(repo) = read_repo(&path, observed) {
                    found.push(repo);
                }
            } else if level + 1 < depth {
                todo.push((path, level + 1));
            } else {
                observed.cutoff();
            }
        }
    }
    found.sort_by(|a, b| a.dir.cmp(&b.dir));
    found
}

/// A workspace radar found: a git repo whose folder holds other clones.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Offer {
    pub dir: String,
    pub name: String,
    /// How many clones are under it.
    pub clones: usize,
    /// Where they are under it (`repositories`), or empty.
    pub repos: String,
}

/// Where a workspace might be: the folders above this one (below your home
/// folder), and the folders in the places people keep work in.
#[cfg(test)]
pub fn candidates(cwd: &Path, home: &Path) -> Vec<PathBuf> {
    candidates_observed(cwd, home, &mut Observation::default())
}

pub(crate) fn candidates_observed(
    cwd: &Path,
    home: &Path,
    observed: &mut Observation,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut up = Some(cwd);
    while let Some(d) = up {
        if d == home || d.parent().is_none() {
            break;
        }
        out.push(d.to_path_buf());
        up = d.parent();
    }
    for place in ["work", "code", "projects", "dev", "src"] {
        let base = home.join(place);
        let Some(entries) = scan_dir(&base, false, observed) else {
            continue;
        };
        let mut dirs = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    observed.error();
                    continue;
                }
            };
            let path = entry.path();
            if !entry.file_name().to_string_lossy().starts_with('.')
                && is_dir(&path, true, observed)
            {
                dirs.push(path);
            }
        }
        dirs.sort();
        out.extend(dirs);
    }
    out.dedup();
    out
}

/// Preserve discovery from either available context source. Missing context
/// makes the observation incomplete, even when the other source finds offers.
pub(crate) fn discover(
    cwd: Option<&Path>,
    home: Option<&Path>,
    known: &[PathBuf],
) -> (Vec<Offer>, Observation) {
    let mut observed = Observation::default();
    if cwd.is_none() {
        observed.error();
    }
    if home.is_none() {
        observed.error();
    }
    let places = candidates_observed(
        cwd.unwrap_or_else(|| Path::new("")),
        home.unwrap_or_else(|| Path::new("")),
        &mut observed,
    );
    let offers = workspaces_observed(&places, known, &mut observed);
    (offers, observed)
}

/// The workspaces among `places` that are not `known` yet: each a git repo
/// that holds at least one other clone, its clones under `repositories` when
/// it has that folder.
#[cfg(test)]
pub fn workspaces(places: &[PathBuf], known: &[PathBuf]) -> Vec<Offer> {
    workspaces_observed(places, known, &mut Observation::default())
}

pub(crate) fn workspaces_observed(
    places: &[PathBuf],
    known: &[PathBuf],
    observed: &mut Observation,
) -> Vec<Offer> {
    let mut out: Vec<Offer> = Vec::new();
    for dir in places {
        if known.contains(dir)
            || out.iter().any(|o| Path::new(&o.dir) == dir)
            || !clone_observed(dir, observed)
        {
            continue;
        }
        let repos = if is_dir(&dir.join("repositories"), false, observed) {
            "repositories"
        } else {
            ""
        };
        let n = clones_observed(&dir.join(repos), DEPTH, observed).len();
        if n == 0 {
            continue;
        }
        out.push(Offer {
            dir: dir.display().to_string(),
            name: dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            clones: n,
            repos: repos.to_string(),
        });
    }
    out
}

/// What a folder holds, as a workspace: said while its folder is typed,
/// and what adding it fills in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Described {
    /// Where its clones are under it (`repositories`), or empty: itself.
    pub repos: String,
    pub clones: usize,
    /// It has a `.okf.yaml` (notes found and made as it says).
    pub okf: bool,
    /// The ticket projects (`ABC`) its notes are about, most first: from
    /// the folders named `<KEY>…` under `artifacts/`.
    pub projects: Vec<String>,
}

impl Described {
    /// In a few words: `14 clones under repositories/ · notes by .okf.yaml ·
    /// tickets ABC, DEF`.
    pub fn words(&self) -> String {
        let mut out = vec![match (self.clones, self.repos.as_str()) {
            (0, _) => "no clones under it".to_string(),
            (n, "") => format!("{n} clone{} in it", if n == 1 { "" } else { "s" }),
            (n, r) => format!("{n} clone{} under {r}/", if n == 1 { "" } else { "s" }),
        }];
        if self.okf {
            out.push("notes by .okf.yaml".into());
        }
        if !self.projects.is_empty() {
            out.push(format!("tickets {}", self.projects.join(", ")));
        }
        out.join(" · ")
    }
}

/// Where a workspace's clones are kept, by the names people use for them.
const CLONE_FOLDERS: [&str; 4] = ["repositories", "repos", "projects", "code"];

/// Whether `top` looks like a workspace worth looking into: a git repo, or
/// a folder with one of the folders clones are kept in. Cheap: two checks,
/// so a path being typed (`/`, `/tmp`) is never walked.
pub fn worth_describing(top: &Path) -> bool {
    is_clone(top) || CLONE_FOLDERS.iter().any(|f| top.join(f).is_dir())
}

/// What `top` holds as a workspace: where its clones are and how many, its
/// notes, and the ticket projects its notes are about.
pub fn describe(top: &Path) -> Described {
    let repos = CLONE_FOLDERS
        .iter()
        .find(|f| top.join(f).is_dir())
        .map_or(String::new(), |f| f.to_string());
    let clones = clones(&top.join(&repos), DEPTH).len();
    Described {
        repos,
        clones,
        okf: top.join(".okf.yaml").is_file(),
        projects: projects_in(&top.join("artifacts")),
    }
}

/// The projects of the ticket keys that folders under `dir` are named by
/// (`ABC-12-stats`, `ABC-12`), three folders down, most named first.
fn projects_in(dir: &Path) -> Vec<String> {
    let mut count: std::collections::BTreeMap<String, usize> = Default::default();
    let mut todo = vec![(dir.to_path_buf(), 0)];
    while let Some((d, level)) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten().filter(|e| e.path().is_dir()) {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(project) = key_project(&name) {
                *count.entry(project).or_default() += 1;
            } else if level < 2 {
                todo.push((e.path(), level + 1));
            }
        }
    }
    let mut v: Vec<(String, usize)> = count.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().map(|(p, _)| p).collect()
}

/// `ABC` of a name that starts with a ticket key, `ABC-12…`.
fn key_project(name: &str) -> Option<String> {
    let (project, rest) = name.split_once('-')?;
    let starts = project.chars().next().is_some_and(|c| c.is_ascii_uppercase());
    let word = project.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    let num = rest.chars().take_while(char::is_ascii_digit).count();
    (starts && word && project.len() >= 2 && num > 0 && !rest[num..].starts_with(|c: char| c.is_ascii_alphanumeric()))
        .then(|| project.to_string())
}

#[cfg(test)]
pub(crate) mod fixture {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    /// A scratch folder under the temp dir, removed on drop.
    pub struct Scratch(pub PathBuf);

    #[test]
    fn equally_named_fixtures_do_not_remove_each_others_files() {
        let first = Scratch::new("same-name");
        first.file("README.md", "keep me");
        let second = Scratch::new("same-name");
        assert_ne!(first.0, second.0);
        assert_eq!(
            std::fs::read_to_string(first.path("README.md")).unwrap(),
            "keep me"
        );
        drop(second);
        assert!(first.path("README.md").exists());
    }

    impl Scratch {
        pub fn new(name: &str) -> Self {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("radar-workspace-{name}-{}-{id}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        pub fn path(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }

        /// A file with `text`, its folders made.
        pub fn file(&self, rel: &str, text: &str) -> PathBuf {
            let p = self.path(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
            p
        }

        /// A clone with an `origin` at `rel`, made by hand (no git run).
        pub fn clone(&self, rel: &str, remote: &str) -> PathBuf {
            self.file(
                &format!("{rel}/.git/config"),
                &format!("[core]\n\tbare = false\n[remote \"origin\"]\n\turl = {remote}\n"),
            );
            self.file(&format!("{rel}/.git/HEAD"), "ref: refs/heads/main\n");
            self.path(rel)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::Scratch;
    use super::*;

    #[test]
    fn remotes_name_their_repo_in_every_form() {
        for url in [
            "https://example.com/acme/team-a/api.git",
            "ssh://git@example.com:2222/acme/team-a/api.git",
            "git@example.com:acme/team-a/api.git",
        ] {
            assert_eq!(
                remote_project(url).as_deref(),
                Some("acme/team-a/api"),
                "{url}"
            );
        }
        assert_eq!(remote_project("/srv/git/api"), None);
    }

    #[test]
    fn clones_are_found_at_any_depth_without_running_git() {
        let s = Scratch::new("clones");
        s.clone(
            "repositories/team-a/api",
            "git@example.com:acme/team-a/api.git",
        );
        s.clone(
            "repositories/team-a/web",
            "git@example.com:acme/team-a/web.git",
        );
        s.clone("repositories/deep/er/docs", "git@example.com:acme/docs.git");
        s.file(
            "repositories/node_modules/x/.git/config",
            "[remote \"origin\"]\n url = git@e.com:a/b.git\n",
        );
        s.file("repositories/notes.txt", "x");
        let found = clones(&s.path("repositories"), DEPTH);
        let names: Vec<&str> = found.iter().map(|r| r.project.as_str()).collect();
        assert_eq!(names, ["acme/docs", "acme/team-a/api", "acme/team-a/web"]);
        assert!(
            clones(&s.path("repositories"), 2).len() == 2,
            "depth bounds the look"
        );
    }

    #[test]
    fn a_workspace_is_a_git_repo_that_holds_other_clones() {
        let s = Scratch::new("offer");
        // A workspace of its own, with clones under repositories/.
        s.clone("work/acme", "git@example.com:acme/workspace.git");
        s.clone(
            "work/acme/repositories/a/api",
            "git@example.com:acme/a/api.git",
        );
        s.clone(
            "work/acme/repositories/a/web",
            "git@example.com:acme/a/web.git",
        );
        // A lone clone is not a workspace.
        s.clone("work/solo", "git@example.com:me/solo.git");
        // A folder of clones with no repo at the top is not one either.
        s.clone("work/plain/x", "git@example.com:me/x.git");
        let places = vec![
            s.path("work/acme"),
            s.path("work/solo"),
            s.path("work/plain"),
        ];
        let offers = workspaces(&places, &[]);
        assert_eq!(offers.len(), 1, "{offers:?}");
        assert_eq!(
            (
                offers[0].name.as_str(),
                offers[0].clones,
                offers[0].repos.as_str()
            ),
            ("acme", 2, "repositories")
        );
        assert!(
            workspaces(&places, &[s.path("work/acme")]).is_empty(),
            "known ones are not offered"
        );
    }

    #[test]
    fn places_to_look_are_the_folders_above_and_the_places_work_lives() {
        let s = Scratch::new("places");
        std::fs::create_dir_all(s.path("home/work/acme")).unwrap();
        std::fs::create_dir_all(s.path("home/work/.hidden")).unwrap();
        std::fs::create_dir_all(s.path("home/work/acme/repositories/api")).unwrap();
        let home = s.path("home");
        let got = candidates(&s.path("home/work/acme/repositories/api"), &home);
        assert!(got.contains(&s.path("home/work/acme")));
        assert!(got.contains(&s.path("home/work/acme/repositories")));
        assert!(!got.contains(&home), "home itself is not a workspace");
        assert!(!got.iter().any(|p| p.ends_with(".hidden")));
    }

    #[test]
    fn discovery_distinguishes_optional_absence_from_unreadable_entries() {
        let s = Scratch::new("discovery-evidence");
        std::fs::create_dir(s.path("home")).unwrap();
        let mut observed = Observation::default();
        let places = candidates_observed(&s.path("home"), &s.path("home"), &mut observed);
        assert!(places.is_empty());
        assert_eq!(observed, Observation::default());
        s.file("home/work", "a file is not a readable search directory");
        candidates_observed(&s.path("home"), &s.path("home"), &mut observed);
        assert_eq!(observed.errors, 1);
        assert!(scan_dir(&s.path("gone-child"), true, &mut observed).is_none());
        assert_eq!(
            observed.errors, 2,
            "a queued child disappearing is an incomplete traversal"
        );
    }

    #[test]
    fn clone_scan_observes_limits_and_errors_but_not_missing_origin() {
        let s = Scratch::new("clone-evidence");
        s.clone("one", "git@example.com:acme/one.git");
        s.clone("two", "git@example.com:acme/two.git");
        let mut observed = Observation::default();
        assert_eq!(clones_limited(&s.0, DEPTH, 1, &mut observed).len(), 1);
        assert_eq!(observed.cutoffs, 1);
        let s = Scratch::new("clone-depth-evidence");
        s.clone("nested/one", "git@example.com:acme/one.git");
        let mut observed = Observation::default();
        assert!(clones_limited(&s.0, 1, MOST_FOLDERS, &mut observed).is_empty());
        assert_eq!(observed.cutoffs, 1);
        let s = Scratch::new("clone-read-evidence");
        s.file("no-origin/.git/config", "[core]\n bare = false\n");
        std::fs::create_dir_all(s.path("broken/.git/config")).unwrap();
        let mut observed = Observation::default();
        assert!(clones_observed(&s.0, DEPTH, &mut observed).is_empty());
        assert_eq!(observed.errors, 1);
    }

    #[test]
    fn missing_cwd_keeps_home_discovery_and_reports_incomplete_context() {
        let s = Scratch::new("discovery-context");
        s.clone("home/work/workspace", "git@example.com:acme/workspace.git");
        s.clone(
            "home/work/workspace/repositories/repo",
            "git@example.com:acme/repo.git",
        );
        let (offers, observed) = discover(None, Some(&s.path("home")), &[]);
        assert_eq!(offers.len(), 1);
        assert_eq!(observed.errors, 1);
    }

    #[test]
    fn empty_git_pointer_is_incomplete_without_changing_legacy_fallback() {
        let s = Scratch::new("empty-git-pointer");
        s.file("repo/.git", "gitdir:   ");
        s.file(
            "repo/config",
            "[remote \"origin\"]\n url = git@example.com:acme/repo.git\n",
        );
        let mut observed = Observation::default();
        assert_eq!(clones_observed(&s.0, DEPTH, &mut observed).len(), 1);
        assert_eq!(observed.errors, 1);
    }
}
