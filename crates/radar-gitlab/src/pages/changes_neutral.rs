//! The Changes tab as a renderer-neutral file tree and diff.

use super::*;
use crate::model::{Discussion, FileDiff};
use radar_core::view::{
    self as v, Axis, ColumnSpec, DiffUnder, Ink, Node as ViewNode, PageLayout, PaneId, PaneView,
    Seg, Size, Text, TreeRow,
};

pub(super) fn view(
    p: &GitlabPlugin,
    page: &Page,
    project_path: &str,
    iid: u64,
    cx: &v::ViewCx<'_>,
) -> Option<v::ScreenView> {
    // A previous wide render must not leave a jump to a file pane when the
    // current projection is loading, empty, or no longer has a page frame.
    page.tree_shown.set(false);
    let mut view = super::neutral::mr_commits_view(p, page, project_path, iid, cx)?;
    let frame = view.page.as_mut()?;
    frame.tab = mr::CHANGES;
    let fetched = p.mr_pages.get(&mr_key(project_path, iid));
    let detail = changes::detail(p, project_path, iid);
    page.detail_painted.set(detail.is_some());
    page.diff_painted.set(false);
    let Some(detail) = detail else {
        let line = fetched
            .and_then(|f| v::kit::fetching(f.loading, f.value.is_some(), f.error.as_deref()))
            .unwrap_or_else(v::kit::loading);
        frame.panes = vec![PaneView::new(
            PaneId::Middle,
            "changes",
            ViewNode::Document {
                lines: vec![line],
                cut: false,
            },
        )];
        return Some(view);
    };
    if let Some(error) = fetched.and_then(|f| f.error.as_deref()) {
        frame.notice = Some(Text::from(vec![
            Seg::icon_or(Icon::Error, "✗", Ink::Bad),
            Seg::new(" merge request details refresh failed: ", Ink::Bad),
            Seg::new(error, Ink::Bad).strong(),
        ]));
    }
    if detail.files.is_empty() {
        let mut pane = PaneView::new(
            PaneId::Middle,
            "changes",
            ViewNode::State {
                state: v::State::Empty,
                text: "This merge request changes no files.".into(),
            },
        );
        pane.inset = true;
        frame.panes = vec![pane];
        return Some(view);
    }

    let wide = cx.width >= 100 && !p.cfg.hide_files;
    page.tree_shown.set(wide);
    let tree_w = (cx.width / 3).clamp(30, 50);
    let diff_w = if wide {
        cx.width.saturating_sub(tree_w)
    } else {
        cx.width
    };
    let nav = files_pane(p, page, detail, tree_w, wide, cx);
    let diff = diff_pane(p, page, project_path, iid, detail, diff_w, wide, cx);
    frame.panes = vec![nav, diff];
    frame.layout = Some(if wide {
        PageLayout::split(
            Axis::Horizontal,
            [
                (Size::Length(tree_w), PageLayout::pane(0)),
                (Size::Fill(1), PageLayout::pane(1)),
            ],
        )
    } else {
        PageLayout::pane(1)
    });
    Some(view)
}

fn files_pane(
    p: &GitlabPlugin,
    page: &Page,
    detail: &MrDetail,
    width: u16,
    wide: bool,
    cx: &v::ViewCx<'_>,
) -> PaneView {
    let seen = page.seen.borrow();
    let count = detail
        .files
        .iter()
        .filter(|f| seen.contains(&f.path))
        .count();
    let title = format!("files · {count} of {} seen", detail.files.len());
    let tops = changes::entries(&detail.files);
    let at = page.file.get().min(detail.files.len().saturating_sub(1));
    {
        let mut state = page.tree.borrow_mut();
        radar_core::ui::tree::open_new(
            &mut state,
            &changes::skeleton(&tops),
            &mut page.tree_seen.borrow_mut(),
        );
        if wide
            && cx.focus != PaneId::Nav
            && let Some(path) = changes::path_to(&tops, &format!("f:{at}"))
        {
            let opened = state.opened();
            let depth = (1..path.len())
                .find(|k| !opened.contains(&path[..*k]))
                .unwrap_or(path.len());
            if state.selected() != &path[..depth] {
                state.select(path[..depth].to_vec());
            }
        }
    }
    let state = page.tree.borrow();
    let room = width.saturating_sub(14) as usize;
    let rows = tops
        .iter()
        .map(|entry| file_tree_row(detail, &seen, at, &state, entry, room, 0, &mut Vec::new()))
        .collect();
    let body = ViewNode::Tree(v::Tree {
        header: Text::new(),
        columns: vec![ColumnSpec::new("size", 7, 0)],
        ordered_columns: true,
        rows,
        selected: state.selected().to_vec(),
        open: {
            // A set: sorted, so two hosts send the same view.
            let mut open: Vec<Vec<String>> = state.opened().iter().cloned().collect();
            open.sort();
            open
        },
        name_width: room as u16,
        look: Some(v::TreeLook {
            lines: p.cfg.style_for("lines"),
            band: p.cfg.style_for("band"),
            heading: p.cfg.style_for("heading"),
            band_level: 1,
        }),
        header_target: None,
        note: None,
        cut_names: false,
        editing: None,
        pointer_action: true,
    });
    PaneView::new(PaneId::Nav, title, body)
}

