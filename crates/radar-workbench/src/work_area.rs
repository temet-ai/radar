//! The M11 work area, as view data: map, timeline, work-area tabs, agent area
//! and action rail. Each is a `radar_core::view` extension node whose capability
//! is `work.map`, `work.timeline`, `work.tabs`, `work.agent` or `work.rail`,
//! carried by an extra pane after the screen's own Nav/Middle/Detail panes.
//!
//! Every extension holds `{ pending, status, target, ... }` data (the fields
//! the Ink work-area layout reads) and a `Document` fallback with the same
//! content, so a renderer that does not know the capability still shows it.
//! A section the state does not have is absent, never an empty pane: the
//! agent area exists only while a question (a prepared change waiting for a
//! confirmation) is pending, and the rail only while actions are offered.

use super::*;
use radar_core::journal::work_map::HandoffRecord;
use radar_core::view::{ActionSpec, Ink, Node, PaneId, PaneView, Seg, Text};
use serde_json::json;

/// Most entries a section lists; the rest is counted, never dropped silently.
const MOST: usize = 24;

fn extension(
    kind: &str,
    title: &str,
    pending: bool,
    status: Option<&str>,
    mut data: serde_json::Value,
    lines: Vec<Text>,
) -> PaneView {
    data["pending"] = json!(pending);
    if let Some(status) = status {
        data["status"] = json!(status);
    }
    data["target"] = json!(format!("work-area:{kind}"));
    PaneView::new(
        PaneId::Detail,
        title,
        Node::Extension {
            capability: format!("work.{kind}"),
            version: 1,
            data,
            fallback: Box::new(Node::Document { lines, cut: false }),
        },
    )
}

fn tail(text: &str) -> &str {
    &text[text.len().saturating_sub(8)..]
}

pub(super) fn sections(
    p: &WorkbenchPlugin,
    _cx: &ViewCx<'_>,
    actions: &[ActionSpec],
) -> Vec<PaneView> {
    let mut out = vec![map(p), timeline(p), tabs(p)];
    out.extend(agent(p));
    out.extend(rail(actions));
    out
}

fn map(p: &WorkbenchPlugin) -> PaneView {
    let items = p.visible_items();
    let groups: Vec<_> = p
        .groups
        .iter()
        .map(|g| {
            json!({
                "target": screen::group_target(p, Some(&g.id)),
                "label": g.text,
                "selected": p.filter.as_ref() == Some(&g.id),
                "ambiguous": g.ambiguous,
            })
        })
        .collect();
    let rows: Vec<_> = items
        .iter()
        .take(MOST)
        .map(|item| {
            json!({
                "target": screen::item_target(p, &item.item_id),
                "title": p.item_title(&item.item_id),
                "kind": item.kind.as_str(),
                "selected": p.selected.as_ref() == Some(&item.item_id),
            })
        })
        .collect();
    let mut lines = vec![Text::of(
        format!("{} groups · {} items on page {}", p.groups.len(), items.len(), p.page + 1),
        Ink::Muted,
    )];
    for item in items.iter().take(MOST) {
        // The selected item is shown by a style on its line (GUIDE 2.12), not a glyph.
        let line = format!("{} · {}", p.item_title(&item.item_id), item.kind.as_str());
        lines.push(if p.selected.as_ref() == Some(&item.item_id) {
            Text::new().push(Seg::new(line, Ink::Accent).strong())
        } else {
            Text::plain(line)
        });
    }
    if items.len() > MOST {
        lines.push(Text::of(format!("… {} more on this page", items.len() - MOST), Ink::Muted));
    }
    let status = if p.pending.is_some() {
        Some("loading")
    } else if p.error.is_some() {
        Some("failed")
    } else {
        None
    };
    extension(
        "map",
        "map",
        false,
        status,
        json!({ "groups": groups, "items": rows, "omitted": items.len().saturating_sub(MOST) }),
        lines,
    )
}

