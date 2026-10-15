//! Runtime coverage without touching the user's journal or services.
use super::*;
use crate::host::{JournalRecord, View};
use crate::journal::{Journal, Query};
use crate::plugin::{Effect, Menu, Msg, Outcome, Pane, ScreenSpec};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

struct Prepared;
struct ReadPrepared;

struct Probe {
    id: &'static str,
    key: char,
    result: JournalOutcome,
    capturing: bool,
}

impl Probe {
    fn action(&self) -> Effect {
        Effect::Journal(ActionDescriptor::new(
            "probe.change",
            Origin::Plugin(self.id.into()),
        ))
    }

    fn task(&self) -> Effect {
        let result = self.result;
        Effect::Spawn(Box::pin(async move { Box::new(result) as Msg }))
    }

    fn read(&self, kind: &'static str, result: JournalOutcome) -> Effect {
        Effect::SpawnAction {
            action: ActionDescriptor::new(kind, Origin::Plugin(self.id.into())),
            task: Box::pin(async move { Box::new(result) as Msg }),
        }
    }
}

impl Plugin for Probe {
    fn id(&self) -> &'static str {
        self.id
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn owns(&self) -> Vec<crate::compose::Owned> {
        if self.id == "alpha" {
            vec![crate::compose::Owned::link("probe-item", "items")]
        } else {
            vec![]
        }
    }
    fn journal_subject(
        &self,
        _: &crate::compose::Subject,
    ) -> Option<crate::journal::SubjectSnapshot> {
        Some(crate::journal::SubjectSnapshot::new(
            crate::journal::SubjectKey::new("alpha", "instance-one", "probe-item", "42").unwrap(),
            crate::journal::SubjectRole::Primary,
        ))
    }
    fn capturing(&self) -> bool {
        self.capturing
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "main",
            name: "probe",
            key: self.key,
        }]
    }
    fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::ScreenView> {
        use crate::view as v;
        Some(v::ScreenView::new(scope, vec![v::PaneView::new(
            v::PaneId::Middle, "probe", v::Node::Document { lines: vec![], cut: false },
        )]))
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &crate::view::Intent) -> Outcome {
        let crate::view::Intent::Key { chord } = intent else { return Outcome::Ignored };
        if self.capturing {
            return Outcome::Handled(vec![]);
        }
        Outcome::Handled(match chord.key {
            crate::view::Key::Char('a') => vec![self.action(), self.task()],
            crate::view::Key::Char('c') => vec![
                self.action(),
                self.read("probe.read", JournalOutcome::Succeeded),
                self.read("probe.other", JournalOutcome::Failed),
                Effect::Observe {
                    action: ActionDescriptor::new("probe.skipped", Origin::Plugin(self.id.into())),
                    outcome: JournalOutcome::Blocked,
                },
                self.task(),
                Effect::Emit {
                    topic: "probe.changed",
                    payload: serde_json::json!({"secret": "SECRET payload"}),
                },
                Effect::Report(JournalOutcome::Started),
            ],
            crate::view::Key::Char('u') => vec![
                self.read("probe.first", JournalOutcome::Succeeded),
                self.read("probe.second", JournalOutcome::Failed),
                self.task(),
            ],
            crate::view::Key::Char('r') => vec![Effect::SpawnAction {
                action: ActionDescriptor::new("probe.read", Origin::Plugin(self.id.into())),
                task: Box::pin(async { Box::new(ReadPrepared) as Msg }),
            }],
            crate::view::Key::Char('p') => vec![
                self.action(),
                Effect::Spawn(Box::pin(async { Box::new(Prepared) as Msg })),
            ],
            crate::view::Key::Char('m') => vec![
                self.action(),
                Effect::Menu(Menu::confirm(
                    "secret-menu-id",
                    "SECRET title",
                    "SECRET yes",
                    "SECRET no",
                )),
            ],
            crate::view::Key::Char('t') => vec![
                self.action(),
                Effect::Menu(Menu::typed(
                    "typed-secret-menu",
                    "SECRET typed title",
                    "SECRET initial value",
                )),
            ],
            crate::view::Key::Char('d') => vec![Effect::Act {
                part: "beta",
                subject: crate::compose::Subject::new("probe-item", "42", "SECRET label")
                    .with("title", "SECRET subject text"),
                id: "SECRET action value".into(),
            }],
            crate::view::Key::Char('e') => vec![
                self.action(),
                Effect::Emit {
                    topic: "probe.changed",
                    payload: serde_json::json!({"secret": "SECRET payload"}),
                },
            ],
            crate::view::Key::Char('f') => vec![Effect::Focus(Pane::Detail)],
            crate::view::Key::Char('s') => vec![Effect::Show(crate::compose::Subject::new(
                "probe-item",
                "42",
                "SECRET label",
            ))],
            crate::view::Key::Char('v') => vec![Effect::Open(
                "https://user:SECRET@host.test/reset/glpat-abc123456?token=SECRET#SECRET".into(),
            )],
            crate::view::Key::Char('o') => vec![Effect::Spawn(Box::pin(async { Box::new(()) as Msg }))],
            _ => vec![],
        })
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        match msg.downcast::<JournalOutcome>() {
            Ok(result) => vec![Effect::Report(*result)],
            Err(msg) if msg.is::<Prepared>() => vec![Effect::Menu(Menu::confirm(
                "prepare", "ready?", "send", "stop",
            ))],
            Err(msg) if msg.is::<ReadPrepared>() => vec![
                Effect::Observe {
                    action: ActionDescriptor::new("probe.summary", Origin::Plugin(self.id.into())),
                    outcome: JournalOutcome::Succeeded,
                },
                self.task(),
            ],
            Err(_) => vec![],
        }
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        if menu == "typed-secret-menu" || value == "yes" {
            vec![
                Effect::Report(JournalOutcome::Confirmed),
                Effect::Report(JournalOutcome::Started),
                self.task(),
            ]
        } else {
            vec![Effect::Report(JournalOutcome::Cancelled)]
        }
    }
    fn on_bus(&mut self, _: &str, _: &serde_json::Value) -> Vec<Effect> {
        vec![self.task()]
    }
    fn act(&mut self, _: &crate::compose::Subject, _: &str) -> Vec<Effect> {
        vec![self.action(), self.task()]
    }
}

