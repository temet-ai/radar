//! A merge request's Changes tab: its files beside the diff of the one
//! open (board D). The files come with the page; a file's diff is asked of
//! GitLab when it is opened, a few files at a time, and kept until the
//! merge request's head moves. Only the rows in view are drawn, so a long
//! file pages like any other.
//!
//! Keys: `↑↓ j k PgUp PgDn g G` move the cursor through the diff, `n`/`N`
//! the next and previous file, `}`/`{` the next and previous hunk (past
//! the last hunk, the next file's first).

use std::collections::BTreeSet;

use radar_core::plugin::{Binding, Effect, Outcome, Pane};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent};
use radar_core::ui::diff::{self, DiffLine};
use radar_core::ui::{Node, tree};
use radar_core::ui;

use super::{Kind, Page, mr_key};
use crate::client::DIFF_PAGE;
use crate::model::{Discussion, FileDiff, FileStat, MrDetail};
use crate::GitlabPlugin;

/// A merge request's file diffs, as far as they were asked for.
#[derive(Debug, Default)]
pub(crate) struct Diffs {
    /// The head commit they were asked for at; a new one drops them.
    pub head: String,
    /// By the file's new path.
    pub files: std::collections::BTreeMap<String, FileDiff>,
    /// Pages asked for (answered or not), and those not answered yet.
    pub asked: BTreeSet<usize>,
    pub loading: BTreeSet<usize>,
    /// Exact pending dispatch owner per page, in this cached head.
    pub requests: std::collections::BTreeMap<usize, crate::page_ownership::Read>,
    pub error: Option<String>,
    /// A page came back short: there are none after it.
    pub ended: Option<usize>,
}

pub(crate) fn detail<'a>(p: &'a GitlabPlugin, project: &str, iid: u64) -> Option<&'a MrDetail> {
    p.mr_pages
        .get(&mr_key(project, iid))
        .and_then(|f| f.value.as_ref())
        .map(|(_, d)| d)
}

/// The file the page has open, if the page's files are known.
pub(crate) fn file<'a>(p: &'a GitlabPlugin, page: &Page, project: &str, iid: u64) -> Option<&'a FileStat> {
    let d = detail(p, project, iid)?;
    d.files.get(page.file.get().min(d.files.len().saturating_sub(1)))
}

/// Its diff, once GitLab sent it.
pub(crate) fn file_diff<'a>(p: &'a GitlabPlugin, project: &str, iid: u64, path: &str) -> Option<&'a FileDiff> {
    p.diffs.get(&mr_key(project, iid))?.files.get(path)
}

/// The open file's lines, parsed.
pub(crate) fn lines(p: &GitlabPlugin, page: &Page, project: &str, iid: u64) -> Vec<DiffLine> {
    file(p, page, project, iid)
        .and_then(|f| file_diff(p, project, iid, &f.path))
        .map(|d| diff::parse(&d.diff))
        .unwrap_or_default()
}

/// Ask GitLab for the open file's diff if it is not held: the page of
/// files it should be on, else the first page not asked yet.
pub(crate) fn want(p: &mut GitlabPlugin) -> Vec<Effect> {
    let Some(page) = p.pages.last() else { return vec![] };
    let Kind::Mr { project, iid } = page.kind.clone() else { return vec![] };
    if page.tab != super::mr::CHANGES {
        return vec![];
    }
    let Some(d) = detail(p, &project, iid) else { return vec![] };
    // The first time, the file open is the tree's first.
    if !page.placed.replace(true)
        && let Some(first) = order(&entries(&d.files)).first()
    {
        page.file.set(*first);
    }
    let n = d.files.len();
    let at = page.file.get().min(n.saturating_sub(1));
    let Some(path) = d.files.get(at).map(|f| f.path.clone()) else { return vec![] };
    let head = d.refs.as_ref().map(|r| r.head.clone()).unwrap_or_default();
    let key = mr_key(&project, iid);
    let ds = p.diffs.entry(key.clone()).or_default();
    if ds.head != head {
        *ds = Diffs {
            head: head.clone(),
            ..Diffs::default()
        };
    }
    page.seen.borrow_mut().insert(path.clone());
    if ds.files.contains_key(&path) || !ds.loading.is_empty() {
        return vec![];
    }
    let pages = n.div_ceil(DIFF_PAGE).max(1);
    let last = ds.ended.unwrap_or(pages).min(pages);
    let first = at / DIFF_PAGE + 1;
    let Some(want) = std::iter::once(first)
        .chain(1..=last)
        .find(|pg| *pg <= last && !ds.asked.contains(pg))
    else {
        return vec![];
    };
    ds.asked.insert(want);
    ds.loading.insert(want);
    ds.error = None;
    let url = p.cfg.url.clone();
    let provider = p.provider();
    let action = crate::diff_history::action(&url, &project, iid, want);
    let read = crate::page_ownership::next(p);
    p.diffs.get_mut(&key).expect("dispatch cache").requests.insert(want, read);
    vec![crate::diff_history::spawn(
        action,
        key,
        read,
        head,
        want,
        async move {
            provider.run(|c| {
                let project = project.clone();
                async move { c.diffs_observed(&project, iid, want).await }
            })
            .await
            .map_err(|e| format!("{e:#}"))
        },
    )]
}

