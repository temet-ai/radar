use super::*;
use radar_core::compose::Parts;
use radar_core::headless::Headless;
use radar_core::ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Position,
};
use radar_core::view::{Node, PaneId, Target};

fn view(p: &WorkbenchPlugin, pane: Pane) -> ScreenView {
    screen::view(
        p,
        &ViewCx {
            now: 1000,
            focus: pane.into(),
            width: 160,
            height: 40,
            parts: Parts::none(),
            icon_lead: 0,
        },
    )
}

use radar_core::journal::work_map::{ResolutionState, Selection};
use radar_core::journal::{
    ActionDescriptor, Journal, Origin, SessionId, SubjectRole, SubjectSnapshot,
};
use radar_core::writes::Mode;
use std::collections::VecDeque;

struct Fixture {
    p: WorkbenchPlugin,
    journal: Journal,
    session: SessionId,
    path: std::path::PathBuf,
    action: ActionId,
    mode: Mode,
    menu: Option<Menu>,
    cached: Option<serde_json::Value>,
    owner_ready: bool,
    lose_apply_reply: bool,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir()
            .join(format!("radar-work-view-{}", ActionId::new().as_str()))
            .join("journal.sqlite3");
        let mut journal = Journal::open(&path).unwrap();
        let session = journal.start_session().unwrap();
        let action = ActionId::new();
        journal
            .begin_with_id_at(
                &session,
                &action,
                ActionDescriptor::new("source.selected", Origin::Host),
                None,
                1000,
            )
            .unwrap();
        let p = WorkbenchPlugin {
            seed: "10000000-0000-4000-8000-000000000000".into(),
            ..Default::default()
        };
        Self {
            p,
            journal,
            session,
            path,
            action,
            mode: Mode::Edit,
            menu: None,
            cached: None,
            owner_ready: true,
            lose_apply_reply: false,
        }
    }
    fn pump(&mut self, effects: Vec<Effect>) {
        let mut queue: VecDeque<_> = effects.into();
        let mut steps = 0;
        while let Some(effect) = queue.pop_front() {
            steps += 1;
            assert!(steps < 100);
            match effect {
                Effect::Work { id, request } => {
                    let lose_reply =
                        request.is_mutation() && std::mem::take(&mut self.lose_apply_reply);
                    let result = work_api::test_support::execute(
                        &mut self.journal,
                        &self.session,
                        *request,
                        self.mode,
                        1000,
                    );
                    let result = if lose_reply {
                        assert!(matches!(result, Ok(Response::Applied(_))));
                        Err(work_api::Error::OutcomeUnknown)
                    } else {
                        result
                    };
                    queue.extend(self.p.on_msg(Box::new(WorkReply { id, result })));
                }
                Effect::ResolveSources { id, subjects } => {
                    let omitted = if self.owner_ready { 0 } else { subjects.len() };
                    let sources = if self.owner_ready {
                        subjects
                            .into_iter()
                            .filter_map(|subject| {
                                key_for(&subject).map(|key| ResolvedSource { subject, key })
                            })
                            .collect()
                    } else {
                        vec![]
                    };
                    queue.extend(self.p.on_msg(Box::new(ResolvedSources {
                        id,
                        action_id: Some(self.action.clone()),
                        sources,
                        omitted,
                    })));
                }
                Effect::Menu(menu) => self.menu = Some(menu),
                Effect::PersistCache => self.cached = self.p.snapshot(),
                Effect::Show(subject) => {
                    if let Showing::Page { effects, .. } = self.p.show(&subject) {
                        queue.extend(effects)
                    }
                }
                Effect::Focus(_) | Effect::Notify(_) => {}
                other => panic!("unexpected effect {other:?}"),
            }
        }
    }
    fn start(&mut self) {
        let fx = self.p.tick(1000);
        self.pump(fx);
    }
    fn menu(&mut self, id: &str, value: &str) {
        let id = if matches!(id, "work.group" | "work.new-group")
            && self.p.prepared.is_none()
            && self.p.selected.is_some()
            && self.p.pending.is_none()
            && self.p.capture.is_none()
        {
            let effects = self.p.group_menu();
            self.pump(effects);
            let menu = self.menu.as_ref().unwrap().id.clone();
            if id == "work.new-group" {
                let effects = self.p.on_menu(&menu, "new");
                self.pump(effects);
                self.menu.as_ref().unwrap().id.clone()
            } else {
                menu
            }
        } else {
            // Recovery tests intentionally replay malformed/stale IDs. Keep
            // those deliveries direct instead of opening a new valid menu.
            id.into()
        };
        let fx = self.p.on_menu(&id, value);
        self.pump(fx);
    }
    fn create(&mut self, title: &str) -> ItemId {
        self.menu("work.new", title);
        let task = self.p.prepared.as_ref().unwrap().task.clone();
        self.menu("work.confirm", "yes");
        assert!(self.p.error.is_none(), "{:?}", self.p.error);
        task
    }
    fn link_new(&mut self, title: &str, sources: Vec<Subject>) -> ItemId {
        let mut descriptor = ActionDescriptor::new("source.selected", Origin::Host);
        descriptor.subjects = sources
            .iter()
            .map(|s| SubjectSnapshot::new(key_for(s).unwrap(), SubjectRole::Context))
            .collect();
        self.action = ActionId::new();
        self.journal
            .begin_with_id_at(&self.session, &self.action, descriptor, None, 1000)
            .unwrap();
        let fx = self
            .p
            .act_with_related(&sources[0], "work.link", &sources[1..]);
        self.pump(fx);
        self.menu("work.choose", "new");
        self.menu("work.new", title);
        for i in 1..sources.len() {
            self.menu("work.sources", &format!("toggle:{i}"));
        }
        self.menu("work.sources", "review");
        let task = self.p.prepared.as_ref().unwrap().task.clone();
        self.menu("work.confirm", "yes");
        assert!(self.p.error.is_none(), "{:?}", self.p.error);
        task
    }
    fn into_plugin(mut self) -> WorkbenchPlugin {
        std::mem::take(&mut self.p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}
fn source(kind: &str, id: &str) -> Subject {
    Subject::new(kind, id, format!("Selected {kind}"))
        .with("provider", "forge")
        .with("instance", "https://forge.example/")
}
fn key_for(s: &Subject) -> Option<SubjectKey> {
    SubjectKey::new(s.field("provider")?, s.field("instance")?, &s.kind, &s.id).ok()
}

#[test]
fn create_link_group_and_restart_use_the_real_audited_engine() {
    let mut f = Fixture::new();
    f.start();
    let sources = vec![
        source("pull-request", "acme/api/7"),
        source("ticket", "ABC-12"),
        source("checkout", "/tmp/acme-api"),
        source("workflow-run", "acme/api/42/1"),
    ];
    let task = f.link_new("Ship the selected change", sources.clone());
    assert_eq!(f.p.item_title(&task), "Ship the selected change");
    assert_eq!(f.p.sources().len(), 4);
    assert!(f.p.sources().iter().all(|(id, _, basis, _, _)| id != &task
        && *basis == radar_core::journal::work_map::Basis::UserAsserted));
    f.menu("work.new-group", "release candidate");
    f.menu("work.confirm", "yes");
    assert!(
        model::labels_for(&task, &f.p.pages())
            .iter()
            .any(|l| l.text == "release candidate" && l.namespace == model::GROUP_NAMESPACE)
    );
    let saved = f.p.snapshot().unwrap();
    let mut restarted = WorkbenchPlugin::default();
    restarted.restore(saved);
    f.p = restarted;
    f.start();
    assert_eq!(f.p.selected, Some(task.clone()));
    assert_eq!(f.p.item_title(&task), "Ship the selected change");
    assert_eq!(f.p.sources().len(), 4);
    let key = key_for(&sources[0]).unwrap();
    let resolution = f.journal.resolve_subject(&key, &f.action, None).unwrap();
    assert_eq!(resolution.state, ResolutionState::Resolved);
    let source_item = resolution.items[0].clone();
    assert_ne!(source_item, task);
    let evidence = f
        .journal
        .handoff(
            &Selection {
                items: vec![task],
                actions: vec![],
            },
            Default::default(),
            None,
        )
        .unwrap();
    assert_eq!(evidence.authority, "observations_only");
    assert!(evidence.completeness_not_guaranteed);
}

#[test]
fn existing_source_identity_is_reused_for_a_second_task() {
    let mut f = Fixture::new();
    f.start();
    let origin = source("ticket", "ABC-12");
    let first = f.link_new("First task", vec![origin.clone()]);
    let source_id = f.p.sources()[0].0.clone();
    let second = f.link_new("Second task", vec![origin]);
    assert_ne!(first, second);
    assert_eq!(f.p.sources()[0].0, source_id);
    assert!(f.p.prepared.is_none());
}

#[test]
fn cancel_read_only_and_stale_source_cannot_mutate() {
    let mut f = Fixture::new();
    f.start();
    f.menu("work.new", "Cancelled task");
    f.menu("work.confirm", "no");
    assert!(f.p.items.as_ref().unwrap().page.items.is_empty());
    f.mode = Mode::ReadOnly;
    f.menu("work.new", "Blocked task");
    f.menu("work.confirm", "yes");
    assert_eq!(f.p.error, Some(work_api::Error::PermissionDenied));
    assert!(f.p.prepared.is_some());
    f.mode = Mode::Edit;
    f.menu("work.confirm", "no");
    let s = source("ticket", "ABC-12");
    let fx = f.p.act(&s, "work.link");
    f.pump(fx);
    f.menu("work.choose", "new");
    f.menu("work.new", "Stale source");
    f.menu("work.sources", "review");
    f.owner_ready = false;
    f.menu("work.confirm", "yes");
    assert!(f.p.prepared.is_none());
    assert!(f.p.notice.as_ref().unwrap().contains("changed"));
    assert!(
        f.journal
            .query_items(&ItemFilter::default(), 100, None)
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn uncertain_retry_keeps_identical_ids_and_does_not_duplicate_mutations() {
    let mut f = Fixture::new();
    f.start();
    f.menu("work.new", "Review retry");
    let prepared = f.p.prepared.clone().unwrap();
    let response = work_api::test_support::execute(
        &mut f.journal,
        &f.session,
        Request::Apply(prepared.batch.clone()),
        Mode::Edit,
        1000,
    )
    .unwrap();
    assert!(matches!(response, Response::Applied(_)));
    // The writer committed, but the UI lost the reply. A restart retains intent.
    let saved = f.p.snapshot().unwrap();
    let mut restored = WorkbenchPlugin::default();
    restored.restore(saved);
    f.p = restored;
    assert_eq!(
        serde_json::to_value(&f.p.prepared.as_ref().unwrap().batch).unwrap(),
        serde_json::to_value(&prepared.batch).unwrap()
    );
    f.menu("work.confirm", "yes"); // Inspect the restored identities.
    f.menu("work.confirm", "yes");
    assert!(f.p.prepared.is_none());
    assert_eq!(
        f.journal
            .query_items(&ItemFilter::default(), 100, None)
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(f.p.item_title(&prepared.task), "Review retry");
}

#[test]
fn delayed_reads_and_semantic_requests_do_not_retarget_selection() {
    let mut f = Fixture::new();
    f.start();
    let first = f.create("One");
    let second = f.create("Two");
    let request = ViewIntent::Select {
        target: Target::new(screen::item_target(&f.p, &first)),
    };
    let fx = f.p.select_item(second.clone());
    f.pump(fx);
    f.p.text_filter = "Two".into();
    assert!(matches!(
        f.p.intent("work", Pane::Middle, &request),
        Outcome::Ignored
    ));
    let response = work_api::test_support::execute(
        &mut f.journal,
        &f.session,
        Request::Detail(work_api::DetailRequest {
            item_id: first,
            snapshot: None,
        }),
        Mode::ReadOnly,
        1000,
    );
    let fx = f.p.on_msg(Box::new(WorkReply {
        id: 0,
        result: response,
    }));
    assert!(fx.is_empty());
    assert_eq!(f.p.selected, Some(second));
}

#[test]
fn new_task_after_first_page_remains_selected_and_named() {
    let mut f = Fixture::new();
    f.start();
    for i in 0..18 {
        f.create(&format!("Task {i}"));
    }
    let selected = f.p.selected.clone().unwrap();
    assert_eq!(f.p.item_title(&selected), "Task 17");
    assert!(f.p.visible_items().iter().any(|i| i.item_id == selected));
    assert!(f.p.items.as_ref().unwrap().page.next.is_some());
}

#[test]
fn later_item_page_group_is_discoverable_and_reusable_after_restart() {
    use radar_core::journal::work_map::Mutation;
    let mut f = Fixture::new();
    f.start();
    let first = f.create("First task");
    for i in 1..18 {
        f.create(&format!("Task {i}"));
    }
    let later = f.p.selected.clone().unwrap();
    assert!(
        !f.p.items
            .as_ref()
            .unwrap()
            .page
            .items
            .iter()
            .any(|i| i.item_id == later)
    );
    f.menu("work.new-group", "Later-page group");
    let group =
        f.p.prepared
            .as_ref()
            .unwrap()
            .batch
            .changes
            .iter()
            .find_map(|change| match &change.mutation {
                Mutation::CreateTag {
                    tag_id, namespace, ..
                } if namespace.as_str() == model::GROUP_NAMESPACE => Some(tag_id.clone()),
                _ => None,
            })
            .unwrap();
    f.menu("work.confirm", "yes");
    f.p.selected = Some(first.clone());
    let saved = f.p.snapshot().unwrap();
    let mut restarted = WorkbenchPlugin::default();
    restarted.restore(saved);
    f.p = restarted;
    f.start();

    assert_eq!(f.p.page, 0, "Discovery must not traverse item pages");
    assert_eq!(f.p.selected.as_ref(), Some(&first));
    assert!(
        f.p.groups.iter().any(|label| label.id == group),
        "A group only assigned beyond the first item page must survive restart discovery"
    );
    f.menu("work.group", group.as_str());
    assert!(f.p.prepared.as_ref().unwrap().batch.changes.iter().any(|change| {
        matches!(&change.mutation, Mutation::AssignTag {tag_id, target: radar_core::journal::work_map::TagTarget::Item(item)} if tag_id == &group && item == &first)
    }));
    f.menu("work.confirm", "yes");
    assert!(
        model::labels_for(&first, &f.p.pages())
            .iter()
            .any(|label| label.id == group)
    );
}

fn catalog_changes(f: &mut Fixture, mutations: Vec<radar_core::journal::work_map::Mutation>) {
    use radar_core::journal::work_map::ChangeId;
    let batch = Batch::new(
        mutations
            .into_iter()
            .map(|mutation| work_api::Intent::asserted(ChangeId::new(), mutation))
            .collect(),
    );
    assert!(matches!(
        work_api::test_support::execute(
            &mut f.journal,
            &f.session,
            Request::Apply(batch),
            Mode::Edit,
            1000,
        ),
        Ok(Response::Applied(_))
    ));
}

fn empty_catalog_group(f: &mut Fixture, index: usize, label: &str) -> TagId {
    use radar_core::journal::work_map::{Mutation, SemanticLabel};
    let id = TagId::parse(&format!("50000000-0000-4000-8000-{index:012x}")).unwrap();
    catalog_changes(
        f,
        vec![Mutation::CreateTag {
            tag_id: id.clone(),
            namespace: TagKey::new(model::GROUP_NAMESPACE).unwrap(),
            key: TagKey::new(id.as_str()).unwrap(),
            label: SemanticLabel::new(label).unwrap(),
        }],
    );
    id
}

fn catalog_intent(f: &mut Fixture, pane: Pane, intent: ViewIntent) {
    let Outcome::Handled(effects) = f.p.intent("work", pane, &intent) else {
        panic!("catalog intent not handled: {intent:?}")
    };
    f.pump(effects);
}

#[test]
fn catalog_pages_keep_item_selection_cutoff_and_empty_group_filter() {
    let mut f = Fixture::new();
    f.start();
    let task = f.create("Keep this selected task");
    let mut ids = Vec::new();
    for index in 0..18 {
        ids.push(empty_catalog_group(
            &mut f,
            index,
            &format!("Empty group {index}"),
        ));
    }
    let effects = f.p.read_items(true);
    f.pump(effects);
    let snapshot = f.p.snapshot.clone();
    let detail = serde_json::to_value(&f.p.detail.as_ref().unwrap().primary).unwrap();
    let items = serde_json::to_value(&f.p.items.as_ref().unwrap().page).unwrap();
    assert_eq!(f.p.groups.len(), 16);
    assert!(
        view(&f.p, Pane::Nav)
            .actions
            .iter()
            .any(|a| a.id == "work.next-groups")
    );
    catalog_intent(
        &mut f,
        Pane::Nav,
        ViewIntent::Invoke {
            action: "work.next-groups".into(),
        },
    );
    assert_eq!(f.p.group_page, 1);
    assert_eq!(f.p.groups.len(), 2);
    assert_eq!(f.p.page, 0);
    assert_eq!(f.p.selected.as_ref(), Some(&task));
    assert_eq!(f.p.snapshot, snapshot);
    assert_eq!(
        serde_json::to_value(&f.p.detail.as_ref().unwrap().primary).unwrap(),
        detail
    );
    assert_eq!(
        serde_json::to_value(&f.p.items.as_ref().unwrap().page).unwrap(),
        items
    );
    let target = screen::group_target(&f.p, Some(&ids[17]));
    catalog_intent(
        &mut f,
        Pane::Nav,
        ViewIntent::Select {
            target: Target::new(target),
        },
    );
    assert_eq!(f.p.filter.as_ref(), Some(&ids[17]));
    assert!(f.p.visible_items().is_empty());
    assert!(f.p.selected.is_none());
    assert_eq!(f.p.snapshot, snapshot);
    catalog_intent(
        &mut f,
        Pane::Nav,
        ViewIntent::Invoke {
            action: "work.previous-groups".into(),
        },
    );
    assert_eq!(f.p.filter.as_ref(), Some(&ids[17]));
    let nav = &view(&f.p, Pane::Nav).panes[0];
    let Node::List { rows, selected, .. } = &nav.body else {
        panic!()
    };
    assert_eq!(selected, &Some(rows.len() - 1));
    assert!(rows.last().unwrap().text.words().contains("Selected group"));
    assert!(f.p.visible_items().is_empty());
}

#[test]
fn group_menu_pages_and_reuses_empty_exact_identity_without_name_merging() {
    use radar_core::journal::work_map::{Mutation, TagTarget};
    let mut f = Fixture::new();
    f.start();
    let task = f.create("Task for empty group");
    let mut ids = Vec::new();
    for index in 0..18 {
        ids.push(empty_catalog_group(&mut f, index, "Same visible name"));
    }
    let effects = f.p.read_items(true);
    f.pump(effects);
    let effects = f.p.group_menu();
    f.pump(effects);
    let first_menu = f.menu.clone().unwrap();
    assert_eq!(
        first_menu
            .items
            .iter()
            .filter(|row| row.label == "Same visible name")
            .count(),
        16
    );
    f.menu(&first_menu.id, "next");
    let second_menu = f.menu.clone().unwrap();
    assert_ne!(first_menu.id, second_menu.id);
    assert_eq!(f.p.selected.as_ref(), Some(&task));
    f.menu(&second_menu.id, ids[17].as_str());
    let prepared = f.p.prepared.as_ref().unwrap();
    assert_eq!(prepared.batch.changes.len(), 1);
    assert!(
        matches!(&prepared.batch.changes[0].mutation, Mutation::AssignTag {
        tag_id, target: TagTarget::Item(item),
    } if tag_id == &ids[17] && item == &task)
    );
    f.menu("work.confirm", "yes");
    assert!(
        model::labels_for(&task, &f.p.pages())
            .iter()
            .any(|g| g.id == ids[17])
    );
}

#[test]
fn group_catalog_cutoff_changes_only_on_refresh() {
    let mut f = Fixture::new();
    for index in 0..18 {
        empty_catalog_group(&mut f, index, &format!("Group {index}"));
    }
    f.start();
    let old = f.p.snapshot.clone().unwrap();
    let late = empty_catalog_group(&mut f, 99, "Created after the snapshot");
    catalog_intent(
        &mut f,
        Pane::Nav,
        ViewIntent::Invoke {
            action: "work.next-groups".into(),
        },
    );
    assert_eq!(f.p.snapshot.as_ref(), Some(&old));
    assert!(!f.p.groups.iter().any(|g| g.id == late));
    let selected = f.p.groups[0].id.clone();
    let target = screen::group_target(&f.p, Some(&selected));
    catalog_intent(
        &mut f,
        Pane::Nav,
        ViewIntent::Select {
            target: Target::new(target),
        },
    );
    assert_eq!(f.p.snapshot.as_ref(), Some(&old));
    catalog_intent(
        &mut f,
        Pane::Nav,
        ViewIntent::Invoke {
            action: "work.refresh".into(),
        },
    );
    assert!(f.p.snapshot.as_ref().unwrap().map_sequence() > old.map_sequence());
    assert_eq!(f.p.filter.as_ref(), Some(&selected));
    assert_eq!(
        f.p.group_catalog.as_ref().unwrap().snapshot,
        f.p.items.as_ref().unwrap().page.snapshot
    );
    catalog_intent(
        &mut f,
        Pane::Nav,
        ViewIntent::Invoke {
            action: "work.next-groups".into(),
        },
    );
    assert!(f.p.groups.iter().any(|g| g.id == late));
}

#[test]
fn ambiguous_retracted_and_partial_catalog_labels_are_honest() {
    use radar_core::journal::work_map::{AssertionTarget, Mutation, SemanticLabel};
    let mut f = Fixture::new();
    f.start();
    f.create("Group label evidence");
    let id = empty_catalog_group(&mut f, 0, "Original label");
    catalog_changes(
        &mut f,
        vec![
            Mutation::RenameTag {
                tag_id: id.clone(),
                label: SemanticLabel::new("First assertion").unwrap(),
            },
            Mutation::RenameTag {
                tag_id: id.clone(),
                label: SemanticLabel::new("Second assertion").unwrap(),
            },
        ],
    );
    let effects = f.p.read_items(true);
    f.pump(effects);
    assert!(f.p.groups[0].ambiguous);
    assert_eq!(f.p.groups[0].text, "Original label");
    f.menu("work.group", id.as_str());
    assert!(f.p.prepared.is_none());
    let claim = f.p.group_catalog.as_ref().unwrap().tags[0]
        .renames
        .iter()
        .find(|claim| claim.label.as_str() == "Second assertion")
        .unwrap()
        .change_id
        .clone();
    catalog_changes(
        &mut f,
        vec![Mutation::Retract {
            claim_id: claim,
            expected: AssertionTarget::LabelAssertion { tag_id: id.clone() },
        }],
    );
    let effects = f.p.read_items(true);
    f.pump(effects);
    assert!(!f.p.groups[0].ambiguous);
    assert_eq!(f.p.groups[0].text, "First assertion");
    let mut partial = f.p.group_catalog.as_ref().unwrap().tags[0].clone();
    partial.labels_truncated = true;
    let label = model::catalog_label(&partial);
    assert!(
        label.ambiguous,
        "A bounded agreeing sample is not consensus"
    );
    assert_eq!(label.text, "Original label");
}

#[test]
fn stale_group_menu_typed_menu_targets_and_replies_cannot_apply() {
    let mut f = Fixture::new();
    f.start();
    let first = f.create("First selected task");
    let second = f.create("Second selected task");
    let mut ids = Vec::new();
    for index in 0..18 {
        ids.push(empty_catalog_group(
            &mut f,
            index,
            &format!("Group {index}"),
        ));
    }
    let effects = f.p.read_items(true);
    f.pump(effects);
    let effects = f.p.group_menu();
    f.pump(effects);
    let old_menu = f.menu.clone().unwrap();
    let old_target = screen::group_target(&f.p, Some(&ids[0]));
    f.menu(&old_menu.id, "new");
    let old_typed = f.menu.clone().unwrap();
    catalog_intent(
        &mut f,
        Pane::Nav,
        ViewIntent::Invoke {
            action: "work.next-groups".into(),
        },
    );
    f.menu(&old_menu.id, ids[0].as_str());
    f.menu(&old_typed.id, "Stale new group");
    assert!(f.p.prepared.is_none());
    assert!(matches!(
        f.p.intent(
            "work",
            Pane::Nav,
            &ViewIntent::Select {
                target: Target::new(old_target),
            }
        ),
        Outcome::Ignored
    ));

    let effects = f.p.group_menu();
    f.pump(effects);
    let before_selection = f.menu.clone().unwrap();
    let target = screen::item_target(&f.p, &first);
    catalog_intent(
        &mut f,
        Pane::Middle,
        ViewIntent::Select {
            target: Target::new(target),
        },
    );
    f.menu(&before_selection.id, ids[17].as_str());
    assert!(f.p.prepared.is_none());

    let effects = f.p.read_groups(AfterGroups::Stay);
    let Effect::Work { id, request } = effects.into_iter().next().unwrap() else {
        panic!()
    };
    let result =
        work_api::test_support::execute(&mut f.journal, &f.session, *request, Mode::Edit, 1000);
    let Showing::Page { effects, .. } =
        f.p.show(&Subject::new("work-item", second.as_str(), "Second task"))
    else {
        panic!()
    };
    f.pump(effects);
    let current = serde_json::to_value(&f.p.group_catalog).unwrap();
    assert!(f.p.on_msg(Box::new(WorkReply { id, result })).is_empty());
    assert_eq!(serde_json::to_value(&f.p.group_catalog).unwrap(), current);
    assert_eq!(f.p.selected.as_ref(), Some(&second));
}

#[test]
fn catalog_failure_and_pending_save_preserve_current_item_and_recovery() {
    let mut f = Fixture::new();
    f.start();
    let task = f.create("Retain selected task");
    for index in 0..18 {
        empty_catalog_group(&mut f, index, &format!("Group {index}"));
    }
    let effects = f.p.read_items(true);
    f.pump(effects);
    let Outcome::Handled(effects) = f.p.intent(
        "work",
        Pane::Nav,
        &ViewIntent::Invoke {
            action: "work.next-groups".into(),
        },
    ) else {
        panic!()
    };
    let Effect::Work { id, .. } = effects.into_iter().next().unwrap() else {
        panic!()
    };
    let effects = f.p.on_msg(Box::new(WorkReply {
        id,
        result: Err(work_api::Error::Unavailable),
    }));
    f.pump(effects);
    assert_eq!(f.p.group_error, Some(work_api::Error::Unavailable));
    assert_eq!(f.p.selected.as_ref(), Some(&task));
    assert!(f.p.detail.is_some());
    assert!(
        view(&f.p, Pane::Nav).panes[0]
            .head
            .as_ref()
            .unwrap()
            .words()
            .contains("unavailable")
    );
    catalog_intent(
        &mut f,
        Pane::Nav,
        ViewIntent::Invoke {
            action: "work.previous-groups".into(),
        },
    );
    assert!(f.p.group_error.is_none());
    assert_eq!(f.p.groups.len(), 16);
    let group = f.p.groups[0].id.clone();
    f.menu("work.group", group.as_str());
    let batch = serde_json::to_value(&f.p.prepared.as_ref().unwrap().batch).unwrap();
    let effects = f.p.on_menu("work.confirm", "yes");
    assert!(matches!(f.p.pending, Some((_, Pending::Apply(_)))));
    assert!(matches!(
        f.p.intent(
            "work",
            Pane::Nav,
            &ViewIntent::Invoke {
                action: "work.next-groups".into(),
            }
        ),
        Outcome::Ignored
    ));
    assert!(
        f.p.on_msg(Box::new(WorkReply {
            id,
            result: Err(work_api::Error::Unavailable)
        }))
        .is_empty()
    );
    assert_eq!(
        serde_json::to_value(&f.p.prepared.as_ref().unwrap().batch).unwrap(),
        batch
    );
    assert!(matches!(f.p.pending, Some((_, Pending::Apply(_)))));
    f.pump(effects);
    assert!(f.p.prepared.is_none());
    assert_eq!(f.p.selected.as_ref(), Some(&task));
}

#[test]
fn unknown_group_catalog_is_never_presented_as_empty() {
    let mut p = WorkbenchPlugin::default();
    let heading = |p: &WorkbenchPlugin| view(p, Pane::Nav).panes[0].head.as_ref().unwrap().words();
    assert_eq!(heading(&p), "Groups not loaded");
    let effects = p.tick(1000);
    assert_eq!(heading(&p), "Loading groups…");
    let Effect::Work { id, .. } = effects.into_iter().next().unwrap() else {
        panic!()
    };
    p.on_msg(Box::new(WorkReply {
        id,
        result: Err(work_api::Error::Unavailable),
    }));
    assert_eq!(heading(&p), "Groups not loaded");
    assert!(p.group_catalog.is_none());
    let mut empty = Fixture::new();
    empty.start();
    assert_eq!(heading(&empty.p), "No groups on page");
}

#[test]
fn default_view_and_exact_source_actions_use_shared_semantics() {
    let mut f = Fixture::new();
    f.start();
    f.link_new("Source navigation", vec![source("ticket", "ABC-12")]);
    let source = f.p.sources()[0].clone();
    let key = source.1.clone();
    let request = ViewIntent::Activate {
        target: Target::new(screen::source_target(&f.p, &source)),
    };
    let Outcome::Handled(effects) = f.p.intent("work", Pane::Detail, &request) else {
        panic!()
    };
    assert!(matches!(&effects[0], Effect::ShowSource(actual) if actual == &key));
    let text = serde_json::to_string(&view(&f.p, Pane::Detail)).unwrap();
    assert!(text.contains("authored"));
    assert!(text.contains("permission or completion"));
}

fn sample() -> WorkbenchPlugin {
    let mut f = Fixture::new();
    f.start();
    f.link_new(
        "Ship explicit work-item navigation",
        vec![
            source("pull-request", "acme/api/7"),
            source("ticket", "ABC-12"),
            source("checkout", "/home/user/projects/api"),
            source("workflow-run", "acme/api/42/1"),
        ],
    );
    f.menu("work.new-group", "release candidate");
    f.menu("work.confirm", "yes");
    f.into_plugin()
}
fn catalog_sample() -> Fixture {
    let mut f = Fixture::new();
    f.start();
    f.create("Keep selected while browsing groups");
    for index in 0..18 {
        empty_catalog_group(&mut f, index, &format!("Unassigned group {index}"));
    }
    let effects = f.p.read_items(true);
    f.pump(effects);
    f
}
fn host(p: WorkbenchPlugin) -> radar_core::Host {
    let mut host = radar_core::Host::new(vec![Box::new(p)], radar_core::plugin::DEMO_NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host
}
#[test]
fn rendered_work_states_80_and_160_columns() {
    let mut cases = vec![("items", host(sample()))];
    let mut catalog = catalog_sample();
    catalog.p.groups[1].ambiguous = true;
    let mut catalog = host(catalog.into_plugin());
    catalog.focus = Pane::Nav;
    cases.push(("group_catalog", catalog));
    let mut filtered = catalog_sample();
    catalog_intent(
        &mut filtered,
        Pane::Nav,
        ViewIntent::Invoke {
            action: "work.next-groups".into(),
        },
    );
    let id = filtered.p.groups[0].id.clone();
    let target = screen::group_target(&filtered.p, Some(&id));
    catalog_intent(
        &mut filtered,
        Pane::Nav,
        ViewIntent::Select {
            target: Target::new(target),
        },
    );
    catalog_intent(
        &mut filtered,
        Pane::Nav,
        ViewIntent::Invoke {
            action: "work.previous-groups".into(),
        },
    );
    let mut filtered = host(filtered.into_plugin());
    filtered.focus = Pane::Nav;
    cases.push(("selected_empty_group", filtered));
    let mut failed_catalog = sample();
    failed_catalog.group_catalog = None;
    failed_catalog.groups.clear();
    failed_catalog.group_error = Some(work_api::Error::Unavailable);
    let mut failed_catalog = host(failed_catalog);
    failed_catalog.focus = Pane::Nav;
    cases.push(("group_catalog_failed", failed_catalog));
    let mut group_chooser = catalog_sample();
    let effects = group_chooser.p.group_menu();
    group_chooser.pump(effects);
    let group_menu = group_chooser.menu.clone().unwrap();
    let mut group_chooser = host(group_chooser.into_plugin());
    group_chooser.menu = Some(radar_core::host::OpenMenu::open(
        radar_core::host::Owner::Plugin(0),
        group_menu,
    ));
    cases.push(("group_chooser", group_chooser));
    let mut detail = host(sample());
    detail.focus = Pane::Detail;
    cases.push(("context", detail));
    let mut empty = Fixture::new();
    empty.start();
    cases.push(("empty", host(empty.into_plugin())));
    let unavailable = WorkbenchPlugin {
        error: Some(work_api::Error::Disabled),
        ..Default::default()
    };
    cases.push(("unavailable", host(unavailable)));
    let mut loading = WorkbenchPlugin::default();
    loading.read_items(true);
    cases.push(("loading", host(loading)));
    let mut partial = sample();
    partial.detail.as_mut().unwrap().expansion_truncated = true;
    partial
        .detail
        .as_mut()
        .unwrap()
        .primary
        .known_recording_gaps = true;
    let mut partial = host(partial);
    partial.focus = Pane::Detail;
    cases.push(("partial", partial));
    let mut menu = host(sample());
    menu.update(radar_core::host::Event::Key(KeyEvent::new(
        KeyCode::Char('x'),
        KeyModifiers::NONE,
    )));
    cases.push(("actions", menu));
    let mut chooser = Fixture::new();
    chooser.start();
    let selected = source("pull-request", "acme/api/7");
    let fx = chooser.p.act_with_related(
        &selected,
        "work.link",
        &[
            source("ticket", "ABC-12"),
            source("checkout", "/home/user/projects/api"),
            source("workflow-run", "acme/api/42/1"),
        ],
    );
    chooser.pump(fx);
    chooser.menu("work.choose", "new");
    chooser.menu(
        "work.new",
        "A deliberately named task with an explicit local title",
    );
    let picker = chooser.menu.clone().unwrap();
    let mut picker_host = host(WorkbenchPlugin::default());
    picker_host.menu = Some(radar_core::host::OpenMenu::open(
        radar_core::host::Owner::Plugin(0),
        picker,
    ));
    cases.push(("choose_sources", picker_host));
    chooser.menu("work.sources", "toggle:1");
    chooser.menu("work.sources", "toggle:2");
    chooser.menu("work.sources", "toggle:3");
    chooser.menu("work.sources", "review");
    let confirm = chooser.menu.clone().unwrap();
    let mut confirm_host = host(WorkbenchPlugin::default());
    confirm_host.menu = Some(radar_core::host::OpenMenu::open(
        radar_core::host::Owner::Plugin(0),
        confirm,
    ));
    cases.push(("confirm_links", confirm_host));
    let mut history = Fixture::new();
    history.start();
    history.link_new(
        "Inspect original source evidence",
        vec![source("ticket", "ABC-12")],
    );
    history.menu("work.history", "related");
    let mut history_host = host(history.into_plugin());
    history_host.focus = Pane::Detail;
    cases.push(("evidence", history_host));
    let mut snapshots = Vec::new();
    for (name, host) in cases {
        for (w, h) in [(80, 24), (160, 40)] {
            let text = radar_core::shell::to_text(&host, w, h);
            if let Ok(dir) = std::env::var("RADAR_WORK_RENDER_DIR") {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(
                    format!("{dir}/{name}_{w}x{h}.svg"),
                    radar_core::shell::to_svg(&host, w, h),
                )
                .unwrap();
                std::fs::write(format!("{dir}/{name}_{w}x{h}.txt"), &text).unwrap();
            }
            snapshots.push((
                format!("work_{name}_{w}x{h}"),
                if name == "evidence" {
                    normalize_ids(&text)
                } else {
                    text
                },
            ));
        }
    }
    for (name, text) in snapshots {
        insta::assert_snapshot!(name, text);
    }
}

struct SourcePart;
impl Plugin for SourcePart {
    fn id(&self) -> &'static str {
        "forge"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "sources",
            name: "sources",
            key: 'E',
        }]
    }
    fn view(&self, screen: &str, _: &ViewCx<'_>) -> Option<ScreenView> {
        (screen == "sources").then(|| {
            ScreenView::new(
                "sources",
                vec![radar_core::view::PaneView::new(
                    PaneId::Middle,
                    "Source records",
                    Node::State {
                        state: radar_core::view::State::Empty,
                        text: "Selected ticket ABC-12".into(),
                    },
                )],
            )
        })
    }
    fn owns(&self) -> Vec<Owned> {
        vec![Owned::page("ticket", "source records")]
    }
    fn subject(&self, _: &str, _: Pane) -> Option<Subject> {
        Some(source("ticket", "ABC-12"))
    }
    fn journal_subject(&self, s: &Subject) -> Option<SubjectSnapshot> {
        (s.kind == "ticket")
            .then(|| key_for(s))
            .flatten()
            .map(|key| SubjectSnapshot::new(key, SubjectRole::Primary))
    }
    fn subject_from_journal(&self, key: &SubjectKey) -> Option<Subject> {
        let s = source("ticket", &key.id);
        (key_for(&s).as_ref() == Some(key)).then_some(s)
    }
    fn show(&mut self, s: &Subject) -> Showing {
        if self.journal_subject(s).is_some() {
            Showing::Page {
                screen: "sources",
                effects: vec![],
            }
        } else {
            Showing::Cannot
        }
    }
}
fn host_pump(host: &mut radar_core::Host, f: &mut Fixture, effects: Vec<radar_core::host::HostEffect>) {
    use radar_core::host::{Event, HostEffect, JournalRecord};
    let mut queue: VecDeque<_> = effects.into();
    let mut steps = 0;
    while let Some(effect) = queue.pop_front() {
        steps += 1;
        assert!(steps < 500);
        match effect {
            HostEffect::Journal(record) => match record {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                } => f
                    .journal
                    .begin_with_id_at(&f.session, &id, descriptor, parent.as_ref(), 1000)
                    .unwrap(),
                JournalRecord::Outcome {
                    id,
                    outcome,
                    metadata,
                } => {
                    f.journal.record_at(&id, outcome, metadata, 1000).unwrap();
                }
                JournalRecord::Gap { .. } => panic!("unexpected fixture gap"),
            },
            HostEffect::Work {
                plugin,
                id,
                request,
                mode,
            } => {
                let result = work_api::test_support::execute(
                    &mut f.journal,
                    &f.session,
                    *request,
                    mode,
                    1000,
                );
                queue.extend(host.update(Event::Msg {
                    plugin,
                    msg: Box::new(WorkReply { id, result }),
                }));
            }
            HostEffect::Tracked { effect, .. } => queue.push_front(*effect),
            HostEffect::WriteCache { .. } | HostEffect::Save(_) => {}
            _ => panic!("unexpected runner effect"),
        }
    }
}
fn host_key(host: &mut radar_core::Host, f: &mut Fixture, code: KeyCode) {
    radar_core::shell::to_text(host, 80, 24);
    let effects = host.update(radar_core::host::Event::Key(KeyEvent::new(
        code,
        KeyModifiers::NONE,
    )));
    host_pump(host, f, effects);
}

#[test]
fn real_host_menu_create_link_enter_context_and_cancel_flow() {
    use radar_core::host::{Event, View as HostView};
    let mut f = Fixture::new();
    let mut host = radar_core::Host::new(
        vec![Box::new(SourcePart), Box::new(WorkbenchPlugin::default())],
        1000,
    )
    .unwrap();
    host.load_config(Some("")).unwrap();
    host.enable_journal();
    host_key(&mut host, &mut f, KeyCode::Char('x'));
    let menu = host.menu.as_mut().unwrap();
    menu.row = menu
        .menu
        .items
        .iter()
        .position(|i| i.label.contains("add to work item"))
        .unwrap();
    host_key(&mut host, &mut f, KeyCode::Enter);
    assert_eq!(host.menu.as_ref().unwrap().menu.id, "work.choose");
    host_key(&mut host, &mut f, KeyCode::Enter);
    assert_eq!(host.menu.as_ref().unwrap().menu.id, "work.new");
    let effects = host.update(Event::Paste("Menu-created work".into()));
    host_pump(&mut host, &mut f, effects);
    host_key(&mut host, &mut f, KeyCode::Enter);
    assert_eq!(host.menu.as_ref().unwrap().menu.id, "work.sources");
    let menu = host.menu.as_mut().unwrap();
    menu.row = menu
        .menu
        .items
        .iter()
        .position(|i| i.value == "review")
        .unwrap();
    host_key(&mut host, &mut f, KeyCode::Enter);
    assert_eq!(host.menu.as_ref().unwrap().menu.id, "work.confirm");
    host_key(&mut host, &mut f, KeyCode::Char('y'));
    assert!(host.menu.is_none());
    assert!(matches!(host.view, HostView::Screen(_)));
    assert!(radar_core::shell::to_text(&host, 80, 24).contains("Menu-created work"));
    host_key(&mut host, &mut f, KeyCode::Enter);
    assert_eq!(host.focus, Pane::Detail);
    host_key(&mut host, &mut f, KeyCode::Char('o'));
    assert!(matches!(host.view, HostView::Screen(_)));
    assert_eq!(host.screen().unwrap().1.id, "sources");
    host_key(&mut host, &mut f, KeyCode::Char('I'));
    host_key(&mut host, &mut f, KeyCode::Char('n'));
    let effects = host.update(Event::Paste("Discard this preview".into()));
    host_pump(&mut host, &mut f, effects);
    host_key(&mut host, &mut f, KeyCode::Enter);
    host_key(&mut host, &mut f, KeyCode::Esc);
    assert!(host.menu.is_none());
    host_key(&mut host, &mut f, KeyCode::Char('n'));
    assert_eq!(host.menu.as_ref().unwrap().menu.id, "work.new");
    assert_eq!(
        f.journal
            .query_items(&ItemFilter::default(), 100, None)
            .unwrap()
            .items
            .len(),
        2
    );
}

fn normalize_ids(text: &str) -> String {
    let mut output = text.to_string();
    let mut ids = Vec::new();
    for (i, _) in text.char_indices() {
        if let Some(candidate) = text.get(i..i + 36)
            && candidate.as_bytes().get(8) == Some(&b'-')
            && ItemId::parse(candidate).is_ok()
        {
            ids.push(candidate.to_string());
        }
    }
    for id in ids {
        output = output.replace(&id, "00000000-0000-4000-8000-000000000000");
    }
    // The real SQLite engine supplies wall-clock recording times. Keep their
    // width but make the evidence pictures deterministic across test runs.
    for timestamp in text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|value| value.len() == 13)
    {
        output = output.replace(timestamp, "1700000000000");
    }
    output
}

