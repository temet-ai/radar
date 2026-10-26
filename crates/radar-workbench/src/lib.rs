//! Explicit local tasks, labels and source relations over the shared work map.
//! The part owns presentation; the host owns audited storage and source routing.
#[cfg(any(test, feature = "demo"))]
mod demo;
mod interaction;
mod model;
mod screen;
mod work_area;
#[cfg(test)]
mod tests;

use radar_core::compose::{Action, Adds, Contribution, Owned, Point, Showing};
use radar_core::journal::work_map::{
    Handoff, ItemCursor, ItemFilter, ItemId, Snapshot, TagCursor, TagId, TagKey, TagPage,
};
type SourceBinding = (
    ItemId,
    SubjectKey,
    radar_core::journal::work_map::Basis,
    i64,
    Option<i64>,
);
use radar_core::journal::{ActionId, SubjectKey};
use radar_core::plugin::{Msg, ScreenSpec, Typing, Verdict};
use radar_core::source_context::{ResolvedSource, ResolvedSources, WorkReply};
use radar_core::view::{Chord, Intent as ViewIntent, Key, ScreenView, ViewCx};
use radar_core::work_api::{self, Batch, DetailResponse, ItemsResponse, Request, Response};
use radar_core::{Binding, Effect, Menu, MenuItem, Outcome, Pane, Plugin, Subject};

#[derive(Clone, Copy)]
enum Pending {
    Items,
    Groups(AfterGroups),
    Detail,
    Evidence,
    Inspect,
    Resolve(usize),
    /// True when an earlier apply may already have committed.
    Apply(bool),
}

#[derive(Clone, Copy)]
enum AfterGroups {
    Items,
    Menu,
    Stay,
}

struct Candidate {
    source: ResolvedSource,
    included: bool,
    resolved: Option<(Option<ItemId>, i64)>,
}

struct LinkDraft {
    action: ActionId,
    candidates: Vec<Candidate>,
    omitted: usize,
    task: Option<ItemId>,
    title: Option<String>,
}

#[derive(Clone)]
struct Prepared {
    batch: Batch,
    task: ItemId,
    summary: Vec<String>,
    attempted: bool,
    reconcile: bool,
    restored: bool,
    blocked: bool,
}

impl Prepared {
    fn links_sources(&self) -> bool {
        self.batch.changes.iter().any(|change| {
            matches!(
                change.mutation,
                radar_core::journal::work_map::Mutation::BindSubject { .. }
                    | radar_core::journal::work_map::Mutation::RelateItems {
                        kind: radar_core::journal::work_map::RelationKind::References,
                        ..
                    }
            )
        })
    }
}

pub struct WorkbenchPlugin {
    seed: String,
    serial: u64,
    request: u64,
    pending: Option<(u64, Pending)>,
    started: bool,
    items: Option<ItemsResponse>,
    cursors: Vec<Option<ItemCursor>>,
    page: usize,
    snapshot: Option<Snapshot>,
    selected: Option<ItemId>,
    source_at: usize,
    detail: Option<DetailResponse>,
    evidence: Option<Handoff>,
    filter: Option<TagId>,
    text_filter: String,
    groups: Vec<model::Label>,
    group_catalog: Option<TagPage>,
    group_cursors: Vec<Option<TagCursor>>,
    group_page: usize,
    group_revision: u64,
    group_error: Option<work_api::Error>,
    notice: Option<String>,
    error: Option<work_api::Error>,
    link: Option<LinkDraft>,
    prepared: Option<Prepared>,
    capture: Option<u64>,
    origin: Option<Subject>,
    verifying: bool,
}

impl Default for WorkbenchPlugin {
    fn default() -> Self {
        Self {
            seed: ItemId::new().as_str().into(),
            serial: 0,
            request: 0,
            pending: None,
            started: false,
            items: None,
            cursors: vec![None],
            page: 0,
            snapshot: None,
            selected: None,
            source_at: 0,
            detail: None,
            evidence: None,
            filter: None,
            text_filter: String::new(),
            groups: vec![],
            group_catalog: None,
            group_cursors: vec![None],
            group_page: 0,
            group_revision: 0,
            group_error: None,
            notice: None,
            error: None,
            link: None,
            prepared: None,
            capture: None,
            origin: None,
            verifying: false,
        }
    }
}

