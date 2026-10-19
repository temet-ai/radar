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
const READ_PROGRAMS: &[&str] =
    &["ls", "cat", "head", "tail", "wc", "pwd", "grep", "rg", "file", "stat", "nl", "jq", "cut", "tr"];
/// Arguments that make a reading program run or write something.
const DANGEROUS_ARGS: &[&str] =
    &["--output", "-o", "--ext-diff", "--textconv", "--exec", "--pre", "--open-files-in-pager", "-O", "--compress-program"];

pub fn decide(request: &PermissionRequest, cwd: &Path) -> Verdict {
    match (&request.kind, &request.target) {
        (PermissionKind::Write, _) | (_, PermissionTarget::Write { .. }) => {
            Verdict::Deny("this review session may not change files; report the change in your review instead")
        }
        (_, PermissionTarget::Mcp { server, tool, .. })
            if server == crate::tools::SERVER_NAME && radar_core::context::TOOLS.iter().any(|name| tool == name || tool == &format!("radar-{name}") || tool == &format!("mcp__radar__{name}")) => Verdict::Allow,
        (_, PermissionTarget::Mcp { .. }) => {
            Verdict::Deny("this review session may not use tools that post or change things; the review stays local")
        }
        (_, PermissionTarget::Url { .. }) => Verdict::Deny("this review session may not use the network"),
        (_, PermissionTarget::Path { path }) => {
            if inside(cwd, path) || tool_output(path) || in_workspace(cwd, path) {
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

/// `path` lies in the outermost Git repository that holds the worktree (a
/// workspace repository with the project's AGENTS.md and review checklist,
/// the worktree under its `repositories/`), never at or above the home folder.
/// Only a file or folder read asks this; commands still keep to the worktree.
fn in_workspace(cwd: &Path, path: &str) -> bool {
    if path.split('/').any(|part| part == "..") {
        return false;
    }
    let real = |q: &Path| std::fs::canonicalize(q).ok();
    let (Some(cwd), Some(path)) = (real(cwd), real(Path::new(path))) else { return false };
    let home = std::env::var_os("HOME").and_then(|h| real(Path::new(&h))).filter(|h| cwd.starts_with(h));
    let root = cwd
        .ancestors()
        .skip(1)
        .take_while(|a| home.as_deref().is_none_or(|h| *a != h))
        .filter(|a| a.join(".git").exists())
        .last();
    root.is_some_and(|r| path.starts_with(r))
}

/// A file the CLI wrote itself to hold a tool's output too long to show
/// (Copilot: `/tmp/<n>-copilot-tool-output-<id>.txt`). Reading it back is
/// reading what the session was already given.
fn tool_output(path: &str) -> bool {
    let p = Path::new(path);
    let in_tmp = p.parent().is_some_and(|d| d == Path::new("/tmp") || d == std::env::temp_dir());
    in_tmp && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.contains("-copilot-tool-output-"))
}

/// A command that only reads: one simple command, or a chain of them
/// (`a && b`, `a || b`, `a; b`, `a | head`) where every part reads.
fn read_only_command(command: &str, cwd: &Path) -> bool {
    let command = crate::settings::config::unwrap_shell(command);
    commands(command).is_some_and(|chain| chain.iter().all(|words| simple_read(words, cwd)))
}

/// The words of each command in a chain split at `;`, `&&`, `||` and `|`,
/// with quotes taken off. `None` when the line holds anything else the shell
/// acts on outside quotes: a redirection other than `2>/dev/null` or `2>&1`,
/// a lone `&`, a substitution, a glob, braces, a subshell, a leading `~`, a
/// new line. Inside double quotes `$` and backticks still count.
fn commands(line: &str) -> Option<Vec<Vec<String>>> {
    let mut chain = vec![];
    let mut words: Vec<String> = vec![];
    let mut word: Option<String> = None;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next()? {
                        '\'' => break,
                        q => w.push(q),
                    }
                }
            }
            '"' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next()? {
                        '"' => break,
                        '$' | '`' => return None,
                        '\\' => match chars.next()? {
                            e @ ('"' | '\\') => w.push(e),
                            '$' | '`' => return None,
                            e => {
                                w.push('\\');
                                w.push(e);
                            }
                        },
                        q => w.push(q),
                    }
                }
            }
            ' ' | '\t' => words.extend(word.take()),
            ';' | '|' | '&' => {
                let second = chars.next_if_eq(&c).is_some();
                if c == '&' && !second {
                    return None;
                }
                words.extend(word.take());
                if words.is_empty() {
                    return None;
                }
                chain.push(std::mem::take(&mut words));
            }
            '>' if word.as_deref() == Some("2") => {
                let rest: String = chars.clone().collect();
                let target = ["/dev/null", "&1"].into_iter().find(|t| rest.starts_with(t))?;
                if rest[target.len()..].starts_with(|c: char| !matches!(c, ' ' | '\t' | ';' | '|' | '&')) {
                    return None;
                }
                for _ in 0..target.len() {
                    chars.next();
                }
                word = None;
            }
            '~' if word.as_deref().is_none_or(|w| w.ends_with(['=', ':'])) => return None,
            '>' | '<' | '`' | '$' | '(' | ')' | '{' | '}' | '\\' | '*' | '?' | '!' | '#' | '\n' | '\r' => return None,
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word.take());
    if words.is_empty() {
        return None;
    }
    chain.push(words);
    Some(chain)
}

/// `sed` that only prints lines by number (`sed -n '18,52p'`): any other
/// script can write a file (`w`) or run one (`e`).
fn sed_prints(args: &[&str]) -> bool {
    let mut script = None;
    for a in args {
        match *a {
            "-n" | "--quiet" | "--silent" | "-E" | "-r" => {}
            a if a.starts_with('-') => return false,
            a if script.is_none() => script = Some(a),
            _ => {}
        }
    }
    script.is_some_and(|s| {
        s.split(';').all(|part| {
            part.trim().strip_suffix('p').is_some_and(|range| {
                !range.is_empty() && range.split(',').all(|n| n == "$" || (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))
            })
        })
    })
}

/// `git` subcommands that only list when given flags alone (`git branch -avv`,
/// `git remote -v`), and the flags that make them change something.
const GIT_LIST: &[&str] = &["branch", "remote", "tag", "stash"];
const GIT_LIST_CHANGES: &[&str] =
    &["-d", "-D", "-m", "-M", "-c", "-C", "-f", "-u", "--delete", "--move", "--copy", "--force", "--set-upstream-to", "--unset-upstream", "--edit-description"];

fn simple_read(words: &[String], cwd: &Path) -> bool {
    let Some((program, args)) = words.split_first() else { return false };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if args.iter().any(|a| DANGEROUS_ARGS.iter().any(|d| a == d || a.starts_with(&format!("{d}=")))) {
        return false;
    }
    if !args.iter().filter(|a| !a.starts_with('-')).all(|a| inside(cwd, a) || tool_output(a)) {
        return false;
    }
    match program.as_str() {
        "sed" => sed_prints(&args),
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
