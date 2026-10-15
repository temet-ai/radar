// T4 wires these into the repo and merge request actions; until then nothing calls them.
#![allow(dead_code)]

//! Git actions on a checkout, as blocking plain functions: a new branch with
//! cb's checks, a named stash, rebase, push, a worktree beside the clone and
//! a configured command after it. Nothing here prompts; the caller asks
//! first (`plan_new_branch` says what the question has to say) and runs these
//! off the event loop.

use std::path::Path;
use std::process::{Command, Stdio};

use radar_core::system;

/// What a new branch would meet, read from the checkout without changing it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plan {
    /// The branch it would start from: the one given, else origin's HEAD, else `main`.
    pub base: String,
    /// Tracked files with uncommitted changes.
    pub dirty: usize,
    pub exists_local: bool,
    /// A remote-tracking `origin/<branch>` is known (as of the last fetch).
    pub exists_remote: bool,
    /// Commits on the branch that no remote has: they are lost if it is recreated.
    pub unpushed: usize,
    /// The branch checked out now, empty when detached.
    pub current: String,
}

/// git in `dir` with its raw output: success, stdout, stderr. Never asks for
/// a password: the terminal is radar's.
fn run(dir: &Path, args: &[&str]) -> Result<(bool, String, String), String> {
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
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    ))
}

/// The last line git printed on a failure, without its `fatal:` prefix.
fn why(err: &str) -> String {
    err.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("git failed")
        .trim()
        .trim_start_matches("fatal: ")
        .trim_start_matches("error: ")
        .to_string()
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let (ok, out, err) = run(dir, args)?;
    if ok { Ok(out) } else { Err(why(&err)) }
}

fn yes(dir: &Path, args: &[&str]) -> bool {
    run(dir, args).map(|r| r.0).unwrap_or(false)
}

fn dirty_count(dir: &Path) -> usize {
    git(dir, &["status", "--porcelain", "--untracked-files=no"])
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

fn current_branch(dir: &Path) -> String {
    git(dir, &["symbolic-ref", "--short", "-q", "HEAD"]).unwrap_or_default()
}

fn has_local(dir: &Path, branch: &str) -> bool {
    yes(
        dir,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
}

fn has_remote(dir: &Path, branch: &str) -> bool {
    yes(
        dir,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/remotes/origin/{branch}"),
        ],
    )
}

fn base_of(dir: &Path, base: Option<&str>) -> String {
    if let Some(b) = base.map(str::trim).filter(|b| !b.is_empty()) {
        return b.to_string();
    }
    git(
        dir,
        &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"],
    )
    .ok()
    .and_then(|s| s.strip_prefix("origin/").map(str::to_string))
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| "main".to_string())
}

fn refuse_name(branch: &str) -> Result<(), String> {
    if branch.is_empty() || branch.starts_with('-') {
        return Err(format!("\"{branch}\" is not a branch radar will use"));
    }
    Ok(())
}

fn refuse_dirty(dir: &Path) -> Result<(), String> {
    match dirty_count(dir) {
        0 => Ok(()),
        n => Err(format!(
            "{n} {} uncommitted changes: stash or commit them first",
            if n == 1 { "file has" } else { "files have" }
        )),
    }
}

/// Fetch `base` from origin and bring the local `base` up to it when that is
/// a fast-forward, without leaving the branch checked out now.
fn fresh_base(dir: &Path, base: &str) -> Result<(), String> {
    git(dir, &["fetch", "origin", base])?;
    if !has_local(dir, base) {
        return Ok(());
    }
    // Best effort: a local base that has diverged is left alone.
    if current_branch(dir) == base {
        let _ = run(dir, &["merge", "--ff-only", &format!("origin/{base}")]);
    } else {
        let _ = run(dir, &["fetch", "origin", &format!("{base}:{base}")]);
    }
    Ok(())
}

/// Read what making `branch` would meet. Changes nothing.
pub fn plan_new_branch(dir: &Path, branch: &str, base: Option<&str>) -> Plan {
    let exists_local = has_local(dir, branch);
    let unpushed = if exists_local {
        git(dir, &["rev-list", "--count", branch, "--not", "--remotes"])
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    } else {
        0
    };
    Plan {
        base: base_of(dir, base),
        dirty: dirty_count(dir),
        exists_local,
        exists_remote: has_remote(dir, branch),
        unpushed,
        current: current_branch(dir),
    }
}