impl WorkbenchPlugin {
    fn next_id(&mut self) -> String {
        self.serial = self
            .serial
            .checked_add(1)
            .expect("local intent counter exhausted");
        assert!(self.serial < (1u64 << 48));
        format!("{}{:012x}", &self.seed[..24], self.serial)
    }
    fn send(&mut self, kind: Pending, request: Request) -> Vec<Effect> {
        self.request = self.request.wrapping_add(1);
        self.pending = Some((self.request, kind));
        self.error = None;
        vec![Effect::Work {
            id: self.request,
            request: Box::new(request),
        }]
    }
    fn read_items(&mut self, fresh: bool) -> Vec<Effect> {
        if fresh {
            self.page = 0;
            self.cursors = vec![None];
            self.snapshot = None;
            self.groups.clear();
            self.group_catalog = None;
            self.group_cursors = vec![None];
            self.group_page = 0;
            self.group_revision = self.group_revision.wrapping_add(1);
            self.group_error = None;
        }
        self.send(
            Pending::Items,
            Request::Items(work_api::ItemsRequest {
                filter: ItemFilter {
                    all_tags: self.filter.clone().into_iter().collect(),
                },
                cursor: self.cursors.get(self.page).cloned().flatten(),
                snapshot: self.snapshot.clone(),
            }),
        )
    }
    fn read_groups(&mut self, after: AfterGroups) -> Vec<Effect> {
        self.group_revision = self.group_revision.wrapping_add(1);
        self.group_error = None;
        self.send(
            Pending::Groups(after),
            Request::Tags(work_api::TagsRequest {
                namespace: TagKey::new(model::GROUP_NAMESPACE).expect("fixed namespace"),
                cursor: self.group_cursors.get(self.group_page).cloned().flatten(),
                snapshot: self.snapshot.clone(),
            }),
        )
    }
    fn after_groups(&mut self, after: AfterGroups) -> Vec<Effect> {
        match after {
            AfterGroups::Items => {
                if self.link.is_some() && self.prepared.is_none() {
                    return self.task_menu();
                }
                if self.selected.is_none() {
                    self.selected = self.visible_items().first().map(|i| i.item_id.clone());
                }
                self.read_detail()
            }
            AfterGroups::Menu => self.group_menu(),
            AfterGroups::Stay => vec![],
        }
    }
    fn read_detail(&mut self) -> Vec<Effect> {
        self.detail = None;
        self.evidence = None;
        self.source_at = 0;
        let Some(item_id) = self.selected.clone() else {
            return vec![];
        };
        self.send(
            Pending::Detail,
            Request::Detail(work_api::DetailRequest {
                item_id,
                snapshot: self.snapshot.clone(),
            }),
        )
    }
    fn pages(&self) -> Vec<&Handoff> {
        let mut pages = Vec::new();
        if let Some(items) = &self.items
            && let Some(evidence) = &items.evidence
        {
            pages.push(evidence);
        }
        if let Some(detail) = &self.detail {
            pages.push(&detail.primary);
            if let Some(related) = &detail.related {
                pages.push(related);
            }
        }
        if let Some(evidence) = &self.evidence {
            pages.push(evidence);
        }
        pages
    }
    fn item_title(&self, item: &ItemId) -> String {
        model::title(item, &self.pages())
    }
    fn visible_items(&self) -> Vec<radar_core::journal::work_map::ItemEntry> {
        let mut items = self
            .items
            .as_ref()
            .map(|items| {
                items
                    .page
                    .items
                    .iter()
                    .filter(|item| {
                        self.text_filter.is_empty()
                            || self
                                .item_title(&item.item_id)
                                .to_lowercase()
                                .contains(&self.text_filter.to_lowercase())
                            || item.item_id.as_str().contains(&self.text_filter)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if self.text_filter.is_empty()
            && let Some(selected) = &self.selected
            && !items.iter().any(|item| &item.item_id == selected)
            && let Some(detail) = &self.detail
            && let Some(item) = detail.primary.records.iter().find_map(|record| {
                let radar_core::journal::work_map::HandoffRecord::Map { change } = record else {
                    return None;
                };
                match &change.request.mutation {
                    radar_core::journal::work_map::Mutation::CreateItem { item_id, kind }
                        if item_id == selected =>
                    {
                        Some(radar_core::journal::work_map::ItemEntry {
                            item_id: item_id.clone(),
                            kind: kind.clone(),
                            created_sequence: change.sequence,
                        })
                    }
                    _ => None,
                }
            })
        {
            items.insert(0, item);
        }
        items
    }
    fn select_item(&mut self, item: ItemId) -> Vec<Effect> {
        if self.selected.as_ref() == Some(&item) && self.detail.is_some() {
            return vec![];
        }
        self.selected = Some(item);
        let mut effects = vec![Effect::PersistCache];
        effects.extend(self.read_detail());
        effects
    }
    fn notice(&mut self, text: impl Into<String>) -> Vec<Effect> {
        let text = text.into();
        self.notice = Some(text.clone());
        vec![Effect::Notify(text)]
    }
    fn error_notice(&mut self, text: impl Into<String>) -> Vec<Effect> {
        let text = text.into();
        self.notice = Some(text.clone());
        vec![Effect::error(None, text)]
    }
    fn sources(&self) -> Vec<SourceBinding> {
        let Some(detail) = &self.detail else {
            return vec![];
        };
        let mut result = Vec::new();
        let mut pages = vec![&detail.primary];
        if let Some(related) = &detail.related {
            pages.push(related);
        }
        for change in model::claims(&pages) {
            if change.retracted {
                continue;
            }
            if let radar_core::journal::work_map::Mutation::BindSubject {
                item_id,
                subject,
                from_sequence,
                through_sequence,
            } = &change.request.mutation
                && !result.iter().any(|(id, key, basis, from, through)| {
                    id == item_id
                        && key == subject
                        && *basis == change.request.provenance.basis
                        && from == from_sequence
                        && through == through_sequence
                })
            {
                result.push((
                    item_id.clone(),
                    subject.clone(),
                    change.request.provenance.basis,
                    *from_sequence,
                    *through_sequence,
                ));
            }
        }
        result
    }
    fn received(&mut self, reply: WorkReply) -> Vec<Effect> {
        let Some((id, kind)) = self.pending else {
            return vec![];
        };
        if reply.id != id {
            return vec![];
        }
        self.pending = None;
        let response = match reply.result {
            Ok(response) => response,
            Err(error) => {
                if let Pending::Groups(after) = kind {
                    self.group_error = Some(error);
                    self.groups.clear();
                    self.group_catalog = None;
                    return self.after_groups(after);
                }
                self.error = Some(error);
                if matches!(kind, Pending::Inspect)
                    && let Some(prepared) = &mut self.prepared
                {
                    prepared.reconcile = true;
                    prepared.blocked = true;
                }
                if matches!(kind, Pending::Apply(false))
                    && matches!(
                        error,
                        work_api::Error::PermissionDenied
                            | work_api::Error::InvalidInput
                            | work_api::Error::Conflict
                            | work_api::Error::Unavailable
                            | work_api::Error::Disabled
                    )
                    && let Some(prepared) = &mut self.prepared
                {
                    prepared.attempted = false;
                }
                let mut effects = self.error_notice(error.to_string());
                effects.push(Effect::PersistCache);
                return effects;
            }
        };
        match (kind, response) {
            (Pending::Items, Response::Items(items)) => {
                self.snapshot = Some(items.page.snapshot.clone());
                self.items = Some(items);
                if self.group_catalog.is_none() && self.group_error.is_none() {
                    return self.read_groups(AfterGroups::Items);
                }
                self.after_groups(AfterGroups::Items)
            }
            (Pending::Groups(after), Response::Tags(catalog))
                if self.snapshot.as_ref() == Some(&catalog.snapshot)
                    && catalog.namespace.as_str() == model::GROUP_NAMESPACE =>
            {
                self.groups = catalog.tags.iter().map(model::catalog_label).collect();
                self.group_catalog = Some(catalog);
                self.after_groups(after)
            }
            (Pending::Groups(after), _) => {
                self.group_error = Some(work_api::Error::InvalidInput);
                self.groups.clear();
                self.group_catalog = None;
                self.after_groups(after)
            }
            (Pending::Detail, Response::Detail(detail))
                if Some(&detail.selected) == self.selected.as_ref() =>
            {
                self.detail = Some(detail);
                vec![]
            }
            (Pending::Evidence, Response::Evidence(evidence)) => {
                self.evidence = Some(evidence);
                vec![]
            }
            (Pending::Inspect, Response::BatchState(state)) => {
                let Some(prepared) = &mut self.prepared else {
                    return vec![];
                };
                match state {
                    work_api::BatchState::AllPresent => {
                        prepared.attempted = true;
                        prepared.reconcile = false;
                        prepared.blocked = false;
                        self.notice = Some("The retained changes are already recorded. Review the original identities to finish recovery.".into());
                    }
                    work_api::BatchState::NonePresent => {
                        prepared.attempted = false;
                        prepared.reconcile = false;
                        prepared.blocked = prepared.links_sources() && self.link.is_none();
                        self.notice = Some(if prepared.blocked {
                            "No retained change IDs were found at the checked snapshot. Cancel this preview and select the exact sources again, or copy the retained batch for inspection."
                        } else {
                            "No retained change IDs were found at the checked snapshot. Sources will be validated again before saving the original batch."
                        }.into());
                    }
                    work_api::BatchState::Partial => {
                        prepared.blocked = true;
                        prepared.reconcile = true;
                        prepared.attempted = true;
                        self.notice = Some("Only part of the retained batch is recorded. Original IDs are kept. Copy the batch for inspection, then review recovery again.".into());
                    }
                }
                let mut effects = vec![Effect::PersistCache];
                effects.extend(self.confirm_prepared());
                effects
            }
            (Pending::Resolve(index), Response::Resolved(resolved)) => {
                self.resolved(index, resolved)
            }
            (Pending::Apply(_), Response::Applied(receipt)) => {
                let task = self.prepared.take().map(|p| p.task);
                self.link = None;
                self.verifying = false;
                if let Some(task) = task {
                    self.selected = Some(task);
                }
                self.detail = None;
                self.evidence = None;
                self.source_at = 0;
                self.notice = Some(
                    if receipt.recording_degraded {
                        "Local change committed; activity recording is degraded."
                    } else {
                        "Local work saved."
                    }
                    .into(),
                );
                let mut effects = vec![
                    Effect::PersistCache,
                    Effect::Show(Subject::new(
                        "work-item",
                        self.selected.as_ref().map_or("", ItemId::as_str),
                        "Work item",
                    )),
                ];
                effects.extend(self.read_items(true));
                effects
            }
            _ => self.error_notice(
                "The local result did not match the current request. Refresh to inspect it.",
            ),
        }
    }
}

impl Plugin for WorkbenchPlugin {
    fn id(&self) -> &'static str {
        "workbench"
    }
    fn name(&self) -> &'static str {
        "Work"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "work",
            name: "work items",
            key: 'I',
        }]
    }
    fn owns(&self) -> Vec<Owned> {
        vec![Owned::page("work-item", "local work items")]
    }
    fn show(&mut self, subject: &Subject) -> Showing {
        if subject.kind != "work-item" {
            return Showing::Cannot;
        }
        let Ok(item) = ItemId::parse(&subject.id) else {
            return Showing::Cannot;
        };
        let mut effects = vec![Effect::Focus(Pane::Middle)];
        if self.selected.as_ref() != Some(&item)
            || (self.detail.is_none() && self.pending.is_none())
        {
            if matches!(self.pending, Some((_, Pending::Apply(_)))) {
                return Showing::Done(
                    self.notice("Finish the current local save before showing another item."),
                );
            }
            self.selected = Some(item);
            self.detail = None;
            self.evidence = None;
            self.source_at = 0;
            effects.extend(self.read_items(true));
        }
        Showing::Page {
            screen: "work",
            effects,
        }
    }

