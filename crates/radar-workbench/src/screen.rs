use super::*;
use radar_core::journal::work_map::HandoffRecord;
use radar_core::view::{
    ActionSpec, EntryView, Ink, ListRow, Node, PaneId, PaneView, Seg, State, Text,
};

pub(super) fn action_id(key: &str, pane: Pane) -> &str {
    match key {
        "n" => "work.new",
        "r" => "work.refresh",
        "/" => "work.filter",
        "t" => "work.group",
        "v" => "work.evidence",
        "o" => "work.open-source",
        "c" => "work.copy-source",
        "y" => "work.copy-retained",
        "]" if pane == Pane::Nav => "work.next-groups",
        "[" if pane == Pane::Nav => "work.previous-groups",
        "]" => "work.next-page",
        "[" => "work.previous-page",
        _ => "",
    }
}

fn target(p: &WorkbenchPlugin, kind: &str, id: &str) -> String {
    // A target survives focus/selection, but never a different snapshot or filter.
    serde_json::to_string(&(kind, id, &p.snapshot, p.page, &p.filter, &p.text_filter)).unwrap()
}
pub(super) fn item_target(p: &WorkbenchPlugin, id: &ItemId) -> String {
    target(p, "item", id.as_str())
}
pub(super) fn group_target(p: &WorkbenchPlugin, id: Option<&TagId>) -> String {
    serde_json::to_string(&(
        "group",
        id.map_or("all", TagId::as_str),
        &p.snapshot,
        p.group_page,
        p.group_revision,
    ))
    .unwrap()
}
pub(super) fn source_target(p: &WorkbenchPlugin, source: &SourceBinding) -> String {
    target(p, "source", &serde_json::to_string(source).unwrap())
}
fn heading(text: impl Into<String>) -> Text {
    Text::new().push(Seg::new(text, Ink::Emphasis).strong())
}

