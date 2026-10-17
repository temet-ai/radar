//! How git joins the other parts (radar-core's `compose`). It owns nothing yet;
//! it adds to what the hosts draw, knowing only the shared vocabulary:
//!
//! - `repo`: its full path is its id (and `project`);
//! - `merge-request`: `project`, the repo's full path, and `branch`, its
//!   source branch.
//!
//! To each, from the index of checkouts (never read here): the line under
//! its title (`checkout ~/code/api · java · on main`, or why there is none
//! and where to set the folders), and the actions `e` an editor on the
//! checkout, `t` a terminal in it, `s` look again; on a merge request also
//! `c` check out its branch, asked first.

use std::path::Path;

use radar_core::plugin::{Effect, Menu, Tone};
use radar_core::ui::icon::Icon;
use radar_core::{Action, Adds, Chip, Contribution, Point, Subject};

use crate::GitPlugin;
use crate::act;
use crate::local::{self, Checkout};

const REPO: &str = "repo";
const MERGE_REQUEST: &str = "merge-request";

pub(crate) const ADDS: [Adds; 2] = [
    Adds::new(
        REPO,
        "where it is checked out here, an editor and a terminal there",
    ),
    Adds::new(
        MERGE_REQUEST,
        "check out its branch here, an editor and a terminal",
    ),
];

/// The menu that asks before a branch is checked out: `git-checkout:dir\tbranch`.
const CHECKOUT_MENU: &str = "git-checkout";

/// The repo a subject is in, and the branch it is about (a merge request's).
fn about(s: &Subject) -> Option<(&str, Option<&str>)> {
    if s.is(REPO) {
        Some((s.field("project").unwrap_or(&s.id), None))
    } else if s.is(MERGE_REQUEST) {
        Some((
            s.field("project")?,
            s.field("branch").filter(|b| !b.is_empty()),
        ))
    } else {
        None
    }
}

/// The checkout acted on: the one picked, else the first.
fn checkout<'a>(p: &'a GitPlugin, project: &str) -> Option<&'a Checkout> {
    local::chosen(p, project)
}

/// A row with no key of its own: chosen from the `x` menu.
fn keyless(what: impl Into<String>, id: &str, icon: Icon) -> Action {
    let mut a = Action::new(' ', what, id).icon(icon);
    a.key = None;
    a
}

/// The ticket a subject is about, for naming a branch: the `key` field, else
/// the first key written in its branch, title or label.
fn ticket_of(s: &Subject) -> Option<(String, String, String)> {
    let title = s.field("title").unwrap_or(&s.label).to_string();
    let key = s
        .field("key")
        .filter(|k| !k.is_empty())
        .map(str::to_string)
        .or_else(|| s.field("branch").and_then(act::find_key))
        .or_else(|| act::find_key(&title))
        .or_else(|| act::find_key(&s.label))?;
    Some((key, title, s.field("type").unwrap_or_default().to_string()))
}

/// Why there is no checkout to act on, and what to do about it.
pub(crate) fn no_checkout(p: &GitPlugin) -> String {
    if p.local.scanning {
        "still looking for checkouts on this machine".into()
    } else if p.cfg.roots.is_empty() {
        "no local checkout known: add the folders you keep code in, in settings, git › checkouts"
            .into()
    } else {
        let roots: Vec<String> = p.cfg.roots.iter().map(|r| local::display_path(p, r)).collect();
        format!("no checkout of this repo in {}", roots.join(", "))
    }
}

