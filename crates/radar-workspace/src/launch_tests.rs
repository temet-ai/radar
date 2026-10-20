//! Mocked launch observations through the real reducers and host correlation.
//! No applications, clipboard writes, filesystem scans or databases are used.

use super::*;
use radar_core::Host;
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::ActionId;
use radar_core::view::{Chord, Intent, Key};

#[test]
fn launch_scopes_capture_workspace_and_exact_target_without_ui_text() {
    let mut space = Space::new("~/work");
    space.editor = "SECRET_COMMAND".into();
    let top = config::journal_path(&space.dir)
        .unwrap()
        .display()
        .to_string();
    let target = Target::File(format!("{top}/repo/README.md"));
    let Effect::Journal(action) = journal_launch("workspace.editor.launch", &space, &target) else {
        panic!("launch descriptor")
    };
    assert_eq!(action.subjects.len(), 2);
    assert_eq!(action.subjects[0].key.instance, top);
    assert_eq!(action.subjects[0].role, SubjectRole::Primary);
    assert_eq!(action.subjects[1].key.kind, "note");
    assert_eq!(action.subjects[1].key.id, format!("{top}/repo/README.md"));
    assert_eq!(action.subjects[1].role, SubjectRole::Destination);
    assert!(!format!("{action:?}").contains("SECRET_COMMAND"));
    let Effect::Journal(unsafe_target) = journal_launch(
        "workspace.editor.launch",
        &space,
        &Target::File("/work/token=SECRET".into()),
    ) else {
        panic!("launch descriptor")
    };
    assert!(unsafe_target.metadata.redacted);
    assert_eq!(unsafe_target.metadata.omitted_subjects, 1);
    assert_eq!(unsafe_target.subjects.len(), 1);
    assert!(!format!("{unsafe_target:?}").contains("SECRET"));
}

#[test]
fn launch_results_preserve_notifications_and_never_report_clipboard_success() {
    let mut p = WorkspacePlugin::new();
    for (result, expected, notification, copy) in [
        (
            FolderLaunch::Dispatched("Editor".into()),
            JournalOutcome::Succeeded,
            "opened Editor",
            false,
        ),
        (
            FolderLaunch::Unavailable {
                why: "no desktop".into(),
                by_hand: "command".into(),
            },
            JournalOutcome::Blocked,
            "no desktop: `command` is on your clipboard",
            true,
        ),
        (
            FolderLaunch::Unconfirmed {
                why: "error".into(),
                by_hand: "command".into(),
            },
            JournalOutcome::Unknown,
            "could not open Editor (error): `command` is on your clipboard",
            true,
        ),
    ] {
        let fx = p.on_work(WorkMsg::Launched {
            result,
            what: "Editor".into(),
        });
        assert!(matches!(fx[0], Effect::Report(outcome) if outcome == expected));
        assert_eq!(
            fx.iter().filter(|e| matches!(e, Effect::Copy(_))).count(),
            usize::from(copy)
        );
        if expected == JournalOutcome::Succeeded {
            assert!(matches!(fx.last(), Some(Effect::Notify(said)) if said == notification));
        } else {
            assert!(matches!(fx.last(), Some(Effect::Notice(message))
                if message.tone == radar_core::view::chrome::NoticeTone::Error && message.text == notification));
        }
        assert_eq!(
            fx.iter().filter(|e| matches!(e, Effect::Report(_))).count(),
            1
        );
    }
    assert!(matches!(
        p.on_work(WorkMsg::Failed("SECRET_ERROR".into()))[0],
        Effect::Report(JournalOutcome::Unknown)
    ));
    let no_editor = p.on_work(WorkMsg::PickEditor {
        menu: "unused".into(),
        why: "SECRET_COMMAND".into(),
        installed: vec![],
    });
    assert!(matches!(
        no_editor[0],
        Effect::Report(JournalOutcome::Blocked)
    ));
    assert!(!no_editor.iter().any(|e| matches!(e, Effect::Menu(_))));
}

// The harness only enters a production launch path. All task replies, editor
// choices, effect routing and cancellation use the real plugin and host code.
enum Begin {
    Editor(Space, Target),
    Terminal(Space, String),
}

struct Launches(WorkspacePlugin);
impl Plugin for Launches {
    fn id(&self) -> &'static str {
        "workspace"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        self.0.config()
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        match msg.downcast::<Begin>() {
            Ok(begin) => match *begin {
                Begin::Editor(space, target) => {
                    self.0.cfg.spaces = vec![space.clone()];
                    self.0.open_editor(&space, target)
                }
                Begin::Terminal(space, dir) => {
                    self.0.cfg.spaces = vec![space.clone()];
                    self.0.open_terminal(&space, dir)
                }
            },
            Err(msg) => self.0.on_msg(msg),
        }
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        self.0.on_menu(menu, value)
    }
}

fn host() -> Host {
    let mut host = Host::new(vec![Box::new(Launches(WorkspacePlugin::new()))], 1).unwrap();
    host.enable_journal();
    host
}

fn begin(host: &mut Host, begin: Begin) -> Vec<HostEffect> {
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(begin),
    })
}

fn task(fx: &[HostEffect]) -> (ActionId, Option<ActionId>) {
    fx.iter()
        .find_map(|e| match e {
            HostEffect::Tracked {
                action,
                semantic,
                effect,
            } if matches!(&**effect, HostEffect::Spawn { .. }) => {
                Some((action.clone(), semantic.clone()))
            }
            _ => None,
        })
        .expect("launch task")
}

fn reply(host: &mut Host, context: (ActionId, Option<ActionId>), msg: WorkMsg) -> Vec<HostEffect> {
    host.update(Event::Tracked {
        action: context.0,
        semantic: context.1,
        event: Box::new(Event::Msg {
            plugin: 0,
            msg: Box::new(msg),
        }),
    })
}