fn row_at(host: &radar_core::Host, pane: Pane, target: &str) -> Position {
    radar_core::shell::to_text(host, 160, 40);
    host.hits
        .borrow()
        .iter()
        .find_map(|(rect, hit)| match hit {
            radar_core::host::Hit::Plugin {
                pane: hit_pane, id, ..
            } if *hit_pane == pane && id == target => Some(Position {
                x: rect.x,
                y: rect.y,
            }),
            _ => None,
        })
        .unwrap()
}
#[test]
fn mouse_select_double_click_and_context_menu_keep_exact_targets() {
    use radar_core::host::{Event, MouseKind};
    let mut f = Fixture::new();
    f.start();
    f.link_new("First visible task", vec![source("ticket", "ABC-1")]);
    f.link_new("Second visible task", vec![source("ticket", "ABC-2")]);
    let plugin = std::mem::take(&mut f.p);
    let mut host = radar_core::Host::new(vec![Box::new(SourcePart), Box::new(plugin)], 1000).unwrap();
    host.load_config(Some("")).unwrap();
    host.enable_journal();
    host_key(&mut host, &mut f, KeyCode::Char('I'));
    for at_ms in [1000, 1200] {
        let current = host.screen_view_in(1, "work", 160, 40).unwrap();
        let Node::List { rows, .. } = &current.pane(PaneId::Middle).unwrap().body else {
            panic!()
        };
        let target = rows
            .iter()
            .find(|row| row.text.words() == "First visible task")
            .unwrap()
            .target
            .0
            .clone();
        let at = row_at(&host, Pane::Middle, &target);
        let fx = host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: at.x,
            row: at.y,
            at_ms,
        });
        host_pump(&mut host, &mut f, fx);
    }
    assert_eq!(host.focus, Pane::Detail);
    let view = host.screen_view_in(1, "work", 160, 40).unwrap();
    let Node::Entries { rows, .. } = &view.pane(PaneId::Detail).unwrap().body else {
        panic!()
    };
    let target = rows[0].target.0.clone();
    let at = row_at(&host, Pane::Detail, &target);
    let fx = host.update(Event::Mouse {
        kind: MouseKind::Context,
        column: at.x,
        row: at.y,
        at_ms: 2000,
    });
    host_pump(&mut host, &mut f, fx);
    assert!(
        host.menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .any(|row| row.label.contains("open selected source"))
    );
    assert!(radar_core::shell::to_text(&host, 160, 40).contains("First visible task"));
}

