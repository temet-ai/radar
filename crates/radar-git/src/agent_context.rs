//! What local git tells the host so an agent can be briefed on a branch, or
//! on a file and line (send to agent), and the worktree the review journey
//! asks for (a branch at an exact sha).
//!
//! Everything here is read from what the plugin already holds (the Branches
//! screen's reads) or, for a file, from the file itself inside its checkout;
//! nothing is fetched to answer a context and nothing is pushed. Branch
//! names and commit subjects come from the repository's commits and are given
//! as they are.

use std::path::{Path, PathBuf};

use radar_core::agent_context::{
    AgentContext, AgentContextRequest, ContextField, WorktreeError, WorktreeReady, WorktreeRequest,
};
use radar_core::journal::SubjectKey;
use radar_core::plugin::Effect;

use crate::act::{self, WorktreeResult, WorktreeStatus};
use crate::local;
use crate::review_worktree::{self, Request};
use crate::screen::{Branch, Detail, Sel, StatusRead, Tree};
use crate::GitPlugin;

/// The kind of a branch in a [`SubjectKey`] (the journal's, too).
pub(crate) const BRANCH: &str = "branch";
/// The kind of a file, or a file and line, in a [`SubjectKey`].
pub(crate) const FILE: &str = "file";
/// The words the brief heads the quoted material with.
pub(crate) const BRANCH_KIND: &str = "Git branch";
pub(crate) const FILE_KIND: &str = "Source file";
/// Lines of a file shown around the one asked about, each way.
pub(crate) const CONTEXT_LINES: usize = 20;
/// Lines shown when a range is asked about, at most.
pub(crate) const MAX_RANGE: usize = 80;
/// The row key and id of the action on a branch row.
pub(crate) const SEND_ID: &str = radar_core::agent_context::SEND_TO_AGENT_ACTION;

/// The scoped identity of a branch: the clone it was read from, and its name.
pub(crate) fn branch_key(dir: &str, name: &str) -> Option<SubjectKey> {
    let dir = local::journal_path(dir)?.display().to_string();
    SubjectKey::new("local", &dir, BRANCH, name).ok()
}

/// The scoped identity of a file, or of a line (`path:12`) or lines
/// (`path:12-30`) of it, relative to its checkout.
pub(crate) fn file_key(dir: &str, path: &str, lines: Option<(u32, u32)>) -> Option<SubjectKey> {
    let dir = local::journal_path(dir)?.display().to_string();
    let id = match lines {
        None => path.to_owned(),
        Some((a, b)) if a == b => format!("{path}:{a}"),
        Some((a, b)) => format!("{path}:{a}-{b}"),
    };
    SubjectKey::new("local", &dir, FILE, &id).ok()
}

/// A file key's id taken apart: the path and the lines.
fn split_file_id(id: &str) -> (&str, Option<(u32, u32)>) {
    let Some((path, tail)) = id.rsplit_once(':') else { return (id, None) };
    let lines = match tail.split_once('-') {
        Some((a, b)) => a.parse().ok().zip(b.parse().ok()),
        None => tail.parse().ok().map(|n| (n, n)),
    };
    match lines {
        Some((a, b)) if a >= 1 && b >= a => (path, Some((a, b))),
        _ => (id, None),
    }
}

/// The clone a known checkout belongs to (the folder itself when it is one).
fn clone_dir(p: &GitPlugin, dir: &str) -> Option<String> {
    p.local
        .linked
        .iter()
        .chain(p.local.index.iter().flat_map(|i| i.checkouts.iter()))
        .find(|c| c.dir == dir)
        .map(|c| if c.main.is_empty() { c.dir.clone() } else { c.main.clone() })
}

/// The context for one branch of a clone the Branches screen has read.
pub(crate) fn branch_context(p: &GitPlugin, dir: &str, name: &str) -> Option<AgentContext> {
    let d = p.branches.details.get(dir)?;
    let b = d.branches.iter().find(|b| b.name == name)?;
    Some(describe(p, d, b))
}

