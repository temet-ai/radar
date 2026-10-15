//! Configured local workspaces and the repositories found inside them.

use radar_core::design::theme::Slot;
use radar_core::plugin::{Binding, Outcome, Pane, ScreenSpec};
use radar_core::view_tui::Cx;
use radar_core::view::{self as v, Ink, ListRow, Node, PaneId, PaneView, Text};
use radar_core::ratatui::buffer::Buffer;
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent};
use radar_core::ratatui::layout::{Constraint, Layout, Rect};
use radar_core::ratatui::widgets::{Block, Borders, Widget};
use radar_core::ui;

use crate::{Target, WorkspacePlugin};

pub const SCREEN: ScreenSpec = ScreenSpec {
    id: "local-repositories",
    name: "local workspaces",
    key: 'W',
};

#[derive(Clone)]
struct Row {
    space: usize,
    repo: Option<usize>,
    label: String,
}

fn rows(p: &WorkspacePlugin) -> Vec<Row> {
    let mut rows = Vec::new();
    for (space, source) in p.cfg.spaces.iter().enumerate() {
        rows.push(Row {
            space,
            repo: None,
            label: source.label(),
        });
        if let Some(index) = p.state.indexes.get(&source.dir) {
            for (repo, found) in index.repositories.iter().enumerate() {
                let name = found.dir.rsplit('/').next().unwrap_or(&found.dir);
                rows.push(Row {
                    space,
                    repo: Some(repo),
                    label: format!("  {name}"),
                });
            }
        }
    }
    rows
}

pub fn view(p: &WorkspacePlugin, cx: &v::ViewCx<'_>) -> v::ScreenView {
    let list = rows(p);
    let selected = p.state.screen_row.min(list.len().saturating_sub(1));
    let items = list.iter().enumerate().map(|(i, row)| {
        ListRow::new(format!("row:{i}"), Text::of(&row.label, if row.repo.is_none() { Ink::Emphasis } else { Ink::Secondary }))
    }).collect();
    let detail = list.get(selected).map_or_else(
        || vec![Text::of("Add a local workspace in Settings.", Ink::Secondary)],
        |row| {
            let space = &p.cfg.spaces[row.space];
            if let Some(repo) = row.repo.and_then(|i| p.state.indexes.get(&space.dir)?.repositories.get(i)) {
                vec![Text::plain(&repo.project), Text::plain(&repo.dir), Text::new(), Text::of("Enter opens this folder in your editor", Ink::Muted)]
            } else {
                vec![Text::plain(&space.dir), Text::new(), Text::of("Repositories are read from this workspace's clones folder.", Ink::Muted), Text::of("Enter opens the workspace in your editor", Ink::Muted)]
            }
        });
    let mut screen = v::ScreenView::new(SCREEN.id, vec![
        PaneView::new(PaneId::Middle, "local repositories", Node::List { rows: items, selected: Some(selected), editing: None }),
        PaneView::new(PaneId::Detail, "details", Node::Document { lines: detail, cut: false }),
    ]);
    screen.actions = keys(p, Pane::from(cx.focus)).iter().map(|b| v::ActionSpec::named(action_id(b), b)).collect();
    screen
}

/// The stable id of a row of the local repositories screen, chosen here by hand from
/// its key (`workspace.local_repositories.<verb>`); a label edit never changes it.
/// A heading has no id.
fn action_id(b: &Binding) -> &'static str {
    if b.is_heading() {
        return "";
    }
    match b.keys.as_str() {
        "⏎" => "workspace.local_repositories.open",
        "r" => "workspace.local_repositories.look_again",
        other => unreachable!("local repositories has no stable id for key {other:?}"),
    }
}