fn timeline(p: &WorkbenchPlugin) -> PaneView {
    let pages = p.pages();
    let source = p
        .evidence
        .as_ref()
        .or_else(|| p.detail.as_ref().map(|d| &d.primary));
    let mut entries = Vec::new();
    let mut lines = Vec::new();
    let mut total = 0;
    if let Some(page) = source {
        for record in &page.records {
            total += 1;
            if entries.len() >= MOST {
                continue;
            }
            let (label, sequence) = match record {
                HandoffRecord::Map { change } => (
                    format!("map · {}", model::basis(change.request.provenance.basis)),
                    change.sequence,
                ),
                HandoffRecord::Action { event, .. } => {
                    (format!("action · {}", event.kind.as_str()), event.sequence)
                }
                _ => ("record".to_string(), 0),
            };
            lines.push(Text::plain(format!("#{sequence} {label}")));
            entries.push(json!({ "sequence": sequence, "label": label }));
        }
        if page.truncated {
            lines.push(Text::of("More evidence remains (v).", Ink::Warn));
        }
        if page.known_recording_gaps {
            lines.push(Text::of("Recording gaps are known.", Ink::Warn));
        }
    }
    if lines.is_empty() {
        lines.push(Text::of(
            if pages.is_empty() { "No evidence loaded." } else { "No events on this page." },
            Ink::Muted,
        ));
    }
    if total > MOST {
        lines.push(Text::of(format!("… {} more", total - MOST), Ink::Muted));
    }
    extension(
        "timeline",
        "timeline",
        false,
        source.map(|s| if s.truncated { "partial" } else { "complete" }),
        json!({ "entries": entries, "omitted": total.saturating_sub(MOST) }),
        lines,
    )
}

fn tabs(p: &WorkbenchPlugin) -> PaneView {
    let evidence = p.evidence.is_some();
    let mut tabs = vec![
        json!({ "id": "sources", "label": "linked sources", "active": !evidence,
                "count": p.sources().len() }),
        json!({ "id": "evidence", "label": "evidence", "active": evidence }),
    ];
    if let Some(id) = &p.selected {
        tabs.insert(0, json!({ "id": "item", "label": tail(id.as_str()), "active": false }));
    }
    let lines = tabs
        .iter()
        .map(|t| {
            let mark = if t["active"] == json!(true) { "● " } else { "○ " };
            Text::plain(format!("{mark}{}", t["label"].as_str().unwrap_or("")))
        })
        .collect();
    extension("tabs", "tabs", false, None, json!({ "tabs": tabs }), lines)
}

/// The agent area: here, the question waiting on the user. There is no agent
/// session in this part, so the section exists only while a confirmation or
/// a recovery notice is pending, and then it is marked `pending`.
fn agent(p: &WorkbenchPlugin) -> Option<PaneView> {
    let prepared = p.prepared.as_ref()?;
    let mut lines = vec![Text::of(
        if prepared.blocked {
            "Waiting: this saved change needs review before it can be applied."
        } else {
            "Waiting for your confirmation: apply the prepared local change?"
        },
        Ink::Warn,
    )];
    lines.extend(prepared.summary.iter().take(MOST).map(Text::plain));
    if let Some(note) = &p.notice {
        lines.push(Text::of(note, Ink::Info));
    }
    Some(extension(
        "agent",
        "question",
        true,
        Some(if prepared.blocked { "blocked" } else { "awaiting confirmation" }),
        json!({ "question": lines.first().map(Text::words), "summary": prepared.summary }),
        lines,
    ))
}

fn rail(actions: &[ActionSpec]) -> Option<PaneView> {
    let offered: Vec<_> = actions.iter().filter(|a| !a.id.is_empty()).collect();
    if offered.is_empty() {
        return None;
    }
    let rows: Vec<_> = offered
        .iter()
        .map(|a| json!({ "id": a.id, "label": a.label, "key": a.key }))
        .collect();
    let lines = offered
        .iter()
        .map(|a| Text::plain(a.label.clone()))
        .collect();
    Some(extension("rail", "actions", false, None, json!({ "actions": rows }), lines))
}