fn host() -> Host {
    let plugins = vec![
        Box::new(Probe {
            id: "alpha",
            key: 'A',
            result: JournalOutcome::Succeeded,
            capturing: false,
        }) as Box<dyn Plugin>,
        Box::new(Probe {
            id: "beta",
            key: 'B',
            result: JournalOutcome::Failed,
            capturing: false,
        }) as Box<dyn Plugin>,
        Box::new(Probe {
            id: "gamma",
            key: 'C',
            result: JournalOutcome::Partial,
            capturing: false,
        }) as Box<dyn Plugin>,
    ];
    let mut host = Host::new(plugins, 0).unwrap();
    host.enable_journal();
    host
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn records(effects: &[HostEffect]) -> Vec<JournalRecord> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Journal(record) => Some(record.clone()),
            _ => None,
        })
        .collect()
}

fn spawn(effects: Vec<HostEffect>) -> (ActionId, Option<ActionId>, usize, crate::plugin::Task) {
    spawns(effects)
        .into_iter()
        .next()
        .expect("a correlated task")
}

fn spawns(
    effects: Vec<HostEffect>,
) -> Vec<(ActionId, Option<ActionId>, usize, crate::plugin::Task)> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            HostEffect::Tracked {
                action,
                semantic,
                effect,
            } => match *effect {
                HostEffect::Spawn { plugin, task } => Some((action, semantic, plugin, task)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn begun(records: &[JournalRecord], kind: &str) -> ActionId {
    records
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. } if descriptor.kind.as_str() == kind => {
                Some(id.clone())
            }
            _ => None,
        })
        .expect("action was begun")
}

fn parent(records: &[JournalRecord], id: &ActionId) -> Option<ActionId> {
    records
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin {
                id: existing,
                parent,
                ..
            } if existing == id => Some(parent.clone()),
            _ => None,
        })
        .expect("action was begun")
}

fn has_outcome(records: &[JournalRecord], id: &ActionId, expected: JournalOutcome) -> bool {
    records.iter().any(|record| matches!(record,
        JournalRecord::Outcome { id: existing, outcome, .. } if existing == id && *outcome == expected))
}

