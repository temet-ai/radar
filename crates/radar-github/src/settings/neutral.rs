//! GitHub settings as renderer-neutral rows and intents.

use radar_core::plugin::Outcome;
use radar_core::ui::connection;
use radar_core::view::{self as v, Ink, ListRow, Node, SectionView, Text};

use super::{GithubPlugin, Row};

pub fn view(p: &GithubPlugin, section: &str, _cx: &v::ViewCx<'_>) -> Option<SectionView> {
    let (rows, selected) = match section {
        "connection" => {
            let token = if p.cfg.token.is_empty() {
                "public access, no token"
            } else {
                "configured token"
            };
            let values = [
                p.cfg.api.as_str(),
                token,
                "GitHub connection",
                "once on start, then on request",
            ];
            let rows = super::CONNECTION
                .iter()
                .zip(values)
                .enumerate()
                .map(|(i, (label, value))| {
                    ListRow::new(
                        format!("setting:{i}"),
                        connection::field_text(
                            Text::plain(" "),
                            label,
                            Text::of(value, Ink::Primary),
                        ),
                    )
                })
                .collect();
            (rows, p.connection_row)
        }
        "repositories" => {
            let rows = super::order(p)
                .into_iter()
                .enumerate()
                .map(|(i, row)| {
                    let label = match row {
                        Row::Add => "create a group or add a repository".to_owned(),
                        Row::Group(g) => format!("◆  {}", p.cfg.groups[g].name),
                        Row::Repository(g, r) => {
                            let group = &p.cfg.groups[g];
                            let full = &group.repositories[r];
                            let name = group
                                .owner
                                .as_ref()
                                .and_then(|owner| full.strip_prefix(&format!("{owner}/")))
                                .unwrap_or(full);
                            let branch = if r + 1 == group.repositories.len() {
                                "└─ "
                            } else {
                                "├─ "
                            };
                            format!("  {branch}{name}")
                        }
                    };
                    ListRow::new(format!("setting:{i}"), Text::plain(label))
                })
                .collect();
            (rows, p.repository_row)
        }
        _ => return None,
    };
    let editing = p.editor.as_ref().map(|e| v::Editing {
        text: e.input.value(),
        caret: e.input.cursor(),
        row: selected,
        at: if section == "connection" {
            1 + connection::value_x(1)
        } else {
            1
        },
    });
    let typing = p
        .editor
        .as_ref()
        .map(|e| super::validate(&e.field, &e.input.value()));
    let detail_lines = if section == "connection" {
        let mut d = connection::DetailText::new(match p.connection_row {
            0 => super::API_ABOUT,
            1 => {
                "Paste a GitHub token, use env:NAME, or leave empty for public access. Private repositories require read permission."
            }
            2 => "Check the signed-in account or public API access.",
            _ => "Repositories load on start and when explicitly refreshed.",
        });
        match p.connection_row {
            0 => {
                d.accepts = Some(super::API_ACCEPTS.into());
                d.now = Some(p.cfg.api.clone());
                d.config = Some("[github] api".into());
            }
            1 => d.config = Some("[github] token".into()),
            _ => {}
        }
        d.typing = typing;
        d.text()
    } else {
        let mut lines = vec![Text::of(
            "Create groups and choose repositories. Press a to browse, e to edit, d to remove, or m to move a repository.",
            Ink::Secondary,
        )];
        if let Some(result) = typing {
            lines.push(Text::of("as you type", Ink::Muted));
            lines.push(Text::of(result.unwrap_or_else(|reason| reason), Ink::Info));
        }
        lines
    };
    Some(SectionView {
        version: v::VERSION,
        scope: section.into(),
        body: Node::List {
            rows,
            selected: Some(selected),
            editing,
        },
        detail_title: super::title(p, section),
        detail: detail_lines,
        detail_blocks: vec![],
        actions: super::keys(p, section)
            .iter()
            .map(crate::action_spec)
            .collect(),
    })
}

pub fn intent(p: &mut GithubPlugin, section: &str, intent: &v::Intent) -> Outcome {
    if !matches!(section, "connection" | "repositories") {
        return Outcome::Ignored;
    }
    match intent {
        v::Intent::Select { target }
        | v::Intent::Context { target }
        | v::Intent::Activate { target } => {
            let out = super::click(p, section, target.as_str());
            if matches!(intent, v::Intent::Activate { .. }) && matches!(out, Outcome::Handled(_)) {
                return super::key(p, section, v::Chord::new(v::Key::Enter));
            }
            out
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => {
            let rows = super::keys(p, section);
            crate::chord_for(&rows, intent)
                .map_or(Outcome::Ignored, |chord| super::key(p, section, chord))
        }
        v::Intent::Fold { .. }
        | v::Intent::Show { .. }
        | v::Intent::End { .. }
        | v::Intent::EditSelect { .. }
        | v::Intent::EditUnselect
        | v::Intent::Dock(_) => Outcome::Ignored,
    }
}