#[allow(clippy::too_many_arguments)]
fn file_tree_row(
    detail: &MrDetail,
    seen: &std::collections::BTreeSet<String>,
    open_at: usize,
    state: &radar_core::ui::tree::TreeState<String>,
    entry: &changes::Entry,
    room: usize,
    depth: usize,
    path: &mut Vec<String>,
) -> TreeRow {
    path.push(entry.id());
    let (name, adds, dels, children) = match entry {
        changes::Entry::Dir {
            name,
            adds,
            dels,
            kids,
            ..
        } => {
            let open = state.opened().contains(path.as_slice());
            let row = Text::from(vec![
                Seg::icon(if open { Icon::FolderOpen } else { Icon::Folder }),
                Seg::new(
                    changes::tail_fit(name, room.saturating_sub(depth * 3 + 4)),
                    Ink::Secondary,
                )
                .strong(),
            ]);
            let children = kids
                .iter()
                .map(|child| {
                    file_tree_row(detail, seen, open_at, state, child, room, depth + 1, path)
                })
                .collect();
            (row, *adds, *dels, children)
        }
        changes::Entry::File {
            index,
            name,
            adds,
            dels,
        } => {
            let full = &detail.files[*index].path;
            let mark = if *index == open_at {
                "▎ "
            } else if seen.contains(full) {
                "✓ "
            } else {
                "· "
            };
            let open = detail
                .discussions
                .iter()
                .filter(|thread| {
                    thread.is_open() && thread.on_line().is_some_and(|(path, _, _)| path == full)
                })
                .count();
            let tail = if open > 0 {
                format!(" ◆{open}")
            } else {
                String::new()
            };
            let mut row = Text::from(vec![
                Seg::new(
                    mark,
                    if *index == open_at {
                        Ink::Accent
                    } else {
                        Ink::Muted
                    },
                ),
                Seg::icon(Icon::for_file(full)),
                if *index == open_at {
                    Seg::new(
                        radar_core::ui::fit(name, room.saturating_sub(depth * 3 + 2 + tail.len()))
                            .trim_end(),
                        Ink::Accent,
                    )
                    .strong()
                } else {
                    Seg::new(
                        radar_core::ui::fit(name, room.saturating_sub(depth * 3 + 2 + tail.len()))
                            .trim_end(),
                        Ink::Primary,
                    )
                },
            ]);
            if open > 0 {
                row.add(Seg::new(tail, Ink::Warn));
            }
            (row, *adds, *dels, Vec::new())
        }
    };
    let row = TreeRow::new(entry.id(), name)
        .cell("size", Text::of(format!("+{adds} −{dels}"), Ink::Muted))
        .children(children);
    path.pop();
    row
}