#[test]
fn inferred_tag_creation_is_not_presented_as_an_authored_title() {
    use radar_core::journal::work_map::{Basis, HandoffRecord, Mutation};
    let mut f = Fixture::new();
    f.start();
    let task = f.create("Explicit local title");
    let mut page = f.p.detail.as_ref().unwrap().primary.clone();
    assert_eq!(model::title(&task, &[&page]), "Explicit local title");
    for record in &mut page.records {
        if let HandoffRecord::Map { change } = record
            && matches!(change.request.mutation, Mutation::CreateTag { .. })
        {
            change.request.provenance.basis = Basis::Inferred;
        }
    }
    assert!(model::labels_for(&task, &[&page]).is_empty());
    assert_ne!(model::title(&task, &[&page]), "Explicit local title");
}

#[test]
fn semantic_controls_cannot_replace_an_inflight_or_uncertain_mutation() {
    let mut f = Fixture::new();
    f.start();
    f.create("Existing task");
    assert!(
        view(&f.p, Pane::Middle)
            .actions
            .iter()
            .any(|action| action.id == "work.group")
    );
    let request = ViewIntent::Invoke {
        action: "work.group".into(),
    };
    let fx = f.p.on_menu("work.new", "Retained task");
    f.pump(fx);
    let batch = f.p.prepared.as_ref().unwrap().batch.clone();
    f.p.prepared.as_mut().unwrap().attempted = true;
    assert!(match f.p.intent("work", Pane::Middle, &request) {
        Outcome::Ignored => true,
        Outcome::Handled(effects) => effects.is_empty(),
    });
    assert_eq!(
        serde_json::to_value(&f.p.prepared.as_ref().unwrap().batch).unwrap(),
        serde_json::to_value(&batch).unwrap()
    );
    f.p.pending = Some((999, Pending::Apply(true)));
    assert!(matches!(
        f.p.intent("work", Pane::Middle, &request),
        Outcome::Ignored
    ));
    assert!(matches!(f.p.pending, Some((999, Pending::Apply(true)))));
}