/// Stash the tracked changes under `name`, and say how to get them back.
pub fn stash(dir: &Path, name: &str) -> Result<String, String> {
    if dirty_count(dir) == 0 {
        return Err("nothing to stash".to_string());
    }
    git(dir, &["stash", "push", "-m", name])?;
    Ok(format!(
        "stashed as \"{name}\": find it with git stash list, bring it back with git stash pop"
    ))
}

/// Make `branch` the one checked out in `dir`, off a fresh `base`. A branch
/// that exists is switched to, or with `recreate` reset to `origin/<base>`
/// (its commits that no remote has are lost); one only on origin is checked
/// out tracking it; else a new one starts at `origin/<base>`. Refuses while
/// tracked files are dirty: the caller stashes first.
pub fn new_branch(dir: &Path, branch: &str, base: &str, recreate: bool) -> Result<String, String> {
    refuse_name(branch)?;
    refuse_name(base)?;
    refuse_dirty(dir)?;
    fresh_base(dir, base)?;
    let origin_base = format!("origin/{base}");
    if has_local(dir, branch) {
        if recreate {
            git(dir, &["checkout", "-B", branch, "--no-track", &origin_base])?;
            return Ok(format!("recreated {branch} from {origin_base}"));
        }
        git(dir, &["checkout", branch])?;
        return Ok(format!("switched to {branch}"));
    }
    // Learn of it if it is on origin and not yet fetched.
    let _ = run(dir, &["fetch", "origin", branch]);
    if has_remote(dir, branch) {
        git(
            dir,
            &[
                "checkout",
                "-b",
                branch,
                "--track",
                &format!("origin/{branch}"),
            ],
        )?;
        return Ok(format!("checked out {branch}, tracking origin/{branch}"));
    }
    git(dir, &["checkout", "-b", branch, "--no-track", &origin_base])?;
    Ok(format!("created {branch} from {origin_base}"))
}