#[tokio::test]
async fn scoped_actions_restore_following_spawn_emit_and_report_context() {
    let mut host = host();
    let effects = host.update(key(KeyCode::Char('c')));
    let records = records(&effects);
    let outer = begun(&records, "probe.change");
    let read = begun(&records, "probe.read");
    let other = begun(&records, "probe.other");
    let skipped = begun(&records, "probe.skipped");
    let emit = begun(&records, "bus.emit");
    assert_eq!(parent(&records, &read), Some(outer.clone()));
    assert_eq!(parent(&records, &other), Some(outer.clone()));
    assert_eq!(parent(&records, &skipped), Some(outer.clone()));
    assert_eq!(parent(&records, &emit), Some(outer.clone()));
    assert!(has_outcome(&records, &outer, JournalOutcome::Started));
    assert!(has_outcome(&records, &skipped, JournalOutcome::Blocked));
    assert!(!has_outcome(&records, &outer, JournalOutcome::Blocked));
    assert!(!has_outcome(&records, &read, JournalOutcome::Started));
    let tasks = spawns(effects);
    assert_eq!(tasks.len(), 5);
    assert_eq!(tasks[0].1, Some(read.clone()));
    assert_eq!(parent(&records, &tasks[0].0), Some(read));
    assert_eq!(tasks[1].1, Some(other.clone()));
    assert_eq!(parent(&records, &tasks[1].0), Some(other));
    assert_eq!(tasks[2].1, Some(outer.clone()));
    assert_eq!(parent(&records, &tasks[2].0), Some(outer.clone()));
    for (action, semantic, plugin, _) in &tasks[3..] {
        assert!(semantic.is_none(), "bus recipients keep independent intent");
        assert_ne!(*plugin, 0);
        let delivery = parent(&records, action).unwrap();
        assert_eq!(parent(&records, &delivery), Some(emit.clone()));
    }
    assert!(!format!("{records:?}").contains("SECRET"));
    for (action, semantic, plugin, task) in tasks.into_iter().take(2).rev() {
        let done = host.update(tracked_event(
            Event::Msg {
                plugin,
                msg: task.await,
            },
            Some(action),
            semantic,
        ));
        assert!(
            !super::journal_tests::records(&done)
                .iter()
                .any(|record| matches!(record,
            JournalRecord::Outcome { id, .. } if id == &outer))
        );
    }
}

#[tokio::test]
async fn spawn_action_siblings_complete_out_of_order_without_claiming_unrelated_work() {
    let mut host = host();
    let effects = host.update(key(KeyCode::Char('u')));
    let started = records(&effects);
    let first = begun(&started, "probe.first");
    let second = begun(&started, "probe.second");
    let causal = begun(&started, "plugin.input");
    assert_ne!(first, second);
    assert_eq!(parent(&started, &first), Some(causal.clone()));
    assert_eq!(parent(&started, &second), Some(causal.clone()));
    let tasks = spawns(effects);
    assert_eq!(tasks.len(), 3);
    assert_eq!(tasks[0].1, Some(first.clone()));
    assert_eq!(tasks[1].1, Some(second.clone()));
    assert_eq!(tasks[2].1, None);
    assert_eq!(parent(&started, &tasks[2].0), Some(causal));
    let expected = [JournalOutcome::Succeeded, JournalOutcome::Failed];
    for (index, (action, semantic, plugin, task)) in tasks.into_iter().take(2).enumerate().rev() {
        assert_eq!(parent(&started, &action), semantic);
        let done = records(&host.update(tracked_event(
            Event::Msg {
                plugin,
                msg: task.await,
            },
            Some(action.clone()),
            semantic.clone(),
        )));
        assert!(has_outcome(&done, &action, expected[index]));
        assert!(has_outcome(
            &done,
            semantic.as_ref().unwrap(),
            expected[index]
        ));
        let other = if index == 0 { &second } else { &first };
        assert!(!done.iter().any(|record| matches!(record,
            JournalRecord::Outcome { id, .. } if id == other)));
        assert!(!has_outcome(&done, &action, JournalOutcome::Unknown));
    }
}

