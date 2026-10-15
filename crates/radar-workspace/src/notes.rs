//! A ticket's notes: files whose frontmatter `ticket` is its key, and every
//! file in a folder named `<KEY>` or `<KEY>-…` under the workspace's
//! knowledge folders (done and archive included). `.okf.yaml` says where
//! knowledge lives (`okf.rs`); without it a folder named by the key is the
//! fallback, searched in the notes template's folder, else the workspace.
//!
//! The index is built off the event loop, at start and when asked, and kept
//! in the cache: drawing never reads a file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Space;
use crate::find::{self, Observation};
use crate::okf::{self, Okf};

/// The most files one index reads.
const MOST_FILES: usize = 20_000;
/// How deep below a knowledge folder files are looked for.
const DEEPEST: usize = 12;
/// Lines of a README kept for the ticket's section.
const LEAD_LINES: usize = 6;

/// One note of one ticket.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// The ticket's key (`ABC-12`).
    pub key: String,
    /// Its full path.
    pub path: String,
    /// Its path under the workspace.
    pub rel: String,
    /// The file's name.
    pub name: String,
    /// The ticket's folder, when it lies in one (relative to the workspace).
    pub folder: String,
    pub title: String,
    /// Frontmatter `type` (`spec`, `ticket`, …).
    pub kind: String,
    pub status: String,
    pub date: String,
    pub readme: bool,
    /// A README's first lines, for the ticket's section.
    pub lead: Vec<String>,
}

/// What an index of a workspace found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub at: i64,
    /// The workspace's folder was not there.
    pub missing: bool,
    /// A `.okf.yaml` was read.
    pub okf: bool,
    /// The knowledge folders looked in (relative; empty: the workspace).
    pub roots: Vec<String>,
    /// How many files were read.
    pub files: usize,
    /// How many clones are under the workspace's clones folder.
    pub clones: usize,
    #[serde(default)]
    pub repositories: Vec<find::Repo>,
    /// Where a new ticket's notes go (`.okf.yaml`'s create path), if told.
    pub create: String,
    pub notes: Vec<Note>,
}

/// The ticket key a folder or file name begins with: `ABC-12` for
/// `abc-12-stats`, `ABC-12`, `ABC-12_x`. The number is taken whole.
pub fn key_of(name: &str) -> Option<String> {
    let name = name.trim();
    let dash = name.find('-')?;
    let project = &name[..dash];
    if project.is_empty()
        || !project.chars().next()?.is_ascii_alphabetic()
        || !project.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return None;
    }
    let digits: String = name[dash + 1..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        return None;
    }
    let end = dash + 1 + digits.len();
    match name[end..].chars().next() {
        None | Some('-' | '_' | ' ' | '.') => {
            Some(format!("{}-{digits}", project.to_ascii_uppercase()))
        }
        _ => None,
    }
}

/// The key a frontmatter value names: `ABC-12`, or a link ending in it.
pub fn key_in(value: &str) -> Option<String> {
    value
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .filter(|t| !t.is_empty())
        .find_map(|t| key_of(t).filter(|k| k.eq_ignore_ascii_case(t)))
}

/// The flat `key: value` lines of a file's frontmatter, and the text after.
pub fn frontmatter(text: &str) -> (Vec<(String, String)>, Vec<&str>) {
    let mut lines = text.lines();
    let mut fields = Vec::new();
    if lines.clone().next().map(str::trim) != Some("---") {
        return (fields, text.lines().collect());
    }
    lines.next();
    let mut closed = false;
    for l in lines.by_ref() {
        if l.trim() == "---" {
            closed = true;
            break;
        }
        if l.starts_with(char::is_whitespace) || l.trim_start().starts_with('-') {
            continue;
        }
        if let Some((k, v)) = l.split_once(':') {
            let v = v.trim();
            let v = v.trim_matches(|c| c == '"' || c == '\'');
            fields.push((k.trim().to_ascii_lowercase(), v.to_string()));
        }
    }
    if !closed {
        return (Vec::new(), text.lines().collect());
    }
    (fields, lines.collect())
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> &'a str {
    fields
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .unwrap_or_default()
}

