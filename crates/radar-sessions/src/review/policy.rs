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

/// `path` is `cwd` or below it, with no `..` going out.
fn inside(cwd: &Path, path: &str) -> bool {
    // A range (`main...HEAD`) has `..` inside a word, not as a path part.
    if path.split('/').any(|part| part == "..") {
        return false;
    }
    let p = Path::new(path);
    p.is_relative() || p.starts_with(cwd)
}

/// One simple command that only reads. Anything with a shell operator,
/// substitution or redirection is not simple and goes to the operator.
fn read_only_command(command: &str, cwd: &Path) -> bool {
    if command.contains(['|', '&', ';', '>', '<', '`', '$', '\n', '(', ')', '{', '}', '\\', '*', '?', '~', '!']) {
        return false;
    }
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
        "git" => args.first().is_some_and(|sub| GIT_READ.contains(sub)),
        p => READ_PROGRAMS.contains(&p),
    }
}