#[test]
fn selecting_an_existing_task_adds_a_separate_repository_source() {
    let mut f = Fixture::new();
    f.start();
    let task = f.link_new("Existing task", vec![source("ticket", "ABC-12")]);
    let origin = source("repository", "acme/api");
    let mut descriptor = ActionDescriptor::new("source.selected", Origin::Host);
    descriptor.subjects.push(SubjectSnapshot::new(
        key_for(&origin).unwrap(),
        SubjectRole::Context,
    ));
    f.action = ActionId::new();
    f.journal
        .begin_with_id_at(&f.session, &f.action, descriptor, None, 1000)
        .unwrap();
    let fx = f.p.act(&origin, "work.link");
    f.pump(fx);
    f.menu("work.choose", task.as_str());
    f.menu("work.sources", "review");
    f.menu("work.confirm", "yes");
    assert_eq!(f.p.selected.as_ref(), Some(&task));
    assert_eq!(f.p.item_title(&task), "Existing task");
    let sources = f.p.sources();
    assert_eq!(sources.len(), 2);
    assert!(
        sources.iter().any(|(id, key, _, _, _)| id != &task
            && key.kind == "repository"
            && key.id == "acme/api")
    );
    assert_eq!(
        f.journal
            .query_items(&ItemFilter::default(), 100, None)
            .unwrap()
            .items
            .iter()
            .filter(|item| item.kind.as_str() == "task")
            .count(),
        1
    );
}