fn describe(p: &GitPlugin, d: &Detail, b: &Branch) -> AgentContext {
    let key = branch_key(&d.dir, &b.name).expect("a branch read from a clone has a key");
    let mut c = AgentContext::new(BRANCH_KIND, key, &b.name)
        .field(ContextField::new("branch", &b.name))
        .field(ContextField::new("clone", &d.dir))
        .field(ContextField::new("tip", &b.sha))
        .field(ContextField::new("tip_commit_subject", &b.subject));
    if !d.base.is_empty() {
        c = c.field(ContextField::new("base_branch", &d.base));
    }
    let upstream = if b.upstream.is_empty() {
        "none".to_owned()
    } else if b.gone {
        format!("{} (gone from the remote)", b.upstream)
    } else {
        b.upstream.clone()
    };
    c = c
        .field(ContextField::new("upstream", upstream))
        .field(ContextField::new("ahead_of_upstream", b.ahead.to_string()))
        .field(ContextField::new("behind_upstream", b.behind.to_string()))
        .field(ContextField::new("commits_no_remote_has", b.unpushed.to_string()))
        .field(ContextField::new("merged_into_base", if b.merged { "yes" } else { "no" }));
    if let Some(t) = d.trees.iter().find(|t| t.branch == b.name) {
        c = c.field(ContextField::new("checked_out_in", &t.dir));
        if t.status == StatusRead::Current && t.dirty > 0 {
            c = c.field(ContextField::new("uncommitted_files", t.dirty.to_string()));
        }
    }
    if let Some(remote) = p
        .local
        .linked
        .iter()
        .chain(p.local.index.iter().flat_map(|i| i.checkouts.iter()))
        .find(|k| k.dir == d.dir)
        .map(|k| k.remote.as_str())
        .filter(|r| !r.is_empty())
    {
        c = c.field(ContextField::new("origin", remote));
    }
    c
}

/// The context for a file, or lines of it, inside a known checkout. The file
/// is read from disk, bounded, and only when it stays inside the checkout.
pub(crate) fn file_context(p: &GitPlugin, dir: &str, id: &str) -> Option<AgentContext> {
    clone_dir(p, dir).or_else(|| p.branches.details.contains_key(dir).then(|| dir.to_owned()))?;
    let (rel, lines) = split_file_id(id);
    let abs = inside(Path::new(dir), rel)?;
    let key = file_key(dir, rel, lines)?;
    let mut c = AgentContext::new(FILE_KIND, key, rel)
        .field(ContextField::new("path", rel))
        .field(ContextField::new("checkout", dir));
    if let Some(branch) = p
        .branches
        .details
        .get(dir)
        .and_then(|d| d.trees.first())
        .map(|t| t.branch.as_str())
        .filter(|b| !b.is_empty())
    {
        c = c.field(ContextField::new("checked_out_branch", branch));
    }
    if let Some((a, b)) = lines {
        c = c.field(ContextField::new(
            "lines",
            if a == b { a.to_string() } else { format!("{a}-{b}") },
        ));
        if let Some(text) = excerpt(&abs, a, b) {
            c = c.field(ContextField::new("excerpt", text));
        }
    }
    Some(c)
}

/// `rel` under `root`, if it names a file there that does not leave it
/// (no `..`, no absolute path, no symlink out).
fn inside(root: &Path, rel: &str) -> Option<PathBuf> {
    let r = Path::new(rel);
    if rel.is_empty()
        || r.is_absolute()
        || r.components().any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return None;
    }
    let abs = std::fs::canonicalize(root.join(r)).ok()?;
    let root = std::fs::canonicalize(root).ok()?;
    (abs.starts_with(&root) && abs.is_file()).then_some(abs)
}

/// The lines around `a..=b`, numbered, bounded: the asked lines (at most
/// [`MAX_RANGE`]) and [`CONTEXT_LINES`] either side. None for a file that
/// is not text.
fn excerpt(abs: &Path, a: u32, b: u32) -> Option<String> {
    let text = std::fs::read_to_string(abs).ok()?;
    let (a, b) = (a as usize, (b as usize).min(a as usize + MAX_RANGE - 1));
    let from = a.saturating_sub(CONTEXT_LINES).max(1);
    let to = b + CONTEXT_LINES;
    let rows: Vec<String> = text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l))
        .filter(|(n, _)| (from..=to).contains(n))
        .map(|(n, l)| format!("{}{n:>5}  {l}", if (a..=b).contains(&n) { ">" } else { " " }))
        .collect();
    (!rows.is_empty()).then(|| rows.join("\n"))
}

/// Asked of the plugin by the host for a key of this part.
pub(crate) fn agent_context(p: &GitPlugin, request: &AgentContextRequest) -> Option<AgentContext> {
    let s = &request.subject;
    if s.provider != "local" {
        return None;
    }
    match s.kind.as_str() {
        BRANCH => branch_context(p, &s.instance, &s.id),
        FILE => file_context(p, &s.instance, &s.id),
        _ => None,
    }
}