/// A page of diffs arrived. One for an older head is dropped.
pub(crate) fn on_diffs(
    p: &mut GitlabPlugin,
    key: String,
    head: String,
    page: usize,
    result: Result<Vec<FileDiff>, String>,
) -> Vec<Effect> {
    let Some(ds) = p.diffs.get_mut(&key) else { return vec![] };
    if ds.head != head {
        return vec![];
    }
    ds.loading.remove(&page);
    match result {
        Ok(files) => {
            if files.len() < DIFF_PAGE {
                ds.ended = Some(page);
            }
            for f in files {
                ds.files.insert(f.new_path.clone(), f);
            }
        }
        // Asked again on the next look.
        Err(e) => {
            ds.asked.remove(&page);
            ds.error = Some(e);
            return vec![];
        }
    }
    super::unpaint(p, &key, false);
    want(p)
}

/// True while a diff is being fetched.
pub(crate) fn loading(p: &GitlabPlugin) -> bool {
    p.diffs.values().any(|d| !d.loading.is_empty())
}

// --- drawing -------------------------------------------------------------------

/// `src/main/…/stats/` and `StatsController.java`: a path's folder, shortened
/// to `room`, and its name.
pub(super) fn split(path: &str, room: usize) -> (String, String) {
    let (dir, name) = match path.rsplit_once('/') {
        Some((dir, name)) => (format!("{dir}/"), name.to_string()),
        None => (String::new(), path.to_string()),
    };
    // A name longer than the room ends in `…`, without its folder.
    if room > 0 && ui::width(&name) > room {
        return (String::new(), ui::fit(&name, room).trim_end().to_string());
    }
    let room = room.saturating_sub(ui::width(&name));
    if ui::width(&dir) <= room {
        return (dir, name);
    }
    // Keep the end of the folder, where it differs.
    let keep = room.saturating_sub(1);
    let tail: String = dir.chars().rev().take(keep).collect::<Vec<_>>().into_iter().rev().collect();
    (if keep == 0 { String::new() } else { format!("…{tail}") }, name)
}

// --- the folder tree -----------------------------------------------------------

/// A row of the files tree: a folder (its collapsed name, `src/main/java`,
/// the sums of the files under it) or a file (its index in the merge
/// request's list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Entry {
    Dir {
        id: String,
        name: String,
        adds: u32,
        dels: u32,
        kids: Vec<Entry>,
    },
    File {
        index: usize,
        name: String,
        adds: u32,
        dels: u32,
    },
}

impl Entry {
    pub(super) fn id(&self) -> String {
        match self {
            Self::Dir { id, .. } => id.clone(),
            Self::File { index, .. } => format!("f:{index}"),
        }
    }

    fn counts(&self) -> (u32, u32) {
        match self {
            Self::Dir { adds, dels, .. } | Self::File { adds, dels, .. } => (*adds, *dels),
        }
    }
}

#[derive(Default)]
struct Trie {
    dirs: std::collections::BTreeMap<String, Trie>,
    files: Vec<(String, usize)>,
}

