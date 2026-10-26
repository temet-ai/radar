//! Reviewing a merge request from its page: the Threads tab as a list
//! (a first row to comment on the merge request, then every thread with
//! its notes), and the keys that write (`crate::write`): `c` comments on
//! the diff's line, replies in the thread under it or the one at the
//! cursor, or comments on the merge request; `d` resolves a thread or
//! opens it again; `a` approves or takes the approval back. `⏎` on a
//! thread on the diff opens the Changes tab at its line.

use radar_core::plugin::{Binding, Effect, Outcome};
use radar_core::ui::diff::{self, Kind as LineKind};
use radar_core::view::ActionGroup;

use super::changes::{self, detail, file_diff};
use super::{Kind, Page, find_mr, mr};
use crate::GitlabPlugin;
use crate::model::{Discussion, MrDetail, MrState};
use crate::write::{self, Change, LineAt, Target};

/// The Threads tab's threads in their order: open ones first, then plain
/// comments, then resolved ones.
pub(crate) fn threads(d: &MrDetail) -> Vec<&Discussion> {
    let rank = |x: &Discussion| match (x.resolvable, x.resolved) {
        (true, false) => 0,
        (false, _) => 1,
        (true, true) => 2,
    };
    let mut ds: Vec<&Discussion> = d.discussions.iter().collect();
    ds.sort_by_key(|x| rank(x));
    ds
}

/// What a thread is on, in words: `StatsController.java:21`, or the
/// merge request.
pub(crate) fn place(x: &Discussion, iid: u64) -> String {
    x.place().unwrap_or_else(|| format!("!{iid}"))
}

/// The thread at the Threads tab's cursor (row 0 is the comment row).
fn at_row(d: &MrDetail, row: usize) -> Option<&Discussion> {
    threads(d).get(row.checked_sub(1)?).copied()
}

/// The Threads tab's rows: the comment row and the threads.
pub(super) fn len(p: &GitlabPlugin, project: &str, iid: u64) -> Option<usize> {
    detail(p, project, iid).map(|d| 1 + d.discussions.len())
}

/// `⏎` on the Threads tab: a thread on the diff opens there, at its line.
pub(super) fn enter(p: &GitlabPlugin, page: &Page, project: &str, iid: u64) -> bool {
    if !page.detail_painted.get() { return false; }
    let Some(d) = detail(p, project, iid) else { return false };
    let Some(x) = at_row(d, page.row.get()) else { return false };
    let Some((path, _, _)) = x.on_line() else { return false };
    let Some(i) = d.files.iter().position(|f| f.path == path) else { return false };
    page.file.set(i);
    page.placed.set(true);
    page.diff.reset();
    *page.jump.borrow_mut() = Some(x.id.clone());
    true
}

/// A comment on the diff's cursor line: its place, or why there is none.
fn line_at(p: &GitlabPlugin, page: &Page, project: &str, iid: u64) -> Result<LineAt, String> {
    let d = detail(p, project, iid).ok_or("the page has not loaded yet")?;
    let f = changes::file(p, page, project, iid).ok_or("no file open")?;
    let fd = file_diff(p, project, iid, &f.path).ok_or("its diff has not loaded yet")?;
    let refs = d
        .refs
        .clone()
        .ok_or("GitLab did not say which commits the diff is between: r loads the page again")?;
    let lines = diff::parse(&fd.diff);
    let l = lines.get(page.diff.cursor()).ok_or("no line here")?;
    match l.kind {
        LineKind::Hunk | LineKind::Note => Err("put the cursor on a line of code first".into()),
        _ => Ok(LineAt {
            new_path: fd.new_path.clone(),
            old_path: fd.old_path.clone(),
            old: if l.kind == LineKind::Added { None } else { l.old },
            new: l.new,
            refs,
        }),
    }
}

/// The thread a key acts on here: under the diff's cursor line (an open
/// one first), or at the Threads tab's cursor.
fn thread_here(p: &GitlabPlugin, page: &Page, project: &str, iid: u64) -> Option<Discussion> {
    if page.tab == mr::CHANGES {
        let mut here = changes::at_cursor(p, page, project, iid);
        here.sort_by_key(|x| !x.is_open());
        return here.into_iter().next();
    }
    if page.tab == mr::THREADS {
        return at_row(detail(p, project, iid)?, page.row.get()).cloned();
    }
    None
}

/// Whether `c`, `d` act on this tab (elsewhere `c` checks out).
pub(super) fn writes_here(page: &Page) -> bool {
    page.contributed().is_none() && (page.tab == mr::CHANGES || page.tab == mr::THREADS)
}

/// The question before approving a merge request (or taking your approval
/// back), or why not: the page's `a` and the tree row's.
pub(crate) fn ask_approve(p: &mut GitlabPlugin, project: &str, iid: u64) -> Vec<Effect> {
    let Some(mr) = find_mr(p, project, iid) else {
        return vec![Effect::Notify(format!("!{iid} is not in the last sync"))];
    };
    if mr.state != MrState::Opened {
        return vec![Effect::Notify(format!("!{iid} is {} already", mr.state.word()))];
    }
    let approve = !mr.approved_by.contains(&p.me);
    let sha = detail(p, project, iid).and_then(|d| d.refs.as_ref()).map(|r| r.head.clone());
    write::ask(p, Change::Approve { project: project.to_string(), iid, approve, sha })
}

