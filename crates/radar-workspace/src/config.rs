//! The `[workspace]` table: the workspaces you work from.
//!
//! ```toml
//! [[workspace.space]]
//! dir = "~/work/acme"
//! name = "acme"
//! tickets = ["ABC"]           # the tracker projects worked here
//! repos = "repositories"      # where its clones are, under dir ("" is dir itself)
//! open_at = "top"             # e and t open at the top, so every repo is in view
//! notes = "okf"               # read .okf.yaml, or a path template
//! ```

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The `[workspace]` table.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Workspaces {
    #[serde(default, rename = "space", skip_serializing_if = "Vec::is_empty")]
    pub spaces: Vec<Space>,
}

/// One workspace: a folder that holds the clones you work across.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Space {
    /// Its folder; `~` is your home folder.
    pub dir: String,
    /// What it is called; empty: the folder's name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The tracker projects (`ABC`) worked here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tickets: Vec<String>,
    /// Where its clones are, under `dir`; empty: `dir` itself.
    #[serde(default = "default_repos", skip_serializing_if = "is_default_repos")]
    pub repos: String,
    /// `top`: the workspace's folder; `repo`: the ticket's notes folder.
    #[serde(
        default = "default_open_at",
        skip_serializing_if = "is_default_open_at"
    )]
    pub open_at: String,
    /// A command run after a worktree is made (T7/T4 use it).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub after_worktree: String,
    /// `okf`: read `.okf.yaml`; or a path template (`artifacts/{date}/{key}-{slug}`).
    #[serde(default = "default_notes", skip_serializing_if = "is_default_notes")]
    pub notes: String,
    /// The editor's command line; empty: radar asks which installed one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub editor: String,
}

pub const OPEN_AT: [&str; 2] = ["top", "repo"];

fn default_repos() -> String {
    "repositories".into()
}

fn is_default_repos(s: &String) -> bool {
    s == "repositories"
}

fn default_open_at() -> String {
    "top".into()
}

fn is_default_open_at(s: &String) -> bool {
    s == "top"
}

fn default_notes() -> String {
    "okf".into()
}

fn is_default_notes(s: &String) -> bool {
    s == "okf"
}

impl Space {
    pub fn new(dir: impl Into<String>) -> Self {
        Self {
            dir: dir.into(),
            name: String::new(),
            tickets: Vec::new(),
            repos: default_repos(),
            open_at: default_open_at(),
            after_worktree: String::new(),
            notes: default_notes(),
            editor: String::new(),
        }
    }

    /// Its folder on this machine.
    pub fn top(&self) -> PathBuf {
        expand(&self.dir)
    }

    /// The folder its clones are under.
    pub fn clones_dir(&self) -> PathBuf {
        let repos = self.repos.trim().trim_matches('/');
        if repos.is_empty() {
            self.top()
        } else {
            self.top().join(repos)
        }
    }

    /// What it is called: its name, else its folder's.
    pub fn label(&self) -> String {
        if !self.name.trim().is_empty() {
            return self.name.trim().to_string();
        }
        self.top()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.dir.clone())
    }

    /// Whether tickets of tracker project `project` are worked here.
    pub fn serves(&self, project: &str) -> bool {
        self.tickets.iter().any(|t| t.eq_ignore_ascii_case(project))
    }
}

impl Workspaces {
    pub fn is_default(&self) -> bool {
        self.spaces.is_empty()
    }

    /// The workspace tickets of `project` are worked in: the one that lists
    /// it, else the first.
    pub fn for_project(&self, project: &str) -> Option<&Space> {
        self.spaces
            .iter()
            .find(|s| s.serves(project))
            .or(self.spaces.first())
    }

    /// Put things right after reading: an `open_at` that is not one of the
    /// two becomes `top`.
    pub fn tidy(&mut self) {
        for s in &mut self.spaces {
            if !OPEN_AT.contains(&s.open_at.as_str()) {
                s.open_at = default_open_at();
            }
            s.tickets.retain(|t| !t.trim().is_empty());
        }
    }
}

fn home() -> Option<PathBuf> {
    dirs::home_dir()
}

/// `~/code` → your home folder's `code`; anything else as it is.
pub fn expand(path: &str) -> PathBuf {
    let path = path.trim();
    match (
        path.strip_prefix("~/").or((path == "~").then_some("")),
        home(),
    ) {
        (Some(""), Some(h)) => h,
        (Some(rest), Some(h)) => h.join(rest),
        _ => PathBuf::from(path),
    }
}

/// The same local folder identity for configured, relative and displayed paths.
/// This expands home and resolves relative paths without reading their contents.
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
                Some(rest) if rest.starts_with('/') || rest.starts_with('\\') => format!("~{rest}"),
                _ => path.to_string(),
            }
        }
        None => path.to_string(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_round_trips_and_leaves_defaults_out() {
        let text = r#"
[[space]]
dir = "~/work/acme"
name = "acme"
tickets = ["ABC"]
after_worktree = "sync.sh"

[[space]]
dir = "/srv/other"
repos = ""
open_at = "repo"
notes = "notes/{key}"
editor = "nvim"
"#;
        let t: toml::Value = toml::from_str(text).unwrap();
        let w: Workspaces = t.try_into().unwrap();
        assert_eq!(w.spaces.len(), 2);
        assert_eq!(w.spaces[0].repos, "repositories");
        assert_eq!(w.spaces[0].notes, "okf");
        assert_eq!(w.spaces[1].repos, "");
        let out = toml::to_string(&w).unwrap();
        assert!(
            !out.contains("repositories"),
            "defaults are left out:\n{out}"
        );
        assert!(out.contains("open_at = \"repo\""), "{out}");
        let back: Workspaces = toml::from_str(&out).unwrap();
        assert_eq!(back, w);
        assert!(Workspaces::default().is_default());
    }

    #[test]
    fn the_workspace_for_a_project_is_the_one_that_lists_it_else_the_first() {
        let mut a = Space::new("/w/a");
        let mut b = Space::new("/w/b");
        a.tickets = vec!["ABC".into()];
        b.tickets = vec!["XYZ".into()];
        let w = Workspaces { spaces: vec![a, b] };
        assert_eq!(w.for_project("xyz").unwrap().dir, "/w/b");
        assert_eq!(w.for_project("ABC").unwrap().dir, "/w/a");
        assert_eq!(w.for_project("QQ").unwrap().dir, "/w/a");
    }

    #[test]
    fn a_name_defaults_to_the_folder_and_clones_are_under_repos() {
        let mut s = Space::new("/w/acme");
        assert_eq!(s.label(), "acme");
        assert_eq!(s.clones_dir(), PathBuf::from("/w/acme/repositories"));
        s.repos.clear();
        s.name = "Acme".into();
        assert_eq!(s.label(), "Acme");
        assert_eq!(s.clones_dir(), PathBuf::from("/w/acme"));
        let mut w = Workspaces { spaces: vec![s] };
        w.spaces[0].open_at = "elsewhere".into();
        w.tidy();
        assert_eq!(w.spaces[0].open_at, "top");
    }
}
