//! Deleting branches and worktrees, one at a time (`d` on the Branches
//! screen) or the merged and gone ones together (`b`, as the operator's
//! `wt-cleanup` does): which are safe to take, and the git that takes them.
//! Nothing here asks; the screen asks first and says what is lost, and runs
//! these off the event loop.

use std::collections::BTreeSet;
use std::path::Path;

use radar_core::journal::Outcome;

use crate::screen::{Branch, Detail, StatusRead, git};

/// Why a branch may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Merged into the repo's base branch.
    Merged,
    /// Its upstream was deleted on the remote (a merge request merged and
    /// its branch removed).
    Gone,
}

impl Why {
    pub fn words(self) -> &'static str {
        match self {
            Why::Merged => "merged",
            Why::Gone => "upstream gone",
        }
    }
}

/// One thing prune would remove: a branch, or a worktree and its branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The clone it belongs to.
    pub clone: String,
    /// The worktree's folder, empty for a branch on its own.
    pub tree: String,
    pub branch: String,
    pub why: Why,
    /// Commits on the branch that no remote has.
    pub lost: usize,
}

impl Candidate {
    /// A line for the question: `worktree ../api-fix · fix/x · merged`.
    pub fn line(&self) -> String {
        let mut line = if self.tree.is_empty() {
            format!("branch {}", self.branch)
        } else {
            format!(
                "worktree {} · {}",
                crate::local::tilde(&self.tree),
                self.branch
            )
        };
        line.push_str(&format!(" · {}", self.why.words()));
        if self.lost > 0 {
            let s = if self.lost == 1 { "" } else { "s" };
            line.push_str(&format!(" · {} commit{s} on no remote", self.lost));
        }
        line
    }
}

fn why(d: &Detail, b: &Branch) -> Option<Why> {
    if b.name == d.base {
        return None;
    }
    if b.gone {
        return Some(Why::Gone);
    }
    // A branch that points where the base does has nothing on it yet (a
    // new one): it is "merged" only in the way an empty room is tidy.
    let base = d.branches.iter().find(|x| x.name == d.base);
    (b.merged && base.is_none_or(|x| x.sha != b.sha)).then_some(Why::Merged)
}

/// What prune would remove from these clones: worktrees (never a clone
/// itself) that are clean and on a merged or gone branch, with that branch,
/// and branches that are merged or gone and checked out nowhere. The base
/// branch, a branch checked out anywhere and a dirty worktree are kept.
pub fn select<'a>(details: impl IntoIterator<Item = &'a Detail>) -> Vec<Candidate> {
    let mut out = Vec::new();
    for d in details {
        for t in d.trees.iter().skip(1) {
            if t.status != StatusRead::Current || t.dirty > 0 || t.branch.is_empty() {
                continue;
            }
            let Some(b) = d.branches.iter().find(|b| b.name == t.branch) else {
                continue;
            };
            if let Some(w) = why(d, b) {
                out.push(Candidate {
                    clone: d.dir.clone(),
                    tree: t.dir.clone(),
                    branch: b.name.clone(),
                    why: w,
                    lost: lost(w, b),
                });
            }
        }
        for b in &d.branches {
            if d.trees.iter().any(|t| t.branch == b.name) {
                continue;
            }
            if let Some(w) = why(d, b) {
                out.push(Candidate {
                    clone: d.dir.clone(),
                    tree: String::new(),
                    branch: b.name.clone(),
                    why: w,
                    lost: lost(w, b),
                });
            }
        }
    }
    out
}

fn lost(why: Why, b: &Branch) -> usize {
    match why {
        Why::Merged => 0,
        Why::Gone => b.unpushed,
    }
}

/// `git branch -D`: the caller has asked, and said what is lost. Git itself
/// refuses a branch checked out anywhere.
pub fn delete_branch(clone: &Path, name: &str) -> Result<String, String> {
    if let Some(why) = branch_refusal(name) {
        return Err(why);
    }
    git(clone, &["branch", "-D", name])?;
    Ok(format!("deleted branch {name}"))
}

fn branch_refusal(name: &str) -> Option<String> {
    (name.is_empty() || name.starts_with('-'))
        .then(|| format!("\"{name}\" is not a branch radar will delete"))
}