/// The files as GitLab's file tree shows them: folders first, then files,
/// each alphabetical; a folder whose only child is a folder joins it
/// (`src/main/java/acme/stats` is one row).
pub(super) fn entries(files: &[FileStat]) -> Vec<Entry> {
    let mut root = Trie::default();
    for (i, f) in files.iter().enumerate() {
        let mut parts: Vec<&str> = f.path.split('/').collect();
        let name = parts.pop().unwrap_or_default();
        let mut at = &mut root;
        for dir in parts {
            at = at.dirs.entry(dir.to_string()).or_default();
        }
        at.files.push((name.to_string(), i));
    }
    level(&root, "", files)
}

fn level(t: &Trie, prefix: &str, files: &[FileStat]) -> Vec<Entry> {
    let mut out = Vec::new();
    for (name, sub) in &t.dirs {
        let mut path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let mut label = name.clone();
        let mut cur = sub;
        while cur.files.is_empty() && cur.dirs.len() == 1 {
            let (n, s) = cur.dirs.iter().next().expect("one child");
            path = format!("{path}/{n}");
            label = format!("{label}/{n}");
            cur = s;
        }
        let kids = level(cur, &path, files);
        let (adds, dels) = kids
            .iter()
            .map(Entry::counts)
            .fold((0, 0), |(a, d), (x, y)| (a + x, d + y));
        out.push(Entry::Dir {
            id: format!("d:{path}"),
            name: label,
            adds,
            dels,
            kids,
        });
    }
    let mut here = t.files.clone();
    here.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()).then_with(|| a.0.cmp(&b.0)));
    for (name, index) in here {
        let f = &files[index];
        out.push(Entry::File {
            index,
            name,
            adds: f.additions,
            dels: f.deletions,
        });
    }
    out
}

/// The files' indexes in the order the tree shows them, folders skipped:
/// what `n` and `N` step through.
pub(super) fn order(entries: &[Entry]) -> Vec<usize> {
    let mut out = Vec::new();
    for e in entries {
        match e {
            Entry::File { index, .. } => out.push(*index),
            Entry::Dir { kids, .. } => out.extend(order(kids)),
        }
    }
    out
}

/// The ids from the top down to the row `id`.
pub(super) fn path_to(entries: &[Entry], id: &str) -> Option<Vec<String>> {
    for e in entries {
        if e.id() == id {
            return Some(vec![e.id()]);
        }
        if let Entry::Dir { kids, .. } = e
            && let Some(mut rest) = path_to(kids, id)
        {
            rest.insert(0, e.id());
            return Some(rest);
        }
    }
    None
}

/// Just ids and children, for the kit to open each folder the first time.
pub(super) fn skeleton(entries: &[Entry]) -> Vec<Node> {
    entries
        .iter()
        .map(|e| match e {
            Entry::Dir { kids, .. } => Node::new(e.id(), vec![]).children(skeleton(kids)),
            Entry::File { .. } => Node::new(e.id(), vec![]),
        })
        .collect()
}

/// `label` kept to `room` cells, losing its start (where folders agree).
pub(super) fn tail_fit(label: &str, room: usize) -> String {
    if ui::width(label) <= room {
        return label.to_string();
    }
    let keep = room.saturating_sub(1);
    let tail: String = label.chars().rev().take(keep).collect::<Vec<_>>().into_iter().rev().collect();
    if keep == 0 { String::new() } else { format!("…{tail}") }
}

/// The threads on a file, each with the line of the diff it is under
/// (`None`: a line the diff no longer shows). A thread names its new line
/// (kept and added lines) or only its old one (removed lines).
pub(crate) fn threads_on<'a>(d: &'a MrDetail, fd: &FileDiff, lines: &[DiffLine]) -> Vec<(&'a Discussion, Option<usize>)> {
    d.discussions
        .iter()
        .filter_map(|x| {
            let (path, old, new) = x.on_line()?;
            if path != fd.new_path && path != fd.old_path {
                return None;
            }
            let at = lines.iter().position(|l| match (new, old) {
                (Some(n), Some(o)) => l.new == Some(n) && l.old == Some(o),
                (Some(n), None) => l.new == Some(n),
                (None, Some(o)) => l.old == Some(o) && l.new.is_none(),
                (None, None) => false,
            });
            // A kept line named by its new number alone, or the other way.
            let at = at.or_else(|| new.and_then(|n| lines.iter().position(|l| l.new == Some(n))));
            Some((x, at))
        })
        .collect()
}