fn records(fx: &[HostEffect]) -> Vec<&JournalRecord> {
    fx.iter()
        .filter_map(|e| match e {
            HostEffect::Journal(record) => Some(record),
            _ => None,
        })
        .collect()
}

fn terminal(fx: &[HostEffect], id: &ActionId) -> Vec<JournalOutcome> {
    records(fx)
        .iter()
        .filter_map(|r| match r {
            JournalRecord::Outcome {
                id: found, outcome, ..
            } if found == id && outcome.is_terminal() => Some(*outcome),
            _ => None,
        })
        .collect()
}

#[test]
fn concurrent_launch_replies_keep_action_time_scopes_and_copy_as_separate_dispatch() {
    let mut host = host();
    let first = begin(
        &mut host,
        Begin::Editor(
            Space::new("/work/first"),
            Target::File("/work/first/README.md".into()),
        ),
    );
    let a = task(&first);
    let second = begin(
        &mut host,
        Begin::Terminal(Space::new("/work/second"), "/work/second/repo".into()),
    );
    let b = task(&second);
    assert_ne!(a.1, b.1);
    for (fx, scope) in [(&first, "/work/first"), (&second, "/work/second")] {
        let descriptor = records(fx)
            .into_iter()
            .find_map(|r| match r {
                JournalRecord::Begin { descriptor, .. }
                    if descriptor.kind.as_str().starts_with("workspace.") =>
                {
                    Some(descriptor)
                }
                _ => None,
            })
            .unwrap();
        assert!(descriptor.subjects.iter().all(|s| s.key.instance == scope));
    }
    let b_done = reply(
        &mut host,
        b.clone(),
        WorkMsg::Launched {
            result: FolderLaunch::Unconfirmed {
                why: "SECRET_ERROR".into(),
                by_hand: "SECRET_COMMAND".into(),
            },
            what: "SECRET_LABEL".into(),
        },
    );
    assert_eq!(
        terminal(&b_done, b.1.as_ref().unwrap()),
        [JournalOutcome::Unknown]
    );
    assert!(terminal(&b_done, a.1.as_ref().unwrap()).is_empty());
    let copy = records(&b_done)
        .into_iter()
        .find_map(|r| match r {
            JournalRecord::Begin {
                id,
                parent,
                descriptor,
            } if descriptor.kind.as_str() == "system.copy" => {
                assert_eq!(parent.as_ref(), Some(&b.0));
                Some(id.clone())
            }
            _ => None,
        })
        .expect("separate clipboard dispatch");
    assert!(
        terminal(&b_done, &copy).is_empty(),
        "a queued copy is not observed clipboard success"
    );
    assert!(!format!("{:?}", records(&b_done)).contains("SECRET"));
    let a_done = reply(
        &mut host,
        a.clone(),
        WorkMsg::Launched {
            result: FolderLaunch::Dispatched("SECRET_STARTED".into()),
            what: "editor".into(),
        },
    );
    assert_eq!(
        terminal(&a_done, a.1.as_ref().unwrap()),
        [JournalOutcome::Succeeded]
    );
    assert!(terminal(&a_done, b.1.as_ref().unwrap()).is_empty());
    assert!(!format!("{:?}", records(&a_done)).contains("SECRET"));
}

fn editor_menu(host: &mut Host) -> (ActionId, Vec<HostEffect>) {
    let fx = begin(
        host,
        Begin::Editor(
            Space::new("/work/first"),
            Target::File("/work/first/README.md".into()),
        ),
    );
    let context = task(&fx);
    let action = context.1.clone().unwrap();
    let menu = reply(
        host,
        context,
        WorkMsg::PickEditor {
            menu: format!("{PICK_EDITOR}:/work/first\tfile\t/work/first/README.md"),
            why: "SECRET_MISSING_EDITOR".into(),
            installed: vec![("SECRET_EDITOR".into(), "Editor".into())],
        },
    );
    assert!(host.menu.is_some());
    assert!(terminal(&menu, &action).is_empty());
    (action, menu)
}

#[test]
fn choosing_an_editor_completes_the_original_launch_without_another_marker() {
    let mut host = host();
    let (action, _) = editor_menu(&mut host);
    let selected = host.update(Event::Intent(Intent::Key {
        chord: Chord::new(Key::Enter),
    }));
    let context = task(&selected);
    assert_eq!(context.1.as_ref(), Some(&action));
    assert!(!records(&selected).iter().any(|r| matches!(r,
        JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "workspace.editor.launch"
    )));
    let done = reply(
        &mut host,
        context,
        WorkMsg::Launched {
            result: FolderLaunch::Dispatched("SECRET_STARTED".into()),
            what: "editor".into(),
        },
    );
    assert_eq!(terminal(&done, &action), [JournalOutcome::Succeeded]);
    assert!(!format!("{:?}", records(&selected)).contains("SECRET"));
    assert!(!format!("{:?}", records(&done)).contains("SECRET"));
}

#[test]
fn cancelling_the_editor_chooser_cancels_the_original_launch() {
    let mut host = host();
    let (action, _) = editor_menu(&mut host);
    let cancelled = host.update(Event::Intent(Intent::Key {
        chord: Chord::new(Key::Esc),
    }));
    assert_eq!(terminal(&cancelled, &action), [JournalOutcome::Cancelled]);
    assert!(host.menu.is_none());
    assert!(!cancelled.iter().any(|e| matches!(e, HostEffect::Tracked { effect, .. } if matches!(&**effect, HostEffect::Spawn { .. }))));
}