    fn demo(&mut self) {
        #[cfg(any(test, feature = "demo"))]
        {
            *self = demo::sample();
        }
    }

    fn adds(&self) -> Vec<Adds> {
        vec![Adds::new("*", "explicit local work-item links")]
    }
    fn contribute(&self, point: Point, subject: &Subject) -> Option<Contribution> {
        if point != Point::Actions
            || !matches!(
                subject.kind.as_str(),
                "ticket"
                    | "pull-request"
                    | "repository"
                    | "workflow-run"
                    | "checkout"
                    | "repo"
                    | "merge-request"
                    | "pipeline"
                    | "job"
            )
        {
            return None;
        }
        let mut action = Action::new('a', "add to work item…", "work.link").adds().group(radar_core::view::ActionGroup::Ask);
        action.key = None;
        Some(Contribution::Actions(vec![action]))
    }
    fn act_with_related(
        &mut self,
        subject: &Subject,
        action: &str,
        related: &[Subject],
    ) -> Vec<Effect> {
        if action != "work.link" {
            return vec![];
        }
        if self
            .prepared
            .as_ref()
            .is_some_and(|p| !p.attempted && !p.reconcile && !p.blocked)
        {
            self.prepared = None;
        }
        if self.pending.is_some() || self.prepared.is_some() {
            return self.notice("Finish or retry the current local change first.");
        }
        self.link = None;
        self.origin = Some(subject.clone());
        self.filter = None;
        self.text_filter.clear();
        self.items = None;
        self.request = self.request.wrapping_add(1);
        self.capture = Some(self.request);
        let mut subjects = vec![subject.clone()];
        subjects.extend(related.iter().cloned());
        vec![Effect::ResolveSources {
            id: self.request,
            subjects,
        }]
    }
    fn act(&mut self, subject: &Subject, action: &str) -> Vec<Effect> {
        self.act_with_related(subject, action, &subject.related)
    }
    fn tick(&mut self, _: i64) -> Vec<Effect> {
        if self.started {
            return vec![];
        }
        self.started = true;
        self.read_items(true)
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        let msg = match msg.downcast::<WorkReply>() {
            Ok(reply) => return self.received(*reply),
            Err(msg) => msg,
        };
        if let Ok(sources) = msg.downcast::<ResolvedSources>() {
            return self.captured(*sources);
        }
        vec![]
    }
    fn snapshot(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "version": 1, "selected": self.selected,
            "source_draft": self.saved_link(), "pending": self.prepared.as_ref().map(|p| serde_json::json!({"batch":p.batch,"task":p.task,"summary":p.summary,"attempted":p.attempted,"reconcile":p.reconcile,"blocked":p.blocked})) }))
    }
    fn restore(&mut self, value: serde_json::Value) {
        if value["version"] != 1 {
            return;
        }
        self.selected = serde_json::from_value(value["selected"].clone()).ok();
        let pending = &value["pending"];
        if let (Ok(batch), Ok(task), Ok(summary)) = (
            serde_json::from_value::<Batch>(pending["batch"].clone()),
            serde_json::from_value::<ItemId>(pending["task"].clone()),
            serde_json::from_value::<Vec<String>>(pending["summary"].clone()),
        ) && batch.changes.len() <= 16
            && summary.len() <= 32
            && summary.iter().all(|s| s.len() <= 4096)
        {
            let attempted = pending["attempted"].as_bool().unwrap_or(true);
            self.link = self.restore_link(&value["source_draft"], &task, &batch);
            self.prepared = Some(Prepared {
                batch,
                task,
                summary,
                attempted,
                reconcile: true,
                restored: true,
                blocked: pending["blocked"].as_bool().unwrap_or(false),
            });
            self.notice =
                Some("An unfinished local change was retained. Review it before retrying.".into());
        }
    }
    fn busy(&self, _: &str) -> bool {
        self.pending.is_some() || self.capture.is_some()
    }
    fn top_right(&self, _: &str) -> Option<String> {
        Some("local annotations · observations only".into())
    }
    fn panes(&self, _: &str) -> Vec<Pane> {
        vec![Pane::Nav, Pane::Middle, Pane::Detail]
    }
    fn view(&self, scope: &str, cx: &ViewCx<'_>) -> Option<ScreenView> {
        (scope == "work").then(|| screen::view(self, cx))
    }
    fn intent(&mut self, scope: &str, pane: Pane, intent: &ViewIntent) -> Outcome {
        if scope != "work" {
            return Outcome::Ignored;
        }
        self.handle_intent(pane, intent)
    }
    fn menu_key_context(&self, scope: &str, pane: Pane, action: &str) -> Option<String> {
        if scope != "work" {
            return None;
        }
        match action {
            "t" | "v" | "work.group" | "work.evidence" => {
                self.selected.as_ref().map(|id| id.as_str().into())
            }
            "o" | "c" | "⏎" | "work.open-source" | "work.copy-source" if pane == Pane::Detail => {
                self.sources()
                    .get(self.source_at)
                    .map(|source| screen::source_target(self, source))
            }
            _ => None,
        }
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        self.menu(menu, value)
    }
    fn typing(&self, menu: &str, value: &str) -> Typing {
        if menu == "work.new" || menu.starts_with("work.new-group:") {
            return Typing {
                verdict: match radar_core::journal::work_map::SemanticLabel::new(value.trim()) {
                    Ok(_) => Some(Verdict::Good("Local authored label".into())),
                    Err(_) => Some(Verdict::Bad(
                        "Use a short label without controls or credentials.".into(),
                    )),
                },
                ..Typing::default()
            };
        }
        Typing::default()
    }
}