pub(super) fn view(p: &WorkbenchPlugin, cx: &ViewCx<'_>) -> ScreenView {
    let mut groups = vec![ListRow::new(group_target(p, None), "All items")];
    groups.extend(p.groups.iter().map(|group| {
        let row = ListRow::new(
            group_target(p, Some(&group.id)),
            Text::of(
                &group.text,
                if group.ambiguous {
                    Ink::Warn
                } else {
                    Ink::Plain
                },
            ),
        );
        if group.ambiguous {
            row.right(Text::of("ambiguous", Ink::Warn))
        } else if p
            .groups
            .iter()
            .filter(|other| other.text == group.text)
            .count()
            > 1
        {
            row.right(Text::of(
                &group.id.as_str()[group.id.as_str().len() - 8..],
                Ink::Muted,
            ))
        } else {
            row
        }
    }));
    if let Some(id) = &p.filter
        && !p.groups.iter().any(|group| &group.id == id)
    {
        groups.push(
            ListRow::new(group_target(p, Some(id)), "Selected group")
                .right(Text::of(&id.as_str()[id.as_str().len() - 8..], Ink::Muted))
                .detail(" · not on this catalog page"),
        );
    }
    let selected_group = p.filter.as_ref().map_or(0, |id| {
        p.groups
            .iter()
            .position(|g| &g.id == id)
            .map_or(groups.len() - 1, |i| i + 1)
    });
    let mut nav = PaneView::new(
        PaneId::Nav,
        "local groups",
        Node::List {
            rows: groups,
            selected: Some(selected_group),
            editing: None,
        },
    );
    nav.width = Some(25);
    nav.footer = Some(Text::of(
        format!("Groups · page {}", p.group_page + 1),
        Ink::Muted,
    ));
    if matches!(p.pending, Some((_, Pending::Groups(_))))
        || (p.group_catalog.is_none() && matches!(p.pending, Some((_, Pending::Items))))
    {
        nav.head = Some(Text::of("Loading groups…", Ink::Info));
    } else if p.group_error.is_some() {
        nav.head = Some(Text::of("Groups unavailable", Ink::Bad));
    } else if p.group_catalog.is_some() && p.groups.is_empty() {
        nav.head = Some(Text::of("No groups on page", Ink::Muted));
    } else if p.group_catalog.is_none() {
        nav.head = Some(Text::of("Groups not loaded", Ink::Muted));
    }
    let items = p.visible_items();
    let list = if items.is_empty() {
        let (state, text) = if p.pending.is_some() {
            (State::Loading, "Loading local work…")
        } else if let Some(error) = p.error {
            (
                State::Failed,
                match error {
                    work_api::Error::Disabled => {
                        "Local activity storage is unavailable. Check state-directory access, restart, then refresh."
                    }
                    _ => {
                        "Local work could not be loaded. Press r to retry; no changes were inferred."
                    }
                },
            )
        } else if p.items.is_none() {
            (State::Empty, "Press r to load local work.")
        } else if !p.text_filter.is_empty() {
            (
                State::Empty,
                "No items match this page filter. Press / to change or clear it.",
            )
        } else {
            (
                State::Empty,
                "No local work items on this page. Press n to create a task, or use Add to work item on a source's actions menu.",
            )
        };
        Node::State {
            state,
            text: text.into(),
        }
    } else {
        Node::List {
            selected: items
                .iter()
                .position(|item| Some(&item.item_id) == p.selected.as_ref()),
            rows: items
                .iter()
                .map(|item| {
                    ListRow::new(item_target(p, &item.item_id), p.item_title(&item.item_id))
                        .right(Text::of(item.kind.as_str(), Ink::Muted))
                })
                .collect(),
            editing: None,
        }
    };
    let mut middle = PaneView::new(
        PaneId::Middle,
        format!("work items · page {}", p.page + 1),
        list,
    );
    if !p.text_filter.is_empty() {
        middle.footer = Some(Text::of(
            format!("Page filter: {}", p.text_filter),
            Ink::Info,
        ));
    }
    let sources = p.sources();
    let can_open = sources
        .get(p.source_at)
        .is_some_and(|source| cx.parts.has(&source.1.kind));
    let mut context = context(p);
    if !can_open && !sources.is_empty() && p.evidence.is_none() {
        context.insert(
            0,
            {
                let mut line = Text::of("Source owner unavailable here; ", Ink::Warn);
                line.extend(radar_core::mnemonic::text("c", "copy its exact locator", "status.warning"));
                line.add(Seg::new(".", Ink::Warn));
                line
            },
        );
    }
    let body = if p.evidence.is_some() || sources.is_empty() {
        let mut lines = context;
        if p.evidence.is_none() {
            lines.insert(0, Text::of(if p.pending.is_some() {
                "Loading bounded source evidence…"
            } else { "No source binding in this evidence page. Add an exact source from its actions menu; no links are inferred." }, Ink::Muted));
        }
        Node::Document { lines, cut: false }
    } else {
        let width = if cx.width >= 120 {
            radar_core::ui::detail_width(cx.width)
        } else if cx.width >= 84 {
            cx.width.saturating_sub(23)
        } else {
            cx.width
        };
        let mut rows = sources
            .iter()
            .enumerate()
            .map(|(index, source @ (_, key, basis, _, _))| {
                let mut row = EntryView::new(
                    source_target(p, source),
                    Text::of(
                        format!("{} · {}", key.kind, key.id),
                        if *basis == radar_core::journal::work_map::Basis::Inferred {
                            Ink::Warn
                        } else {
                            Ink::Primary
                        },
                    ),
                );
                row.body.push(Text::of(model::basis(*basis), Ink::Info));
                if index == p.source_at {
                    for line in &context {
                        row.body.extend(wrap_context(line, width.saturating_sub(4)));
                    }
                }
                row
            })
            .collect::<Vec<_>>();
        if let Some(first) = rows.first_mut() {
            let mut hint = Text::of("Exact sources · ", Ink::Muted);
            hint.extend(radar_core::mnemonic::text("↑↓", "select", "text.muted"));
            hint.add(Seg::new(" · ", Ink::Muted));
            hint.extend(radar_core::mnemonic::text("x", "extra actions", "text.muted"));
            first.before.push(hint);
        }
        Node::Entries {
            rows,
            selected: Some(p.source_at),
            empty: "No linked sources.".into(),
        }
    };
    let mut detail = PaneView::new(
        PaneId::Detail,
        if p.evidence.is_some() {
            "evidence"
        } else {
            "linked sources and context"
        },
        body,
    );
    if let Some(error) = p.error {
        detail.head = Some(Text::of(error.to_string(), Ink::Bad));
    }
    if p.detail.as_ref().is_some_and(|d| {
        d.expansion_truncated
            || d.primary.truncated
            || d.related.as_ref().is_some_and(|h| h.truncated)
    }) {
        detail.notes.push(Text::of(
            "Partial bounded evidence; inspect original history with v.",
            Ink::Warn,
        ));
    }
    if p.detail.as_ref().is_some_and(|d| {
        d.primary.known_recording_gaps || d.related.as_ref().is_some_and(|h| h.known_recording_gaps)
    }) {
        detail.notes.push(Text::of(
            "Recording degraded: known history gaps.",
            Ink::Warn,
        ));
    }
    if sources
        .iter()
        .any(|source| sources.iter().filter(|other| other.1 == source.1).count() > 1)
    {
        let mut warning = Text::of("Multiple binding claims for the same source; intervals may differ. ", Ink::Warn);
        warning.extend(radar_core::mnemonic::text("v", "View history", "status.warning"));
        warning.add(Seg::new(".", Ink::Warn));
        detail.notes.push(warning);
    }
    detail.notes.push(Text::of(
        "Observations only. Relations grant no identity, permission or completion.",
        Ink::Muted,
    ));
    detail.notes_most = Some(5);
    // Plain documents use the shared adapter's scrollable card path. Their
    // notices are already part of context(), so no separate fixed footer or
    // head is needed (the generic decorated-document path does not scroll).
    if matches!(detail.body, Node::Document { .. }) {
        detail.notes.clear();
        detail.head = None;
    }
    let mut view = ScreenView::new("work", vec![nav, middle, detail]);
    if p.pending.is_none() && p.capture.is_none() {
        view.actions = p
            .bindings(cx.focus.into())
            .iter()
            .filter(|binding| binding.keys != "o" || can_open)
            .map(|binding| ActionSpec::named(action_id(&binding.keys, cx.focus.into()), binding))
            .collect();
    }
    view.panes.extend(work_area::sections(p, cx, &view.actions));
    view
}