/// Whether a thread shows its notes: open ones and plain comments do,
/// resolved ones are folded to a line; `space` turns either around.
pub(crate) fn shown_open(x: &Discussion, flipped: &BTreeSet<String>) -> bool {
    let default = !x.resolved;
    default != flipped.contains(&x.id)
}

// --- keys ------------------------------------------------------------------------

/// Open file `i` at its first line.
fn open_file(p: &mut GitlabPlugin, i: usize) -> Vec<Effect> {
    if let Some(page) = p.pages.last() {
        page.file.set(i);
        page.placed.set(true);
        page.diff.reset();
        if let Kind::Mr { project, iid } = &page.kind
            && let Some(d) = detail(p, project, *iid)
        {
            select(page, &entries(&d.files), &format!("f:{i}"));
        }
    }
    want(p)
}

/// The tree's cursor on row `id`, the folders over it open.
fn select(page: &Page, tops: &[Entry], id: &str) {
    let Some(path) = path_to(tops, id) else { return };
    let mut state = page.tree.borrow_mut();
    for k in 1..path.len() {
        state.open(path[..k].to_vec());
    }
    state.select(path);
}

/// The file `step` places on from `at` in the tree's order, folders skipped.
fn neighbour(p: &GitlabPlugin, project: &str, iid: u64, at: usize, step: isize) -> Option<usize> {
    let files = &detail(p, project, iid)?.files;
    let order = order(&entries(files));
    let pos = order.iter().position(|i| *i == at)?;
    order.get(pos.checked_add_signed(step)?).copied()
}

/// Keys in the files pane: it moves and folds, `⏎` opens a file or folds
/// a folder; `↑` at its top is not its (the host takes it up).
fn tree_key(p: &mut GitlabPlugin, key: &KeyEvent) -> Option<Outcome> {
    if key.code == KeyCode::Enter {
        return tree_enter(p);
    }
    let page = p.pages.last()?;
    if tree::key(&mut page.tree.borrow_mut(), key) {
        return Some(Outcome::done());
    }
    None
}

/// ⏎ on the selected row of the files tree: open the file, or fold the folder.
fn tree_enter(p: &mut GitlabPlugin) -> Option<Outcome> {
    let page = p.pages.last()?;
    if !page.detail_painted.get() {
        return Some(Outcome::done());
    }
    let id = page.tree.borrow().selected().last().cloned()?;
    if let Some(i) = id.strip_prefix("f:").and_then(|n| n.parse().ok()) {
        return Some(Outcome::Handled(open_file(p, i)));
    }
    page.tree.borrow_mut().toggle_selected();
    Some(Outcome::done())
}

/// Activate the selected row of the Changes tab in `pane`: a file or folder
/// of the tree, or a line of the diff (comment on it, or reply in its
/// thread). `None` when the tab has no files and the page follows the row.
pub(super) fn activate(p: &mut GitlabPlugin, project: &str, iid: u64, pane: Pane) -> Option<Outcome> {
    if files_len(p, project, iid) == 0 {
        return None;
    }
    if pane == Pane::Nav && p.pages.last()?.tree_shown.get() {
        return tree_enter(p);
    }
    super::review::key(p, 'c')
}

fn files_len(p: &GitlabPlugin, project: &str, iid: u64) -> usize {
    detail(p, project, iid).map_or(0, |d| d.files.len())
}