#[tokio::test]
async fn spawn_action_result_continuation_retains_its_semantic_action() {
    let mut host = host();
    let effects = host.update(key(KeyCode::Char('r')));
    let read = begun(&records(&effects), "probe.read");
    let (preparation, semantic, plugin, task) = spawn(effects);
    assert_eq!(semantic, Some(read.clone()));
    let continued = host.update(tracked_event(
        Event::Msg {
            plugin,
            msg: task.await,
        },
        Some(preparation.clone()),
        semantic,
    ));
    let continuation = records(&continued);
    let summary = begun(&continuation, "probe.summary");
    assert_eq!(parent(&continuation, &summary), Some(preparation.clone()));
    assert!(has_outcome(
        &continuation,
        &summary,
        JournalOutcome::Succeeded
    ));
    assert!(!has_outcome(
        &continuation,
        &preparation,
        JournalOutcome::Succeeded
    ));
    let (action, semantic, plugin, task) = spawn(continued);
    assert_eq!(parent(&continuation, &action), Some(preparation.clone()));
    assert_eq!(semantic, Some(read.clone()));
    assert!(has_outcome(
        &continuation,
        &preparation,
        JournalOutcome::Unknown
    ));
    assert!(!continuation.iter().any(|record| matches!(record,
        JournalRecord::Outcome { id, .. } if id == &read)));
    let done = records(&host.update(tracked_event(
        Event::Msg {
            plugin,
            msg: task.await,
        },
        Some(action.clone()),
        semantic,
    )));
    assert!(has_outcome(&done, &read, JournalOutcome::Succeeded));
    assert!(has_outcome(&done, &action, JournalOutcome::Succeeded));
}

#[tokio::test]
async fn spawn_action_still_runs_when_journaling_is_disabled() {
    let mut host = Host::new(host().plugins, 0).unwrap();
    let effects = host.update(key(KeyCode::Char('r')));
    assert!(matches!(
        &effects[..],
        [HostEffect::Spawn { plugin: 0, .. }]
    ));
    let HostEffect::Spawn { plugin, task } = effects.into_iter().next().unwrap() else {
        unreachable!()
    };
    let continued = host.update(Event::Msg {
        plugin,
        msg: task.await,
    });
    assert!(matches!(
        &continued[..],
        [HostEffect::Spawn { plugin: 0, .. }]
    ));
}

#[tokio::test]
async fn out_of_order_providers_keep_their_own_action_and_outcome() {
    let mut host = host();
    let first = host.update(key(KeyCode::Char('a')));
    let first_records = records(&first);
    let (first_action, first_semantic, first_plugin, first_task) = spawn(first);
    host.view = View::Screen(1);
    let second = host.update(key(KeyCode::Char('a')));
    let second_records = records(&second);
    let (second_action, second_semantic, second_plugin, second_task) = spawn(second);
    assert_ne!(first_action, second_action);
    assert_ne!(first_semantic, second_semantic);
    assert_ne!(
        parent(&first_records, &first_action),
        parent(&second_records, &second_action)
    );
    let second_done = host.update(tracked_event(
        Event::Msg {
            plugin: second_plugin,
            msg: second_task.await,
        },
        Some(second_action.clone()),
        second_semantic.clone(),
    ));
    let first_done = host.update(tracked_event(
        Event::Msg {
            plugin: first_plugin,
            msg: first_task.await,
        },
        Some(first_action.clone()),
        first_semantic.clone(),
    ));
    assert!(has_outcome(
        &records(&second_done),
        second_semantic.as_ref().unwrap(),
        JournalOutcome::Failed
    ));
    assert!(has_outcome(
        &records(&first_done),
        first_semantic.as_ref().unwrap(),
        JournalOutcome::Succeeded
    ));
    assert!(has_outcome(
        &records(&second_done),
        &second_action,
        JournalOutcome::Failed
    ));
    assert!(has_outcome(
        &records(&first_done),
        &first_action,
        JournalOutcome::Succeeded
    ));
    assert!(!has_outcome(
        &records(&second_done),
        &first_action,
        JournalOutcome::Failed
    ));
    assert!(!has_outcome(
        &records(&first_done),
        &first_action,
        JournalOutcome::Unknown
    ));
}

