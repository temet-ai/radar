use super::*;
use radar_core::journal::StableCode;
use radar_core::journal::work_map::{
    ChangeId, ItemId, Mutation, RelationKind, ResolutionState, SemanticLabel, TagId, TagKey,
    TagTarget,
};
use radar_core::work_api::{Intent, ResolveRequest, ResolveResponse};
use radar_core::writes::Change;

impl WorkbenchPlugin {
    fn inspect_prepared(&mut self) -> Vec<Effect> {
        let Some(prepared) = &self.prepared else {
            return vec![];
        };
        self.send(
            Pending::Inspect,
            Request::InspectBatch(prepared.batch.clone()),
        )
    }

    fn change(&mut self, mutation: Mutation) -> Intent {
        Intent::asserted(
            ChangeId::parse(&self.next_id()).expect("generated ID"),
            mutation,
        )
    }
    pub(super) fn captured(&mut self, reply: ResolvedSources) -> Vec<Effect> {
        if self.capture != Some(reply.id) {
            return vec![];
        }
        self.capture = None;
        if self.verifying {
            let expected = self
                .link
                .as_ref()
                .map(|draft| {
                    draft
                        .candidates
                        .iter()
                        .filter(|c| c.included)
                        .map(|c| &c.source.key)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if reply.omitted > 0
                || expected.len() != reply.sources.len()
                || !expected
                    .iter()
                    .all(|key| reply.sources.iter().any(|s| &s.key == *key))
            {
                self.verifying = false;
                if self.prepared.as_ref().is_some_and(|p| p.restored) {
                    if let Some(prepared) = &mut self.prepared {
                        prepared.reconcile = true;
                        prepared.blocked = true;
                    }
                    return self.notice("A retained source changed or is unavailable. Original IDs are kept; restore the source connection and review with r, or inspect the copied batch.");
                }
                self.prepared = None;
                self.link = None;
                return self.notice(
                    "A source changed or became unavailable. Select it again before linking.",
                );
            }
            return self.resolve_next(0);
        }
        let Some(action) = reply.action_id else {
            return self.notice("Source linking needs a recorded selection. Check local activity storage and restart.");
        };
        if reply.sources.is_empty() {
            return self
                .notice("This selection has no validated source locator. Nothing was linked.");
        }
        let candidates = reply
            .sources
            .into_iter()
            .map(|source| Candidate {
                included: self.origin.as_ref().is_some_and(|original| {
                    original.kind == source.subject.kind
                        && original.id == source.subject.id
                        && original.url == source.subject.url
                }),
                source,
                resolved: None,
            })
            .collect();
        self.link = Some(LinkDraft {
            action,
            candidates,
            omitted: reply.omitted,
            task: None,
            title: None,
        });
        self.origin = None;
        if self.items.is_none() {
            self.read_items(true)
        } else {
            self.task_menu()
        }
    }
    pub(super) fn task_menu(&self) -> Vec<Effect> {
        let mut rows = vec![MenuItem::pick(
            "Create a new task…",
            "local title supplied by you",
            "new",
        )];
        if let Some(items) = &self.items {
            for item in items
                .page
                .items
                .iter()
                .filter(|i| i.kind.as_str() == "task")
            {
                rows.push(MenuItem::pick(
                    self.item_title(&item.item_id),
                    format!(
                        "{}…{}",
                        &item.item_id.as_str()[..6],
                        &item.item_id.as_str()[30..]
                    ),
                    item.item_id.as_str(),
                ));
            }
            if items.page.next.is_some() {
                rows.push(MenuItem::pick(
                    "More existing items…",
                    "next bounded page",
                    "next",
                ));
            }
            if self.page > 0 {
                rows.push(MenuItem::pick(
                    "Previous items…",
                    "same snapshot",
                    "previous",
                ));
            }
        }
        rows.push(MenuItem::note(
            "Titles are local authored labels. Selecting a task does not merge source identities.",
        ));
        vec![Effect::Menu(Menu::new(
            "work.choose",
            "Add to a local work item",
            rows,
        ))]
    }
    fn source_menu(&self) -> Vec<Effect> {
        let Some(draft) = &self.link else {
            return vec![];
        };
        let mut rows = Vec::new();
        for (i, c) in draft.candidates.iter().enumerate() {
            // Whether it is included is the only thing this row says by its mark, so
            // with icons off the symbol stays ([`icon::plain_or`]).
            let (icon, mark) = if c.included {
                radar_core::ui::icon::plain_or(radar_core::ui::icon::Icon::RunPassed, "✓")
            } else {
                radar_core::ui::icon::plain_or(radar_core::ui::icon::Icon::Todo, "○")
            };
            rows.push(
                MenuItem::pick(
                    format!("{mark}{} · {}", c.source.key.kind, c.source.subject.label),
                    format!(
                        "{} · {} · {}",
                        c.source.key.provider, c.source.key.instance, c.source.key.id
                    ),
                    format!("toggle:{i}"),
                )
                .icon(icon),
            );
        }
        if draft.omitted > 0 {
            rows.push(MenuItem::note(format!(
                "{} unsupported or excess candidate(s) omitted. No identity was inferred.",
                draft.omitted
            )));
        }
        rows.push(MenuItem::note("Choose up to four exact sources. Each keeps its own identity; this records an authored relation."));
        rows.push(MenuItem::pick(
            "Review local links…",
            "nothing changes until you confirm",
            "review",
        ));
        vec![Effect::Menu(Menu::new(
            "work.sources",
            "Choose sources to link",
            rows,
        ))]
    }
    fn resolve_next(&mut self, from: usize) -> Vec<Effect> {
        let next = self.link.as_ref().and_then(|d| {
            d.candidates
                .iter()
                .enumerate()
                .skip(from)
                .find(|(_, c)| c.included)
                .map(|(i, c)| (i, c.source.key.clone(), d.action.clone()))
        });
        if let Some((index, subject, action_id)) = next {
            return self.send(
                Pending::Resolve(index),
                Request::Resolve(ResolveRequest {
                    subject,
                    action_id,
                    snapshot: None,
                }),
            );
        }
        if self.verifying {
            self.verifying = false;
            return self.apply_prepared();
        }
        self.prepare_links()
    }
    pub(super) fn resolved(&mut self, index: usize, response: ResolveResponse) -> Vec<Effect> {
        if response.resolution.truncated || response.resolution.state == ResolutionState::Ambiguous
        {
            self.verifying = false;
            return self.notice("This source has ambiguous or partial identity claims. Review them with radar work before adding a relation.");
        }
        let resolved = (
            response.resolution.items.first().cloned(),
            response.requested_sequence,
        );
        let Some(candidate) = self.link.as_mut().and_then(|d| d.candidates.get_mut(index)) else {
            return vec![];
        };
        if self.verifying && candidate.resolved.as_ref() != Some(&resolved) {
            self.verifying = false;
            if self.prepared.as_ref().is_some_and(|p| p.restored) {
                if let Some(prepared) = &mut self.prepared {
                    prepared.reconcile = true;
                    prepared.blocked = true;
                }
                return self.notice("A retained source identity changed. Original IDs are kept; inspect the batch or reconcile the source before retrying.");
            }
            self.prepared = None;
            self.link = None;
            return self.notice("Source identity changed after review. Select the source again; nothing was applied.");
        }
        candidate.resolved = Some(resolved);
        self.resolve_next(index + 1)
    }
    fn title_changes(&mut self, task: &ItemId, title: &str) -> Vec<Intent> {
        let tag = TagId::parse(&self.next_id()).expect("generated ID");
        vec![
            self.change(Mutation::CreateItem {
                item_id: task.clone(),
                kind: StableCode::new("task"),
            }),
            self.change(Mutation::CreateTag {
                tag_id: tag.clone(),
                namespace: TagKey::new(model::TITLE_NAMESPACE).unwrap(),
                key: TagKey::new(task.as_str()).unwrap(),
                label: SemanticLabel::new(title).expect("validated title"),
            }),
            self.change(Mutation::AssignTag {
                tag_id: tag,
                target: TagTarget::Item(task.clone()),
            }),
        ]
    }
    fn prepare_links(&mut self) -> Vec<Effect> {
        let Some(draft) = self.link.as_ref() else {
            return vec![];
        };
        let Some(task) = draft.task.clone() else {
            return vec![];
        };
        let title = draft.title.clone();
        let chosen = draft
            .candidates
            .iter()
            .filter(|c| c.included)
            .map(|c| (c.source.key.clone(), c.resolved.clone()))
            .collect::<Vec<_>>();
        if chosen.iter().any(|(_, r)| r.is_none()) {
            return self.notice("Wait for source identity checks to finish.");
        }
        let mut changes = title
            .as_deref()
            .map(|title| self.title_changes(&task, title))
            .unwrap_or_default();
        let mut summary = vec![
            format!(
                "Task: {} ({})",
                title.unwrap_or_else(|| self.item_title(&task)),
                task.as_str()
            ),
            "Local user-authored annotations only. Provider data is unchanged.".into(),
        ];
        for (subject, resolved) in chosen {
            let (known, sequence) = resolved.expect("checked");
            let item = if let Some(item) = known {
                item
            } else {
                let item = ItemId::parse(&self.next_id()).unwrap();
                changes.push(self.change(Mutation::CreateItem {
                    item_id: item.clone(),
                    kind: StableCode::try_new(&subject.kind).expect("validated source kind"),
                }));
                changes.push(self.change(Mutation::BindSubject {
                    item_id: item.clone(),
                    subject: subject.clone(),
                    from_sequence: sequence,
                    through_sequence: None,
                }));
                summary.push(format!(
                    "New {} identity bound from action sequence {sequence} onward (no end).",
                    subject.kind
                ));
                item
            };
            changes.push(self.change(Mutation::RelateItems {
                from: task.clone(),
                to: item.clone(),
                kind: RelationKind::References,
            }));
            summary.push(format!(
                "References {} · {} · {} · {}",
                subject.provider, subject.instance, subject.kind, subject.id
            ));
        }
        if changes.is_empty() {
            return self.notice("Choose at least one source to link.");
        }
        self.prepared = Some(Prepared {
            attempted: false,
            reconcile: false,
            restored: false,
            blocked: false,
            batch: Batch::new(changes),
            task,
            summary,
        });
        self.confirm_prepared()
    }
    pub(super) fn confirm_prepared(&self) -> Vec<Effect> {
        let Some(prepared) = &self.prepared else {
            return vec![];
        };
        let mut menu = Menu::confirm(
            "work.confirm",
            "Save these local work annotations?",
            "save locally",
            "cancel",
        )
        .adds();
        for line in &prepared.summary {
            menu = menu.saying(line);
        }
        if prepared.attempted {
            menu=menu.saying("An earlier attempt may already be committed. This retries the same identities; cancel only closes this review.");
        }
        menu = menu.saying(
            "This records your assertion, not observed proof, permission or task completion.",
        );
        if let Some(notice) = &self.notice {
            menu = menu.saying(notice);
        }
        if prepared.blocked {
            menu.items.retain(|item| item.value != "yes");
            menu = menu.saying("Saving is blocked until recovery is reconciled. y outside this dialog copies the retained batch; r checks it again.");
        }
        vec![Effect::Menu(menu)]
    }
    fn apply_prepared(&mut self) -> Vec<Effect> {
        if self.pending.is_some() {
            return vec![];
        }
        let Some(prepared) = &mut self.prepared else {
            return vec![];
        };
        let prior_attempt = prepared.attempted;
        prepared.attempted = true;
        let batch = prepared.batch.clone();
        let mut effects = vec![Effect::PersistCache];
        effects.extend(self.send(Pending::Apply(prior_attempt), Request::Apply(batch)));
        effects
    }
    fn new_task(&mut self, title: &str) -> Vec<Effect> {
        let title = title.trim();
        if SemanticLabel::new(title).is_err() {
            return self.notice("Use a short local title without controls or credentials.");
        }
        let task = ItemId::parse(&self.next_id()).unwrap();
        if let Some(draft) = &mut self.link {
            draft.task = Some(task);
            draft.title = Some(title.into());
            return self.source_menu();
        }
        let changes = self.title_changes(&task, title);
        self.prepared = Some(Prepared {
            attempted: false,
            reconcile: false,
            restored: false,
            blocked: false,
            batch: Batch::new(changes),
            task,
            summary: vec![
                format!("Create task with local title: {title}"),
                "The title is your authored label, not a provider title or identity.".into(),
            ],
        });
        self.confirm_prepared()
    }
    fn group_menu_id(&self, kind: &str) -> String {
        format!(
            "{kind}:{}",
            serde_json::to_string(&(
                &self.selected,
                &self.snapshot,
                self.group_page,
                self.group_revision,
            ))
            .expect("local menu context")
        )
    }
    pub(super) fn group_menu(&self) -> Vec<Effect> {
        if self.selected.is_none() {
            return vec![];
        }
        let mut rows = vec![MenuItem::pick(
            "Create a local group label…",
            "name it yourself",
            "new",
        )];
        rows.extend(self.groups.iter().map(|g| {
            MenuItem::pick(
                &g.text,
                format!(
                    "{} · {}",
                    g.id.as_str(),
                    if g.ambiguous {
                        "label claims ambiguous or incomplete; cannot apply"
                    } else {
                        "existing local group"
                    }
                ),
                g.id.as_str(),
            )
        }));
        if self
            .group_catalog
            .as_ref()
            .is_some_and(|page| page.next.is_some())
        {
            rows.push(MenuItem::pick(
                "More groups…",
                "next bounded catalog page",
                "next",
            ));
        }
        if self.group_page > 0 {
            rows.push(MenuItem::pick(
                "Previous groups…",
                "same snapshot",
                "previous",
            ));
        }
        if self.group_error.is_some() {
            rows.push(MenuItem::note(
                "Group catalog could not be loaded. Close this menu and press r to retry.",
            ));
        }
        rows.push(MenuItem::note("Exact local group identities, including unassigned groups. Equal names do not merge groups."));
        vec![Effect::Menu(Menu::new(
            self.group_menu_id("work.group"),
            format!(
                "Label {} · groups page {}",
                self.item_title(self.selected.as_ref().unwrap()),
                self.group_page + 1
            ),
            rows,
        ))]
    }
    fn prepare_group(&mut self, label: Option<&str>, existing: Option<TagId>) -> Vec<Effect> {
        let Some(task) = self.selected.clone() else {
            return vec![];
        };
        let mut changes = Vec::new();
        let (tag, text) = if let Some(id) = existing {
            let Some(group) = self.groups.iter().find(|g| g.id == id && !g.ambiguous) else {
                return self.notice("Choose a current unambiguous local group.");
            };
            (id, group.text.clone())
        } else {
            let text = label.unwrap_or("").trim();
            let Ok(label) = SemanticLabel::new(text) else {
                return self.notice("Use a short group label without controls or credentials.");
            };
            let id = TagId::parse(&self.next_id()).unwrap();
            changes.push(self.change(Mutation::CreateTag {
                tag_id: id.clone(),
                namespace: TagKey::new(model::GROUP_NAMESPACE).unwrap(),
                key: TagKey::new(id.as_str()).unwrap(),
                label,
            }));
            (id, text.into())
        };
        changes.push(self.change(Mutation::AssignTag {
            tag_id: tag.clone(),
            target: TagTarget::Item(task.clone()),
        }));
        self.prepared = Some(Prepared {
            attempted: false,
            reconcile: false,
            restored: false,
            blocked: false,
            batch: Batch::new(changes),
            task: task.clone(),
            summary: vec![
                format!(
                    "Assign local group “{text}” to {} ({})",
                    self.item_title(&task),
                    task.as_str()
                ),
                format!("Group identity: {}", tag.as_str()),
                "Provider labels and action labels are unchanged.".into(),
            ],
        });
        self.confirm_prepared()
    }
    fn page_items(&mut self, forward: bool) -> Vec<Effect> {
        if forward {
            let Some(next) = self.items.as_ref().and_then(|i| i.page.next.clone()) else {
                return vec![];
            };
            self.cursors.truncate(self.page + 1);
            self.cursors.push(Some(next));
            self.page += 1;
        } else if self.page > 0 {
            self.page -= 1;
        } else {
            return vec![];
        }
        self.selected = None;
        self.read_items(false)
    }
    fn page_groups(&mut self, forward: bool, after: AfterGroups) -> Vec<Effect> {
        if forward {
            let Some(next) = self
                .group_catalog
                .as_ref()
                .and_then(|page| page.next.clone())
            else {
                return vec![];
            };
            self.group_cursors.truncate(self.group_page + 1);
            self.group_cursors.push(Some(next));
            self.group_page += 1;
        } else if self.group_page > 0 {
            self.group_page -= 1;
        } else {
            return vec![];
        }
        self.read_groups(after)
    }
    pub(super) fn menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        if self.pending.is_some() || self.capture.is_some() {
            return vec![];
        }
        let menu = if menu.starts_with("work.group:") {
            if menu != self.group_menu_id("work.group") {
                return self.notice(
                    "The group selection changed. Open the current item's group menu again.",
                );
            }
            "work.group"
        } else if menu.starts_with("work.new-group:") {
            if menu != self.group_menu_id("work.new-group") {
                return self.notice(
                    "The group selection changed. Open the current item's group menu again.",
                );
            }
            "work.new-group"
        } else if matches!(menu, "work.group" | "work.new-group") {
            return vec![];
        } else {
            menu
        };
        if self.prepared.is_some()
            && matches!(
                menu,
                "work.choose" | "work.new" | "work.sources" | "work.group" | "work.new-group"
            )
        {
            return self.notice(
                "Review or explicitly cancel the retained local change before starting another.",
            );
        }
        match menu {
            "work.choose" => match value {
                "new" => vec![Effect::Menu(Menu::typed(
                    "work.new",
                    "Name this local task",
                    "",
                ))],
                "next" => self.page_items(true),
                "previous" => self.page_items(false),
                _ => {
                    let Ok(task) = ItemId::parse(value) else {
                        return vec![];
                    };
                    if !self.items.as_ref().is_some_and(|items| {
                        items
                            .page
                            .items
                            .iter()
                            .any(|i| i.item_id == task && i.kind.as_str() == "task")
                    }) {
                        return self.notice("This task is no longer in the current selection.");
                    }
                    if let Some(d) = &mut self.link {
                        d.task = Some(task);
                        d.title = None;
                    }
                    self.source_menu()
                }
            },
            "work.new" => self.new_task(value),
            "work.sources" => {
                if value == "review" {
                    return self.resolve_next(0);
                }
                if let Some(index) = value
                    .strip_prefix("toggle:")
                    .and_then(|v| v.parse::<usize>().ok())
                    && let Some(d) = &mut self.link
                {
                    let count = d.candidates.iter().filter(|c| c.included).count();
                    if let Some(c) = d.candidates.get_mut(index) {
                        if !c.included && count >= 4 {
                            return self.notice("Choose up to four sources per review; more can be added afterward.");
                        }
                        c.included = !c.included;
                    }
                }
                self.source_menu()
            }
            "work.confirm" => {
                if value != "yes" {
                    if self
                        .prepared
                        .as_ref()
                        .is_some_and(|p| p.attempted || p.reconcile)
                    {
                        return self.notice("Retry paused. The original change identities are retained because the commit may already exist.");
                    }
                    self.prepared = None;
                    self.link = None;
                    self.verifying = false;
                    return vec![Effect::PersistCache];
                }
                if self.prepared.as_ref().is_some_and(|p| p.reconcile) {
                    return self.inspect_prepared();
                }
                if self.prepared.as_ref().is_some_and(|p| p.blocked) {
                    return self.notice("This retained batch needs source selection or recovery inspection before saving.");
                }
                if let Some(why) = radar_core::writes::not_sent(Change::Add) {
                    return self.notice(why);
                }
                // A possibly committed batch must replay its exact original identities.
                // Re-resolving would mistake its own new bindings for changed intent.
                if self
                    .prepared
                    .as_ref()
                    .is_some_and(|prepared| prepared.attempted)
                {
                    return self.apply_prepared();
                }
                if let Some(draft) = &self.link {
                    let subjects = draft
                        .candidates
                        .iter()
                        .filter(|c| c.included)
                        .map(|c| c.source.subject.clone())
                        .collect::<Vec<_>>();
                    if !subjects.is_empty() {
                        self.request = self.request.wrapping_add(1);
                        self.capture = Some(self.request);
                        self.verifying = true;
                        return vec![Effect::ResolveSources {
                            id: self.request,
                            subjects,
                        }];
                    }
                }
                self.apply_prepared()
            }
            "work.group" => {
                if value == "new" {
                    vec![Effect::Menu(Menu::typed(
                        self.group_menu_id("work.new-group"),
                        "Name this local group",
                        "",
                    ))]
                } else if value == "next" {
                    self.page_groups(true, AfterGroups::Menu)
                } else if value == "previous" {
                    self.page_groups(false, AfterGroups::Menu)
                } else {
                    self.prepare_group(None, TagId::parse(value).ok())
                }
            }
            "work.new-group" => self.prepare_group(Some(value), None),
            "work.filter" => {
                self.text_filter = value.trim().chars().take(128).collect();
                let items = self.visible_items();
                if items
                    .iter()
                    .any(|item| Some(&item.item_id) == self.selected.as_ref())
                {
                    vec![]
                } else if let Some(item) = items.first() {
                    self.select_item(item.item_id.clone())
                } else {
                    self.selected = None;
                    self.detail = None;
                    self.evidence = None;
                    vec![]
                }
            }
            "work.history" => {
                let page = match value {
                    "primary" => self.detail.as_ref().map(|d| &d.primary),
                    "related" => self.detail.as_ref().and_then(|d| d.related.as_ref()),
                    "next" => self.evidence.as_ref(),
                    _ => None,
                };
                let Some(page) = page else { return vec![] };
                let cursor = if value == "next" {
                    page.next.clone()
                } else {
                    None
                };
                if value == "next" && cursor.is_none() {
                    return vec![];
                }
                let request = work_api::EvidenceRequest {
                    selection: page.selection.clone(),
                    cursor,
                    snapshot: Some(page.snapshot.clone()),
                };
                self.send(Pending::Evidence, Request::Evidence(request))
            }
            _ => vec![],
        }
    }
    pub(super) fn bindings(&self, pane: Pane) -> Vec<Binding> {
        let mut rows = vec![
            Binding::heading(
                self.selected
                    .as_ref()
                    .map(|i| self.item_title(i))
                    .unwrap_or_else(|| "Local work".into()),
            ),
            Binding::new("n", "new local task…").adds().pane(),
            Binding::new(
                "r",
                if self.prepared.is_some() {
                    "review retained change…"
                } else {
                    "refresh local work"
                },
            ).pane(),
            Binding::new("/", "filter this item page…").pane(),
        ];
        if self.prepared.is_some() {
            rows.push(Binding::new(
                "y",
                "copy retained local batch for inspection",
            ).pane());
        }
        if self
            .prepared
            .as_ref()
            .is_some_and(|p| p.attempted || p.reconcile || p.blocked)
        {
            rows.retain(|row| row.keys != "n");
        }
        if self.selected.is_some() {
            if self.prepared.is_none() {
                rows.push(Binding::new("t", "set local group label…").adds());
            }
            rows.push(Binding::new("v", "inspect bounded evidence…"));
        }
        if pane == Pane::Detail && self.evidence.is_none() && !self.sources().is_empty() {
            rows.push(Binding::new("o", "open selected source"));
            rows.push(Binding::new("c", "copy exact source locator"));
        }
        if pane == Pane::Nav {
            if self
                .group_catalog
                .as_ref()
                .is_some_and(|page| page.next.is_some())
            {
                rows.push(Binding::new("]", "next group page").pane());
            }
            if self.group_page > 0 {
                rows.push(Binding::new("[", "previous group page").pane());
            }
        } else {
            if self.items.as_ref().is_some_and(|i| i.page.next.is_some()) {
                rows.push(Binding::new("]", "next item page").pane());
            }
            if self.page > 0 {
                rows.push(Binding::new("[", "previous item page").pane());
            }
        }
        rows
    }
    pub(super) fn handle_key(&mut self, pane: Pane, chord: Chord) -> Outcome {
        if chord.ctrl || chord.alt || self.pending.is_some() || self.capture.is_some() {
            return Outcome::Ignored;
        }
        if chord.key == Key::Char('n')
            && self
                .prepared
                .as_ref()
                .is_some_and(|p| !p.attempted && !p.reconcile && !p.blocked)
        {
            self.prepared = None;
        }
        let effects = match chord.key {
            Key::Char('n') if self.prepared.is_none() => {
                self.link = None;
                vec![Effect::Menu(Menu::typed(
                    "work.new",
                    "Name this local task",
                    "",
                ))]
            }
            Key::Char('r') => {
                if self.prepared.as_ref().is_some_and(|p| p.reconcile) {
                    self.inspect_prepared()
                } else if self.prepared.is_some() {
                    self.confirm_prepared()
                } else {
                    self.read_items(true)
                }
            }
            Key::Char('y') if self.prepared.is_some() => {
                let batch = &self.prepared.as_ref().unwrap().batch;
                vec![Effect::Copy(serde_json::to_string_pretty(batch).unwrap()), Effect::Notify("Retained batch copied with its original change IDs. Inspect with radar work before retrying.".into())]
            }
            Key::Char('/') => vec![Effect::Menu(Menu::typed(
                "work.filter",
                "Filter this bounded item page",
                &self.text_filter,
            ))],
            Key::Char('t') if self.prepared.is_none() => self.group_menu(),
            Key::Char(']') if pane == Pane::Nav => self.page_groups(true, AfterGroups::Stay),
            Key::Char('[') if pane == Pane::Nav => self.page_groups(false, AfterGroups::Stay),
            Key::Char(']') => self.page_items(true),
            Key::Char('[') => self.page_items(false),
            Key::Char('v') if self.selected.is_some() => {
                let mut rows = vec![MenuItem::pick(
                    "Task evidence",
                    "bounded claims and original outcomes",
                    "primary",
                )];
                if self.detail.as_ref().is_some_and(|d| d.related.is_some()) {
                    rows.push(MenuItem::pick(
                        "Linked source evidence",
                        "one-hop selection only",
                        "related",
                    ));
                }
                if self.evidence.as_ref().is_some_and(|p| p.next.is_some()) {
                    rows.push(MenuItem::pick(
                        "Next evidence page",
                        "same snapshot",
                        "next",
                    ));
                }
                vec![Effect::Menu(Menu::new(
                    "work.history",
                    "Read original evidence",
                    rows,
                ))]
            }
            Key::Enter if pane == Pane::Middle && self.selected.is_some() => {
                vec![Effect::Focus(Pane::Detail)]
            }
            Key::Char('o') | Key::Enter if pane == Pane::Detail && self.evidence.is_none() => self
                .sources()
                .get(self.source_at)
                .map(|(_, key, _, _, _)| vec![Effect::ShowSource(key.clone())])
                .unwrap_or_default(),
            Key::Char('c') if pane == Pane::Detail && self.evidence.is_none() => self
                .sources()
                .get(self.source_at)
                .map(|(_, key, _, _, _)| {
                    vec![
                        Effect::Copy(serde_json::to_string(key).unwrap_or_default()),
                        Effect::Notify("Exact scoped source locator copied.".into()),
                    ]
                })
                .unwrap_or_default(),
            Key::Esc if self.evidence.is_some() => {
                self.evidence = None;
                vec![]
            }
            _ => return Outcome::Ignored,
        };
        Outcome::Handled(effects)
    }

    pub(super) fn handle_intent(&mut self, pane: Pane, intent: &ViewIntent) -> Outcome {
        if self.pending.is_some() || self.capture.is_some() {
            return Outcome::Ignored;
        }
        match intent {
            ViewIntent::Key { chord } => return self.handle_key(pane, *chord),
            ViewIntent::Invoke { action } => {
                let Some(chord) = self.bindings(pane).iter().find_map(|binding| {
                    (screen::action_id(&binding.keys, pane) == action)
                        .then(|| Chord::parse(&binding.keys))
                        .flatten()
                }) else {
                    return Outcome::Ignored;
                };
                return self.handle_key(pane, chord);
            }
            _ => {}
        }
        let (target, activate) = match intent {
            ViewIntent::Select { target } | ViewIntent::Context { target } => {
                (target.as_str(), false)
            }
            ViewIntent::Activate { target } => (target.as_str(), true),
            _ => return Outcome::Ignored,
        };
        // Targets are scoped to the current bounded snapshot and filter. Always
        // check live membership, even when the frontend replays an older view.
        let effects = match pane {
            Pane::Middle => {
                let items = self.visible_items();
                let Some(item) = items
                    .iter()
                    .find(|item| screen::item_target(self, &item.item_id) == target)
                else {
                    return Outcome::Ignored;
                };
                let mut effects = self.select_item(item.item_id.clone());
                if activate {
                    effects.push(Effect::Focus(Pane::Detail));
                }
                effects
            }
            Pane::Nav => {
                let filter = if screen::group_target(self, None) == target {
                    None
                } else if let Some(id) = &self.filter
                    && screen::group_target(self, Some(id)) == target
                {
                    Some(id.clone())
                } else {
                    let Some(group) = self
                        .groups
                        .iter()
                        .find(|group| screen::group_target(self, Some(&group.id)) == target)
                    else {
                        return Outcome::Ignored;
                    };
                    Some(group.id.clone())
                };
                let mut effects = if self.filter == filter {
                    vec![]
                } else {
                    self.selected = None;
                    self.filter = filter;
                    self.detail = None;
                    self.evidence = None;
                    self.page = 0;
                    self.cursors = vec![None];
                    self.read_items(false)
                };
                if activate {
                    effects.push(Effect::Focus(Pane::Middle));
                }
                effects
            }
            Pane::Detail if self.evidence.is_none() => {
                let sources = self.sources();
                let Some(index) = sources
                    .iter()
                    .position(|source| screen::source_target(self, source) == target)
                else {
                    return Outcome::Ignored;
                };
                self.source_at = index;
                if activate {
                    vec![Effect::ShowSource(sources[index].1.clone())]
                } else {
                    vec![]
                }
            }
            Pane::Detail => return Outcome::Ignored,
        };
        Outcome::Handled(effects)
    }
}

impl WorkbenchPlugin {
    pub(super) fn saved_link(&self) -> Option<serde_json::Value> {
        self.prepared.as_ref()?;
        let draft = self.link.as_ref()?;
        let candidates = draft
            .candidates
            .iter()
            .filter(|candidate| candidate.included)
            .map(|candidate| {
                let subject = &candidate.source.subject;
                let fields = subject
                    .fields
                    .iter()
                    .filter(|(key, _)| routing_field(key))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<std::collections::BTreeMap<_, _>>();
                serde_json::json!({
                    "key": candidate.source.key,
                    "kind": subject.kind,
                    "id": subject.id,
                    "url": subject.url,
                    "fields": fields,
                    "resolved": candidate.resolved,
                })
            })
            .collect::<Vec<_>>();
        let saved = serde_json::json!({"action": draft.action, "task": draft.task, "title": draft.title, "candidates": candidates});
        // The cache holds only the selected owners' routing envelope, never
        // recursive related subjects or display labels. Refuse unsafe envelopes.
        self.restore_link(
            &saved,
            &self.prepared.as_ref()?.task,
            &self.prepared.as_ref()?.batch,
        )?;
        Some(saved)
    }