/// A note read from `text`, for `key`; the ticket's folder (relative) if any.
fn read_note(key: &str, path: &Path, rel: &str, folder: &str, text: &str) -> Note {
    let (fields, body) = frontmatter(text);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let readme = name.eq_ignore_ascii_case("readme.md");
    let heading = body
        .iter()
        .find_map(|l| l.strip_prefix("# "))
        .map(|h| h.trim().to_string())
        .unwrap_or_default();
    let title = match field(&fields, "title") {
        "" if heading.is_empty() => name.trim_end_matches(".md").to_string(),
        "" => heading,
        t => t.to_string(),
    };
    let lead = if readme {
        body.iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .take(LEAD_LINES)
            .map(|l| l.chars().take(120).collect())
            .collect()
    } else {
        Vec::new()
    };
    Note {
        key: key.to_string(),
        path: path.display().to_string(),
        rel: rel.to_string(),
        name,
        folder: folder.to_string(),
        title,
        kind: field(&fields, "type").to_string(),
        status: field(&fields, "status").to_string(),
        date: field(&fields, "date").to_string(),
        readme,
        lead,
    }
}

/// The knowledge folders and how files are told to be knowledge, for `space`.
fn plan(
    space: &Space,
    top: &Path,
    observed: &mut Observation,
) -> (Option<Okf>, Vec<String>, String) {
    let okf = if space.notes.trim() == "okf" {
        match okf::read(top) {
            Ok(okf) => Some(okf),
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    observed.error();
                }
                None
            }
        }
    } else {
        None
    };
    let create = okf
        .as_ref()
        .and_then(|o| o.artifacts_path.clone())
        .unwrap_or_default();
    if let Some(o) = &okf {
        let roots = o.roots();
        if !roots.is_empty() {
            return (okf, roots, create);
        }
    }
    // A notes template names its folder: everything before the first `{`.
    let template = space.notes.trim();
    let roots = if template == "okf" || template.is_empty() {
        vec![String::new()]
    } else {
        let folder = okf::static_prefix(template);
        vec![folder]
    };
    (None, roots, create)
}

/// Build the index of `space`: reads the disk, so run it off the event loop.
#[cfg(test)]
pub fn build(space: &Space, now: i64) -> Index {
    build_observed(space, now).0
}

/// Read evidence travels with the task reply, never through the cache schema.
pub(crate) fn build_observed(space: &Space, now: i64) -> (Index, Observation) {
    build_limited(space, now, MOST_FILES, DEEPEST)
}