#[tokio::test]
async fn bus_fanout_keeps_distinct_provider_children_under_one_emit() {
    let mut host = host();
    let effects = host.update(key(KeyCode::Char('e')));
    let records = records(&effects);
    let emit = begun(&records, "bus.emit");
    let deliveries = records
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } if descriptor.kind.as_str() == "bus.deliver" => {
                assert_eq!(parent, &Some(emit.clone()));
                Some((id.clone(), descriptor.origin.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(deliveries.len(), 2);
    assert_ne!(deliveries[0].0, deliveries[1].0);
    assert_eq!(deliveries[0].1, Origin::Plugin("beta".into()));
    assert_eq!(deliveries[1].1, Origin::Plugin("gamma".into()));
    for record in &records {
        if let JournalRecord::Begin {
            descriptor, parent, ..
        } = record
            && descriptor.kind.as_str() == "runtime.task"
        {
            assert!(deliveries.iter().any(|(id, _)| parent.as_ref() == Some(id)));
        }
    }
    assert!(!format!("{records:?}").contains("SECRET"));
    let emitter_semantic = begun(&records, "probe.change");
    for effect in effects {
        if let HostEffect::Tracked {
            action,
            semantic,
            effect,
        } = effect
            && let HostEffect::Spawn { plugin, task } = *effect
        {
            assert!(semantic.is_none());
            let done = host.update(tracked_event(
                Event::Msg {
                    plugin,
                    msg: task.await,
                },
                Some(action),
                semantic,
            ));
            assert!(!super::journal_tests::records(&done).iter().any(|record| matches!(record,
                JournalRecord::Outcome { id, outcome, .. } if id == &emitter_semantic && outcome.is_terminal())));
        }
    }
}

#[test]
fn menu_cancel_and_confirmation_retain_the_original_semantic_parent() {
    let mut host = host();
    let opened = host.update(key(KeyCode::Char('m')));
    let opened_records = records(&opened);
    let menu = begun(&opened_records, "ui.menu");
    let intent = begun(&opened_records, "probe.change");
    // An unrelated completion while the menu remains open must not own it.
    host.update(Event::Tracked {
        action: ActionId::new(),
        semantic: None,
        event: Box::new(Event::Notice("unrelated".into())),
    });
    let accepted = host.update(key(KeyCode::Char('y')));
    let accepted_records = records(&accepted);
    assert!(has_outcome(
        &accepted_records,
        &menu,
        JournalOutcome::Confirmed
    ));
    assert!(has_outcome(
        &accepted_records,
        &intent,
        JournalOutcome::Started
    ));
    let (task, _, _, _) = spawn(accepted);
    assert_eq!(parent(&accepted_records, &task), Some(intent));
    assert!(!format!("{opened_records:?}{accepted_records:?}").contains("SECRET"));

    let opened = host.update(key(KeyCode::Char('m')));
    let menu = begun(&records(&opened), "ui.menu");
    let cancelled = host.update(key(KeyCode::Esc));
    assert!(has_outcome(
        &records(&cancelled),
        &menu,
        JournalOutcome::Cancelled
    ));
    assert!(host.menu.is_none());

    let opened = host.update(key(KeyCode::Char('m')));
    let menu = begun(&records(&opened), "ui.menu");
    let declined = host.update(key(KeyCode::Char('n')));
    assert!(has_outcome(
        &records(&declined),
        &menu,
        JournalOutcome::Cancelled
    ));
}

#[tokio::test]
async fn typed_menu_keeps_text_private_and_retains_confirmation_and_cancellation() {
    let mut host = host();
    let opened = records(&host.update(key(KeyCode::Char('t'))));
    let menu = begun(&opened, "ui.menu");
    let intent = begun(&opened, "probe.change");
    assert!(records(&host.update(key(KeyCode::Char('S')))).is_empty());
    assert!(records(&host.update(Event::Paste("SECRET paste".into()))).is_empty());
    assert!(records(&host.update(key(KeyCode::Backspace))).is_empty());
    host.update(Event::Tracked {
        action: ActionId::new(),
        semantic: None,
        event: Box::new(Event::Notice("unrelated".into())),
    });
    let accepted = host.update(key(KeyCode::Enter));
    let accepted_records = records(&accepted);
    assert!(has_outcome(
        &accepted_records,
        &menu,
        JournalOutcome::Confirmed
    ));
    assert!(has_outcome(
        &accepted_records,
        &menu,
        JournalOutcome::Succeeded
    ));
    assert!(has_outcome(
        &accepted_records,
        &intent,
        JournalOutcome::Started
    ));
    let (task_id, semantic, plugin, task) = spawn(accepted);
    assert_eq!(parent(&accepted_records, &task_id), Some(intent.clone()));
    assert_eq!(semantic, Some(intent.clone()));
    let completed = records(&host.update(tracked_event(
        Event::Msg {
            plugin,
            msg: task.await,
        },
        Some(task_id),
        semantic,
    )));
    assert!(has_outcome(&completed, &intent, JournalOutcome::Succeeded));
    assert!(!format!("{opened:?}{accepted_records:?}{completed:?}").contains("SECRET"));

    let reopened = records(&host.update(key(KeyCode::Char('t'))));
    let menu = begun(&reopened, "ui.menu");
    let intent = begun(&reopened, "probe.change");
    let cancelled = records(&host.update(key(KeyCode::Esc)));
    assert!(has_outcome(&cancelled, &menu, JournalOutcome::Cancelled));
    assert!(has_outcome(&cancelled, &intent, JournalOutcome::Cancelled));
    assert!(host.menu.is_none());
}

#[tokio::test]
async fn action_effect_retains_its_owner_subject_and_semantic_task_correlation() {
    let mut host = host();
    let dispatched = host.update(key(KeyCode::Char('d')));
    let dispatched_records = records(&dispatched);
    let dispatch = begun(&dispatched_records, "plugin.action.dispatch");
    let action = begun(&dispatched_records, "plugin.action");
    let intent = begun(&dispatched_records, "probe.change");
    assert_eq!(parent(&dispatched_records, &action), Some(dispatch));
    assert_eq!(parent(&dispatched_records, &intent), Some(action.clone()));
    let descriptor = dispatched_records
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. } if id == &action => Some(descriptor),
            _ => None,
        })
        .unwrap();
    assert_eq!(descriptor.origin, Origin::Plugin("beta".into()));
    assert_eq!(descriptor.subjects[0].key.provider, "alpha");
    assert_eq!(descriptor.subjects[0].key.instance, "instance-one");
    assert_eq!(descriptor.subjects[0].key.id, "42");
    let (task_id, semantic, plugin, task) = spawn(dispatched);
    assert_eq!(plugin, 1);
    assert_eq!(parent(&dispatched_records, &task_id), Some(intent.clone()));
    assert_eq!(semantic, Some(intent.clone()));
    let completed = records(&host.update(tracked_event(
        Event::Msg {
            plugin,
            msg: task.await,
        },
        Some(task_id.clone()),
        semantic,
    )));
    assert!(has_outcome(&completed, &intent, JournalOutcome::Failed));
    assert!(has_outcome(&completed, &task_id, JournalOutcome::Failed));
    assert!(!format!("{dispatched_records:?}{completed:?}").contains("SECRET"));
}