fn context(p: &WorkbenchPlugin) -> Vec<Text> {
    let sources = p.sources();
    let mut context = Vec::new();
    if let Some(error) = p.error {
        context.push(Text::of(error.to_string(), Ink::Bad));
    }
    if let Some(note) = &p.notice {
        context.push(Text::of(note, Ink::Info));
    }
    if let Some(evidence) = &p.evidence {
        context.push(Text::of(
            "Original evidence · observations only · no execution authority",
            Ink::Info,
        ));
        context.push(Text::plain(format!(
            "Snapshot events {} · map {}",
            evidence.snapshot.event_sequence(),
            evidence.snapshot.map_sequence()
        )));
        if evidence.known_recording_gaps {
            context.push(Text::of(
                "Recording gaps are known. This history is incomplete.",
                Ink::Warn,
            ));
        }
        if evidence.truncated {
            context.push(Text::of(
                "More evidence remains. Use v → Next evidence page.",
                Ink::Warn,
            ));
        }
        for record in &evidence.records {
            match record {
                HandoffRecord::Map { change } => {
                    let kind = match &change.request.mutation {
                        radar_core::journal::work_map::Mutation::CreateItem { .. } => {
                            "identity created"
                        }
                        radar_core::journal::work_map::Mutation::BindSubject { .. } => {
                            "source binding"
                        }
                        radar_core::journal::work_map::Mutation::CreateTag { .. } => "label created",
                        radar_core::journal::work_map::Mutation::RenameTag { .. } => "label assertion",
                        radar_core::journal::work_map::Mutation::AssignTag { .. } => {
                            "label assignment"
                        }
                        radar_core::journal::work_map::Mutation::RelateItems { .. } => {
                            "explicit relation"
                        }
                        radar_core::journal::work_map::Mutation::Retract { .. } => "claim retracted",
                    };
                    context.push(Text::plain(format!(
                        "Map #{} · {kind} · {}{}",
                        change.sequence,
                        model::basis(change.request.provenance.basis),
                        if change.retracted {
                            " · retracted"
                        } else {
                            ""
                        }
                    )));
                    context.extend(mutation_lines(&change.request.mutation));
                    context.push(Text::of(
                        format!(
                            "Claim origin: {:?} · session {} · recording action {}",
                            change.recording.origin,
                            change.recording.session_id.as_str(),
                            change.recording.action_id.as_str()
                        ),
                        Ink::Muted,
                    ));
                    context.push(Text::of(
                        format!(
                            "Observed {} ms · recorded {} ms",
                            change.request.observed_at_ms, change.recorded_at_ms
                        ),
                        Ink::Muted,
                    ));
                    context.push(Text::of(
                        format!(
                            "Change {} · evidence {}",
                            change.request.id.as_str(),
                            change.request.provenance.evidence_event_id.as_str()
                        ),
                        Ink::Muted,
                    ));
                }
                HandoffRecord::Action {
                    event, association, ..
                } => {
                    context.push(Text::plain(format!(
                        "Event #{} · {} · {:?} · {:?}",
                        event.sequence,
                        event.kind.as_str(),
                        event.outcome,
                        association
                    )));
                    context.push(Text::plain(format!(
                        "Origin: {:?} · session {}",
                        event.origin,
                        event.session_id.as_str()
                    )));
                    context.push(Text::of(
                        format!(
                            "Event {} · root {} · correlation {}",
                            event.event_id.as_str(),
                            event.root_action_id.as_str(),
                            event.correlation_id.as_str()
                        ),
                        Ink::Muted,
                    ));
                    if let Some(parent) = &event.parent_action_id {
                        context.push(Text::of(
                            format!("Parent action: {}", parent.as_str()),
                            Ink::Muted,
                        ));
                    }
                    for subject in &event.subjects {
                        context.push(Text::plain(format!(
                            "{:?} subject: {}",
                            subject.role,
                            locator(&subject.key)
                        )));
                    }
                    context.push(Text::of(
                        format!(
                            "Recorded {} ms · observation time {}",
                            event.recorded_at_ms,
                            if event.observation_time_estimated {
                                "estimated"
                            } else {
                                "exact"
                            }
                        ),
                        Ink::Muted,
                    ));
                    if event.metadata.redacted
                        || event.metadata.truncated
                        || event.metadata.omitted_subjects > 0
                    {
                        context.push(Text::of(format!("Original context limited: redacted {} · truncated {} · omitted subjects {}",
                            event.metadata.redacted, event.metadata.truncated, event.metadata.omitted_subjects), Ink::Warn));
                    }
                    context.push(Text::of(
                        format!(
                            "Action {} · observed {} ms",
                            event.action_id.as_str(),
                            event.observed_at_ms
                        ),
                        Ink::Muted,
                    ));
                }
                HandoffRecord::Gap { .. } => context.push(Text::of(
                    "Recording gap: some activity was not recorded.",
                    Ink::Warn,
                )),
            }
        }
        context.push(Text::of(
            "Missing events do not prove no work happened. Escape returns to the item.",
            Ink::Muted,
        ));
    } else if let Some(item) = &p.selected {
        context.push(heading(p.item_title(item)));
        let labels = model::labels_for(item, &p.pages())
            .into_iter()
            .filter(|l| l.namespace == model::GROUP_NAMESPACE)
            .map(|l| {
                if l.ambiguous {
                    format!("{} (ambiguous label)", l.text)
                } else {
                    l.text
                }
            })
            .collect::<Vec<_>>();
        if !labels.is_empty() {
            context.push(Text::plain(format!("Local groups: {}", labels.join(" · "))));
        }
        if let Some((_, key, basis, _, _)) = sources.get(p.source_at) {
            context.push(Text::plain(format!("Selected {}: {}", key.kind, key.id)));
            let mut hint = Text::of(format!("{} link · ", model::basis(*basis)), Ink::Info);
            hint.extend(radar_core::mnemonic::text("c", "copy locator", "status.info"));
            hint.add(Seg::new(" · ", Ink::Info));
            hint.extend(radar_core::mnemonic::text("x", "extra actions", "status.info"));
            context.push(hint);
            let outcome = p
                .detail
                .as_ref()
                .into_iter()
                .flat_map(|d| std::iter::once(&d.primary).chain(d.related.iter()))
                .flat_map(|h| &h.records)
                .filter_map(|record| match record {
                    HandoffRecord::Action { event, .. }
                        if event.subjects.iter().any(|s| s.key == *key) =>
                    {
                        Some(event.as_ref())
                    }
                    _ => None,
                })
                .max_by_key(|event| event.sequence);
            if let Some(event) = outcome {
                context.push(Text::plain(format!(
                    "Locator event in this page: {} · {}",
                    event.kind.as_str(),
                    format!("{:?}", event.outcome).to_lowercase()
                )));
            } else {
                context.push(Text::of(
                    "No source outcome in this bounded page.",
                    Ink::Muted,
                ));
            }
        }
        let mut history = radar_core::mnemonic::text("v", "view history", "status.info");
        history.add(Seg::new(": original claims and outcomes · completeness not guaranteed", Ink::Info));
        context.push(history);
        if let Some(detail) = &p.detail {
            if detail.expansion_truncated {
                context.push(Text::of("Partial one-hop map: more relation evidence may exist. Inspect task evidence with v.",Ink::Warn));
            }
            if detail.primary.known_recording_gaps
                || detail
                    .related
                    .as_ref()
                    .is_some_and(|h| h.known_recording_gaps)
            {
                context.push(Text::of(
                    "Recording degraded: known history gaps.",
                    Ink::Warn,
                ));
            }
            if detail.primary.truncated || detail.related.as_ref().is_some_and(|h| h.truncated) {
                context.push(Text::of(
                    "Bounded evidence page. History and labels may be partial.",
                    Ink::Warn,
                ));
            }
        }
        context.push(Text::of("Titles and groups are authored locally. Relations do not imply identity, permission or completion.",Ink::Muted));
        context.push(Text::of(
            format!("Task item ID: {}", item.as_str()),
            Ink::Muted,
        ));
        if let Some((source, key, _, from, through)) = sources.get(p.source_at) {
            context.push(heading("Source identity details"));
            context.extend([
                Text::plain(format!("Provider: {}", key.provider)),
                Text::plain(format!("Instance: {}", key.instance)),
                Text::of(format!("Source item: {}", source.as_str()), Ink::Muted),
                Text::of(
                    format!(
                        "Action interval: {from} through {}",
                        through.map_or_else(|| "open-ended".into(), |s| s.to_string())
                    ),
                    Ink::Muted,
                ),
            ]);
        }
    } else {
        context.push(Text::plain("Create a local task or select an existing item. No provider labels, relationships or goals are inferred."));
    }
    if p.items
        .as_ref()
        .and_then(|i| i.evidence.as_ref())
        .is_some_and(|h| h.truncated)
    {
        context.push(Text::of(
            "List labels are partial at this snapshot.",
            Ink::Warn,
        ));
    }
    context
}