fn build_limited(
    space: &Space,
    now: i64,
    most_files: usize,
    deepest: usize,
) -> (Index, Observation) {
    let mut observed = Observation::default();
    let top = space.top();
    let mut index = Index {
        at: now,
        ..Index::default()
    };
    match std::fs::metadata(&top) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            index.missing = true;
            return (index, observed);
        }
        Err(error) => {
            index.missing = true;
            if error.kind() != std::io::ErrorKind::NotFound {
                observed.error();
            }
            return (index, observed);
        }
    }
    index.repositories = find::clones_observed(&space.clones_dir(), find::DEPTH, &mut observed);
    index.clones = index.repositories.len();
    let (okf, roots, create) = plan(space, &top, &mut observed);
    index.okf = okf.is_some();
    index.create = create;
    index.roots = roots.clone();
    // The clones folder is never searched when the whole workspace is.
    let repos = space.repos.trim().trim_matches('/').to_string();
    let mut seen = BTreeSet::new();
    let mut files = 0usize;
    for root in &roots {
        let base = if root.is_empty() {
            top.clone()
        } else {
            top.join(root)
        };
        walk(
            &base,
            &top,
            &repos,
            okf.as_ref(),
            &mut index.notes,
            &mut seen,
            &mut files,
            &mut observed,
            most_files,
            deepest,
        );
    }
    index.files = files;
    index
        .notes
        .sort_by(|a, b| a.key.cmp(&b.key).then_with(|| order(a).cmp(&order(b))));
    (index, observed)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    base: &Path,
    top: &Path,
    repos: &str,
    okf: Option<&Okf>,
    notes: &mut Vec<Note>,
    seen: &mut BTreeSet<(String, String)>,
    files: &mut usize,
    observed: &mut Observation,
    most_files: usize,
    deepest: usize,
) {
    // The ticket folder a folder is inside: its key and its path.
    type Inside = Option<(String, String)>;
    let mut todo: Vec<(PathBuf, usize, Inside)> = vec![(base.to_path_buf(), 0, None)];
    while let Some((dir, depth, ticket)) = todo.pop() {
        let Some(entries) = find::scan_dir(&dir, depth != 0, observed) else {
            continue;
        };
        let mut entries: Vec<_> = entries
            .filter_map(|entry| match entry {
                Ok(entry) => Some(entry),
                Err(_) => {
                    observed.error();
                    None
                }
            })
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            let rel = path
                .strip_prefix(top)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            let is_dir = match std::fs::metadata(&path) {
                Ok(metadata) => metadata.is_dir(),
                Err(_) => {
                    observed.error();
                    // Preserve the existing path.is_dir() fallback: a failed
                    // stat can still produce a filename-only ticket note.
                    false
                }
            };
            if is_dir {
                if !repos.is_empty() && rel == repos {
                    continue;
                }
                if depth >= deepest {
                    observed.cutoff();
                    continue;
                }
                let inside = key_of(&name)
                    .map(|k| (k, rel.clone()))
                    .or_else(|| ticket.clone());
                todo.push((path, depth + 1, inside));
                continue;
            }
            *files += 1;
            if *files > most_files {
                observed.cutoff();
                return;
            }
            let is_md = name.to_ascii_lowercase().ends_with(".md");
            let known = okf.is_none_or(|o| o.holds(&rel));
            let mut keys: Vec<(String, String)> = Vec::new();
            if let Some((k, folder)) = &ticket {
                keys.push((k.clone(), folder.clone()));
            }
            let text = if is_md && (known || ticket.is_some()) {
                match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        let mut chars = text.chars();
                        let prefix = chars.by_ref().take(8_000).collect::<String>();
                        if chars.next().is_some() {
                            observed.cutoff();
                        }
                        Some(prefix)
                    }
                    Err(_) => {
                        observed.error();
                        None
                    }
                }
            } else {
                None
            };
            if let Some(text) = &text {
                let (fields, _) = frontmatter(text);
                for f in ["ticket", "resource"] {
                    if let Some(k) = key_in(field(&fields, f))
                        && !keys.iter().any(|(have, _)| *have == k)
                        && known
                    {
                        keys.push((k, String::new()));
                    }
                }
            }
            for (k, folder) in keys {
                if !seen.insert((k.clone(), rel.clone())) {
                    continue;
                }
                notes.push(read_note(
                    &k,
                    &path,
                    &rel,
                    &folder,
                    text.as_deref().unwrap_or_default(),
                ));
            }
        }
    }
}

/// The order of a ticket's notes: its README first, then by type, status
/// and name.
fn order(n: &Note) -> (bool, String, String, String, String) {
    (
        !n.readme,
        // Notes with no type come after those with one.
        format!("{}{}", u8::from(n.kind.is_empty()), n.kind.to_lowercase()),
        n.status.to_lowercase(),
        n.name.to_lowercase(),
        n.rel.clone(),
    )
}

impl Index {
    /// A ticket's notes, README first.
    pub fn for_ticket(&self, key: &str) -> Vec<&Note> {
        let mut v: Vec<&Note> = self
            .notes
            .iter()
            .filter(|n| n.key.eq_ignore_ascii_case(key))
            .collect();
        v.sort_by_key(|n| order(n));
        v
    }