/// `a`, `c`, `d` on a merge request's page; `None` when not theirs here.
pub(super) fn key(p: &mut GitlabPlugin, c: char) -> Option<Outcome> {
    let page = p.pages.last()?;
    let Kind::Mr { project, iid } = page.kind.clone() else { return None };
    let say = |m: String| Some(Outcome::Handled(vec![Effect::Notify(m)]));
    if matches!(c, 'c' | 'd') && writes_here(page)
        && (page.tab == mr::CHANGES && (!page.detail_painted.get() || !page.diff_painted.get())
            || page.tab == mr::THREADS && page.row.get() != 0 && !page.detail_painted.get())
    {
        return Some(Outcome::done());
    }
    match c {
        'a' => {
            find_mr(p, &project, iid)?;
            Some(Outcome::Handled(ask_approve(p, &project, iid)))
        }
        'c' if writes_here(page) => {
            if page.tab == mr::THREADS && page.row.get() == 0 {
                return Some(Outcome::Handled(write::start(p, &project, iid, Target::Note)));
            }
            if let Some(x) = thread_here(p, page, &project, iid) {
                let target = Target::Reply {
                    discussion: x.rest_id().to_string(),
                    place: place(&x, iid),
                };
                return Some(Outcome::Handled(write::start(p, &project, iid, target)));
            }
            match line_at(p, page, &project, iid) {
                Ok(at) => Some(Outcome::Handled(write::start(p, &project, iid, Target::Line(at)))),
                Err(why) => say(format!("cannot comment here: {why}")),
            }
        }
        'd' if writes_here(page) => {
            let Some(x) = thread_here(p, page, &project, iid) else {
                return say("no thread here to resolve".into());
            };
            if !x.resolvable {
                return say("a plain comment cannot be resolved".into());
            }
            let change = Change::Resolve {
                project,
                iid,
                discussion: x.rest_id().to_string(),
                place: place(&x, iid),
                resolve: !x.resolved,
            };
            Some(Outcome::Handled(write::ask(p, change)))
        }
        _ => None,
    }
}

/// The rows of the actions menu for these keys, under `on GitLab`.
pub(super) fn bindings(p: &GitlabPlugin, page: &Page) -> Vec<crate::Row> {
    use crate::act;
    let Kind::Mr { project, iid } = &page.kind else { return vec![] };
    let (project, iid) = (project.as_str(), *iid);
    let Some(mr) = find_mr(p, project, iid) else { return vec![] };
    let mut b = Vec::new();
    if mr.state == MrState::Opened {
        let mine = mr.approved_by.contains(&p.me);
        b.push(act(
            "gitlab.mr.approve",
            Binding::new(
                "a",
                if mine { "take back your approval…" } else { "approve…" },
            ).edits().group(ActionGroup::Change),
        ));
    }
    if !writes_here(page) {
        return b;
    }
    let here = thread_here(p, page, project, iid);
    if page.tab == mr::THREADS
        && let Some((path, _, _)) = here.as_ref().and_then(|x| x.on_line())
    {
        let file = path.rsplit('/').next().unwrap_or(path);
        b.push(act("gitlab.mr.open_thread_in_diff", Binding::new("⏎", format!("open it in the diff of {file}, at its line"))));
    }
    match &here {
        Some(x) => b.push(act("gitlab.mr.comment", Binding::new("c", format!("comment in the thread on {}…", place(x, iid))).adds().group(ActionGroup::Change))),
        None if page.tab == mr::THREADS => b.push(act("gitlab.mr.comment", Binding::new("c", format!("comment on !{iid}…")).adds().group(ActionGroup::Change))),
        None => match line_at(p, page, project, iid) {
            Ok(at) => b.push(act("gitlab.mr.comment", Binding::new("c", format!("comment on {}…", at.words())).adds().group(ActionGroup::Change))),
            Err(why) => b.push(act("gitlab.mr.comment", Binding::new("c", "comment").hint(why.to_string()).group(ActionGroup::Change))),
        },
    }
    if let Some(x) = here.filter(|x| x.resolvable) {
        b.push(act(
            "gitlab.mr.resolve_thread",
            Binding::new(
                "d",
                if x.resolved {
                    format!("open the thread on {} again…", place(&x, iid))
                } else {
                    format!("resolve the thread on {}…", place(&x, iid))
                },
            ).edits().group(ActionGroup::Change),
        ));
    }
    b
}

/// Whether the composer is under the diff's cursor line on this page.
pub(crate) fn inline(p: &GitlabPlugin, page: &Page) -> bool {
    let Some(c) = &p.compose else { return false };
    let Kind::Mr { project, iid } = &page.kind else { return false };
    if page.tab != mr::CHANGES || c.project != *project || c.iid != *iid {
        return false;
    }
    match &c.target {
        Target::Line(at) => line_at(p, page, project, *iid).is_ok_and(|here| here == *at),
        Target::Reply { discussion, .. } => changes::at_cursor(p, page, project, *iid)
            .iter()
            .any(|x| x.rest_id() == discussion),
        Target::Note => false,
    }
}