#[tokio::test]
async fn opaque_completion_is_unknown_and_idle_ticks_are_silent() {
    let mut host = host();
    assert!(records(&host.update(Event::Tick(1))).is_empty());
    let (action, semantic, plugin, task) = spawn(host.update(key(KeyCode::Char('o'))));
    let done = host.update(tracked_event(
        Event::Msg {
            plugin,
            msg: task.await,
        },
        Some(action.clone()),
        semantic,
    ));
    assert!(has_outcome(
        &records(&done),
        &action,
        JournalOutcome::Unknown
    ));
    assert!(!has_outcome(
        &records(&done),
        &action,
        JournalOutcome::Succeeded
    ));
}

/// Typing into a field through `Intent::Key` (Headless, the Ink renderer)
/// is journalled as the terminal's keys are: nothing per key, the field's
/// Enter as `input.commit` and its Esc as `input.cancel`.
#[test]
fn typing_through_intents_is_journalled_as_terminal_keys_are() {
    use crate::view::{Chord, Intent, Key};
    let press = |host: &mut Host, terminal: bool, k: Key| {
        let event = if terminal {
            key(match k {
                Key::Char(c) => KeyCode::Char(c),
                Key::Enter => KeyCode::Enter,
                Key::Esc => KeyCode::Esc,
                _ => unreachable!(),
            })
        } else {
            Event::Intent(Intent::Key { chord: Chord::new(k) })
        };
        records(&host.update(event))
    };
    let kinds = |records: &[JournalRecord]| -> Vec<String> {
        records
            .iter()
            .filter_map(|r| match r {
                JournalRecord::Begin { descriptor, .. } => Some(descriptor.kind.as_str().to_string()),
                _ => None,
            })
            .collect()
    };
    let mut seen = Vec::new();
    for terminal in [true, false] {
        let mut host = host();
        press(&mut host, terminal, Key::Char('t'));
        assert!(host.menu.as_ref().is_some_and(|m| m.input.is_some()), "a typed menu");
        for c in "SECRET".chars() {
            assert!(press(&mut host, terminal, Key::Char(c)).is_empty(), "a key in the field is not recorded");
        }
        let accepted = press(&mut host, terminal, Key::Enter);
        begun(&accepted, "input.commit");
        press(&mut host, terminal, Key::Char('t'));
        let cancelled = press(&mut host, terminal, Key::Esc);
        begun(&cancelled, "input.cancel");
        assert!(host.menu.is_none());
        seen.push((kinds(&accepted), kinds(&cancelled)));
    }
    assert_eq!(seen[0], seen[1], "the terminal and intents record the same");
}