/// Keep the existing Entries geometry for this text-only context, retaining
/// supplied styles/links/targets instead of flattening it to one ink. The shared
/// word wrapper still decides every row and never splits a word or grapheme.
/// A phrase's plain-reader hint belongs only to its last surviving fragment.
/// Empty-source metadata-only Text is outside this visible-context wrapper: as
/// with wrap_words, it produces no rows. Whitespace-only rows remain present.
pub(super) fn wrap_context(text: &Text, width: u16) -> Vec<Text> {
    let source = text.words();
    let mut offset = 0;
    let ranges: Vec<_> = text.0.iter().map(|seg| {
        let start = offset;
        offset += seg.text.len();
        start..offset
    }).collect();
    let mut cursor = 0;
    let mut rows: Vec<Vec<(usize, Seg)>> = Vec::new();
    for words in radar_core::ui::wrap_words(&source, width, 0) {
        let mut row = Vec::new();
        for (word_at, word) in words.split_whitespace().enumerate() {
            let start = cursor + source[cursor..].find(word).expect("wrapped source word");
            let end = start + word.len();
            if word_at > 0 {
                // A collapsed gap keeps the first original whitespace's look.
                let index = ranges.iter().position(|range| range.contains(&cursor)).unwrap();
                let mut gap = text.0[index].clone();
                gap.text = " ".into();
                gap.key_hint = None;
                row.push((index, gap));
            }
            for (index, range) in ranges.iter().enumerate() {
                let from = start.max(range.start);
                let through = end.min(range.end);
                if from < through {
                    let mut fragment = text.0[index].clone();
                    fragment.text = source[from..through].into();
                    fragment.key_hint = None;
                    row.push((index, fragment));
                }
            }
            cursor = end;
        }
        rows.push(row);
    }
    for (index, seg) in text.0.iter().enumerate() {
        let Some(hint) = &seg.key_hint else { continue };
        let previous = rows.iter().enumerate().rev().find_map(|(row, fragments)| {
            fragments.iter().enumerate().rev().find(|(_, (source_index, _))| *source_index <= index)
                .map(|(fragment, _)| (row, fragment))
        });
        if let Some((row, fragment)) = previous
            && rows[row][fragment].1.key_hint.is_none()
        {
            rows[row][fragment].1.key_hint = Some(hint.clone());
        } else {
            // Colliding hints and whitespace-only segments keep a zero-text
            // segment: plain readers retain both keys, visible geometry is fixed.
            let mut empty = seg.clone();
            empty.text.clear();
            if let Some((row, fragment)) = previous {
                rows[row].insert(fragment + 1, (index, empty));
            } else if let Some(row) = rows.first_mut() {
                row.insert(0, (index, empty));
            }
        }
    }
    rows.into_iter().map(|row| Text::from(row.into_iter().map(|(_, seg)| seg).collect::<Vec<_>>())).collect()
}

