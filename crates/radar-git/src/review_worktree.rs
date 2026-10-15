//! A worktree of a branch at an exact commit, for a review: made once, found
//! again by the commit it holds. The review journey (a session reading the
//! change in its own checkout) asks for it through the host; the clone's
//! branches and working folder are never touched, and the remote is only
//! read from (`git fetch`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where the worktree is and whether it was there already.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewWorktree {
    pub path: PathBuf,
    pub sha: String,
    /// An existing worktree already held the commit; nothing was created.
    pub reused: bool,
}

/// The request: the clone, the branch the commit is on (fetched to find the
/// commit when the clone lacks it), the full commit id and where to put it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request<'a> {
    pub clone: &'a Path,
    pub branch: &'a str,
    pub sha: &'a str,
    pub path: &'a Path,
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("SSH_ASKPASS", "false")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
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

fn has_commit(clone: &Path, sha: &str) -> bool {
    git(clone, &["cat-file", "-e", &format!("{sha}^{{commit}}")]).is_ok()
}

/// A commit id as git prints it: 40 (sha-1) or 64 (sha-256) hex digits.
fn valid_sha(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64) && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The worktree paths of the clone with the commit each has checked out.
fn worktrees(clone: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let text = git(clone, &["worktree", "list", "--porcelain"])?;
    let mut found = Vec::new();
    let mut path: Option<PathBuf> = None;
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(p));
        } else if let Some(head) = line.strip_prefix("HEAD ")
            && let Some(p) = path.take()
        {
            found.push((p, head.to_string()));
        }
    }
    Ok(found)
}

/// Make the worktree, or reuse the one that holds the commit.
///
/// The worktree is detached at `sha` (the commit the review is about, not the
/// branch's moving tip), so the branch can stay checked out elsewhere. An
/// existing worktree is reused only when it is clean, so a review never reads
/// edits made in it.
pub fn ensure(req: &Request) -> Result<ReviewWorktree, String> {
    if !valid_sha(req.sha) {
        return Err(format!("\"{}\" is not a full commit id", req.sha));
    }
    if req.branch.is_empty() || req.branch.starts_with('-') {
        return Err(format!("\"{}\" is not a branch radar will use", req.branch));
    }
    let sha = req.sha.to_ascii_lowercase();
    let want = std::path::absolute(req.path).unwrap_or_else(|_| req.path.to_path_buf());
    let known = worktrees(req.clone)?;
    // The planned place first, then any worktree already on the commit.
    let held = known
        .iter()
        .find(|(p, head)| *head == sha && same_place(p, &want))
        .or_else(|| known.iter().find(|(_, head)| *head == sha));
    if let Some((path, _)) = held {
        let dirty = git(path, &["status", "--porcelain"])?;
        if dirty.is_empty() {
            return Ok(ReviewWorktree {
                path: path.clone(),
                sha,
                reused: true,
            });
        }
        if same_place(path, &want) {
            return Err(format!(
                "{} holds uncommitted changes: not reviewing from it",
                path.display()
            ));
        }
    }
    if want.exists() {
        return Err(format!(
            "{} already exists and is not a worktree at {}",
            want.display(),
            &sha[..7]
        ));
    }
    if !has_commit(req.clone, &sha) {
        // Read-only to the remote: fetch the branch, then the commit itself.
        let _ = git(req.clone, &["fetch", "origin", req.branch]);
        if !has_commit(req.clone, &sha) {
            git(req.clone, &["fetch", "origin", &sha])
                .map_err(|e| format!("commit {} is not on origin: {e}", &sha[..7]))?;
        }
    }
    let at = want.to_string_lossy().to_string();
    git(req.clone, &["worktree", "add", "--detach", &at, &sha])?;
    Ok(ReviewWorktree {
        path: want,
        sha,
        reused: false,
    })
}