pub fn intent(p: &mut WorkspacePlugin, pane: Pane, intent: &v::Intent) -> Outcome {
    match intent {
        v::Intent::Select { target } | v::Intent::Context { target } | v::Intent::Activate { target } => {
            let out = click(p, target.as_str());
            if matches!(intent, v::Intent::Activate { .. }) && matches!(out, Outcome::Handled(_)) {
                return key(p, Pane::Middle, radar_core::view_tui::key_of(v::Chord::new(v::Key::Enter)));
            }
            out
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => intent.chord().map_or(Outcome::Ignored, |chord| key(p, pane, radar_core::view_tui::key_of(chord))),
        v::Intent::Fold { .. } | v::Intent::Show { .. } | v::Intent::End { .. } | v::Intent::EditSelect { .. } | v::Intent::EditUnselect | v::Intent::Dock(_) => Outcome::Ignored,
    }
}

#[allow(dead_code)]
pub fn render(p: &WorkspacePlugin, area: Rect, buf: &mut Buffer, cx: &Cx<'_>) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).areas(area);
    Block::default()
        .borders(Borders::ALL)
        .title(" local repositories ")
        .render(left, buf);
    Block::default()
        .borders(Borders::ALL)
        .title(" details ")
        .render(right, buf);
    cx.hit_pane(left, Pane::Middle);
    cx.hit_pane(right, Pane::Detail);
    let list = rows(p);
    let selected = p.state.screen_row.min(list.len().saturating_sub(1));
    let inner = left.inner(radar_core::ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });
    let offset = selected.saturating_sub(inner.height.saturating_sub(1) as usize);
    for (visible, (index, row)) in list
        .iter()
        .enumerate()
        .skip(offset)
        .take(inner.height as usize)
        .enumerate()
    {
        let y = inner.y + visible as u16;
        let style = if index == selected {
            cx.theme
                .bold(Slot::TextEmphasis)
                .patch(cx.theme.bg(Slot::BgSelection))
        } else if row.repo.is_none() {
            cx.theme.bold(Slot::TextSecondary)
        } else {
            cx.theme.s(Slot::TextSecondary)
        };
        buf.set_style(Rect::new(inner.x, y, inner.width, 1), style);
        buf.set_string(inner.x, y, ui::fit(&row.label, inner.width as usize), style);
        cx.hit_row(inner, y, Pane::Middle, format!("row:{index}"));
    }
    let detail = right.inner(radar_core::ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });
    if let Some(row) = list.get(selected) {
        let space = &p.cfg.spaces[row.space];
        let index = p.state.indexes.get(&space.dir);
        let lines = if let Some(repo) = row.repo.and_then(|i| index?.repositories.get(i)) {
            vec![
                repo.project.as_str(),
                repo.dir.as_str(),
                "",
                "Enter opens this folder in your editor",
            ]
        } else {
            vec![
                space.dir.as_str(),
                "",
                "Repositories are read from this workspace's clones folder.",
                "Enter opens the workspace in your editor",
            ]
        };
        for (n, line) in lines.into_iter().enumerate().take(detail.height as usize) {
            buf.set_string(
                detail.x,
                detail.y + n as u16,
                ui::fit(line, detail.width as usize),
                cx.theme.s(Slot::TextSecondary),
            );
        }
    } else if detail.height > 0 {
        buf.set_string(
            detail.x,
            detail.y,
            "Add a local workspace in Settings.",
            cx.theme.s(Slot::TextSecondary),
        );
    }
}

pub fn key(p: &mut WorkspacePlugin, pane: Pane, key: KeyEvent) -> Outcome {
    if pane != Pane::Middle {
        return Outcome::Ignored;
    }
    let list = rows(p);
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            p.state.screen_row = p.state.screen_row.saturating_sub(1);
            Outcome::done()
        }
        KeyCode::Down | KeyCode::Char('j') => {
            p.state.screen_row = (p.state.screen_row + 1).min(list.len().saturating_sub(1));
            Outcome::done()
        }
        KeyCode::Enter => {
            let Some(row) = list.get(p.state.screen_row) else {
                return Outcome::done();
            };
            let space = &p.cfg.spaces[row.space];
            let dir = row
                .repo
                .and_then(|i| p.state.indexes.get(&space.dir)?.repositories.get(i))
                .map_or_else(|| space.dir.clone(), |repo| repo.dir.clone());
            Outcome::Handled(p.open_editor(space, Target::Folder(dir)))
        }
        KeyCode::Char('r') => {
            let mut effects = Vec::new();
            for dir in p
                .cfg
                .spaces
                .iter()
                .map(|s| s.dir.clone())
                .collect::<Vec<_>>()
            {
                effects.extend(p.reindex(&dir, true));
            }
            Outcome::Handled(effects)
        }
        _ => Outcome::Ignored,
    }
}

pub fn click(p: &mut WorkspacePlugin, id: &str) -> Outcome {
    if let Some(row) = id
        .strip_prefix("row:")
        .and_then(|n| n.parse::<usize>().ok())
    {
        if row < rows(p).len() {
            p.state.screen_row = row;
        }
        Outcome::done()
    } else {
        Outcome::Ignored
    }
}

pub fn keys(_: &WorkspacePlugin, pane: Pane) -> Vec<Binding> {
    if pane == Pane::Middle {
        vec![
            Binding::heading("local repositories"),
            Binding::new("⏎", "open the selected folder"),
            Binding::new("r", "look for repositories again"),
        ]
    } else {
        vec![]
    }
}