#[test]
fn state_only_dispatches_are_recorded_without_keys_and_typing_is_not() {
    let mut host = host();
    let effects = host.update(key(KeyCode::Char('f')));
    let records = records(&effects);
    let focus = begun(&records, "ui.focus");
    assert!(has_outcome(&records, &focus, JournalOutcome::Succeeded));
    assert_eq!(host.focus, Pane::Detail);
    assert!(records.iter().any(|record| matches!(record,
        JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "plugin.input" && descriptor.origin == Origin::Plugin("alpha".into()))));
    host.plugins[0] = Box::new(Probe {
        id: "alpha",
        key: 'A',
        result: JournalOutcome::Succeeded,
        capturing: true,
    });
    assert!(host.update(key(KeyCode::Char('S'))).is_empty());
    assert!(host.update(Event::Paste("SECRET paste".into())).is_empty());
}

#[test]
fn cli_descriptor_only_retains_declared_command_names() {
    let matches = clap::Command::new("radar")
        .subcommand(
            clap::Command::new("part")
                .subcommand(clap::Command::new("post").arg(clap::Arg::new("text"))),
        )
        .try_get_matches_from(["radar", "part", "post", "SECRET text"])
        .unwrap();
    let descriptor = cli_descriptor(&matches, &[]);
    assert_eq!(descriptor.metadata.command.unwrap().as_str(), "part.post");
    assert_eq!(descriptor.kind.as_str(), "cli.command");
}

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("radar-runtime-journal-{}", ActionId::new().as_str())))
    }
    fn path(&self) -> PathBuf {
        self.0.join("actions.sqlite3")
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn typed_action_runs_from_request_through_confirmation_to_sqlite_outcome() {
    let temp = TestDir::new();
    let mut recording = Recording::open_at(&temp.path(), SessionOrigin::Tui).unwrap();
    let mut host = host();
    let (tx, mut rx) = unbounded_channel();
    for effect in host.update(key(KeyCode::Char('m'))) {
        perform(effect, &tx, Some(&recording.sink));
    }
    for effect in host.update(key(KeyCode::Char('y'))) {
        perform(effect, &tx, Some(&recording.sink));
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    for effect in host.update(result) {
        perform(effect, &tx, Some(&recording.sink));
    }
    recording.finish();
    assert_eq!(recording.sink.take_warning(), None);
    let journal = Journal::open(&temp.path()).unwrap();
    let events = journal
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    let semantic = events
        .iter()
        .find(|e| e.kind.as_str() == "probe.change")
        .unwrap();
    let task = events
        .iter()
        .find(|e| e.kind.as_str() == "runtime.task" && e.outcome == JournalOutcome::Succeeded)
        .unwrap();
    assert_eq!(task.parent_action_id.as_ref(), Some(&semantic.action_id));
    assert_eq!(task.root_action_id, semantic.root_action_id);
    assert_eq!(task.correlation_id, semantic.correlation_id);
    assert_eq!(task.origin, Origin::Plugin("alpha".into()));
    assert!(
        events
            .iter()
            .any(|e| e.action_id == semantic.action_id && e.outcome == JournalOutcome::Confirmed)
    );
    assert!(
        events
            .iter()
            .any(|e| e.action_id == semantic.action_id && e.outcome == JournalOutcome::Started)
    );
    assert!(
        !events
            .iter()
            .any(|e| e.action_id == task.action_id && e.outcome == JournalOutcome::Unknown)
    );
    let outcomes = journal
        .query(&Query {
            kind: Some(StableCode::new("probe.change")),
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(outcomes.last().unwrap().outcome, JournalOutcome::Succeeded);
    assert!(!serde_json::to_string(&events).unwrap().contains("SECRET"));
}

#[cfg(not(unix))]
#[test]
fn runtime_fails_closed_without_private_storage_support() {
    let temp = TestDir::new();
    assert!(Recording::open_at(&temp.path(), SessionOrigin::Tui).is_err());
}

#[tokio::test]
async fn preparation_menu_and_unrelated_completion_keep_semantic_intent() {
    let mut host = host();
    host.view = View::Screen(1);
    let (other_task, other_semantic, other_plugin, other) =
        spawn(host.update(key(KeyCode::Char('a'))));
    host.view = View::Screen(0);
    let (prepare_task, semantic, plugin, prepare) = spawn(host.update(key(KeyCode::Char('p'))));
    let prepared = host.update(tracked_event(
        Event::Msg {
            plugin,
            msg: prepare.await,
        },
        Some(prepare_task.clone()),
        semantic.clone(),
    ));
    assert!(has_outcome(
        &records(&prepared),
        &prepare_task,
        JournalOutcome::Unknown
    ));
    assert!(host.menu.is_some());
    let unrelated = host.update(tracked_event(
        Event::Msg {
            plugin: other_plugin,
            msg: other.await,
        },
        Some(other_task),
        other_semantic.clone(),
    ));
    assert!(has_outcome(
        &records(&unrelated),
        other_semantic.as_ref().unwrap(),
        JournalOutcome::Failed
    ));
    assert!(host.menu.is_some());
    let confirmed = host.update(key(KeyCode::Char('y')));
    let (task, resumed_semantic, plugin, work) = spawn(confirmed);
    assert_eq!(resumed_semantic, semantic);
    let done = host.update(tracked_event(
        Event::Msg {
            plugin,
            msg: work.await,
        },
        Some(task),
        resumed_semantic,
    ));
    assert!(has_outcome(
        &records(&done),
        semantic.as_ref().unwrap(),
        JournalOutcome::Succeeded
    ));
    assert!(!has_outcome(
        &records(&done),
        other_semantic.as_ref().unwrap(),
        JournalOutcome::Succeeded
    ));
}

#[test]
fn show_and_open_use_scoped_subjects_and_sanitized_targets() {
    let mut host = host();
    let shown = records(&host.update(key(KeyCode::Char('s'))));
    let descriptor = shown
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "ui.show" => {
                Some(descriptor)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(descriptor.subjects[0].key.instance, "instance-one");
    assert_eq!(descriptor.subjects[0].key.id, "42");
    let opened = records(&host.update(key(KeyCode::Char('v'))));
    let descriptor = opened
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { descriptor, .. }
                if descriptor.kind.as_str() == "system.open" =>
            {
                Some(descriptor)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(
        descriptor.metadata.url.as_ref().unwrap().as_str(),
        "https://host.test/"
    );
    assert!(descriptor.metadata.redacted);
    assert!(!format!("{shown:?}{opened:?}").contains("SECRET"));
    assert!(!format!("{opened:?}").contains("glpat-abc123456"));
    assert!(!format!("{opened:?}").contains("/reset"));
}

#[cfg(unix)]
#[tokio::test]
async fn panicked_task_records_unknown_without_panic_payload() {
    let temp = TestDir::new();
    let mut recording = Recording::open_at(&temp.path(), SessionOrigin::Tui).unwrap();
    let sink = &recording.sink;
    let outer = sink.begin("probe.outer", Origin::Plugin("alpha".into()), None);
    let semantic = sink.begin(
        "probe.panics",
        Origin::Plugin("alpha".into()),
        Some(outer.clone()),
    );
    let action = sink.begin(
        "runtime.task",
        Origin::Plugin("alpha".into()),
        Some(semantic.clone()),
    );
    let (tx, mut rx) = unbounded_channel();
    perform(
        HostEffect::Tracked {
            action: action.clone(),
            semantic: Some(semantic.clone()),
            effect: Box::new(HostEffect::Spawn {
                plugin: 0,
                task: Box::pin(async { panic!("SECRET panic payload") }),
            }),
        },
        &tx,
        Some(sink),
    );
    let event = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, Event::Notice(ref note) if note.contains("outcome is unknown")));
    recording.finish();
    assert_eq!(recording.sink.take_warning(), None);
    let journal = Journal::open(&temp.path()).unwrap();
    let events = journal
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    for id in [&action, &semantic] {
        assert!(events.iter().any(|event| &event.action_id == id
            && event.outcome == JournalOutcome::Unknown
            && event.metadata.reason == Some(Reason::Interrupted)));
    }
    let outer_terminal = events
        .iter()
        .filter(|event| event.action_id == outer && event.outcome.is_terminal())
        .collect::<Vec<_>>();
    assert_eq!(outer_terminal.len(), 1);
    assert_eq!(outer_terminal[0].outcome, JournalOutcome::Unknown);
    assert_eq!(
        outer_terminal[0].metadata.reason,
        Some(Reason::NoTerminalObservation)
    );
    assert!(!serde_json::to_string(&events).unwrap().contains("SECRET"));
}

#[path = "runner_journal_tests/action_context.rs"]
mod action_context;