/// A key on the Changes tab, in `pane` (the files are `Nav`, the diff
/// `Middle`); `None` when it is not one of its own.
pub(super) fn key(p: &mut GitlabPlugin, project: &str, iid: u64, pane: Pane, key: &KeyEvent) -> Option<Outcome> {
    let n = files_len(p, project, iid);
    if n == 0 {
        return None;
    }
    // Collapse the files (the diff takes the width) or bring them back;
    // kept in the config. From either pane.
    if key.code == KeyCode::Char('f') {
        p.cfg.hide_files = !p.cfg.hide_files;
        let mut fx = vec![Effect::Notify(
            if p.cfg.hide_files { "files collapsed: f shows them again" } else { "files shown beside the diff" }.into(),
        )];
        if p.cfg.hide_files {
            fx.push(Effect::Focus(Pane::Middle));
        }
        return Some(Outcome::Handled(fx));
    }
    let page = p.pages.last()?;
    let at = page.file.get().min(n - 1);
    if pane == Pane::Nav && page.tree_shown.get() {
        return tree_key(p, key);
    }
    let fx = match key.code {
        // ⏎ on a line comments on it, or replies in its thread (as `c`).
        KeyCode::Enter => return super::review::key(p, 'c'),
        // Side by side or one column; kept in the config.
        KeyCode::Char('v') => {
            p.cfg.side_by_side = !p.cfg.side_by_side;
            vec![Effect::Notify(
                if p.cfg.side_by_side {
                    "the diff side by side, where the window is wide enough"
                } else {
                    "the diff in one column"
                }
                .into(),
            )]
        }
        KeyCode::Char('n') => {
            let Some(to) = neighbour(p, project, iid, at, 1) else {
                return Some(Outcome::Handled(vec![Effect::Notify("that was the last file".into())]));
            };
            open_file(p, to)
        }
        KeyCode::Char('N') => {
            let Some(to) = neighbour(p, project, iid, at, -1) else {
                return Some(Outcome::Handled(vec![Effect::Notify("this is the first file".into())]));
            };
            open_file(p, to)
        }
        KeyCode::Char('}') => {
            let lines = lines(p, page, project, iid);
            if page.diff.next_hunk(&lines) {
                vec![]
            } else if let Some(to) = neighbour(p, project, iid, at, 1) {
                // Past the last hunk: the next file's first.
                open_file(p, to)
            } else {
                vec![Effect::Notify("that was the last hunk of the last file".into())]
            }
        }
        KeyCode::Char(' ') => {
            let here = at_cursor(p, page, project, iid);
            if here.is_empty() {
                vec![Effect::Notify("no thread on this line".into())]
            } else {
                let mut flipped = page.flipped.borrow_mut();
                for x in here {
                    if !flipped.remove(&x.id) {
                        flipped.insert(x.id.clone());
                    }
                }
                vec![]
            }
        }
        KeyCode::Char('{') => {
            let lines = lines(p, page, project, iid);
            if page.diff.previous_hunk(&lines) {
                vec![]
            } else {
                vec![Effect::Notify("this is the first hunk of this file".into())]
            }
        }
        _ => {
            let lines = lines(p, page, project, iid);
            let moved = if page.split_shown.get() {
                page.diff.key_split(key, &lines)
            } else {
                page.diff.key(key, lines.len())
            };
            if !moved {
                return None;
            }
            vec![]
        }
    };
    Some(Outcome::Handled(fx))
}

/// The threads under the diff's cursor line.
pub(crate) fn at_cursor(p: &GitlabPlugin, page: &Page, project: &str, iid: u64) -> Vec<Discussion> {
    let (Some(d), Some(f)) = (detail(p, project, iid), file(p, page, project, iid)) else {
        return vec![];
    };
    let Some(fd) = file_diff(p, project, iid, &f.path) else { return vec![] };
    let lines = diff::parse(&fd.diff);
    let cur = page.diff.cursor().min(lines.len().saturating_sub(1));
    threads_on(d, fd, &lines)
        .into_iter()
        .filter(|(_, at)| *at == Some(cur))
        .map(|(x, _)| x.clone())
        .collect()
}

