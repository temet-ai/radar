//! Read-only evidence and task correlation. Tasks use mock replies or temp files;
//! tests never discover the user's workspaces or open a journal database.

use super::*;
use radar_core::Host;
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::ActionId;

fn indexed(dir: &str, observed: find::Observation) -> WorkMsg {
    WorkMsg::Indexed {
        dir: dir.into(),
        index: notes::Index {
            files: 7,
            clones: 2,
            ..notes::Index::default()
        },
        observed,
        action: read_action("workspace.notes.index", Some(dir)),
    }
}

#[test]
fn read_results_distinguish_bounded_success_partial_and_missing() {
    let mut plugin = WorkspacePlugin::new();
    for (observed, expected) in [
        (find::Observation::default(), JournalOutcome::Succeeded),
        (
            find::Observation {
                errors: 1,
                ..Default::default()
            },
            JournalOutcome::Partial,
        ),
        (
            find::Observation {
                cutoffs: 1,
                ..Default::default()
            },
            JournalOutcome::Partial,
        ),
    ] {
        let effects = plugin.on_work(indexed("/work/original", observed));
        assert!(matches!(effects[0], Effect::Report(outcome) if outcome == expected));
        let counts: Vec<_> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Observe { action, .. } => {
                    Some((action.kind.as_str(), action.metadata.count))
                }
                _ => None,
            })
            .collect();
        assert!(counts.contains(&("workspace.notes.index.files.encountered", Some(7))));
        assert!(counts.contains(&("workspace.notes.index.clones", Some(2))));
    }
    let mut message = indexed("/work/missing", find::Observation::default());
    if let WorkMsg::Indexed { index, .. } = &mut message {
        index.missing = true;
    }
    assert!(matches!(
        plugin.on_work(message)[0],
        Effect::Report(JournalOutcome::Blocked)
    ));
    let mut message = indexed(
        "/work/inaccessible",
        find::Observation {
            errors: 1,
            ..Default::default()
        },
    );
    if let WorkMsg::Indexed { index, .. } = &mut message {
        index.missing = true;
    }
    assert!(matches!(
        plugin.on_work(message)[0],
        Effect::Report(JournalOutcome::Partial)
    ));
}

#[tokio::test]
async fn indexing_keeps_its_captured_settings_until_the_real_task_reply() {
    let scratch = find::fixture::Scratch::new("captured-index");
    scratch.file("ABC-12/README.md", "# SECRET_TITLE\nSECRET_BODY");
    let dir = scratch.0.display().to_string();
    let mut plugin = WorkspacePlugin::new();
    plugin.cfg.spaces.push(Space::new(&dir));
    let effect = plugin.reindex(&dir, true).remove(0);
    let Effect::SpawnAction { task, action } = effect else {
        panic!("index task")
    };
    assert_eq!(action.subjects[0].key.id, dir);
    plugin.cfg.spaces = vec![Space::new("/work/reconfigured")];
    let effects = plugin.on_msg(task.await);
    assert!(matches!(
        effects[0],
        Effect::Report(JournalOutcome::Succeeded)
    ));
    assert_eq!(plugin.state.indexes[&dir].notes.len(), 1);
    assert!(!plugin.state.indexing.contains(&dir));
    assert!(!plugin.state.say.contains(&dir));
    for effect in effects {
        if let Effect::Observe { action, .. } = effect {
            assert_eq!(action.subjects[0].key.id, dir);
            assert!(!format!("{action:?}").contains("SECRET"));
        }
    }
}

#[test]
fn duplicate_reads_are_blocked_without_replacing_the_in_flight_task() {
    let mut plugin = WorkspacePlugin::new();
    plugin.cfg.spaces.push(Space::new("/work/original"));
    assert!(matches!(
        plugin.reindex("/work/original", true)[0],
        Effect::SpawnAction { .. }
    ));
    let effects = plugin.reindex("/work/original", false);
    assert!(matches!(
        effects[0],
        Effect::Observe {
            outcome: JournalOutcome::Blocked,
            ..
        }
    ));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
    );
    assert!(plugin.state.say.contains("/work/original"));
    assert!(matches!(plugin.find_now()[0], Effect::SpawnAction { .. }));
    assert!(matches!(
        plugin.find_now()[0],
        Effect::Observe {
            outcome: JournalOutcome::Blocked,
            ..
        }
    ));
    assert!(plugin.state.finding);
}