/// The line under the title: where it is checked out here.
fn line(p: &GitPlugin, project: &str, branch: Option<&str>) -> Vec<Chip> {
    let found = local::checkouts_for(p, project);
    let mut out = vec![Chip::new("checkout  ", Tone::Muted).icon(Icon::Local)];
    let Some(c) = local::chosen(p, project) else {
        out.push(Chip::new(no_checkout(p), Tone::Muted));
        return out;
    };
    // A terminal there and an editor on it, a click away, first: a long
    // path is what gives way at the end of the line.
    out.push(Chip::new("terminal", Tone::Accent).icon(Icon::Terminal).acts("terminal"));
    out.push(Chip::new("  ", Tone::Muted));
    out.push(Chip::new("editor", Tone::Accent).icon(Icon::Editor).acts("editor"));
    out.push(Chip::new("  ", Tone::Muted));
    out.push(Chip::new(local::display_path(p, &c.dir), Tone::Plain));
    if let Some(k) = p.cfg.type_of(c) {
        out.push(Chip::new(format!(" · {}", k.name), Tone::Muted));
    }
    if p.local.switching.contains(&c.dir) {
        out.push(Chip::new(" · checking out…", Tone::Info));
    } else if branch.is_some_and(|b| b == c.branch) {
        out.push(Chip::new(" · ", Tone::Good));
        out.push(Chip::new(format!("✓ on {}", c.branch), Tone::Good).icon(Icon::Branch));
    } else if !c.branch.is_empty() {
        out.push(Chip::new(" · ", Tone::Muted));
        out.push(Chip::new(format!("on {}", c.branch), Tone::Muted).icon(Icon::Branch));
    }
    if found.len() > 1 {
        out.push(Chip::new(
            format!(" · {} more", found.len() - 1),
            Tone::Muted,
        ));
    }
    out
}

/// What can be done here to the checkout of `project`.
fn actions(p: &GitPlugin, project: &str, branch: Option<&str>) -> Vec<Action> {
    let Some(c) = checkout(p, project) else {
        let what = if p.cfg.roots.is_empty() {
            "look for a checkout · no folders set yet"
        } else {
            "look for a checkout again"
        };
        return vec![
            Action::new('s', what, "scan").icon(Icon::Search),
            keyless("link to a local folder…", "link", Icon::Link),
        ];
    };
    let mut out = Vec::new();
    if let Some(b) = branch {
        let what = if p.local.switching.contains(&c.dir) {
            format!("check out {b} · checking out…")
        } else if c.branch == b {
            format!("update {b} from origin…")
        } else {
            format!("check out {b}…")
        };
        out.push(Action::new('c', what, "checkout").icon(Icon::Branch));
    }
    let editor = p
        .cfg
        .type_of(c)
        .map(|k| k.editor.clone())
        .unwrap_or_default();
    // On a merge request whose branch the checkout is not on, they open on
    // that branch (asked where), and say so.
    let on = branch.filter(|b| *b != c.branch).map(|b| format!(" on {b}…")).unwrap_or_default();
    let editor_words = match (editor.is_empty(), on.is_empty()) {
        (true, true) => "open in an editor…".to_string(),
        (true, false) => format!("open in an editor{on}"),
        (false, _) => format!("open in {editor}{on}"),
    };
    out.push(Action::new('e', editor_words, "editor").icon(Icon::Editor));
    let terminal_words = if on.is_empty() { "open a terminal there".to_string() } else { format!("open a terminal{on}") };
    out.push(Action::new('t', terminal_words, "terminal").icon(Icon::Terminal));
    out.push(Action::new('s', "look for checkouts again", "scan").icon(Icon::Search));
    out.push(Action::new('n', "new branch…", "new").icon(Icon::Branch));
    // GitLab's `u` on a merge request page is its own rebase: here the row has no key.
    let base = act::base_for(p, project);
    let onto = if base.is_empty() {
        "its base".to_string()
    } else {
        base
    };
    let rebase = format!("rebase on {onto}…");
    out.push(if branch.is_some() {
        keyless(rebase, "rebase", Icon::Branch)
    } else {
        Action::new('u', rebase, "rebase").icon(Icon::Branch)
    });
    if !c.branch.is_empty() && !c.branch.starts_with("detached") {
        // Not `U`: a capital is a screen's, and an action a part adds to another's menu is never one (GUIDE 2.26).
        out.push(keyless(format!("push {}…", c.branch), "push", Icon::Branch));
    }
    out.push(keyless("park on a worktree…", "park", Icon::Branch));
    out.push(keyless("link to another local folder…", "link", Icon::Link));
    if local::checkouts_for(p, project).len() > 1 {
        out.push(keyless("use another worktree…", "use", Icon::Local));
    }
    out
}

