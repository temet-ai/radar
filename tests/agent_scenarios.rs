//! The real host and Sessions dock against its owner-local fake provider.
//! Shell steps exercise permission handling only: no CLI, shell, MCP listener,
//! network, repository mutation, or successful review/worktree claim.
//! Production settings still inspect PATH executable metadata, and session
//! setup reads the process's current directory before applying our explicit
//! work directory. Those read-only lookups are not an environment sandbox.
#![cfg(feature = "sessions")]

use std::path::PathBuf;

use radar_core::Host;
use radar_core::dock::{
    DockIntent, DockItem, DockPermission, DockView, PermissionAction, SessionState, ToolState,
};
use radar_core::host::Event;
use radar_core::journal::{ActionId, Journal, Origin, Outcome, Query};
use radar_core::scenario::{JournalRecord, Scenario};
use radar_core::view::Intent;
use radar_core::writes::Mode;
use radar_sessions::fake::{FakeCall, FakeProvider, Step};
use radar_sessions::{
    EventKind, PermissionAnswer, PermissionDecision, RadarSessionId, SessionsPlugin,
};

const NOW: i64 = 1_791_271_000;
const BUDGET: usize = 128;
const FIRST: &str = "Check the Watch patrol notes";
const SECOND: &str = "Try the Watch check again";
const BEFORE: &str = "The fake is waiting to check the patrol notes.";
const DENIED: &str = "The fake turn continued after the refusal.";
const APPROVED: &str = "The fake turn continued after approval.";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("radar-agent-scenario-{}", ActionId::new().as_str()));
        std::fs::create_dir(&path).unwrap();
        let scratch = Self(path.canonicalize().unwrap());
        for name in ["work", "recipes", "reviews"] {
            std::fs::create_dir(scratch.0.join(name)).unwrap();
        }
        std::fs::write(scratch.0.join("work/patrol.txt"), "Watch patrol notes\n").unwrap();
        scratch
    }

    fn config(&self) -> String {
        let path = |name| serde_json::to_string(self.0.join(name).to_str().unwrap()).unwrap();
        format!(
            "[theme]\nicons = 'off'\n\
             [sessions]\nprovider = 'fake'\nnotify = false\n\
             work_dir = {}\nrecipes_dir = {}\nreviews_dir = {}\n\
             project_config = false\nlocal_only = true\n\
             permitted_roots = []\nskill_dirs = []\nallow = []\ndeny = []\npty = []\n\
             [sessions.providers.fake]\nskill_dirs = []\ninstruction_dirs = []\nplugin_dirs = []\n",
            path("work"),
            path("recipes"),
            path("reviews"),
        )
    }

    fn assert_unchanged(&self) {
        assert_eq!(
            std::fs::read(self.0.join("work/patrol.txt")).unwrap(),
            b"Watch patrol notes\n"
        );
        assert_eq!(std::fs::read_dir(self.0.join("work")).unwrap().count(), 1);
        assert_eq!(
            std::fs::read_dir(self.0.join("recipes")).unwrap().count(),
            0
        );
        assert_eq!(
            std::fs::read_dir(self.0.join("reviews")).unwrap().count(),
            0
        );
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn dock(s: &Scenario) -> DockView {
    s.inspect(|host| *host.dock_view().expect("Sessions supplies the actual dock"))
}

fn dispatch(s: &mut Scenario, intent: DockIntent) {
    s.dispatch(Event::Intent(Intent::Dock(intent))).unwrap();
}

// Sessions owns detached adapter jobs; Scenario::is_idle only describes host
// effects. Explicit ticks/wakes and a real dock/provider predicate drive those
// jobs on this scenario's dedicated current-thread runtime.
async fn pump(s: &mut Scenario) {
    tokio::task::yield_now().await;
    s.dispatch(Event::Wake(NOW)).unwrap();
    s.dispatch(Event::Tick(NOW)).unwrap();
    s.settle(BUDGET).await.unwrap();
}

async fn until(
    s: &mut Scenario,
    fake: &FakeProvider,
    phase: &str,
    condition: impl Fn(&DockView) -> bool,
) {
    for _ in 0..BUDGET {
        pump(s).await;
        assert_eq!(fake.script_failure(), None, "{phase}");
        if condition(&dock(s)) {
            return;
        }
    }
    panic!(
        "{phase} did not settle: dock={:?}; calls={:?}",
        dock(s),
        fake.observations()
    );
}

fn permissions(view: &DockView) -> Vec<&DockPermission> {
    view.transcript
        .iter()
        .filter_map(|item| match item {
            DockItem::Permission(permission) => Some(permission),
            _ => None,
        })
        .collect()
}

fn says(view: &DockView, expected: &str) -> bool {
    view.transcript.iter().any(
        |item| matches!(item, DockItem::Assistant { text, saved: true, .. } if text == expected),
    )
}

fn permission_calls(fake: &FakeProvider) -> Vec<(RadarSessionId, PermissionAnswer)> {
    fake.observations()
        .into_iter()
        .filter_map(|call| match call {
            FakeCall::Permission { session_id, answer } => Some((session_id, answer)),
            _ => None,
        })
        .collect()
}

#[test]
fn hosted_fake_turn_waits_for_denial_then_fresh_approval() {
    let scratch = Scratch::new();
    let fake = FakeProvider::new();
    fake.strict_scripts();
    fake.script(vec![
        Step::Say(BEFORE.into()),
        Step::Shell("cargo test".into()),
        Step::Say(DENIED.into()),
    ]);
    fake.script(vec![
        Step::Say(BEFORE.into()),
        Step::Shell("cargo test".into()),
        Step::Say(APPROVED.into()),
    ]);

    // Drop the Scenario first, then this dedicated runtime. Borrowing a shared
    // async-test runtime would leave Sessions' detached subscriber alive.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        // Set the review location before any configure call can read an ambient
        // review directory. The only adapter is the non-environment fake.
        let mut sessions = SessionsPlugin::new().with_review_dir(scratch.0.join("reviews"));
        sessions.add_adapter(Box::new(fake.adapter()));
        let mut host = Host::new(vec![Box::new(sessions)], NOW).unwrap();
        host.load_config(Some(&scratch.config())).unwrap();
        host.set_env_mode(Some((Mode::Write, "scenario".into())));
        let journal_path = scratch.0.join("actions.sqlite3");
        let mut s = Scenario::with_journal(host, &journal_path).unwrap();
        dispatch(&mut s, DockIntent::Show);
        assert!(dock(&s).visible);
        dispatch(&mut s, DockIntent::NewSession { provider: Some("fake".into()) });
        until(&mut s, &fake, "create", |view| {
            fake.has_live_subscriber() && fake.calls("create") == 1 && view.tabs.len() == 1
                && view.tabs[0].state == SessionState::Idle
        }).await;
        let radar = RadarSessionId::new(dock(&s).active.unwrap());
        assert_eq!(fake.session_count(), 1);
        let creates: Vec<_> = fake.observations().into_iter().filter_map(|call| match call {
            FakeCall::Create(request) => Some(request),
            _ => None,
        }).collect();
        assert_eq!(creates.len(), 1);
        let create = &creates[0];
        assert_eq!(create.radar_session_id, radar);
        assert_eq!(create.cwd, scratch.0.join("work"));
        assert!(!create.setup.project_config);
        assert!(create.setup.local_only);
        assert!(!create.setup.review);
        assert!(create.setup.extra_dirs.is_empty());
        assert!(create.setup.skill_dirs.is_empty());
        assert!(create.setup.instruction_dirs.is_empty());
        assert!(create.setup.plugin_dirs.is_empty());
        assert_eq!(create.setup.tool_server, None, "no MCP listener is requested");

        dispatch(&mut s, DockIntent::Send { text: FIRST.into() });
        until(&mut s, &fake, "first permission", |view| {
            view.tabs[0].state == SessionState::Approval && permissions(view).len() == 1
        }).await;
        let waiting = dock(&s);
        let first = permissions(&waiting)[0].clone();
        assert_eq!(first.target, "cargo test");
        assert_eq!(first.kind, "shell");
        assert_eq!(first.answered, None);
        assert!(!first.lapsed);
        assert!(first.actions.contains(&PermissionAction::Deny));
        assert!(says(&waiting, BEFORE));
        assert!(!says(&waiting, DENIED));
        assert!(waiting.transcript.iter().any(|item| matches!(item,
            DockItem::Tool { id, state: ToolState::Running, .. } if Some(id) == first.tool.as_ref())));
        assert_eq!(fake.running(), 1);
        assert_eq!(fake.consumed_scripts(), 1);
        assert_eq!(fake.remaining_scripts(), 1);
        assert!(permission_calls(&fake).is_empty());
        assert!(!fake.persisted_events(&radar).iter().any(|event| matches!(event.kind,
            EventKind::PermissionResolved { .. } | EventKind::ToolCompleted { .. } | EventKind::TurnEnded)));
        // Further host scheduling cannot answer the permission or complete the
        // synthetic tool. The script remains blocked without an actual intent.
        let calls_before_answer = fake.observations();
        for _ in 0..8 { pump(&mut s).await; }
        assert_eq!(fake.observations(), calls_before_answer);
        assert_eq!(dock(&s), waiting);

        dispatch(&mut s, DockIntent::Deny { id: first.id.clone() });
        until(&mut s, &fake, "denied turn", |view| {
            fake.running() == 0 && view.tabs[0].state == SessionState::Idle && says(view, DENIED)
        }).await;
        let denied = dock(&s);
        assert_eq!(permissions(&denied)[0].answered, Some(PermissionAction::Deny));
        assert!(denied.transcript.iter().any(|item| matches!(item,
            DockItem::Tool { id, state: ToolState::Denied, result, .. }
                if Some(id) == first.tool.as_ref() && result == &["denied: denied in radar"])));
        assert!(fake.persisted_events(&radar).iter().any(|event| matches!(&event.kind,
            EventKind::ToolCompleted { call_id, ok: false, .. } if Some(call_id) == first.tool.as_ref())));

        dispatch(&mut s, DockIntent::Send { text: SECOND.into() });
        until(&mut s, &fake, "fresh permission", |view| {
            view.tabs[0].state == SessionState::Approval && permissions(view).len() == 2
        }).await;
        let waiting_again = dock(&s);
        let second = permissions(&waiting_again)[1].clone();
        assert_ne!(second.id, first.id);
        assert_ne!(second.tool, first.tool);
        assert_eq!(second.target, "cargo test");
        assert_eq!(second.answered, None);
        assert!(second.actions.contains(&PermissionAction::ApproveOnce));
        assert_eq!(permissions(&waiting_again)[0].answered, Some(PermissionAction::Deny));
        assert_eq!(permission_calls(&fake).len(), 1);
        assert!(!says(&waiting_again, APPROVED));
        assert!(!fake.persisted_events(&radar).iter().any(|event| matches!(&event.kind,
            EventKind::ToolCompleted { call_id, .. } if Some(call_id) == second.tool.as_ref())));

        dispatch(&mut s, DockIntent::Approve { id: second.id.clone() });
        until(&mut s, &fake, "approved turn", |view| {
            fake.running() == 0 && view.tabs[0].state == SessionState::Idle && says(view, APPROVED)
        }).await;
        let finished = dock(&s);
        assert_eq!(permissions(&finished).iter().map(|p| p.answered).collect::<Vec<_>>(),
            [Some(PermissionAction::Deny), Some(PermissionAction::ApproveOnce)]);
        assert!(finished.transcript.iter().any(|item| matches!(item,
            DockItem::Tool { id, state: ToolState::Done, result, .. }
                if Some(id) == second.tool.as_ref() && result == &["ran cargo test"])));
        assert_eq!(fake.observations(), vec![
            FakeCall::Subscribe,
            FakeCall::Create(create.clone()),
            FakeCall::Send { session_id: radar.clone(), input: radar_sessions::SendInput { text: FIRST.into() } },
            FakeCall::Permission { session_id: radar.clone(), answer: PermissionAnswer {
                request_id: first.id.clone(), decision: PermissionDecision::Deny { reason: Some("denied in radar".into()) },
            } },
            FakeCall::Send { session_id: radar.clone(), input: radar_sessions::SendInput { text: SECOND.into() } },
            FakeCall::Permission { session_id: radar.clone(), answer: PermissionAnswer {
                request_id: second.id.clone(), decision: PermissionDecision::ApproveOnce,
            } },
        ]);
        let history = fake.persisted_events(&radar);
        assert_eq!(history.iter().filter(|event| matches!(event.kind, EventKind::TurnStarted)).count(), 2);
        assert_eq!(history.iter().filter(|event| matches!(event.kind, EventKind::TurnEnded)).count(), 2);
        assert_eq!(history.iter().filter(|event| matches!(event.kind, EventKind::ToolCompleted { .. })).count(), 2);
        assert_eq!(history.iter().filter(|event| matches!(event.kind, EventKind::PermissionResolved { .. })).count(), 2);
        assert_eq!(fake.consumed_scripts(), 2);
        assert_eq!(fake.remaining_scripts(), 0);
        assert_eq!(fake.script_failure(), None);
        assert!(s.is_idle());
        assert!(s.desktop_effects().is_empty());
        scratch.assert_unchanged();

        // This journal proves actual dock dispatch only. Sessions does not emit
        // semantic turn/permission reports; provider events and the dock above
        // establish those results, not the dispatch's Succeeded outcome.
        let dispatches: Vec<_> = s.records().iter().filter_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. } if descriptor.kind.as_str() == "plugin.dock" => {
                assert_eq!(descriptor.origin, Origin::Plugin("sessions".into()));
                Some(id.clone())
            }
            _ => None,
        }).collect();
        assert_eq!(dispatches.len(), 5);
        for id in &dispatches {
            assert_eq!(s.records().iter().filter_map(|record| match record {
                JournalRecord::Outcome { id: actual, outcome, .. } if actual == id && outcome.is_terminal() => Some(*outcome),
                _ => None,
            }).collect::<Vec<_>>(), [Outcome::Succeeded]);
        }
        assert!(!s.records().iter().any(|record| matches!(record,
            JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str().starts_with("sessions."))));
        s.finish_journal().unwrap();
        let journal = Journal::open(&journal_path).unwrap();
        let events = journal.query(&Query { limit: 1000, ..Default::default() }).unwrap();
        assert!(events.len() < 1000, "the bounded journal evidence is complete");
        let persisted_dispatches: Vec<_> = events.iter().filter(|event|
            event.kind.as_str() == "plugin.dock" && event.outcome == Outcome::Requested
        ).collect();
        assert_eq!(persisted_dispatches.len(), dispatches.len());
        for event in persisted_dispatches {
            assert!(dispatches.contains(&event.action_id));
            assert_eq!(event.origin, Origin::Plugin("sessions".into()));
            assert_eq!(events.iter().filter(|candidate| candidate.action_id == event.action_id && candidate.outcome.is_terminal())
                .map(|candidate| candidate.outcome).collect::<Vec<_>>(), [Outcome::Succeeded]);
        }
        assert!(!events.iter().any(|event| event.kind.as_str().starts_with("sessions.")));
        // The captured cache holds session metadata, not permission decisions.
        // No restart/persistence assertion is made from it.
        assert!(s.cache_writes().iter().any(|(owner, _)| *owner == "sessions"));
        drop(s);
    });
    drop(runtime);
    assert!(
        !fake.has_live_subscriber(),
        "runtime teardown closes detached subscription"
    );
    assert_eq!(fake.running(), 0);
    scratch.assert_unchanged();
}