#[tokio::test]
async fn panics_return_unknown_and_release_only_their_original_read_guards() {
    let mut plugin = WorkspacePlugin::new();
    plugin
        .state
        .indexing
        .extend(["/work/first".into(), "/work/second".into()]);
    plugin
        .state
        .say
        .extend(["/work/first".into(), "/work/second".into()]);
    plugin.state.finding = true;
    for dir in [Some("/work/first"), None] {
        let action = read_action(
            if dir.is_some() {
                "workspace.notes.index"
            } else {
                "workspace.discovery"
            },
            dir,
        );
        let effect = blocking_read(action, dir.map(str::to_string), |_| panic!("SECRET_PANIC"));
        let Effect::SpawnAction { task, .. } = effect else {
            panic!("read task")
        };
        let effects = plugin.on_msg(task.await);
        assert!(matches!(
            effects[0],
            Effect::Report(JournalOutcome::Unknown)
        ));
        for effect in effects {
            if let Effect::Observe { action, outcome } = effect {
                assert_eq!(outcome, JournalOutcome::Unknown);
                assert_eq!(action.metadata.reason, Some(Reason::Interrupted));
                assert!(!format!("{action:?}").contains("SECRET_PANIC"));
            }
        }
    }
    assert!(!plugin.state.finding);
    assert!(!plugin.state.indexing.contains("/work/first"));
    assert!(!plugin.state.say.contains("/work/first"));
    assert!(plugin.state.indexing.contains("/work/second"));
    assert!(plugin.state.say.contains("/work/second"));
}

#[test]
fn captured_scope_and_aggregate_counts_exclude_note_and_discovery_payloads() {
    let mut plugin = WorkspacePlugin::new();
    plugin.cfg.spaces.push(Space::new("/work/new-settings"));
    let mut message = indexed("/work/original", find::Observation::default());
    if let WorkMsg::Indexed { index, .. } = &mut message {
        index.notes.push(notes::Note {
            title: "SECRET_TITLE".into(),
            path: "/private/SECRET_NOTE.md".into(),
            lead: vec!["SECRET_BODY".into()],
            ..notes::Note::default()
        });
    }
    for effect in plugin.on_work(message) {
        if let Effect::Observe { action, .. } = effect {
            assert_eq!(action.subjects[0].key.instance, "/work/original");
            let record = format!("{action:?}");
            assert!(!record.contains("SECRET"));
            assert!(!record.contains("new-settings"));
        }
    }
    for effect in plugin.on_work(WorkMsg::Found {
        offers: vec![find::Offer {
            dir: "/private/SECRET_DISCOVERY".into(),
            name: "SECRET_NAME".into(),
            clones: 5,
            repos: "SECRET_REPOS".into(),
        }],
        observed: find::Observation::default(),
        action: read_action("workspace.discovery", None),
    }) {
        if let Effect::Observe { action, .. } = effect {
            assert!(action.subjects.is_empty());
            assert_eq!(action.metadata.count, Some(1));
            assert!(!format!("{action:?}").contains("SECRET"));
        }
    }
    let redacted = read_action("workspace.notes.index", Some("/work/token=SECRET"));
    assert!(redacted.metadata.redacted);
    assert!(redacted.subjects.is_empty());
}