#[test]
fn linked_source_unknown_commit_replays_exact_ids_without_owner_revalidation() {
    for owner_ready in [true, false] {
        let mut f = Fixture::new();
        f.start();
        let fx = f.p.act(&source("ticket", "ABC-12"), "work.link");
        f.pump(fx);
        f.menu("work.choose", "new");
        f.menu("work.new", "Retained linked task");
        f.menu("work.sources", "review");
        let original = f.p.prepared.clone().unwrap();
        f.lose_apply_reply = true;
        f.menu("work.confirm", "yes");
        assert_eq!(f.p.error, Some(work_api::Error::OutcomeUnknown));
        assert!(f.p.prepared.as_ref().unwrap().attempted);
        assert!(f.p.link.is_some());
        let before = f
            .journal
            .query_items(&ItemFilter::default(), 100, None)
            .unwrap();
        f.owner_ready = owner_ready;
        // A read failure and a denied later retry cannot erase earlier uncertainty.
        f.p.pending = Some((999, Pending::Items));
        let fx = f.p.on_msg(Box::new(WorkReply {
            id: 999,
            result: Err(work_api::Error::Unavailable),
        }));
        f.pump(fx);
        assert!(f.p.prepared.as_ref().unwrap().attempted);
        f.mode = Mode::ReadOnly;
        f.menu("work.confirm", "yes");
        assert_eq!(f.p.error, Some(work_api::Error::PermissionDenied));
        assert!(f.p.prepared.as_ref().unwrap().attempted);
        f.menu("work.confirm", "no");
        assert_eq!(
            serde_json::to_value(&f.p.prepared.as_ref().unwrap().batch).unwrap(),
            serde_json::to_value(&original.batch).unwrap()
        );
        f.mode = Mode::Edit;
        f.menu("work.confirm", "yes");
        assert!(f.p.prepared.is_none());
        assert_eq!(f.p.selected.as_ref(), Some(&original.task));
        assert_eq!(f.p.sources().len(), 1);
        let after = f
            .journal
            .query_items(&ItemFilter::default(), 100, None)
            .unwrap();
        assert_eq!(
            before.snapshot.map_sequence(),
            after.snapshot.map_sequence()
        );
        assert_eq!(after.items.len(), 2);
    }
}