/// The key on the selected row of the Branches screen: a branch, or the
/// branch a worktree has checked out.
pub(crate) fn selected_key(p: &GitPlugin) -> Option<SubjectKey> {
    match crate::screen::selected(p)? {
        Sel::Branch(d, b) => branch_key(&d.dir, &b.name),
        Sel::Tree(d, t) => tree_branch(d, t).and_then(|b| branch_key(&d.dir, b)),
        Sel::Clone(_) => None,
    }
}

fn tree_branch<'a>(d: &'a Detail, t: &'a Tree) -> Option<&'a str> {
    d.branches.iter().find(|b| b.name == t.branch).map(|b| b.name.as_str())
}

/// What the action on the selected row asks of the host.
pub(crate) fn send_selected(p: &GitPlugin) -> Vec<Effect> {
    match selected_key(p) {
        Some(subject) => vec![Effect::SendToAgent(AgentContextRequest { subject })],
        None => vec![Effect::Notify("select a branch to ask an agent about it".into())],
    }
}

/// Whether the selected row can be sent (so the key is offered).
pub(crate) fn can_send(p: &GitPlugin) -> bool {
    selected_key(p).is_some()
}

// --- the worktree request ----------------------------------------------------

/// The clone the request's repository is: a project path (`acme/api`) or the
/// remote URL, matched to a checkout this machine has.
fn clone_of_repository(p: &GitPlugin, repository: &str) -> Option<String> {
    let all = || {
        p.local
            .linked
            .iter()
            .chain(p.local.index.iter().flat_map(|i| i.checkouts.iter()))
    };
    let found = local::chosen(p, repository).or_else(|| {
        let want = trim_remote(repository);
        all().find(|c| !c.remote.is_empty() && trim_remote(&c.remote) == want)
    })?;
    Some(if found.main.is_empty() { found.dir.clone() } else { found.main.clone() })
}

fn trim_remote(r: &str) -> &str {
    r.trim_end_matches('/').trim_end_matches(".git")
}

/// Where a review worktree goes: the configured template, per branch and sha.
fn review_path(p: &GitPlugin, clone: &str, branch: &str, sha: &str) -> String {
    let short = &sha[..sha.len().min(7)];
    act::worktree_path(p, clone, &format!("{branch}-{short}"))
}