// The wrapper only enters production read paths. Follow-on notifications and
// synthetic payloads make causal isolation observable without running disk scans.
enum Begin {
    Tick,
    Duplicate,
    ChangeSettings,
}
struct Reads(WorkspacePlugin);
impl Plugin for Reads {
    fn id(&self) -> &'static str {
        "workspace"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn on_msg(&mut self, message: Msg) -> Vec<Effect> {
        match message.downcast::<Begin>() {
            Ok(begin) => match *begin {
                Begin::Tick => {
                    self.0.cfg.spaces = vec![Space::new("/work/first"), Space::new("/work/second")];
                    let mut effects = self.0.tick(1);
                    effects.push(Effect::Notify("after reads".into()));
                    effects
                }
                Begin::Duplicate => {
                    let mut effects = self.0.reindex("/work/first", true);
                    effects.push(Effect::Notify("after duplicate".into()));
                    effects
                }
                Begin::ChangeSettings => {
                    self.0.cfg.spaces = vec![Space::new("/work/changed")];
                    vec![]
                }
            },
            Err(message) => self.0.on_msg(message),
        }
    }
}

fn records(effects: &[HostEffect]) -> Vec<&JournalRecord> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Journal(record) => Some(record),
            _ => None,
        })
        .collect()
}
fn terminal(effects: &[HostEffect], id: &ActionId) -> Vec<JournalOutcome> {
    records(effects)
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Outcome {
                id: found, outcome, ..
            } if found == id && outcome.is_terminal() => Some(*outcome),
            _ => None,
        })
        .collect()
}
fn reply(
    host: &mut Host,
    task: &(ActionId, Option<ActionId>),
    message: WorkMsg,
) -> Vec<HostEffect> {
    host.update(Event::Tracked {
        action: task.0.clone(),
        semantic: task.1.clone(),
        event: Box::new(Event::Msg {
            plugin: 0,
            msg: Box::new(message),
        }),
    })
}

#[test]
fn out_of_order_reads_and_summary_counts_preserve_each_tasks_semantic_scope() {
    let mut host = Host::new(vec![Box::new(Reads(WorkspacePlugin::new()))], 1).unwrap();
    host.enable_journal();
    let effects = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Tick),
    });
    let tasks: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Tracked {
                action,
                semantic,
                effect,
            } if matches!(&**effect, HostEffect::Spawn { .. }) => {
                Some((action.clone(), semantic.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(tasks.len(), 3);
    assert_ne!(tasks[0].1, tasks[1].1);
    assert_ne!(tasks[1].1, tasks[2].1);
    for record in records(&effects) {
        if let JournalRecord::Begin {
            descriptor, parent, ..
        } = record
            && descriptor.kind.as_str() == "ui.notice"
        {
            assert!(tasks.iter().all(|task| parent.as_ref() != task.1.as_ref()));
        }
    }
    let duplicate = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Duplicate),
    });
    for task in &tasks {
        assert!(terminal(&duplicate, task.1.as_ref().unwrap()).is_empty());
    }
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::ChangeSettings),
    });
    let second = reply(
        &mut host,
        &tasks[1],
        indexed(
            "/work/second",
            find::Observation {
                errors: 1,
                ..Default::default()
            },
        ),
    );
    assert_eq!(
        terminal(&second, tasks[1].1.as_ref().unwrap()),
        [JournalOutcome::Partial]
    );
    assert!(terminal(&second, tasks[0].1.as_ref().unwrap()).is_empty());
    let first = reply(
        &mut host,
        &tasks[0],
        indexed("/work/first", find::Observation::default()),
    );
    assert_eq!(
        terminal(&first, tasks[0].1.as_ref().unwrap()),
        [JournalOutcome::Succeeded]
    );
    for record in records(&first) {
        if let JournalRecord::Begin { descriptor, .. } = record
            && descriptor
                .kind
                .as_str()
                .starts_with("workspace.notes.index.")
        {
            assert_eq!(descriptor.subjects[0].key.instance, "/work/first");
        }
    }
    let found = reply(
        &mut host,
        &tasks[2],
        WorkMsg::Found {
            offers: vec![],
            observed: find::Observation::default(),
            action: read_action("workspace.discovery", None),
        },
    );
    assert_eq!(
        terminal(&found, tasks[2].1.as_ref().unwrap()),
        [JournalOutcome::Succeeded]
    );
}