fn headless_send(headless: &mut Headless, fixture: &mut Fixture, intent: ViewIntent) {
    let effects = headless.send(intent);
    host_pump(headless.host_mut(), fixture, effects);
}

#[test]
fn terminal_and_headless_page_catalog_and_select_empty_group_identically() {
    use radar_core::host::Event;
    for (width, height) in [(80, 24), (160, 40)] {
        let (mut terminal_fixture, mut headless_fixture) = (catalog_sample(), catalog_sample());
        let mut terminal = host(std::mem::take(&mut terminal_fixture.p));
        terminal.focus = Pane::Nav;
        let mut headless =
            Headless::requiring_views(host(std::mem::take(&mut headless_fixture.p)), width, height)
                .unwrap();
        headless.host_mut().focus = Pane::Nav;
        let effects = terminal.update(Event::Key(KeyEvent::new(
            KeyCode::Char(']'),
            KeyModifiers::NONE,
        )));
        host_pump(&mut terminal, &mut terminal_fixture, effects);
        headless_send(
            &mut headless,
            &mut headless_fixture,
            ViewIntent::Invoke {
                action: "work.next-groups".into(),
            },
        );
        let terminal_view = terminal.screen_view_in(0, "work", width, height).unwrap();
        assert_eq!(
            normalize_ids(&serde_json::to_string(&terminal_view).unwrap()),
            normalize_ids(&serde_json::to_string(&headless.screen().unwrap()).unwrap())
        );
        let described = headless.screen().unwrap();
        let Node::List { rows, .. } = &described.pane(PaneId::Nav).unwrap().body else {
            panic!()
        };
        let target = rows[1].target.clone();
        let effects = terminal.update(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        host_pump(&mut terminal, &mut terminal_fixture, effects);
        headless_send(
            &mut headless,
            &mut headless_fixture,
            ViewIntent::Select { target },
        );
        let terminal_view = terminal.screen_view_in(0, "work", width, height).unwrap();
        let headless_view = headless.screen().unwrap();
        assert_eq!(
            normalize_ids(&serde_json::to_string(&terminal_view).unwrap()),
            normalize_ids(&serde_json::to_string(&headless_view).unwrap())
        );
        assert!(matches!(
            headless_view.pane(PaneId::Middle).unwrap().body,
            Node::State { .. }
        ));
    }
}

#[test]
fn headless_and_terminal_create_the_same_confirmed_local_task() {
    use radar_core::host::Event;
    let (mut terminal_fixture, mut headless_fixture) = (Fixture::new(), Fixture::new());
    let mut terminal = host(std::mem::take(&mut terminal_fixture.p));
    terminal.enable_journal();
    let mut headless =
        Headless::requiring_views(host(std::mem::take(&mut headless_fixture.p)), 160, 40)
            .unwrap_or_else(|gaps| panic!("missing views: {gaps:?}"));
    host_key(&mut terminal, &mut terminal_fixture, KeyCode::Char('n'));
    headless_send(
        &mut headless,
        &mut headless_fixture,
        ViewIntent::Invoke {
            action: "work.new".into(),
        },
    );
    assert_eq!(
        terminal.menu.as_ref().unwrap().menu.id,
        headless.menu().unwrap().id
    );
    for (host, fixture) in [
        (&mut terminal, &mut terminal_fixture),
        (headless.host_mut(), &mut headless_fixture),
    ] {
        let effects = host.update(Event::Paste("A renderer-neutral task".into()));
        host_pump(host, fixture, effects);
    }
    host_key(&mut terminal, &mut terminal_fixture, KeyCode::Enter);
    headless_send(
        &mut headless,
        &mut headless_fixture,
        ViewIntent::Key {
            chord: Chord::new(Key::Enter),
        },
    );
    assert_eq!(
        terminal.menu.as_ref().unwrap().menu,
        headless.host().menu.as_ref().unwrap().menu
    );
    assert!(headless.menu().unwrap().question);
    assert!(
        headless_fixture
            .journal
            .query_items(&ItemFilter::default(), 100, None)
            .unwrap()
            .items
            .is_empty()
    );
    host_key(&mut terminal, &mut terminal_fixture, KeyCode::Char('y'));
    headless_send(
        &mut headless,
        &mut headless_fixture,
        ViewIntent::Invoke {
            action: "yes".into(),
        },
    );
    for fixture in [&mut terminal_fixture, &mut headless_fixture] {
        let page = fixture
            .journal
            .query_items(&ItemFilter::default(), 100, None)
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].kind.as_str(), "task");
    }
    let terminal_view = terminal.screen_view_in(0, "work", 160, 40).unwrap();
    assert_eq!(
        normalize_ids(&serde_json::to_string(&terminal_view).unwrap()),
        normalize_ids(&serde_json::to_string(&headless.screen().unwrap()).unwrap())
    );
    let view = headless.screen().unwrap();
    let Node::List { rows, .. } = &view.pane(PaneId::Middle).unwrap().body else {
        panic!()
    };
    headless_send(
        &mut headless,
        &mut headless_fixture,
        ViewIntent::Activate {
            target: rows[0].target.clone(),
        },
    );
    assert_eq!(headless.host().focus, Pane::Detail);
}

#[test]
fn headless_confirmation_honors_read_only_mode_and_retains_preview() {
    use radar_core::host::Event;
    let mut fixture = Fixture::new();
    let mut headless = Headless::new(host(std::mem::take(&mut fixture.p)), 80, 24);
    headless
        .host_mut()
        .set_env_mode(Some((Mode::ReadOnly, "RADAR_MODE".into())));
    headless_send(
        &mut headless,
        &mut fixture,
        ViewIntent::Invoke {
            action: "work.new".into(),
        },
    );
    let effects = headless
        .host_mut()
        .update(Event::Paste("Blocked but retained".into()));
    host_pump(headless.host_mut(), &mut fixture, effects);
    headless_send(
        &mut headless,
        &mut fixture,
        ViewIntent::Key {
            chord: Chord::new(Key::Enter),
        },
    );
    assert!(
        headless
            .menu()
            .unwrap()
            .items
            .iter()
            .any(|item| item.label.contains("read only"))
    );
    headless_send(
        &mut headless,
        &mut fixture,
        ViewIntent::Invoke {
            action: "yes".into(),
        },
    );
    assert!(
        fixture
            .journal
            .query_items(&ItemFilter::default(), 100, None)
            .unwrap()
            .items
            .is_empty()
    );
    headless_send(
        &mut headless,
        &mut fixture,
        ViewIntent::Invoke {
            action: "work.refresh".into(),
        },
    );
    let menu = headless.menu().unwrap();
    assert_eq!(menu.id, "work.confirm");
    assert!(
        menu.items
            .iter()
            .any(|item| item.label.contains("Blocked but retained"))
    );
}

#[test]
fn every_neutral_action_and_pointer_target_is_taken() {
    // Each target is exercised against the instance that issued its snapshot.
    // A different journal's equal-looking row deliberately has a different lease.
    for pane in [Pane::Nav, Pane::Middle, Pane::Detail] {
        let mut plugin = sample();
        let described = view(&plugin, pane);
        let targets = match &described.pane(pane.into()).unwrap().body {
            Node::List { rows, .. } => rows
                .iter()
                .map(|row| row.target.clone())
                .collect::<Vec<_>>(),
            Node::Entries { rows, .. } => rows.iter().map(|row| row.target.clone()).collect(),
            _ => vec![],
        };
        for target in targets {
            // Selection may start a read. Complete it before the next selection.
            let mut f = Fixture::new();
            std::mem::swap(&mut f.p, &mut plugin);
            let outcome = f.p.intent("work", pane, &ViewIntent::Select { target });
            assert!(matches!(outcome, Outcome::Handled(_)));
            if let Outcome::Handled(effects) = outcome {
                // Do not read a different database: only verify this advertised
                // target is taken, then reset the transient request locally.
                drop(effects);
                f.p.pending = None;
            }
            std::mem::swap(&mut f.p, &mut plugin);
        }
    }
    for pane in [Pane::Nav, Pane::Middle, Pane::Detail] {
        for action in view(&sample(), pane)
            .actions
            .into_iter()
            .filter(|action| !action.heading)
        {
            let mut plugin = sample();
            assert!(matches!(
                plugin.intent("work", pane, &ViewIntent::Invoke { action: action.id }),
                Outcome::Handled(_)
            ));
        }
    }
}