/// Rebase the branch checked out in `dir` onto a fresh `origin/<base>`. On a
/// conflict git's state is left as it is and the error names the files.
pub fn rebase(dir: &Path, base: &str) -> Result<String, String> {
    refuse_name(base)?;
    let current = current_branch(dir);
    if current.is_empty() {
        return Err("no branch is checked out".to_string());
    }
    if current == base {
        return Err(format!(
            "{base} is the branch checked out: nothing to rebase"
        ));
    }
    refuse_dirty(dir)?;
    fresh_base(dir, base)?;
    let onto = format!("origin/{base}");
    let n: usize = git(dir, &["rev-list", "--count", &format!("{onto}..HEAD")])
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let (ok, _, err) = run(dir, &["rebase", &onto])?;
    if ok {
        return Ok(format!(
            "rebased {current} onto {onto}: {n} commit{} moved",
            if n == 1 { "" } else { "s" }
        ));
    }
    let files = git(dir, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
    if files.is_empty() {
        return Err(format!("{}; git rebase --abort to undo", why(&err)));
    }
    let list: Vec<&str> = files.lines().collect();
    Err(format!(
        "conflicts in {}: fix them and git rebase --continue, or git rebase --abort to undo",
        list.join(", ")
    ))
}

/// Push `branch` to origin and set it as the upstream. `force` is
/// `--force-with-lease`, never a plain force.
pub fn push(dir: &Path, branch: &str, force: bool) -> Result<String, String> {
    refuse_name(branch)?;
    let mut args = vec!["push", "--set-upstream"];
    if force {
        args.push("--force-with-lease");
    }
    args.extend(["origin", branch]);
    git(dir, &args)?;
    Ok(format!(
        "pushed {branch} to origin{}",
        if force { " (force with lease)" } else { "" }
    ))
}

/// A worktree for `branch` at `path` (beside the clone, chosen by the
/// caller): the branch as it is when it exists, tracking origin's when only
/// that exists, else a new one off `origin/<base>`.
pub fn add_worktree(
    clone_dir: &Path,
    path: &Path,
    branch: &str,
    base: &str,
) -> Result<String, String> {
    refuse_name(branch)?;
    refuse_name(base)?;
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    let p = path.to_string_lossy().to_string();
    if has_local(clone_dir, branch) {
        git(clone_dir, &["worktree", "add", &p, branch])?;
        return Ok(format!("{branch} is checked out in {p}"));
    }
    let _ = run(clone_dir, &["fetch", "origin", branch]);
    if has_remote(clone_dir, branch) {
        git(
            clone_dir,
            &[
                "worktree",
                "add",
                "--track",
                "-b",
                branch,
                &p,
                &format!("origin/{branch}"),
            ],
        )?;
        return Ok(format!(
            "{branch} is checked out in {p}, tracking origin/{branch}"
        ));
    }
    fresh_base(clone_dir, base)?;
    git(
        clone_dir,
        &[
            "worktree",
            "add",
            "--no-track",
            "-b",
            branch,
            &p,
            &format!("origin/{base}"),
        ],
    )?;
    Ok(format!("{branch} created from origin/{base} in {p}"))
}

/// Run a configured command in `dir` and return the last line it printed.
pub fn after(dir: &Path, command: &str) -> Result<String, String> {
    let words = system::split_command(command);
    let Some((prog, args)) = words.split_first() else {
        return Err("no command".to_string());
    };
    let out = Command::new(prog)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{prog}: {e}"))?;
    let last = |b: &[u8]| {
        String::from_utf8_lossy(b)
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string())
    };
    if out.status.success() {
        Ok(last(&out.stdout)
            .or_else(|| last(&out.stderr))
            .unwrap_or_default())
    } else {
        Err(last(&out.stderr)
            .or_else(|| last(&out.stdout))
            .unwrap_or_else(|| format!("{prog} failed")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "radar-ops-{name}-{}-{}",
                std::process::id(),
                radar_core::time::now_secs()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn seed(&self) -> PathBuf {
            self.0.join("seed")
        }
        fn clone_dir(&self) -> PathBuf {
            self.0.join("code/api")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// git in `dir` as a throwaway identity, never the user's config.
    fn sh(dir: &Path, args: &[&str]) -> String {
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
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A bare origin with `main`, a `seed` working copy (kept on main) and a
    /// clone of it that can commit by itself.
    fn repos(s: &Scratch) -> PathBuf {
        let origin = s.0.join("origin.git");
        let seed = s.seed();
        std::fs::create_dir_all(&seed).unwrap();
        sh(&s.0, &["init", "--bare", "origin.git"]);
        sh(&seed, &["init"]);
        std::fs::write(seed.join("a.txt"), "one\n").unwrap();
        sh(&seed, &["add", "."]);
        sh(&seed, &["commit", "-m", "first"]);
        sh(
            &seed,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        sh(&seed, &["push", "origin", "main"]);
        let clone = s.clone_dir();
        std::fs::create_dir_all(clone.parent().unwrap()).unwrap();
        sh(
            &s.0,
            &["clone", origin.to_str().unwrap(), clone.to_str().unwrap()],
        );
        sh(&clone, &["config", "user.name", "t"]);
        sh(&clone, &["config", "user.email", "t@example.com"]);
        sh(&clone, &["config", "commit.gpgsign", "false"]);
        clone
    }

    /// A commit on origin's main, made through the seed.
    fn upstream_commit(s: &Scratch, file: &str, text: &str) {
        std::fs::write(s.seed().join(file), text).unwrap();
        sh(&s.seed(), &["add", "."]);
        sh(&s.seed(), &["commit", "-m", &format!("upstream {file}")]);
        sh(&s.seed(), &["push", "origin", "main"]);
    }

    fn local_commit(dir: &Path, file: &str, text: &str) {
        std::fs::write(dir.join(file), text).unwrap();
        sh(dir, &["add", "."]);
        sh(dir, &["commit", "-m", &format!("local {file}")]);
    }

    const BR: &str = "alice/ABC-12";

    #[test]
    fn a_dirty_checkout_is_planned_refused_and_then_stashed() {
        let s = Scratch::new("dirty");
        let c = repos(&s);
        std::fs::write(c.join("a.txt"), "changed\n").unwrap();
        let plan = plan_new_branch(&c, BR, None);
        assert_eq!(plan.dirty, 1);
        assert_eq!(plan.base, "main");
        assert_eq!(plan.current, "main");
        assert!(!plan.exists_local && !plan.exists_remote);
        let err = new_branch(&c, BR, "main", false).unwrap_err();
        assert!(err.contains("1 file has uncommitted changes"), "{err}");
        assert!(!has_local(&c, BR));

        let said = stash(&c, "ABC-12-local-2026-10-09").unwrap();
        assert!(said.contains("git stash list") && said.contains("ABC-12-local-2026-10-09"));
        assert!(sh(&c, &["stash", "list"]).contains("ABC-12-local-2026-10-09"));
        assert_eq!(plan_new_branch(&c, BR, None).dirty, 0);
        new_branch(&c, BR, "main", false).unwrap();
        assert_eq!(current_branch(&c), BR);
        assert_eq!(stash(&c, "x").unwrap_err(), "nothing to stash");
    }

    #[test]
    fn a_new_branch_starts_at_a_fresh_base_and_the_local_base_follows() {
        let s = Scratch::new("fresh");
        let c = repos(&s);
        upstream_commit(&s, "b.txt", "b\n");
        let said = new_branch(&c, BR, "main", false).unwrap();
        assert!(
            said.starts_with("created alice/ABC-12 from origin/main"),
            "{said}"
        );
        assert_eq!(
            sh(&c, &["rev-parse", "HEAD"]),
            sh(&c, &["rev-parse", "origin/main"])
        );
        assert_eq!(
            sh(&c, &["rev-parse", "main"]),
            sh(&c, &["rev-parse", "origin/main"])
        );
        assert!(
            !yes(&c, &["config", "--get-regexp", "branch.alice/ABC-12.*"]),
            "no upstream"
        );
    }

    #[test]
    fn an_existing_branch_is_switched_to_unless_it_is_recreated() {
        let s = Scratch::new("exists");
        let c = repos(&s);
        sh(&c, &["checkout", "-b", BR]);
        local_commit(&c, "x.txt", "x\n");
        sh(&c, &["checkout", "main"]);
        let plan = plan_new_branch(&c, BR, Some("main"));
        assert!(plan.exists_local && !plan.exists_remote);
        assert_eq!(plan.unpushed, 1);
        let said = new_branch(&c, BR, "main", false).unwrap();
        assert_eq!(said, "switched to alice/ABC-12");
        assert_eq!(current_branch(&c), BR);
        assert!(c.join("x.txt").exists());
    }

    #[test]
    fn recreating_counts_the_commits_no_remote_has_and_drops_them() {
        let s = Scratch::new("recreate");
        let c = repos(&s);
        sh(&c, &["checkout", "-b", BR]);
        local_commit(&c, "x.txt", "x\n");
        local_commit(&c, "y.txt", "y\n");
        let plan = plan_new_branch(&c, BR, None);
        assert_eq!(plan.unpushed, 2, "never-pushed commits count");
        assert_eq!(plan.current, BR);
        // dirty refuses even a recreate
        std::fs::write(c.join("x.txt"), "dirty\n").unwrap();
        assert!(new_branch(&c, BR, "main", true).is_err());
        sh(&c, &["checkout", "--", "x.txt"]);
        let said = new_branch(&c, BR, "main", true).unwrap();
        assert!(said.starts_with("recreated"), "{said}");
        assert_eq!(
            sh(&c, &["rev-parse", "HEAD"]),
            sh(&c, &["rev-parse", "origin/main"])
        );
        assert_eq!(plan_new_branch(&c, BR, None).unpushed, 0);
    }

    #[test]
    fn pushed_commits_are_not_counted_as_lost() {
        let s = Scratch::new("pushedcount");
        let c = repos(&s);
        sh(&c, &["checkout", "-b", BR]);
        local_commit(&c, "x.txt", "x\n");
        push(&c, BR, false).unwrap();
        local_commit(&c, "y.txt", "y\n");
        let plan = plan_new_branch(&c, BR, None);
        assert_eq!(plan.unpushed, 1);
        assert!(plan.exists_remote);
    }

    #[test]
    fn a_branch_only_on_origin_is_checked_out_tracking_it() {
        let s = Scratch::new("remote");
        let c = repos(&s);
        sh(&s.seed(), &["checkout", "-b", BR]);
        std::fs::write(s.seed().join("r.txt"), "r\n").unwrap();
        sh(&s.seed(), &["add", "."]);
        sh(&s.seed(), &["commit", "-m", "remote work"]);
        sh(&s.seed(), &["push", "origin", BR]);
        // not fetched yet: new_branch learns of it
        assert!(!plan_new_branch(&c, BR, None).exists_remote);
        let said = new_branch(&c, BR, "main", false).unwrap();
        assert!(said.contains("tracking origin/alice/ABC-12"), "{said}");
        assert_eq!(
            sh(&c, &["rev-parse", "--abbrev-ref", &format!("{BR}@{{u}}")]),
            format!("origin/{BR}")
        );
        assert!(c.join("r.txt").exists());
    }

    #[test]
    fn a_clean_rebase_reports_the_commits_moved() {
        let s = Scratch::new("rebase");
        let c = repos(&s);
        new_branch(&c, BR, "main", false).unwrap();
        local_commit(&c, "x.txt", "x\n");
        local_commit(&c, "y.txt", "y\n");
        upstream_commit(&s, "b.txt", "b\n");
        let said = rebase(&c, "main").unwrap();
        assert!(said.contains("2 commits moved"), "{said}");
        assert_eq!(sh(&c, &["rev-list", "--count", "origin/main..HEAD"]), "2");
        assert_eq!(sh(&c, &["rev-list", "--count", "HEAD..origin/main"]), "0");
        assert!(rebase(&c, BR).unwrap_err().contains("nothing to rebase"));
    }

    #[test]
    fn a_conflicting_rebase_stops_naming_the_files() {
        let s = Scratch::new("conflict");
        let c = repos(&s);
        new_branch(&c, BR, "main", false).unwrap();
        local_commit(&c, "a.txt", "mine\n");
        upstream_commit(&s, "a.txt", "theirs\n");
        let err = rebase(&c, "main").unwrap_err();
        assert!(
            err.contains("a.txt") && err.contains("git rebase --abort"),
            "{err}"
        );
        assert!(c.join(".git/rebase-merge").exists() || c.join(".git/rebase-apply").exists());
        sh(&c, &["rebase", "--abort"]);
        assert_eq!(current_branch(&c), BR);
    }

    #[test]
    fn push_sets_the_upstream_and_only_forces_with_a_lease() {
        let s = Scratch::new("push");
        let c = repos(&s);
        new_branch(&c, BR, "main", false).unwrap();
        local_commit(&c, "x.txt", "x\n");
        let said = push(&c, BR, false).unwrap();
        assert_eq!(said, "pushed alice/ABC-12 to origin");
        assert_eq!(
            sh(&c, &["rev-parse", "--abbrev-ref", &format!("{BR}@{{u}}")]),
            format!("origin/{BR}")
        );
        upstream_commit(&s, "b.txt", "b\n");
        rebase(&c, "main").unwrap();
        assert!(
            push(&c, BR, false).is_err(),
            "a plain push after a rebase is rejected"
        );
        let said = push(&c, BR, true).unwrap();
        assert!(said.contains("force with lease"), "{said}");
        let on_origin = sh(&c, &["ls-remote", "origin", &format!("refs/heads/{BR}")]);
        assert!(on_origin.starts_with(&sh(&c, &["rev-parse", "HEAD"])));
    }

    #[test]
    fn a_worktree_goes_beside_the_clone_creating_or_reusing_the_branch() {
        let s = Scratch::new("worktree");
        let c = repos(&s);
        let beside = s.0.join("code/api-ABC-12");
        let said = add_worktree(&c, &beside, BR, "main").unwrap();
        assert!(said.contains("created from origin/main"), "{said}");
        assert_eq!(current_branch(&beside), BR);
        assert_eq!(current_branch(&c), "main", "the clone is untouched");
        assert!(
            add_worktree(&c, &beside, BR, "main")
                .unwrap_err()
                .contains("already exists")
        );
        // an existing branch is reused
        sh(&c, &["branch", "alice/ABC-13"]);
        let again = s.0.join("code/api-ABC-13");
        add_worktree(&c, &again, "alice/ABC-13", "main").unwrap();
        assert_eq!(current_branch(&again), "alice/ABC-13");
    }

    #[test]
    fn a_command_runs_in_the_folder_and_its_last_line_comes_back() {
        let s = Scratch::new("after");
        let said = after(&s.0, "sh -c 'echo first; pwd'").unwrap();
        assert_eq!(said, std::fs::canonicalize(&s.0).unwrap().to_string_lossy());
        assert_eq!(
            after(&s.0, "sh -c 'echo nope >&2; exit 3'").unwrap_err(),
            "nope"
        );
        assert!(after(&s.0, "").is_err());
    }
}