fn locator(key: &SubjectKey) -> String {
    format!(
        "{} · {} · {} · {}",
        key.provider, key.instance, key.kind, key.id
    )
}

fn mutation_lines(mutation: &radar_core::journal::work_map::Mutation) -> Vec<Text> {
    use radar_core::journal::work_map::{Mutation, TagTarget};
    let values = match mutation {
        Mutation::CreateItem { item_id, kind } => vec![format!(
            "Item {} · kind {}",
            item_id.as_str(),
            kind.as_str()
        )],
        Mutation::BindSubject {
            item_id,
            subject,
            from_sequence,
            through_sequence,
        } => vec![
            format!("Bound item: {}", item_id.as_str()),
            format!("Exact source: {}", locator(subject)),
            format!(
                "Action interval: {from_sequence} through {}",
                through_sequence.map_or_else(|| "open-ended".into(), |value| value.to_string())
            ),
        ],
        Mutation::CreateTag {
            tag_id,
            namespace,
            key,
            label,
        } => vec![
            format!(
                "Tag {} · namespace {} · key {}",
                tag_id.as_str(),
                namespace.as_str(),
                key.as_str()
            ),
            format!("Initial label: {}", label.as_str()),
        ],
        Mutation::RenameTag { tag_id, label } => vec![
            format!("Tag: {}", tag_id.as_str()),
            format!("Label assertion: {}", label.as_str()),
        ],
        Mutation::AssignTag { tag_id, target } => vec![format!(
            "Assign tag {} to {}",
            tag_id.as_str(),
            match target {
                TagTarget::Item(id) => format!("item {}", id.as_str()),
                TagTarget::Action(id) => format!("action {}", id.as_str()),
            }
        )],
        Mutation::RelateItems { from, to, kind } => vec![
            format!("Relation: {kind:?}"),
            format!("From item: {}", from.as_str()),
            format!("To item: {}", to.as_str()),
        ],
        Mutation::Retract { claim_id, expected } => vec![
            format!("Retract claim: {}", claim_id.as_str()),
            format!(
                "Expected typed target: {}",
                serde_json::to_string(expected).unwrap()
            ),
        ],
    };
    values.into_iter().map(Text::plain).collect()
}