#[test]
fn work_domain_source_names_no_terminal_contract() {
    for (name, code) in [
        ("lib", include_str!("lib.rs")),
        ("screen", include_str!("screen.rs")),
        ("interaction", include_str!("interaction.rs")),
        ("model", include_str!("model.rs")),
    ] {
        for forbidden in [
            "ratatui",
            "crossterm",
            "KeyEvent",
            "view_tui",
            "view_ratatui",
            "semantic_view",
            "semantic_action",
        ] {
            assert!(!code.contains(forbidden), "{name} still uses {forbidden}");
        }
    }
}

fn prepared_source(f: &mut Fixture) -> Prepared {
    f.start();
    let effects = f.p.act(&source("ticket", "ABC-12"), "work.link");
    f.pump(effects);
    f.menu("work.choose", "new");
    f.menu("work.new", "Retained source review");
    f.menu("work.sources", "review");
    f.p.prepared.clone().unwrap()
}

#[test]
fn refused_source_apply_after_restart_revalidates_without_replacing_original_ids() {
    let mut f = Fixture::new();
    let prepared = prepared_source(&mut f);
    f.mode = Mode::ReadOnly;
    f.menu("work.confirm", "yes");
    assert!(!f.p.prepared.as_ref().unwrap().attempted);
    let cached = f.p.snapshot().unwrap();
    assert!(cached["source_draft"].is_object());
    f.p = WorkbenchPlugin::default();
    f.p.restore(cached);
    f.mode = Mode::Edit;
    f.owner_ready = false;
    f.menu("work.confirm", "yes"); // Inspect original IDs first.
    assert!(!f.p.prepared.as_ref().unwrap().reconcile);
    f.menu("work.confirm", "yes"); // Owner is unavailable: do not apply.
    assert!(f.p.prepared.as_ref().unwrap().blocked);
    assert_eq!(
        serde_json::to_value(&f.p.prepared.as_ref().unwrap().batch).unwrap(),
        serde_json::to_value(&prepared.batch).unwrap()
    );
    assert!(
        f.journal
            .query_items(&ItemFilter::default(), 100, None)
            .unwrap()
            .items
            .is_empty()
    );
    f.owner_ready = true;
    f.menu("work.confirm", "yes"); // Inspect again after recovery.
    f.menu("work.confirm", "yes"); // Revalidate and save original batch.
    assert!(f.p.prepared.is_none());
    assert_eq!(f.p.selected, Some(prepared.task));
}

#[test]
fn stale_false_cache_reconciles_a_committed_source_batch_before_exact_replay() {
    let mut f = Fixture::new();
    let prepared = prepared_source(&mut f);
    let cached_before_dispatch = f.p.snapshot().unwrap();
    f.menu("work.confirm", "yes");
    let committed = f.journal.work_map_snapshot().unwrap().map_sequence();
    f.p = WorkbenchPlugin::default();
    f.p.restore(cached_before_dispatch);
    f.owner_ready = false;
    f.menu("work.confirm", "yes"); // Read proves every original intent exists.
    assert!(f.p.prepared.as_ref().unwrap().attempted);
    assert_eq!(
        serde_json::to_value(&f.p.prepared.as_ref().unwrap().batch).unwrap(),
        serde_json::to_value(&prepared.batch).unwrap()
    );
    f.menu("work.confirm", "yes"); // Exact replay does not need a live owner.
    assert!(f.p.prepared.is_none());
    assert_eq!(
        f.journal.work_map_snapshot().unwrap().map_sequence(),
        committed
    );
}

