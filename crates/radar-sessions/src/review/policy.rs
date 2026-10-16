//! What a review session may do without asking the operator.
//!
//! The operator's rule (2026-10-13): reads and `git diff|log|show` run; any
//! other command runs only with a per-request approval in the dock; file
//! writes are denied; nothing posts. [`decide`] is a pure function of one
//! permission request. `Allow` and `Deny` are answered by the part for review
//! sessions only (so a session the operator started keeps asking); `Ask`
//! leaves the request waiting in the store until the dock answers it.

use std::path::Path;

use crate::event::{PermissionKind, PermissionRequest, PermissionTarget};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// A read that cannot change anything.
    Allow,
    /// Refused with the reason the model is told.
    Deny(&'static str),
    /// The operator decides, in the dock, for this one request.
    Ask,
}

/// `git` subcommands that only read (the program, not a part).
const GIT_READ: &[&str] = &[
    "diff", "log", "show", "status", "blame", "rev-parse", "rev-list", "ls-files", "ls-tree", "cat-file",
    "merge-base", "shortlog", "describe", "grep",
];
/// Programs that only read files given as arguments.
const READ_PROGRAMS: &[&str] = &["ls", "cat", "head", "tail", "wc", "pwd", "grep", "rg", "file", "stat"];
/// Arguments that make a reading program run or write something.
const DANGEROUS_ARGS: &[&str] = &["--output", "-o", "--ext-diff", "--textconv", "--exec", "--pre", "--open-files-in-pager", "-O"];

pub fn decide(request: &PermissionRequest, cwd: &Path) -> Verdict {
    match (&request.kind, &request.target) {
        (PermissionKind::Write, _) | (_, PermissionTarget::Write { .. }) => {
            Verdict::Deny("this review session may not change files; report the change in your review instead")
        }
        (_, PermissionTarget::Mcp { .. }) => {
            Verdict::Deny("this review session may not use tools that post or change things; the review stays local")
        }
        (_, PermissionTarget::Url { .. }) => Verdict::Deny("this review session may not use the network"),
        (_, PermissionTarget::Path { path }) => {
            if inside(cwd, path) {
                Verdict::Allow
            } else {
                Verdict::Ask
            }
        }
        (_, PermissionTarget::Command { command, .. }) => {
            if read_only_command(command, cwd) {
                Verdict::Allow
            } else {
                Verdict::Ask
            }
        }
        (_, PermissionTarget::Other { .. }) => Verdict::Ask,
    }
}

/// `path` is `cwd` or below it, with no `..` going out. A worktree reached
/// through a symlink (`~/projects` → `/data/projects`) is the same folder.
fn inside(cwd: &Path, path: &str) -> bool {
    // A range (`main...HEAD`) has `..` inside a word, not as a path part.
    if path.split('/').any(|part| part == "..") {
        return false;
    }
    let p = Path::new(path);
    let real = |q: &Path| std::fs::canonicalize(q).unwrap_or_else(|_| q.to_path_buf());
    p.is_relative() || p.starts_with(cwd) || real(p).starts_with(real(cwd))
}

/// A command that only reads: one simple command, or a chain of them
/// (`a && b`, `a || b`, `a; b`, `a | head`) where every part reads. Any other
/// shell operator, substitution or redirection goes to the operator; only
/// `2>/dev/null` is let through, as it writes nothing.
fn read_only_command(command: &str, cwd: &Path) -> bool {
    let command = crate::settings::config::unwrap_shell(command);
    if command.contains('\n') {
        return false;
    }
    let chain = command.replace("2>/dev/null", " ").replace("&&", ";").replace("||", ";");
    if chain.contains(['&', '>', '<', '`', '$', '(', ')', '{', '}', '\\', '*', '?', '~', '!']) {
        return false;
    }
    chain.split([';', '|']).all(|part| simple_read(part.trim(), cwd))
}

/// `git` subcommands that only list when given flags alone (`git branch -avv`,
/// `git remote -v`), and the flags that make them change something.
const GIT_LIST: &[&str] = &["branch", "remote", "tag", "stash"];
const GIT_LIST_CHANGES: &[&str] =
    &["-d", "-D", "-m", "-M", "-c", "-C", "-f", "-u", "--delete", "--move", "--copy", "--force", "--set-upstream-to", "--unset-upstream", "--edit-description"];

fn simple_read(command: &str, cwd: &Path) -> bool {
    let mut words = command.split_whitespace();
    let Some(program) = words.next() else { return false };
    let args: Vec<&str> = words.collect();
    if args.iter().any(|a| DANGEROUS_ARGS.iter().any(|d| a == d || a.starts_with(&format!("{d}=")))) {
        return false;
    }
    if !args.iter().filter(|a| !a.starts_with('-')).all(|a| inside(cwd, a)) {
        return false;
    }
    match program {
        "git" => match args.split_first() {
            Some((sub, _)) if GIT_READ.contains(sub) => true,
            // `git stash list`, `git remote -v`, `git branch -avv`: listing only.
            Some((&"stash", rest)) => rest == ["list"],
            Some((sub, rest)) if GIT_LIST.contains(sub) => rest.iter().all(|a| a.starts_with('-') && !GIT_LIST_CHANGES.contains(a)),
            _ => false,
        },
        p => READ_PROGRAMS.contains(&p),
    }
}