/// Make, or reuse, the worktree. Creating one is journaled like radar-git's
/// other worktrees; reusing one changes nothing and journals nothing.
pub(crate) fn ensure_worktree(
    p: &GitPlugin,
    request: &WorktreeRequest,
) -> (Result<WorktreeReady, WorktreeError>, Vec<Effect>) {
    let Some(clone) = clone_of_repository(p, &request.repository) else {
        return (Err(WorktreeError::UnknownRepository), vec![]);
    };
    let path = review_path(p, &clone, &request.branch, &request.sha);
    let made = review_worktree::ensure(&Request {
        clone: Path::new(&clone),
        branch: &request.branch,
        sha: &request.sha,
        path: Path::new(&path),
    });
    let journal = |path: &Path, creation| {
        act::journal_worktree(&WorktreeResult {
            clone: clone.clone(),
            path: path.display().to_string(),
            branch: request.branch.clone(),
            creation,
            after: None,
        })
    };
    match made {
        Ok(w) if w.reused => (Ok(WorktreeReady { path: w.path, reused: true }), vec![]),
        Ok(w) => {
            let fx = journal(&w.path, WorktreeStatus::Succeeded);
            (Ok(WorktreeReady { path: w.path, reused: false }), fx)
        }
        Err(e) => {
            // A refusal before git ran changed nothing; a folder there means
            // git may have changed something before it failed.
            let fx = if Path::new(&path).exists() {
                journal(Path::new(&path), WorktreeStatus::Unknown)
            } else {
                vec![]
            };
            let err = if e.contains("not on origin") {
                WorktreeError::ShaNotFound
            } else {
                WorktreeError::Failed(e)
            };
            (Err(err), fx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::demo::branches;
    use radar_core::plugin::Pane;
    use radar_core::view::{Chord, Intent, Key};

    const API: &str = "/work/acme/api";

    fn field<'a>(c: &'a AgentContext, name: &str) -> &'a ContextField {
        c.fields.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no {name}"))
    }

    fn select(p: &GitPlugin, child: &str) {
        p.branches.tree.borrow_mut().select(vec![format!("c:{API}"), child.to_string()]);
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("radar-git-ctx-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("src")).unwrap();
            // Resolved: macOS's temporary folder is a link (/var -> /private/var).
            Self(std::fs::canonicalize(&dir).unwrap())
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_branch_context_carries_the_state_and_marks_other_peoples_words() {
        let p = branches();
        let key = branch_key(API, "alice/ABC-12").unwrap();
        let c = agent_context(&p, &AgentContextRequest { subject: key.clone() }).unwrap();
        assert_eq!(c.subject, key);
        assert_eq!(c.kind, BRANCH_KIND);
        assert_eq!(c.title, "alice/ABC-12");
        assert_eq!(field(&c, "base_branch").value, "main");
        assert_eq!(field(&c, "ahead_of_upstream").value, "2");
        assert_eq!(field(&c, "checked_out_in").value, API);
        assert_eq!(field(&c, "uncommitted_files").value, "3");
        assert_eq!(field(&c, "tip_commit_subject").value, "work on alice/ABC-12");
        // the gone upstream is said
        let gone = agent_context(&p, &AgentContextRequest { subject: branch_key(API, "alice/ABC-7").unwrap() }).unwrap();
        assert!(field(&gone, "upstream").value.contains("gone"));
        assert_eq!(field(&gone, "merged_into_base").value, "no");
    }

    #[test]
    fn a_key_that_is_not_this_parts_or_not_held_gets_no_context() {
        let p = branches();
        let ask = |k| agent_context(&p, &AgentContextRequest { subject: k });
        assert!(ask(branch_key(API, "nope").unwrap()).is_none(), "unknown branch");
        assert!(ask(branch_key("/work/other", "main").unwrap()).is_none(), "unknown clone");
        let slack = SubjectKey::new("chat", "T1", "message", "C1/1.2").unwrap();
        assert!(ask(slack).is_none());
        let card = SubjectKey::new("local", API, "checkout", API).unwrap();
        assert!(ask(card).is_none());
    }

    #[test]
    fn the_a_key_asks_the_host_to_send_the_selected_branch() {
        let mut p = branches();
        select(&p, "b:spike");
        assert!(crate::screen::row_hints(&p, Pane::Middle).iter().any(|h| h.keys == "A"));
        let binding = crate::screen::keys(&p, Pane::Middle).into_iter().find(|binding| binding.keys == "A").unwrap();
        let action = radar_core::view::ActionSpec::named(SEND_ID, &binding);
        assert_eq!(action.mnemonic_at, Some(0));
        assert!(action.mnemonic_shift, "uppercase agent shortcut stays discoverable");
        let out = crate::screen::intent(
            &mut p,
            Pane::Middle,
            &Intent::Key { chord: Chord::new(Key::Char('A')) },
        );
        let radar_core::plugin::Outcome::Handled(fx) = out else { panic!("{out:?}") };
        match fx.as_slice() {
            [Effect::SendToAgent(r)] => assert_eq!(r.subject, branch_key(API, "spike").unwrap()),
            other => panic!("{other:?}"),
        }
        // a worktree sends the branch it has checked out; the clone itself nothing
        select(&p, "w:/work/acme/api-hotfix");
        assert_eq!(selected_key(&p), branch_key(API, "alice/ABC-14"));
        p.branches.tree.borrow_mut().select(vec![format!("c:{API}")]);
        assert!(!can_send(&p));
        assert!(crate::screen::row_hints(&p, Pane::Middle).iter().all(|h| h.keys != "A"));
    }

    #[test]
    fn a_file_context_shows_the_lines_asked_with_a_little_around_them() {
        let s = Scratch::new("file");
        let mut p = branches();
        let dir = s.0.display().to_string();
        p.local.index.as_mut().unwrap().checkouts[0].dir = dir.clone();
        let body: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        std::fs::write(s.0.join("src/a.txt"), body).unwrap();
        let key = file_key(&dir, "src/a.txt", Some((50, 52))).unwrap();
        assert_eq!(key.id, "src/a.txt:50-52");
        let c = agent_context(&p, &AgentContextRequest { subject: key }).unwrap();
        assert_eq!(field(&c, "lines").value, "50-52");
        let text = &field(&c, "excerpt").value;
        assert!(text.contains(">   50  line 50") && text.contains(">   52  line 52"), "{text}");
        assert!(text.contains("    30  line 30") && !text.contains("line 29"), "{text}");
        assert!(text.contains("line 72") && !text.contains("line 73"), "{text}");
        // the whole file: no excerpt, just the path
        let whole = agent_context(&p, &AgentContextRequest { subject: file_key(&dir, "src/a.txt", None).unwrap() }).unwrap();
        assert!(whole.fields.iter().all(|f| f.name != "excerpt"));
        assert_eq!(split_file_id("src/a.txt:7"), ("src/a.txt", Some((7, 7))));
        assert_eq!(split_file_id("odd:name"), ("odd:name", None));
    }

    #[test]
    fn a_file_outside_the_checkout_is_never_read() {
        let s = Scratch::new("escape");
        let mut p = branches();
        let dir = s.0.join("repo");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(s.0.join("secret.txt"), "token\n").unwrap();
        p.local.index.as_mut().unwrap().checkouts[0].dir = dir.display().to_string();
        let ask = |id: &str| {
            let key = SubjectKey::new("local", &dir.display().to_string(), FILE, id).unwrap();
            agent_context(&p, &AgentContextRequest { subject: key })
        };
        assert!(ask("../secret.txt:1").is_none());
        assert!(ask("/etc/passwd:1").is_none());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(s.0.join("secret.txt"), dir.join("link.txt")).unwrap();
            assert!(ask("link.txt:1").is_none(), "a symlink out of the checkout");
        }
    }

    // --- the worktree request ----------------------------------------------

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A clone of a scratch origin that has a feature branch the clone has
    /// not fetched, known to the plugin as `acme/team-a/api`.
    fn clone_with_remote_branch(s: &Scratch) -> (GitPlugin, String, String) {
        let seed = s.0.join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        git(&seed, &["init", "-q", "-b", "main"]);
        std::fs::write(seed.join("a.txt"), "a\n").unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-qm", "first"]);
        git(&s.0, &["clone", "-q", "--bare", "seed", "origin.git"]);
        git(&s.0, &["clone", "-q", "origin.git", "api"]);
        git(&seed, &["checkout", "-qb", "feature/x"]);
        std::fs::write(seed.join("b.txt"), "b\n").unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-qm", "work"]);
        git(&seed, &["push", "-q", &s.0.join("origin.git").display().to_string(), "feature/x"]);
        let sha = git(&seed, &["rev-parse", "HEAD"]);
        let clone = s.0.join("api").display().to_string();
        let mut p = branches();
        p.cfg.worktree = format!("{}/{{topic}}", s.0.display());
        let c = &mut p.local.index.as_mut().unwrap().checkouts[0];
        c.dir = clone.clone();
        c.main = String::new();
        (p, clone, sha)
    }

    #[test]
    fn a_worktree_is_made_journaled_then_reused_and_the_clone_is_untouched() {
        let s = Scratch::new("wt");
        let (p, clone, sha) = clone_with_remote_branch(&s);
        let req = WorktreeRequest {
            repository: "acme/team-a/api".into(),
            branch: "feature/x".into(),
            sha: sha.clone(),
            for_subject: None,
        };
        let (made, fx) = ensure_worktree(&p, &req);
        let ready = made.unwrap();
        assert!(!ready.reused);
        assert_eq!(ready.path, s.0.join(format!("x-{}", &sha[..7])));
        assert_eq!(git(&ready.path, &["rev-parse", "HEAD"]), sha);
        assert_eq!(git(Path::new(&clone), &["branch", "--show-current"]), "main");
        let kinds: Vec<String> = fx
            .iter()
            .filter_map(|e| match e {
                Effect::Journal(a) => Some(a.kind.as_str().to_owned()),
                _ => None,
            })
            .collect();
        assert_eq!(kinds, ["git.worktree.add"]);
        let (again, fx) = ensure_worktree(&p, &req);
        let again = again.unwrap();
        assert!(again.reused);
        assert_eq!(again.path, ready.path);
        assert!(fx.is_empty(), "reusing changes nothing: {fx:?}");
    }

    #[test]
    fn an_unknown_repository_or_commit_is_said_not_guessed() {
        let s = Scratch::new("wt-bad");
        let (p, _, _) = clone_with_remote_branch(&s);
        let ask = |repository: &str, sha: String| {
            ensure_worktree(
                &p,
                &WorktreeRequest { repository: repository.into(), branch: "feature/x".into(), sha, for_subject: None },
            )
        };
        assert_eq!(ask("acme/other", "a".repeat(40)).0, Err(WorktreeError::UnknownRepository));
        let (err, fx) = ask("acme/team-a/api", "0".repeat(40));
        assert_eq!(err, Err(WorktreeError::ShaNotFound));
        assert!(fx.is_empty(), "nothing was created, nothing is journaled");
        // by remote URL as well
        let (err, _) = ask("git@example.com:acme/team-a/api", "0".repeat(40));
        assert_eq!(err, Err(WorktreeError::ShaNotFound));
    }
}