#[test]
fn legacy_missing_source_draft_is_inspectable_and_explicitly_cancellable() {
    let mut f = Fixture::new();
    let prepared = prepared_source(&mut f);
    let mut cached = f.p.snapshot().unwrap();
    cached.as_object_mut().unwrap().remove("source_draft");
    f.p = WorkbenchPlugin::default();
    f.p.restore(cached);
    f.menu("work.confirm", "yes");
    assert!(f.p.prepared.as_ref().unwrap().blocked);
    assert!(
        !f.menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .any(|item| item.value == "yes")
    );
    assert_eq!(
        serde_json::to_value(&f.p.prepared.as_ref().unwrap().batch).unwrap(),
        serde_json::to_value(&prepared.batch).unwrap()
    );
    let Outcome::Handled(effects) = f.p.intent(
        "work",
        Pane::Middle,
        &ViewIntent::Invoke {
            action: "work.copy-retained".into(),
        },
    ) else {
        panic!()
    };
    assert!(effects.iter().any(|effect| matches!(effect, Effect::Copy(text) if text.contains(prepared.batch.changes[0].change_id.as_str()))));
    f.menu("work.confirm", "no");
    assert!(f.p.prepared.is_none());
    assert!(
        f.journal
            .query_items(&ItemFilter::default(), 100, None)
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn showing_another_item_and_empty_selection_never_reuse_previous_sources() {
    let mut f = Fixture::new();
    f.start();
    let first = f.link_new("First work", vec![source("ticket", "FIRST-1")]);
    let second = f.link_new("Second work", vec![source("ticket", "SECOND-2")]);
    assert!(
        f.p.sources()
            .iter()
            .all(|(_, key, _, _, _)| key.id == "SECOND-2")
    );
    let Showing::Page { effects, .. } =
        f.p.show(&Subject::new("work-item", first.as_str(), "First work"))
    else {
        panic!()
    };
    assert!(f.p.sources().is_empty());
    f.pump(effects);
    assert!(
        f.p.sources()
            .iter()
            .all(|(_, key, _, _, _)| key.id == "FIRST-1")
    );
    assert_ne!(f.p.selected, Some(second));
    f.p.selected = None;
    assert!(f.p.read_detail().is_empty());
    assert!(f.p.sources().is_empty());
    assert!(
        !view(&f.p, Pane::Detail)
            .actions
            .iter()
            .any(|action| matches!(action.id.as_str(), "work.open-source" | "work.copy-source"))
    );
}

#[test]
fn interval_variants_and_conflicting_labels_remain_distinct_in_both_views() {
    use radar_core::journal::work_map::{ChangeId, Mutation, SemanticLabel};
    let mut f = Fixture::new();
    f.start();
    f.link_new("Explicit evidence", vec![source("ticket", "ABC-12")]);
    f.menu("work.new-group", "original group");
    f.menu("work.confirm", "yes");
    let detail = f.p.detail.as_mut().unwrap();
    let mut interval = detail
        .related
        .as_ref()
        .unwrap()
        .records
        .iter()
        .find_map(|record| match record {
            radar_core::journal::work_map::HandoffRecord::Map { change }
                if matches!(change.request.mutation, Mutation::BindSubject { .. }) =>
            {
                Some(change.clone())
            }
            _ => None,
        })
        .unwrap();
    interval.request.id = ChangeId::new();
    if let Mutation::BindSubject {
        from_sequence,
        through_sequence,
        ..
    } = &mut interval.request.mutation
    {
        *from_sequence = 100;
        *through_sequence = Some(200);
    }
    detail
        .related
        .as_mut()
        .unwrap()
        .records
        .push(radar_core::journal::work_map::HandoffRecord::Map { change: interval });
    let template = detail.primary.records.iter().find_map(|record| match record {
        radar_core::journal::work_map::HandoffRecord::Map { change } if matches!(&change.request.mutation, Mutation::CreateTag { namespace, .. } if namespace.as_str() == model::GROUP_NAMESPACE) => Some(change.clone()),
        _ => None,
    }).unwrap();
    let Mutation::CreateTag { tag_id, .. } = &template.request.mutation else {
        panic!()
    };
    let tag_id = tag_id.clone();
    for label in ["first label assertion", "second label assertion"] {
        let mut claim = template.clone();
        claim.request.id = ChangeId::new();
        claim.request.mutation = Mutation::RenameTag {
            tag_id: tag_id.clone(),
            label: SemanticLabel::new(label).unwrap(),
        };
        detail
            .primary
            .records
            .push(radar_core::journal::work_map::HandoffRecord::Map { change: claim });
    }
    let sources = f.p.sources();
    assert_eq!(sources.len(), 2);
    assert_ne!(
        screen::source_target(&f.p, &sources[0]),
        screen::source_target(&f.p, &sources[1])
    );
    let described = serde_json::to_string(&view(&f.p, Pane::Detail)).unwrap();
    assert!(described.contains("ambiguous") && described.contains("Multiple binding claims"));
    let mut evidence = f.p.detail.as_ref().unwrap().primary.clone();
    evidence.records.extend(
        f.p.detail
            .as_ref()
            .unwrap()
            .related
            .as_ref()
            .unwrap()
            .records
            .clone(),
    );
    f.p.evidence = Some(evidence);
    let neutral = serde_json::to_string(&view(&f.p, Pane::Detail)).unwrap();
    for text in [
        "first label assertion",
        "second label assertion",
        "100 through 200",
        "Exact source:",
        "Origin:",
        "session",
        "Context subject:",
    ] {
        assert!(neutral.contains(text), "missing {text}");
    }
    let mut rendered = host(f.into_plugin());
    rendered.focus = Pane::Detail;
    let mut frames = String::new();
    let mut previous = String::new();
    for _ in 0..32 {
        let frame = radar_core::shell::to_text(&rendered, 80, 24);
        if frame == previous {
            break;
        }
        frames.push_str(&frame);
        previous = frame;
        rendered.update(radar_core::host::Event::Key(KeyEvent::new(
            KeyCode::PageDown,
            KeyModifiers::NONE,
        )));
    }
    for text in [
        "first label assertion",
        "second label assertion",
        "100 through 200",
        "Exact source:",
        "Origin:",
    ] {
        assert!(frames.contains(text), "terminal omitted {text}");
    }
}

#[test]
fn restarted_source_draft_tick_and_stale_menus_cannot_replace_retained_ids() {
    for attempted in [false, true] {
        let mut f = Fixture::new();
        let prepared = prepared_source(&mut f);
        f.p.prepared.as_mut().unwrap().attempted = attempted;
        let cached = f.p.snapshot().unwrap();
        let mut first_tick = WorkbenchPlugin::default();
        first_tick.restore(cached.clone());
        f.p = WorkbenchPlugin::default();
        f.p.restore(cached);
        f.menu = None;
        f.start();
        assert!(
            f.menu.is_none(),
            "startup must not reopen a source chooser over retained recovery"
        );
        for (menu, value) in [
            ("work.choose", "new"),
            ("work.new", "Replacement"),
            ("work.sources", "review"),
            ("work.group", "new"),
            ("work.new-group", "Replacement"),
        ] {
            f.menu(menu, value);
            assert_eq!(
                serde_json::to_value(&f.p.prepared.as_ref().unwrap().batch).unwrap(),
                serde_json::to_value(&prepared.batch).unwrap()
            );
        }
        let mut h = host(first_tick);
        h.enable_journal();
        let effects = h.update(radar_core::host::Event::Tick(1001));
        host_pump(&mut h, &mut f, effects);
        assert!(h.menu.is_none());
        host_key(&mut h, &mut f, KeyCode::Char('r'));
        assert_eq!(h.menu.as_ref().unwrap().menu.id, "work.confirm");
    }
}

#[test]
fn retained_source_cache_keeps_only_bounded_routing_fields() {
    let mut f = Fixture::new();
    prepared_source(&mut f);
    let subject = &mut f.p.link.as_mut().unwrap().candidates[0].source.subject;
    for field in [
        "title",
        "assignee",
        "description",
        "status",
        "head_sha",
        "branch",
    ] {
        subject
            .fields
            .insert(field.into(), "Do not retain display data".into());
    }
    let cached = f.p.snapshot().unwrap();
    let fields = cached["source_draft"]["candidates"][0]["fields"]
        .as_object()
        .unwrap();
    assert_eq!(fields.len(), 2);
    assert!(fields.contains_key("provider") && fields.contains_key("instance"));
    let mut restored = WorkbenchPlugin::default();
    restored.restore(cached);
    f.p = restored;
    f.menu("work.confirm", "yes");
    f.menu("work.confirm", "yes");
    assert!(f.p.prepared.is_none());
    assert_eq!(f.p.sources().len(), 1);
}

#[test]
fn restored_source_targets_match_the_reference_multiset_and_keys_are_unique() {
    use radar_core::journal::work_map::{ChangeId, Mutation, RelationKind};
    use radar_core::work_api::Intent;

    let mut f = Fixture::new();
    let prepared = prepared_source(&mut f);
    let mut saved = f.p.saved_link().unwrap();
    let a = ItemId::parse(ActionId::new().as_str()).unwrap();
    let b = ItemId::parse(ActionId::new().as_str()).unwrap();
    let mut first = saved["candidates"][0].clone();
    first["resolved"][0] = serde_json::to_value(&a).unwrap();
    let mut second = first.clone();
    second["id"] = "ABC-13".into();
    second["key"]["id"] = "ABC-13".into();
    saved["candidates"] = serde_json::json!([first.clone(), second]);
    let reference = |to: &ItemId| {
        Intent::asserted(
            ChangeId::parse(ActionId::new().as_str()).unwrap(),
            Mutation::RelateItems {
                from: prepared.task.clone(),
                to: to.clone(),
                kind: RelationKind::References,
            },
        )
    };
    let repeated = Batch::new(vec![reference(&a), reference(&a)]);
    assert!(
        f.p.restore_link(&saved, &prepared.task, &repeated)
            .is_some()
    );
    let missing_target = Batch::new(vec![reference(&a), reference(&b)]);
    assert!(
        f.p.restore_link(&saved, &prepared.task, &missing_target)
            .is_none(),
        "two candidates for A cannot validate a retained reference to B"
    );
    saved["candidates"] = serde_json::json!([first.clone(), first]);
    assert!(
        f.p.restore_link(&saved, &prepared.task, &repeated)
            .is_none(),
        "duplicate exact source keys are not separate candidates"
    );
}

#[test]
fn partial_conflicting_and_unreadable_inspection_retain_ids_and_block_replacement() {
    fn inspect(f: &mut Fixture, batch: &Batch, error: Option<work_api::Error>) {
        let effects = f.p.on_menu("work.confirm", "yes");
        assert_eq!(effects.len(), 1);
        let Effect::Work { id, request } = effects.into_iter().next().unwrap() else {
            panic!("recovery must inspect before considering another apply")
        };
        let Request::InspectBatch(retained) = *request else {
            panic!("blocked recovery must never dispatch an apply")
        };
        assert_eq!(
            serde_json::to_value(&retained).unwrap(),
            serde_json::to_value(batch).unwrap()
        );
        let result = if let Some(error) = error {
            Err(error)
        } else {
            let result = work_api::test_support::execute(
                &mut f.journal,
                &f.session,
                Request::InspectBatch(retained),
                Mode::Edit,
                1000,
            );
            assert!(matches!(
                result,
                Ok(Response::BatchState(work_api::BatchState::Partial))
            ));
            result
        };
        let effects = f.p.on_msg(Box::new(WorkReply { id, result }));
        assert!(
            !effects.iter().any(
                |effect| matches!(effect, Effect::Work { request, .. } if request.is_mutation())
            )
        );
        f.pump(effects);
    }

    for error in [
        None,
        Some(work_api::Error::Conflict),
        Some(work_api::Error::Storage),
    ] {
        let mut f = Fixture::new();
        let prepared = prepared_source(&mut f);
        let cached = f.p.snapshot().unwrap();
        let first = Batch::new(vec![prepared.batch.changes[0].clone()]);
        assert!(matches!(
            work_api::test_support::execute(
                &mut f.journal,
                &f.session,
                Request::Apply(first),
                Mode::Edit,
                1000,
            ),
            Ok(Response::Applied(_))
        ));
        let map_sequence = f.journal.work_map_snapshot().unwrap().map_sequence();
        f.p = WorkbenchPlugin::default();
        f.p.restore(cached);
        f.menu = None;
        f.start();
        assert!(f.menu.is_none());
        inspect(&mut f, &prepared.batch, error);
        assert!(f.p.prepared.as_ref().unwrap().blocked);
        assert!(f.p.prepared.as_ref().unwrap().reconcile);

        f.menu("work.confirm", "no");
        assert!(matches!(
            f.p.intent(
                "work",
                Pane::Middle,
                &ViewIntent::Invoke {
                    action: "work.new".into(),
                },
            ),
            Outcome::Ignored
        ));
        let effects = f.p.confirm_prepared();
        f.pump(effects);
        assert_eq!(f.menu.as_ref().unwrap().id, "work.confirm");
        assert!(
            !f.menu
                .as_ref()
                .unwrap()
                .items
                .iter()
                .any(|item| item.value == "yes")
        );
        for (menu, value) in [
            ("work.choose", "new"),
            ("work.new", "Replacement"),
            ("work.sources", "review"),
            ("work.group", "new"),
            ("work.new-group", "Replacement"),
        ] {
            f.menu(menu, value);
            assert_eq!(
                serde_json::to_value(&f.p.prepared.as_ref().unwrap().batch).unwrap(),
                serde_json::to_value(&prepared.batch).unwrap()
            );
        }
        inspect(&mut f, &prepared.batch, error);
        assert!(f.p.prepared.as_ref().unwrap().blocked);
        assert_eq!(
            f.journal.work_map_snapshot().unwrap().map_sequence(),
            map_sequence,
            "inspection and blocked replacement must leave the partial commit unchanged"
        );
        let retained = f.p.snapshot().unwrap();
        assert_eq!(
            retained["pending"]["batch"],
            serde_json::to_value(&prepared.batch).unwrap()
        );
    }
}

#[test]
fn optional_source_owner_controls_open_but_preserves_copy_and_context() {
    let mut f = Fixture::new();
    f.start();
    f.link_new("Keep exact local context", vec![source("ticket", "ABC-12")]);
    let absent = view(&f.p, Pane::Detail);
    assert!(
        !absent
            .actions
            .iter()
            .any(|action| action.id == "work.open-source")
    );
    assert!(
        absent
            .actions
            .iter()
            .any(|action| action.id == "work.copy-source")
    );
    assert!(
        serde_json::to_string(&absent)
            .unwrap()
            .contains("Source owner unavailable here")
    );
    let parts: Vec<Box<dyn Plugin>> = vec![Box::new(SourcePart)];
    let present = screen::view(
        &f.p,
        &ViewCx {
            now: 1000,
            focus: PaneId::Detail,
            width: 160,
            height: 40,
            parts: Parts::new(&parts, None),
            icon_lead: 0,
        },
    );
    assert!(
        present
            .actions
            .iter()
            .any(|action| action.id == "work.open-source")
    );
    assert!(
        present
            .actions
            .iter()
            .any(|action| action.id == "work.copy-source")
    );
    assert!(
        !serde_json::to_string(&present)
            .unwrap()
            .contains("Source owner unavailable here")
    );
}

#[test]
fn headless_lists_the_screens_actions_under_their_stable_ids() {
    let mut fixture = Fixture::new();
    let headless = Headless::new(host(std::mem::take(&mut fixture.p)), 160, 40);
    let actions = headless.screen().unwrap().actions;
    let id_of = |key: &str| actions.iter().find(|a| a.key.is_some() && !a.label.is_empty() && a.id.starts_with("work.") && a.key == Chord::parse(key)).map(|a| a.id.clone());
    assert_eq!(id_of("n").as_deref(), Some("work.new"));
    assert!(actions.iter().filter(|a| a.heading).all(|a| a.id.is_empty()));
    assert!(actions.iter().all(|a| a.id.is_empty() || a.id.starts_with("work.")));
}