#[allow(clippy::too_many_arguments)]
fn diff_pane(
    p: &GitlabPlugin,
    page: &Page,
    project_path: &str,
    iid: u64,
    detail: &MrDetail,
    width: u16,
    wide: bool,
    cx: &v::ViewCx<'_>,
) -> PaneView {
    let at = page.file.get().min(detail.files.len() - 1);
    let stat = &detail.files[at];
    let held = changes::file_diff(p, project_path, iid, &stat.path);
    let mut title = if wide {
        String::new()
    } else {
        format!("{} of {} · ", at + 1, detail.files.len())
    };
    let room = (width as usize).saturating_sub(radar_core::ui::width(&title) + 18);
    let (dir, name) = changes::split(&stat.path, room);
    title.push_str(&format!("{dir}{name}"));
    if let Some(how) = held.and_then(FileDiff::how) {
        title.push_str(&format!(" · {how}"));
    }
    let mut pane = PaneView::new(
        PaneId::Middle,
        title,
        ViewNode::Document {
            lines: vec![],
            cut: false,
        },
    );
    pane.title_icon = Some(Icon::for_file(&stat.path));
    let ds = p.diffs.get(&mr_key(project_path, iid));
    let Some(fd) = held else {
        let pages = detail.files.len().div_ceil(crate::client::DIFF_PAGE).max(1);
        let line = match ds {
            Some(ds) if ds.error.is_some() => {
                v::kit::failed(ds.error.as_deref().unwrap_or_default())
            }
            Some(ds)
                if ds.loading.is_empty()
                    && (1..=ds.ended.unwrap_or(pages).min(pages))
                        .all(|pg| ds.asked.contains(&pg)) =>
            {
                Text::of("GitLab sent no diff for this file.", Ink::Muted)
            }
            _ => v::kit::loading(),
        };
        pane.body = ViewNode::Document {
            lines: vec![line],
            cut: false,
        };
        return pane;
    };
    if fd.too_large {
        pane.body = ViewNode::State { state: v::State::Empty, text: "GitLab does not send this file's diff: it is too large. Open the merge request in GitLab to see it.".into() };
        return pane;
    }
    let lines = radar_core::ui::diff::parse(&fd.diff);
    let split = p.cfg.side_by_side && width.saturating_sub(2) >= 90;
    page.split_shown.set(split);
    if let Some(id) = page.jump.borrow_mut().take()
        && let Some((_, Some(index))) = changes::threads_on(detail, fd, &lines)
            .iter()
            .find(|(thread, _)| thread.id == id)
    {
        page.diff.set(*index);
    }
    let mut under = Vec::new();
    let mut away = 0;
    for (thread, at) in changes::threads_on(detail, fd, &lines) {
        match at {
            Some(index) => under.push(DiffUnder {
                line: index,
                lines: thread_lines(
                    thread,
                    changes::shown_open(thread, &page.flipped.borrow()),
                    cx.now,
                    width.saturating_sub(12),
                ),
                input: None,
            }),
            None => away += 1,
        }
    }
    if away > 0 && !lines.is_empty() {
        under.push(DiffUnder {
            line: 0,
            lines: vec![Text::of(
                format!(
                    "◆ {away} thread{} on lines this diff no longer has: see the Threads tab",
                    if away == 1 { "" } else { "s" }
                ),
                Ink::Muted,
            )],
            input: None,
        });
    }
    if let Some(compose) = p.compose.as_ref().filter(|_| review::inline(p, page)) {
        let mut block = vec![Text::from(vec![
            Seg::new(compose.title(), Ink::Accent).strong(),
        ])];
        block.extend(
            compose
                .composer
                .lines
                .iter()
                .cloned()
                .map(|line| Text::of(line, Ink::Primary)),
        );
        block.push(Text::new());
        under.push(DiffUnder {
            line: page.diff.cursor(),
            lines: block,
            input: Some(v::InlineInput {
                text: compose.composer.input.value(),
                caret: compose.composer.input.cursor(),
                selection: compose.composer.input.selection(),
            }),
        });
    }
    page.diff.set_height(
        cx.height.saturating_sub(
            7 + u16::from(
                p.mr_pages
                    .get(&mr_key(project_path, iid))
                    .and_then(|f| f.error.as_ref())
                    .is_some(),
            ),
        ) as usize,
    );
    page.diff_painted.set(true);
    pane.body = ViewNode::Diff {
        lines,
        under,
        selected: page.diff.cursor(),
        top: page.diff.top(),
        split,
        language: radar_core::ui::highlight::language_of(&stat.path),
        empty: "No lines changed: a rename, a mode change or a binary file.".into(),
        painted: Vec::new(),
        words: Vec::new(),
    };
    pane
}

fn thread_lines(thread: &Discussion, open: bool, now: i64, width: u16) -> Vec<Text> {
    let count = thread.notes.len();
    let (icon, mark, label, ink) = match (thread.resolvable, thread.resolved) {
        (true, false) => (Icon::Thread, "◆", "open thread", Ink::Warn),
        (true, true) => (Icon::Resolved, "✓", "resolved", Ink::Good),
        (false, _) => (Icon::Comment, "○", "comment", Ink::Secondary),
    };
    let heading = format!(
        "{label} · {count} note{}",
        if count == 1 { "" } else { "s" }
    );
    if !open {
        let note = thread
            .notes
            .first()
            .map(|note| {
                format!(
                    " · {}: {}",
                    radar_core::ui::short_name(&note.author),
                    note.body.replace('\n', " ")
                )
            })
            .unwrap_or_default();
        return vec![Text::from(vec![
            Seg::icon_or_lead(icon, format!("{mark} "), ink),
            Seg::new(heading, ink),
            Seg::new(note, Ink::Muted),
        ])];
    }
    let mut out = vec![Text::from(vec![
        Seg::icon_or_lead(icon, format!("{mark} "), ink),
        Seg::new(heading, ink),
    ])];
    for (i, note) in thread.notes.iter().enumerate() {
        let mut who = Text::from(vec![Seg::new("│ ", Ink::AccentMuted)]);
        if i > 0 {
            who.add(Seg::new("↳ ", Ink::Muted));
        }
        who.add(Seg::icon(Icon::Person));
        who.add(Seg::new(note.author.clone(), Ink::Secondary).strong());
        who.add(Seg::new(
            format!(" · {} ago", radar_core::time::ago_str(&note.at, now)),
            Ink::Muted,
        ));
        out.push(who);
        for line in radar_core::ui::wrap_words(&note.body, width, 2) {
            out.push(Text::from(vec![
                Seg::new("│ ", Ink::AccentMuted),
                Seg::new(line, Ink::Primary),
            ]));
        }
    }
    out
}