    pub(super) fn restore_link(
        &self,
        saved: &serde_json::Value,
        task: &ItemId,
        batch: &Batch,
    ) -> Option<LinkDraft> {
        if serde_json::to_vec(saved).ok()?.len() > 65_536 {
            return None;
        }
        let action = serde_json::from_value::<ActionId>(saved["action"].clone()).ok()?;
        let saved_task = serde_json::from_value::<ItemId>(saved["task"].clone()).ok()?;
        if &saved_task != task {
            return None;
        }
        let title = saved["title"].as_str().map(str::to_owned);
        if title
            .as_deref()
            .is_some_and(|title| SemanticLabel::new(title).is_err())
        {
            return None;
        }
        let rows = saved["candidates"].as_array()?;
        if rows.is_empty() || rows.len() > 4 {
            return None;
        }
        fn safe(value: &str, max: usize) -> bool {
            !value.is_empty()
                && value.len() <= max
                && !value.chars().any(char::is_control)
                && !radar_core::log::contains_secret(value)
        }
        let mut candidates = Vec::new();
        for row in rows {
            let key =
                serde_json::from_value::<radar_core::journal::SubjectKey>(row["key"].clone()).ok()?;
            if radar_core::journal::SubjectKey::new(&key.provider, &key.instance, &key.kind, &key.id)
                .ok()?
                != key
                || candidates
                    .iter()
                    .any(|candidate: &Candidate| candidate.source.key == key)
            {
                return None;
            }
            let kind = row["kind"].as_str()?;
            let id = row["id"].as_str()?;
            if !safe(kind, 96) || !safe(id, 4096) {
                return None;
            }
            let mut fields = serde_json::from_value::<std::collections::BTreeMap<String, String>>(
                row["fields"].clone(),
            )
            .ok()?;
            if fields.len() > 32
                || fields.iter().any(|(key, value)| {
                    !safe(key, 96)
                        || value.len() > 4096
                        || value.chars().any(char::is_control)
                        || radar_core::log::contains_secret(value)
                })
            {
                return None;
            }
            fields.retain(|key, _| routing_field(key));
            let url = row["url"].as_str().map(str::to_owned);
            if url.as_deref().is_some_and(|url| {
                !safe(url, 4096)
                    || radar_core::journal::SafeUrl::parse(url).is_ok_and(|safe| safe.as_str() != url)
                    || radar_core::journal::SafeUrl::parse(url).is_err()
            }) {
                return None;
            }
            let resolved =
                serde_json::from_value::<(Option<ItemId>, i64)>(row["resolved"].clone()).ok()?;
            if resolved.1 < 1 {
                return None;
            }
            let mut subject = Subject::new(kind, id, format!("{} · {}", key.kind, key.id));
            subject.url = url;
            subject.fields = fields;
            candidates.push(Candidate {
                source: ResolvedSource { subject, key },
                included: true,
                resolved: Some(resolved),
            });
        }
        let mut references = batch
            .changes
            .iter()
            .filter_map(|change| match &change.mutation {
                Mutation::RelateItems {
                    from,
                    to,
                    kind: RelationKind::References,
                } if from == task => Some(to),
                _ => None,
            })
            .collect::<Vec<_>>();
        if references.len() != candidates.len() {
            return None;
        }
        for candidate in &candidates {
            let (existing, sequence) = candidate.resolved.as_ref()?;
            let target = match existing {
                Some(item) => item,
                None => batch
                    .changes
                    .iter()
                    .find_map(|change| match &change.mutation {
                        Mutation::BindSubject {
                            item_id,
                            subject,
                            from_sequence,
                            through_sequence: None,
                        } if subject == &candidate.source.key && from_sequence == sequence => {
                            Some(item_id)
                        }
                        _ => None,
                    })?,
            };
            // Consume exactly one relation per candidate. Distinct locators can
            // resolve to the same item, but their multiplicity must match the
            // retained batch so no other target escapes revalidation.
            let index = references
                .iter()
                .position(|reference| *reference == target)?;
            references.swap_remove(index);
        }
        if batch.changes.iter().any(|change| matches!(&change.mutation,
            Mutation::BindSubject { subject, .. } if !candidates.iter().any(|candidate| &candidate.source.key == subject)
        )) { return None; }
        Some(LinkDraft {
            action,
            task: Some(saved_task),
            title,
            candidates,
            omitted: 0,
        })
    }
}

// Shared subject locator vocabulary only; display/people/status text is not
// required to prove an owner's scoped identity after restart.
fn routing_field(key: &str) -> bool {
    matches!(
        key,
        "provider"
            | "instance"
            | "site"
            | "project"
            | "repository"
            | "repository_url"
            | "number"
            | "iid"
            | "run_id"
            | "attempt"
            | "pipeline"
            | "job"
    )
}