pub(crate) fn contribute(p: &GitPlugin, point: Point, s: &Subject) -> Option<Contribution> {
    let (project, branch) = about(s)?;
    match point {
        Point::Line => Some(Contribution::Line(line(p, project, branch))),
        // A repo cloned here: the clone's icon, and the branch it is on.
        Point::Mark if s.is(REPO) => {
            let c = local::chosen(p, project)?;
            let words = vec![Chip::new("  ", Tone::Muted), Chip::new(c.branch.clone(), Tone::Accent).icon(Icon::Branch)];
            Some(Contribution::Mark { icon: Icon::Local, words })
        }
        Point::Actions => Some(Contribution::Actions(actions(p, project, branch))),
        _ => None,
    }
}

/// One of the actions git added was chosen.
pub(crate) fn act(p: &mut GitPlugin, s: &Subject, action: &str) -> Vec<Effect> {
    let Some((project, branch)) = about(s) else {
        return vec![];
    };
    if action == "scan" {
        return local::scan_now(p, true);
    }
    if action == "link" {
        return act::ask_link(p, project);
    }
    let Some(c) = checkout(p, project).cloned() else {
        return vec![Effect::Notify(no_checkout(p))];
    };
    match action {
        "new" | "park" => {
            // A branch is named from a ticket: its key is in the page's branch or title.
            let Some((key, title, kind)) = ticket_of(s) else {
                return vec![Effect::Notify(
                    "no ticket key on this page to name a branch from: start work on the ticket (s there), or see Settings › git › branches".into(),
                )];
            };
            if let Some(why) = act::name_problem(p, project) {
                return vec![Effect::Notify(why)];
            }
            let name = act::branch_for(
                p,
                project,
                &crate::branch::Ticket {
                    key: &key,
                    title: &title,
                    kind: &kind,
                },
            );
            let op = if action == "new" {
                act::Op::New
            } else {
                act::Op::Park
            };
            act::begin(p, op, project, &c, name)
        }
        "rebase" => act::begin(p, act::Op::Rebase, project, &c, String::new()),
        "push" => act::begin(p, act::Op::Push, project, &c, String::new()),
        "use" => act::ask_use(p, project),
        // On a merge request: on its branch (asked where, when no checkout is).
        "editor" | "terminal" => match branch {
            Some(b) if b != c.branch => act::open_on(p, action, project, &c, b),
            _ => act::open_in(p, action, &c),
        },
        "checkout" => match branch {
            Some(b) => ask_checkout(&c, b),
            None => vec![],
        },
        _ => vec![],
    }
}

/// The question before `branch` is checked out in `c`.
fn ask_checkout(c: &Checkout, branch: &str) -> Vec<Effect> {
    // The page shows the whole path; a long one is its folder here.
    let mut place = local::tilde(&c.dir);
    if place.chars().count() > 40 {
        place = Path::new(&c.dir)
            .file_name()
            .map_or(place, |n| n.to_string_lossy().into_owned());
    }
    vec![Effect::Menu(
        Menu::confirm(
            format!("{CHECKOUT_MENU}:{}\t{branch}", c.dir),
            format!("check out {branch} in {place}?"),
            "check it out",
            format!("stay on {}", c.branch),
        )
        .saying("fetches it and switches; refuses while files have uncommitted changes")
        .icon(Icon::Local),
    )]
}

/// A choice in a menu git opened; `None` if not one of these.
pub(crate) fn on_menu(p: &mut GitPlugin, menu: &str, value: &str) -> Option<Vec<Effect>> {
    let (kind, arg) = menu.split_once(':')?;
    match kind {
        local::PICK_EDITOR => Some(local::on_pick(p, arg, value)),
        act::OPEN_MENU => Some(act::on_open(p, arg, value)),
        CHECKOUT_MENU => {
            if value != "yes" {
                return Some(vec![]);
            }
            let (dir, branch) = arg.split_once('\t')?;
            Some(local::switch_branch(p, dir, branch))
        }
        _ => None,
    }
}