fn same_place(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("radar-review-wt-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
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
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// `origin` (bare) with a feature branch, and a clone that has not fetched it.
    fn repos(s: &Scratch) -> (PathBuf, PathBuf, String) {
        let origin = s.0.join("origin.git");
        let seed = s.0.join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        sh(&seed, &["init", "-q", "-b", "main"]);
        std::fs::write(seed.join("a.txt"), "a\n").unwrap();
        sh(&seed, &["add", "."]);
        sh(&seed, &["commit", "-qm", "first"]);
        sh(&s.0, &["clone", "-q", "--bare", "seed", "origin.git"]);
        sh(&s.0, &["clone", "-q", "origin.git", "clone"]);
        sh(&seed, &["checkout", "-qb", "feature/x"]);
        std::fs::write(seed.join("b.txt"), "b\n").unwrap();
        sh(&seed, &["add", "."]);
        sh(&seed, &["commit", "-qm", "work"]);
        sh(&seed, &["push", "-q", &origin.to_string_lossy(), "feature/x"]);
        let sha = sh(&seed, &["rev-parse", "HEAD"]);
        (s.0.join("clone"), origin, sha)
    }

    #[test]
    fn a_commit_the_clone_lacks_is_fetched_and_checked_out_detached() {
        let s = Scratch::new("make");
        let (clone, _, sha) = repos(&s);
        assert!(!has_commit(&clone, &sha));
        let at = s.0.join("review-x");
        let made = ensure(&Request { clone: &clone, branch: "feature/x", sha: &sha, path: &at }).unwrap();
        assert!(!made.reused);
        assert_eq!(sh(&at, &["rev-parse", "HEAD"]), sha);
        assert!(at.join("b.txt").exists());
        assert_eq!(sh(&at, &["branch", "--show-current"]), "", "detached");
        assert_eq!(sh(&clone, &["branch", "--show-current"]), "main", "the clone is untouched");
    }

    #[test]
    fn asking_again_reuses_the_worktree_even_from_another_path() {
        let s = Scratch::new("reuse");
        let (clone, _, sha) = repos(&s);
        let at = s.0.join("review-x");
        ensure(&Request { clone: &clone, branch: "feature/x", sha: &sha, path: &at }).unwrap();
        let other = s.0.join("elsewhere");
        let again = ensure(&Request { clone: &clone, branch: "feature/x", sha: &sha, path: &other }).unwrap();
        assert!(again.reused);
        assert!(same_place(&again.path, &at));
        assert!(!other.exists());
    }

    #[test]
    fn an_edited_worktree_is_not_reviewed_from_and_a_foreign_folder_is_refused() {
        let s = Scratch::new("refuse");
        let (clone, _, sha) = repos(&s);
        let at = s.0.join("review-x");
        ensure(&Request { clone: &clone, branch: "feature/x", sha: &sha, path: &at }).unwrap();
        std::fs::write(at.join("b.txt"), "edited\n").unwrap();
        let err = ensure(&Request { clone: &clone, branch: "feature/x", sha: &sha, path: &at }).unwrap_err();
        assert!(err.contains("uncommitted"), "{err}");
        let taken = s.0.join("taken");
        std::fs::create_dir_all(&taken).unwrap();
        // the only worktree on that commit is edited, and the new place is taken
        let err = ensure(&Request { clone: &clone, branch: "feature/x", sha: &sha, path: &taken }).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        let sha_main = sh(&clone, &["rev-parse", "HEAD"]);
        let again = ensure(&Request { clone: &clone, branch: "main", sha: &sha_main, path: &taken }).unwrap();
        assert!(again.reused, "the clone itself holds main's commit, clean");
    }

    #[test]
    fn a_short_or_unknown_commit_is_refused_without_touching_anything() {
        let s = Scratch::new("bad");
        let (clone, _, sha) = repos(&s);
        let at = s.0.join("review-x");
        let err = ensure(&Request { clone: &clone, branch: "feature/x", sha: &sha[..8], path: &at }).unwrap_err();
        assert!(err.contains("full commit id"), "{err}");
        let missing = "0".repeat(40);
        let err = ensure(&Request { clone: &clone, branch: "feature/x", sha: &missing, path: &at }).unwrap_err();
        assert!(err.contains("not on origin"), "{err}");
        assert!(!at.exists());
        let err = ensure(&Request { clone: &clone, branch: "-x", sha: &sha, path: &at }).unwrap_err();
        assert!(err.contains("not a branch"), "{err}");
    }
}