/// A click on a file or a line of the diff.
pub(super) fn click(p: &mut GitlabPlugin, id: &str) -> Option<Vec<Effect>> {
    let numeric = id.starts_with("node:") || id.starts_with("line:") || id.starts_with("comment:");
    if numeric {
        let page = p.pages.last()?;
        let Some((project, iid)) = on_changes(page) else { return Some(vec![]); };
        if !page.detail_painted.get() || detail(p, &project, iid).is_none() {
            return Some(vec![]);
        }
        if !id.starts_with("node:") && !page.diff_painted.get() { return Some(vec![]); }
    }
    if let Some(node) = id.strip_prefix("node:") {
        if let Some(i) = node.strip_prefix("f:").and_then(|n| n.parse().ok()) {
            return Some(open_file(p, i));
        }
        // A folder: the cursor goes to it (the click focused the files).
        let page = p.pages.last()?;
        let Kind::Mr { project, iid } = &page.kind else { return Some(vec![]) };
        let d = detail(p, project, *iid)?;
        select(page, &entries(&d.files), node);
        return Some(vec![]);
    }
    if let Some(i) = id.strip_prefix("comment:").and_then(|n| n.parse::<usize>().ok()) {
        p.pages.last()?.diff.set(i);
        return Some(match super::review::key(p, 'c') {
            Some(radar_core::plugin::Outcome::Handled(fx)) => fx,
            _ => vec![],
        });
    }
    let i: usize = id.strip_prefix("line:")?.parse().ok()?;
    p.pages.last()?.diff.set(i);
    Some(vec![])
}

/// The Changes tab's rows of the actions menu, for the focused pane: the
/// files' keys in the files pane, the diff's on the diff.
pub(super) fn bindings(p: &GitlabPlugin, page: &Page, project: &str, iid: u64, pane: Pane) -> Vec<crate::Row> {
    use crate::{act, head};
    let Some(d) = detail(p, project, iid) else { return vec![] };
    let n = d.files.len();
    if n == 0 {
        return vec![];
    }
    let files = act(
        "gitlab.mr.toggle_files",
        Binding::new("f", if p.cfg.hide_files { "show the files beside the diff" } else { "collapse the files" }),
    );
    if pane == Pane::Nav && page.tree_shown.get() {
        return vec![
            head(Binding::heading("files")),
            files,
            act("gitlab.mr.fold_folder", Binding::new("space", "fold or unfold the folder")),
            act("gitlab.mr.open_file", Binding::new("⏎", "open the file, or fold the folder")),
        ];
    }
    let at = page.file.get().min(n - 1);
    let name = |i: usize| split(&d.files[i].path, 0).1;
    let next = neighbour(p, project, iid, at, 1);
    let mut b = vec![];
    b.push(head(Binding::heading("diff")));
    b.push(files);
    b.push(act(
        "gitlab.mr.toggle_side_by_side",
        Binding::new(
            "v",
            if p.cfg.side_by_side { "view the diff in one column" } else { "view the diff side by side" },
        ),
    ));
    if let Some(to) = next {
        b.push(act("gitlab.mr.next_file", Binding::new("n", format!("next file: {}", name(to)))));
    }
    if let Some(to) = neighbour(p, project, iid, at, -1) {
        b.push(act("gitlab.mr.previous_file", Binding::new("N", format!("open previous file: {}", name(to)))));
    }
    let lines = lines(p, page, project, iid);
    let cur = page.diff.cursor();
    if diff::hunks(&lines).iter().any(|&h| h > cur) || next.is_some() {
        b.push(act("gitlab.mr.next_hunk", Binding::new("}", "next hunk")));
    }
    if diff::hunks(&lines).iter().any(|&h| h < cur) {
        b.push(act("gitlab.mr.previous_hunk", Binding::new("{", "previous hunk")));
    }
    let here = at_cursor(p, page, project, iid);
    if let Some(x) = here.first() {
        let open = shown_open(x, &page.flipped.borrow());
        b.push(act(
            "gitlab.mr.fold_thread",
            Binding::new(
                "space",
                if open { "fold the thread on this line" } else { "unfold the thread on this line" },
            ),
        ));
    }
    b
}

/// The page is about a merge request whose Changes tab shows: its project and iid.
pub(crate) fn on_changes(page: &Page) -> Option<(String, u64)> {
    match &page.kind {
        Kind::Mr { project, iid } if page.tab == super::mr::CHANGES && page.contributed().is_none() => {
            Some((project.clone(), *iid))
        }
        _ => None,
    }
}