/// `git worktree remove`; `force` only when the caller has said that the
/// uncommitted work in it is lost.
pub fn remove_tree(clone: &Path, dir: &str, force: bool) -> Result<String, String> {
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(dir);
    git(clone, &args)?;
    Ok(format!("removed the worktree {}", crate::local::tilde(dir)))
}

/// One independently observable part of a prune. Configured commands and
/// their output deliberately are not part of the step's identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Worktree { tree: String, branch: String },
    Branch { branch: String },
    After,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Succeeded,
    /// An opaque process error cannot prove whether anything was changed.
    Unknown,
    /// The existing branch-name guard refused before invoking git.
    InvalidInput,
    /// Not attempted because an earlier removal stopped the run.
    Skipped,
}

impl StepStatus {
    pub fn outcome(self) -> Outcome {
        match self {
            Self::Succeeded => Outcome::Succeeded,
            Self::Unknown => Outcome::Unknown,
            Self::InvalidInput | Self::Skipped => Outcome::Blocked,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepResult {
    pub clone: String,
    pub step: Step,
    pub status: StepStatus,
}

/// The display message retains the existing UI behavior. Only typed steps
/// determine journal outcomes; the message can contain private process output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub said: Result<String, String>,
    pub steps: Vec<StepResult>,
}

impl RunResult {
    pub fn outcome(&self) -> Outcome {
        if self.steps.iter().all(|s| s.status == StepStatus::Succeeded) {
            Outcome::Succeeded
        } else if self
            .steps
            .iter()
            .any(|s| s.status == StepStatus::Succeeded && !matches!(s.step, Step::After))
        {
            Outcome::Partial
        } else if self.steps.iter().any(|s| s.status == StepStatus::Unknown) {
            Outcome::Unknown
        } else {
            Outcome::Blocked
        }
    }
}

/// Take every candidate, in order, stopping at the first refusal; then the
/// configured `after` command in each clone that lost a worktree. Preserve
/// known step results and skipped work separately from the display message.
pub fn run(list: &[Candidate], after: &str) -> RunResult {
    run_with(list, after, |clone, step| match step {
        Step::Worktree { tree, .. } => remove_tree(clone, tree, false).map(|_| ()),
        Step::Branch { branch } => delete_branch(clone, branch).map(|_| ()),
        Step::After => crate::ops::after(clone, after).map(|_| ()),
    })
}

fn run_with(
    list: &[Candidate],
    after: &str,
    mut execute: impl FnMut(&Path, &Step) -> Result<(), String>,
) -> RunResult {
    let (mut trees, mut branches) = (0usize, 0usize);
    let mut hook_clones = BTreeSet::new();
    let mut steps = Vec::new();
    for c in list {
        if !c.tree.is_empty() {
            steps.push(StepResult {
                clone: c.clone.clone(),
                step: Step::Worktree {
                    tree: c.tree.clone(),
                    branch: c.branch.clone(),
                },
                status: StepStatus::Skipped,
            });
            hook_clones.insert(c.clone.clone());
        }
        steps.push(StepResult {
            clone: c.clone.clone(),
            step: Step::Branch {
                branch: c.branch.clone(),
            },
            status: StepStatus::Skipped,
        });
    }
    if !after.is_empty() {
        steps.extend(hook_clones.into_iter().map(|clone| StepResult {
            clone,
            step: Step::After,
            status: StepStatus::Skipped,
        }));
    }
    let said = |trees: usize, branches: usize| {
        let (t, b) = (
            if trees == 1 { "" } else { "s" },
            if branches == 1 { "" } else { "es" },
        );
        format!("{trees} worktree{t} and {branches} branch{b}")
    };
    let mut refusal = None;
    let mut hook_errors = String::new();
    for result in &mut steps {
        if refusal.is_some() {
            continue;
        }
        let invalid = match &result.step {
            Step::Branch { branch } => branch_refusal(branch),
            _ => None,
        };
        let answer = if let Some(why) = invalid {
            result.status = StepStatus::InvalidInput;
            Err(why)
        } else {
            let answer = execute(Path::new(&result.clone), &result.step);
            result.status = if answer.is_ok() {
                StepStatus::Succeeded
            } else {
                StepStatus::Unknown
            };
            answer
        };
        match (&result.step, answer) {
            (Step::Worktree { .. }, Ok(())) => trees += 1,
            (Step::Branch { .. }, Ok(())) => branches += 1,
            (Step::After, Ok(())) => {}
            (Step::After, Err(e)) => {
                hook_errors.push_str(&format!(
                    " · the after command failed in {}: {e}",
                    crate::local::tilde(&result.clone)
                ));
            }
            (step, Err(e)) => {
                let target = match step {
                    Step::Worktree { tree, .. } => crate::local::tilde(tree),
                    Step::Branch { branch } => branch.clone(),
                    Step::After => unreachable!(),
                };
                refusal = Some(format!(
                    "{target}: {e}; removed so far: {}",
                    said(trees, branches)
                ));
            }
        }
    }
    RunResult {
        said: match refusal {
            Some(why) => Err(why),
            None => Ok(format!("removed {}{hook_errors}", said(trees, branches))),
        },
        steps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::read;
    use std::path::PathBuf;
    use std::process::Command;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "radar-prune-{name}-{}-{}",
                std::process::id(),
                radar_core::time::now_secs()
            ));
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

    fn commit(dir: &Path, file: &str) {
        std::fs::write(dir.join(file), file).unwrap();
        sh(dir, &["add", "."]);
        sh(dir, &["commit", "-m", file]);
    }

    /// A clone of a bare origin, on `main`, with:
    /// - `done`: merged into main and pushed with it;
    /// - `gone`: pushed, then deleted on origin and pruned here;
    /// - `fresh`: a new branch where main is;
    /// - `wip`: one commit nobody has;
    /// - worktrees `wt-done` (merged, clean), `wt-dirty` (merged, a change)
    ///   and `wt-wip` (one commit nobody has).
    fn repos(s: &Scratch) -> PathBuf {
        let origin = s.0.join("origin.git");
        let seed = s.0.join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        sh(&s.0, &["init", "--bare", "origin.git"]);
        sh(&seed, &["init"]);
        commit(&seed, "a.txt");
        sh(
            &seed,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        sh(&seed, &["push", "origin", "main"]);
        sh(&seed, &["checkout", "-b", "gone"]);
        commit(&seed, "gone.txt");
        sh(&seed, &["push", "origin", "gone"]);
        sh(&seed, &["checkout", "main"]);
        let clone = s.0.join("code/api");
        std::fs::create_dir_all(clone.parent().unwrap()).unwrap();
        sh(
            &s.0,
            &["clone", origin.to_str().unwrap(), clone.to_str().unwrap()],
        );
        // merged branches: made here, merged into main, main pushed
        for name in ["done", "wt-done", "wt-dirty"] {
            sh(&clone, &["checkout", "-b", name]);
            commit(&clone, &format!("{name}.txt"));
            sh(&clone, &["checkout", "main"]);
            sh(&clone, &["merge", "--no-ff", "-m", name, name]);
        }
        sh(&clone, &["push", "origin", "main"]);
        sh(&clone, &["branch", "fresh"]);
        sh(&clone, &["checkout", "-b", "wip"]);
        commit(&clone, "wip.txt");
        sh(&clone, &["checkout", "main"]);
        // gone: tracked here, deleted on origin
        sh(&clone, &["branch", "--track", "gone", "origin/gone"]);
        sh(&seed, &["push", "origin", "--delete", "gone"]);
        sh(&clone, &["fetch", "--prune"]);
        for (name, new) in [("wt-done", false), ("wt-dirty", false), ("wt-wip", true)] {
            let dir = s.0.join(format!("code/api-{name}"));
            if new {
                sh(
                    &clone,
                    &["worktree", "add", "-b", name, dir.to_str().unwrap()],
                );
                commit(&dir, "x.txt");
            } else {
                sh(&clone, &["worktree", "add", dir.to_str().unwrap(), name]);
            }
        }
        std::fs::write(s.0.join("code/api-wt-dirty/a.txt"), "changed").unwrap();
        clone
    }

    #[test]
    fn prune_takes_merged_and_gone_clean_ones_and_keeps_the_rest() {
        let s = Scratch::new("select");
        let clone = repos(&s);
        let d = read(clone.to_str().unwrap());
        assert_eq!(d.error, "");
        assert_eq!(d.base, "main");
        let found: Vec<(String, String, Why)> = select([&d])
            .into_iter()
            .map(|c| {
                (
                    c.tree.rsplit('/').next().unwrap_or("").to_string(),
                    c.branch,
                    c.why,
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                (
                    "api-wt-done".to_string(),
                    "wt-done".to_string(),
                    Why::Merged
                ),
                ("".to_string(), "done".to_string(), Why::Merged),
                ("".to_string(), "gone".to_string(), Why::Gone),
            ],
            "main (current and base), fresh (nothing on it), wip (unmerged), wt-wip (unmerged) \
             and wt-dirty (a change in it) stay: {d:#?}"
        );
    }

    #[test]
    fn running_prune_removes_them_in_order_and_runs_the_after_command() {
        let s = Scratch::new("run");
        let clone = repos(&s);
        let d = read(clone.to_str().unwrap());
        let list = select([&d]);
        let after = s.0.join("after.log");
        let result = run(&list, &format!("touch {}", after.display()));
        assert_eq!(result.outcome(), Outcome::Succeeded);
        assert_eq!(result.steps.len(), 5);
        assert!(
            result
                .steps
                .iter()
                .all(|step| step.status == StepStatus::Succeeded)
        );
        let said = result.said.unwrap();
        assert_eq!(said, "removed 1 worktree and 3 branches");
        assert!(after.exists(), "the after command ran once in the clone");
        let branches = sh(&clone, &["branch", "--format=%(refname:short)"]);
        let mut left: Vec<&str> = branches.lines().collect();
        left.sort_unstable();
        assert_eq!(left, ["fresh", "main", "wip", "wt-dirty", "wt-wip"]);
        assert!(!s.0.join("code/api-wt-done").exists());
        assert!(
            s.0.join("code/api-wt-dirty").exists(),
            "the dirty worktree is untouched"
        );
    }

    #[test]
    fn a_refusal_stops_the_run_and_says_what_was_done() {
        let s = Scratch::new("stop");
        let clone = repos(&s);
        let d = read(clone.to_str().unwrap());
        let mut list = select([&d]);
        // Something got in the way since it was asked: the worktree has work.
        std::fs::write(s.0.join("code/api-wt-done/new.txt"), "x").unwrap();
        sh(&s.0.join("code/api-wt-done"), &["add", "new.txt"]);
        let result = run(&list, "");
        assert_eq!(result.outcome(), Outcome::Unknown);
        assert_eq!(result.steps[0].status, StepStatus::Unknown);
        assert!(
            result.steps[1..]
                .iter()
                .all(|step| step.status == StepStatus::Skipped)
        );
        let err = result.said.unwrap_err();
        assert!(
            err.contains("removed so far: 0 worktrees and 0 branches"),
            "{err}"
        );
        assert!(s.0.join("code/api-wt-done").exists());
        list.remove(0);
        assert_eq!(run(&list, "").outcome(), Outcome::Succeeded);
    }

    fn candidate(clone: &str, tree: &str, branch: &str) -> Candidate {
        Candidate {
            clone: clone.into(),
            tree: tree.into(),
            branch: branch.into(),
            why: Why::Merged,
            lost: 0,
        }
    }

    #[test]
    fn a_later_refusal_retains_prior_removals_and_all_skipped_steps() {
        let list = [
            candidate("/work/a", "", "done"),
            candidate("/work/b", "/work/b-tree", "tree"),
            candidate("/work/c", "", "later"),
        ];
        let mut calls = 0;
        let result = run_with(&list, "configured hook", |_, _| {
            calls += 1;
            if calls == 2 {
                Err("opaque refusal".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(calls, 2, "no later branch or hook is attempted");
        assert_eq!(result.outcome(), Outcome::Partial);
        assert_eq!(
            result.steps.iter().map(|s| s.status).collect::<Vec<_>>(),
            [
                StepStatus::Succeeded,
                StepStatus::Unknown,
                StepStatus::Skipped,
                StepStatus::Skipped,
                StepStatus::Skipped
            ]
        );
        assert_eq!(result.steps[4].step, Step::After);
        assert_eq!(result.steps[4].clone, "/work/b");
        assert!(
            result
                .said
                .unwrap_err()
                .ends_with("removed so far: 0 worktrees and 1 branch")
        );
    }

    #[test]
    fn a_removed_tree_then_a_refused_branch_is_partial() {
        let list = [candidate("/work/a", "/work/a-tree", "done")];
        let result = run_with(&list, "configured hook", |_, step| match step {
            Step::Worktree { .. } => Ok(()),
            Step::Branch { .. } => Err("opaque refusal".into()),
            Step::After => panic!("hooks wait for all removals"),
        });
        assert_eq!(result.outcome(), Outcome::Partial);
        assert_eq!(result.steps[0].status, StepStatus::Succeeded);
        assert_eq!(result.steps[1].status, StepStatus::Unknown);
        assert_eq!(result.steps[2].status, StepStatus::Skipped);
        assert!(
            result
                .said
                .unwrap_err()
                .ends_with("removed so far: 1 worktree and 0 branches")
        );
    }

    #[test]
    fn a_known_name_refusal_is_blocked_without_invoking_git() {
        for branch in ["", "-D"] {
            let list = [
                candidate("/work/a", "", branch),
                candidate("/work/a", "", "later"),
            ];
            let result = run_with(&list, "configured hook", |_, _| {
                panic!("preflight must refuse")
            });
            assert_eq!(result.outcome(), Outcome::Blocked);
            assert_eq!(result.steps[0].status, StepStatus::InvalidInput);
            assert_eq!(result.steps[1].status, StepStatus::Skipped);
            assert_eq!(result.steps.len(), 2, "branch-only runs have no hooks");
        }
        let result = run_with(&[candidate("/work/a", "/work/tree", "-D")], "", |_, _| {
            Ok(())
        });
        assert_eq!(
            result.outcome(),
            Outcome::Partial,
            "preflight can follow a known removal"
        );
    }

    #[test]
    fn hook_failure_is_partial_and_later_clones_still_run_once() {
        let list = [
            candidate("/work/b", "/work/b-tree", "same"),
            candidate("/work/a", "/work/a-tree", "same"),
            candidate("/work/a", "/work/a-other", "other"),
            candidate("/work/c", "", "branch-only"),
        ];
        let mut calls = Vec::new();
        let result = run_with(&list, "private configured command", |clone, step| {
            calls.push((clone.display().to_string(), step.clone()));
            if matches!(step, Step::After) && clone == Path::new("/work/a") {
                Err("private hook output".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(result.outcome(), Outcome::Partial);
        assert_eq!(calls.len(), 9);
        assert!(
            calls[..7]
                .iter()
                .all(|(_, step)| !matches!(step, Step::After))
        );
        assert_eq!(
            calls[7..],
            [
                ("/work/a".into(), Step::After),
                ("/work/b".into(), Step::After)
            ]
        );
        assert_eq!(result.steps[7].status, StepStatus::Unknown);
        assert_eq!(result.steps[8].status, StepStatus::Succeeded);
        let said = result.said.unwrap();
        assert_eq!(
            said,
            "removed 3 worktrees and 4 branches · the after command failed in /work/a: private hook output"
        );
    }

    #[test]
    fn no_candidates_is_success_without_work_or_hooks() {
        let result = run_with(&[], "configured hook", |_, _| panic!("no work"));
        assert_eq!(result.outcome(), Outcome::Succeeded);
        assert_eq!(result.said.unwrap(), "removed 0 worktrees and 0 branches");
        assert!(result.steps.is_empty());
    }

    #[test]
    fn single_deletes_refuse_what_git_refuses() {
        let s = Scratch::new("single");
        let clone = repos(&s);
        assert_eq!(
            delete_branch(&clone, "done").unwrap(),
            "deleted branch done"
        );
        assert!(
            delete_branch(&clone, "main").is_err(),
            "the branch checked out stays"
        );
        assert!(delete_branch(&clone, "-D").is_err());
        let dirty = s.0.join("code/api-wt-dirty");
        assert!(remove_tree(&clone, dirty.to_str().unwrap(), false).is_err());
        assert!(dirty.exists());
        assert!(remove_tree(&clone, dirty.to_str().unwrap(), true).is_ok());
        assert!(!dirty.exists());
    }
}