    /// The folder a ticket's notes lie in: its README's, else the first
    /// note's folder, relative to the workspace.
    pub fn folder_of(&self, key: &str) -> Option<String> {
        self.for_ticket(key)
            .into_iter()
            .map(|n| n.folder.clone())
            .find(|f| !f.is_empty())
    }
}

/// A note's line in the Notes tab: `spec · draft · spec-backend.md`.
pub fn line(n: &Note) -> String {
    let kind = if n.kind.is_empty() && n.readme {
        "readme"
    } else {
        n.kind.as_str()
    };
    [kind, n.status.as_str(), n.name.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find::fixture::Scratch;

    const README: &str = "---\ntitle: \"ABC-12: Add stats\"\ntype: ticket\nstatus: planning\ndate: 2026-01-01\nticket: ABC-12\n---\n\n# ABC-12: Add stats\n\nTicket: a story. Planned from 2026-01-01.\n\n## What we know\n";
    const SPEC: &str = "---\ntype: spec\nstatus: draft\nticket: ABC-12\n---\n# Backend\n";

    /// A workspace with nested clones, OKF notes, a ticket folder, a note
    /// elsewhere that names the ticket and one under done/.
    fn workspace() -> Scratch {
        let s = Scratch::new("notes");
        s.clone("repositories/a/api", "git@example.com:acme/a/api.git");
        s.clone("repositories/a/web", "git@example.com:acme/a/web.git");
        s.file(
            ".okf.yaml",
            "sources:\n  documents:\n    - name: docs\n      include:\n        - docs/**/*.md\n      exclude:\n        - docs/knowledge/**\n  artifacts:\n    - name: art\n      include:\n        - artifacts/**/*.md\n      exclude:\n        - \"**/done/**\"\ncreate:\n  locations:\n    artifacts:\n      path: artifacts/{date}\n",
        );
        s.file("artifacts/2026-01-01/ABC-12-stats/README.md", README);
        s.file("artifacts/2026-01-01/ABC-12-stats/spec-backend.md", SPEC);
        s.file(
            "artifacts/2026-01-01/ABC-12-stats/spec-ui.md",
            "---\ntype: spec\nstatus: approved\n---\n# UI\n",
        );
        s.file("artifacts/2026-01-01/ABC-12-stats/diagram.png", "x");
        s.file("docs/design/stats.md", "---\ntype: design\nticket: https://tracker.example/browse/ABC-12\n---\n# Stats design\n");
        s.file(
            "artifacts/done/2025-12-01/ABC-12-earlier/notes.md",
            "no frontmatter\n",
        );
        s.file(
            "artifacts/2026-01-01/ABC-123-other/README.md",
            "---\nticket: ABC-123\n---\n# other\n",
        );
        s.file("docs/knowledge/bundle/x.md", "---\nticket: ABC-12\n---\n");
        s.file("repositories/a/api/docs/ABC-12-in-a-clone/n.md", "x");
        s
    }

    fn space(s: &Scratch) -> Space {
        Space::new(s.0.display().to_string())
    }

    #[test]
    fn keys_are_read_from_folders_and_values() {
        assert_eq!(key_of("ABC-12-stats").as_deref(), Some("ABC-12"));
        assert_eq!(key_of("abc-12").as_deref(), Some("ABC-12"));
        assert_eq!(key_of("ABC-123x"), None);
        assert_eq!(key_of("stats-page"), None);
        assert_eq!(key_of("2026-01-01"), None);
        assert_eq!(
            key_in("https://tracker.example/browse/ABC-12").as_deref(),
            Some("ABC-12")
        );
        assert_eq!(key_in("\"ABC-12\"").as_deref(), Some("ABC-12"));
        assert_eq!(key_in("later"), None);
    }

    #[test]
    fn a_tickets_notes_are_its_folder_and_every_file_that_names_it_readme_first() {
        let s = workspace();
        let index = build(&space(&s), 7);
        assert!(index.okf && !index.missing);
        assert_eq!(index.clones, 2);
        assert_eq!(
            index
                .repositories
                .iter()
                .map(|r| r.project.as_str())
                .collect::<Vec<_>>(),
            ["acme/a/api", "acme/a/web"]
        );
        assert_eq!(index.create, "artifacts/{date}");
        assert_eq!(index.roots, ["artifacts", "docs"]);
        let got: Vec<String> = index
            .for_ticket("ABC-12")
            .iter()
            .map(|n| n.rel.clone())
            .collect();
        assert_eq!(
            got,
            [
                "artifacts/2026-01-01/ABC-12-stats/README.md",
                // by type, then status: design (the docs note), spec approved, spec draft, then untyped
                "docs/design/stats.md",
                "artifacts/2026-01-01/ABC-12-stats/spec-ui.md",
                "artifacts/2026-01-01/ABC-12-stats/spec-backend.md",
                "artifacts/2026-01-01/ABC-12-stats/diagram.png",
                "artifacts/done/2025-12-01/ABC-12-earlier/notes.md",
            ],
            "{got:#?}"
        );
        // ABC-123 is another ticket; the bundle and the clones are not notes.
        assert_eq!(index.for_ticket("ABC-123").len(), 1);
        assert!(
            !index
                .notes
                .iter()
                .any(|n| n.rel.contains("knowledge") || n.rel.starts_with("repositories"))
        );
    }

    #[test]
    fn a_note_carries_its_frontmatter_and_a_readme_its_lead() {
        let s = workspace();
        let index = build(&space(&s), 7);
        let all = index.for_ticket("ABC-12");
        let readme = all[0];
        assert!(readme.readme);
        assert_eq!(
            (
                readme.kind.as_str(),
                readme.status.as_str(),
                readme.date.as_str(),
                readme.title.as_str()
            ),
            ("ticket", "planning", "2026-01-01", "ABC-12: Add stats")
        );
        assert_eq!(readme.folder, "artifacts/2026-01-01/ABC-12-stats");
        assert_eq!(readme.lead[0], "# ABC-12: Add stats");
        assert_eq!(line(readme), "ticket · planning · README.md");
        let spec = all.iter().find(|n| n.name == "spec-backend.md").unwrap();
        assert_eq!(line(spec), "spec · draft · spec-backend.md");
        assert_eq!(spec.title, "Backend");
        assert!(spec.lead.is_empty(), "only a README keeps its lead");
        assert_eq!(
            line(all.iter().find(|n| n.name == "diagram.png").unwrap()),
            "diagram.png"
        );
        assert_eq!(
            index.folder_of("ABC-12").as_deref(),
            Some("artifacts/2026-01-01/ABC-12-stats")
        );
    }

    #[test]
    fn without_an_okf_file_a_folder_named_by_the_key_is_the_fallback() {
        let s = Scratch::new("fallback");
        s.clone("repositories/a/api", "git@example.com:acme/a/api.git");
        s.file("plans/ABC-12-stats/README.md", README);
        s.file("plans/ABC-12-stats/spec.md", "no frontmatter\n");
        s.file("elsewhere/ticketless.md", "---\nticket: ABC-12\n---\n# n\n");
        s.file("repositories/a/api/ABC-12-x/n.md", "x");
        let index = build(&space(&s), 1);
        assert!(!index.okf);
        let got: Vec<String> = index
            .for_ticket("ABC-12")
            .iter()
            .map(|n| n.rel.clone())
            .collect();
        assert_eq!(
            got,
            // README first; the rest have no type or status, so by name.
            [
                "plans/ABC-12-stats/README.md",
                "plans/ABC-12-stats/spec.md",
                "elsewhere/ticketless.md"
            ],
            "{got:#?}"
        );
        // A notes template narrows the look to its folder.
        let mut sp = space(&s);
        sp.notes = "plans/{key}-{slug}".into();
        let index = build(&sp, 1);
        assert_eq!(index.roots, ["plans"]);
        assert_eq!(index.for_ticket("ABC-12").len(), 2);
    }

    #[test]
    fn a_missing_workspace_is_an_empty_index_that_says_so() {
        let index = build(&Space::new("/definitely/not/here"), 3);
        assert!(index.missing);
        assert!(index.notes.is_empty());
        assert!(index.for_ticket("ABC-12").is_empty());
    }

    #[test]
    fn the_index_survives_the_cache() {
        let s = workspace();
        let index = build(&space(&s), 7);
        let back: Index = serde_json::from_value(serde_json::to_value(&index).unwrap()).unwrap();
        assert_eq!(back, index);
    }

    #[test]
    fn observed_index_keeps_cache_shape_and_reports_real_read_errors() {
        let s = Scratch::new("read-evidence");
        s.file("ABC-12/README.md", README);
        std::fs::write(s.path("ABC-12/broken.md"), [0xff]).unwrap();
        std::fs::create_dir(s.path(".okf.yaml")).unwrap();
        let (index, observed) = build_observed(&space(&s), 1);
        assert_eq!(
            observed.errors, 2,
            "one invalid UTF-8 note and an unreadable OKF"
        );
        assert!(observed.incomplete());
        assert_eq!(
            index.for_ticket("ABC-12").len(),
            2,
            "the failed note retains its existing filename-only fallback"
        );
        let json = serde_json::to_value(&index).unwrap();
        assert!(json.get("observed").is_none());
        assert!(json.get("errors").is_none());
        assert_eq!(serde_json::from_value::<Index>(json).unwrap(), index);
    }

    #[test]
    fn missing_optional_roots_and_config_are_complete_empty_reads() {
        let s = Scratch::new("optional-evidence");
        let mut space = space(&s);
        let (index, observed) = build_observed(&space, 1);
        assert!(!index.missing);
        assert_eq!(observed, Observation::default());
        space.notes = "not-created/{key}".into();
        let (_, observed) = build_observed(&space, 1);
        assert_eq!(observed, Observation::default());
    }

    #[test]
    fn file_depth_and_text_cutoffs_are_observed_without_changing_payloads() {
        let s = Scratch::new("limit-evidence");
        s.file("ABC-12/first.md", "# First");
        s.file("ABC-12/second.md", "# Second");
        let (index, observed) = build_limited(&space(&s), 1, 1, DEEPEST);
        assert_eq!(
            index.files, 2,
            "preserve the existing cutoff sentinel in the UI/cache count"
        );
        assert_eq!(index.notes.len(), 1);
        assert_eq!(observed.cutoffs, 1);
        let (index, observed) = build_limited(&space(&s), 1, MOST_FILES, 0);
        assert!(index.notes.is_empty());
        assert_eq!(observed.cutoffs, 1);
        let s = Scratch::new("text-evidence");
        s.file("ABC-12/README.md", &"a".repeat(8_001));
        let (index, observed) = build_observed(&space(&s), 1);
        assert_eq!(index.notes.len(), 1);
        assert_eq!(observed.cutoffs, 1);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_note_keeps_filename_only_fallback_and_observes_failure() {
        let s = Scratch::new("dangling-evidence");
        s.file("ABC-12/README.md", README);
        std::os::unix::fs::symlink(s.path("gone"), s.path("ABC-12/dangling.md")).unwrap();
        let (index, observed) = build_observed(&space(&s), 1);
        assert_eq!(index.notes.len(), 2);
        assert!(observed.errors >= 1);
    }

    #[cfg(unix)]
    #[test]
    fn inaccessible_root_retains_missing_ui_but_has_error_evidence() {
        let s = Scratch::new("root-evidence");
        std::os::unix::fs::symlink("loop", s.path("loop")).unwrap();
        let (index, observed) =
            build_observed(&Space::new(s.path("loop").display().to_string()), 1);
        assert!(index.missing);
        assert_eq!(observed.errors, 1);
    }
}
