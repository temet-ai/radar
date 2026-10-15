//! Bootstrap evidence is emitted only after the complete auth/profile task.
//! Fixtures and transports are synthetic; no test reaches a live provider.

use std::sync::{Arc, Mutex};

use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionDescriptor, ActionId, Outcome, Reason, SubjectRole};
use radar_core::plugin::{Effect, Msg, Task};
use radar_core::{Host, Plugin};
use serde_json::{Value, json};

use super::{bootstrap, bootstrap_action};
use crate::client::{BootstrapRead, Me, Recorded, Reply, TimezoneEvidence};
use crate::{SlackMsg, SlackPlugin, Via, demo, read::Read};

const TEAM: &str = "T000ACME1";
const NEXT_TEAM: &str = "T000NEXT1";
const USER: &str = "U0PRIVATEUSER99";

fn result(team: &str, user: &str, timezone: TimezoneEvidence) -> Result<BootstrapRead, String> {
    Ok(BootstrapRead {
        me: Me {
            team_id: team.into(),
            user_id: user.into(),
            user: "private_user_canary".into(),
            team: "private_team_canary".into(),
            url: "https://private_workspace_canary.example/".into(),
            scopes: vec!["private_scope_canary".into()],
            tz_offset: Some(39_600),
        },
        timezone,
    })
}

fn observe(effects: &[Effect]) -> Vec<(&ActionDescriptor, Outcome)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Observe { action, outcome } => Some((action, *outcome)),
            _ => None,
        })
        .collect()
}

fn report(effects: &[Effect], expected: Outcome) {
    let reports: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Report(outcome) => Some(*outcome),
            _ => None,
        })
        .collect();
    assert_eq!(reports, [expected]);
}

fn private_payloads_are_absent(debug: &str) {
    for secret in [USER, "private_", "SLACK_PRIVATE_TOKEN"] {
        assert!(!debug.contains(secret), "history leaked {secret}: {debug}");
    }
}

fn count(effects: &[Effect], kind: &str, expected: Outcome) {
    let matches: Vec<_> = observe(effects)
        .into_iter()
        .filter(|(action, _)| action.kind.as_str() == kind)
        .collect();
    assert_eq!(matches.len(), 1, "{effects:?}");
    assert_eq!(matches[0].1, expected);
    assert_eq!(matches[0].0.metadata.count, Some(1));
    assert!(!matches[0].0.metadata.truncated);
}

#[test]
fn dispatch_bootstrap_is_unscoped_and_does_not_claim_provider_progress() {
    let action = bootstrap_action();
    assert_eq!(action.kind.as_str(), "slack.sync.bootstrap");
    assert!(action.subjects.is_empty());
    assert_eq!(action.metadata.count, None);
    private_payloads_are_absent(&format!("{action:?}"));
}

#[test]
fn bootstrap_auth_error_has_no_identity_or_profile_evidence() {
    let effects = bootstrap(
        &bootstrap_action(),
        &Err("private_provider_error_canary".into()),
    );
    report(&effects, Outcome::Failed);
    assert_eq!(observe(&effects).len(), 1);
    count(&effects, "slack.sync.bootstrap.auth", Outcome::Failed);
    let (auth, _) = observe(&effects)[0];
    assert!(auth.subjects.is_empty());
    assert_eq!(auth.metadata.reason, Some(Reason::Unspecified));
    private_payloads_are_absent(&format!("{effects:?}"));
}

#[test]
fn bootstrap_profile_evidence_distinguishes_explicit_defaulted_and_failed() {
    for (timezone, kind, observed, aggregate) in [
        (
            TimezoneEvidence::Explicit,
            "slack.sync.bootstrap.timezone.returned",
            Outcome::Succeeded,
            Outcome::Succeeded,
        ),
        (
            TimezoneEvidence::Defaulted,
            "slack.sync.bootstrap.timezone.defaulted",
            Outcome::Partial,
            Outcome::Partial,
        ),
        (
            TimezoneEvidence::Failed,
            "slack.sync.bootstrap.timezone.failed",
            Outcome::Failed,
            Outcome::Partial,
        ),
    ] {
        let effects = bootstrap(&bootstrap_action(), &result(TEAM, USER, timezone));
        report(&effects, aggregate);
        assert_eq!(observe(&effects).len(), 3);
        count(&effects, "slack.sync.bootstrap.auth", Outcome::Succeeded);
        count(
            &effects,
            "slack.sync.bootstrap.identity",
            Outcome::Succeeded,
        );
        count(&effects, kind, observed);
        assert!(
            observe(&effects)[0].0.subjects.is_empty(),
            "auth transport evidence is unscoped"
        );
        assert_eq!(observe(&effects)[1].0.subjects.len(), 1);
        assert_eq!(observe(&effects)[2].0.subjects.len(), 1);
        private_payloads_are_absent(&format!("{effects:?}"));
        assert!(
            !format!("{effects:?}").contains("39600"),
            "raw offset entered evidence"
        );
        let subjects: Vec<_> = observe(&effects)
            .into_iter()
            .flat_map(|(action, _)| action.subjects.iter())
            .collect();
        assert!(
            !subjects.is_empty(),
            "safe learned workspace must be discoverable"
        );
        for subject in subjects {
            assert_eq!(subject.role, SubjectRole::Result);
            assert_eq!(subject.key.provider, "slack");
            assert_eq!(subject.key.instance, TEAM);
            assert_eq!(subject.key.kind, "workspace");
            assert_eq!(subject.key.id, "workspace");
        }
    }
}

#[test]
fn malformed_identity_is_partial_and_only_a_valid_learned_team_can_be_a_subject() {
    for (team, user, safe_team) in [
        ("", USER, false),
        ("T", USER, false),
        ("Tbad", USER, false),
        ("T000ACME1\n", USER, false),
        ("T000ACME1?private_canary", USER, false),
        ("xoxp-private_canary", USER, false),
        (TEAM, "", true),
        (TEAM, "U", true),
        (TEAM, "W", true),
        (TEAM, "Ubad", true),
        (TEAM, "U0?private_canary", true),
    ] {
        let effects = bootstrap(
            &bootstrap_action(),
            &result(team, user, TimezoneEvidence::Explicit),
        );
        report(&effects, Outcome::Partial);
        count(&effects, "slack.sync.bootstrap.auth", Outcome::Succeeded);
        count(&effects, "slack.sync.bootstrap.identity", Outcome::Partial);
        count(
            &effects,
            "slack.sync.bootstrap.timezone.returned",
            Outcome::Succeeded,
        );
        let subjects: Vec<_> = observe(&effects)
            .into_iter()
            .flat_map(|(action, _)| action.subjects.iter())
            .collect();
        assert_eq!(!subjects.is_empty(), safe_team, "{team:?} / {user:?}");
        if !safe_team {
            for (action, _) in observe(&effects).into_iter().skip(1) {
                assert!(action.metadata.redacted);
                assert_eq!(action.metadata.omitted_subjects, 1);
            }
        }
        for subject in subjects {
            assert_eq!(subject.key.instance, TEAM);
            assert_eq!(subject.role, SubjectRole::Result);
        }
        private_payloads_are_absent(&format!("{effects:?}"));
    }
    for user in ["U0VALIDUSER", "W0VALIDUSER"] {
        let effects = bootstrap(
            &bootstrap_action(),
            &result(TEAM, user, TimezoneEvidence::Explicit),
        );
        report(&effects, Outcome::Succeeded);
        assert!(!format!("{effects:?}").contains(user));
    }
}

#[test]
fn oversized_team_or_user_identity_does_not_escape_subject_bounds() {
    let oversized_team = format!("T{}", "A".repeat(2048));
    let oversized_user = format!("U{}", "B".repeat(2048));
    for (team, user, safe_team) in [
        (oversized_team.as_str(), USER, false),
        (TEAM, oversized_user.as_str(), true),
    ] {
        let effects = bootstrap(
            &bootstrap_action(),
            &result(team, user, TimezoneEvidence::Explicit),
        );
        report(&effects, Outcome::Partial);
        count(&effects, "slack.sync.bootstrap.identity", Outcome::Partial);
        for (action, _) in observe(&effects).into_iter().skip(1) {
            assert_eq!(!action.subjects.is_empty(), safe_team);
            assert_eq!(action.metadata.omitted_subjects, u32::from(!safe_team));
        }
        assert!(!format!("{effects:?}").contains(&oversized_team));
        assert!(!format!("{effects:?}").contains(&oversized_user));
    }
}

#[test]
fn long_uppercase_identities_within_bounds_are_valid_but_control_and_token_payloads_are_not() {
    for length in [64, 2048] {
        let team = format!("T{}", "A".repeat(length - 1));
        let user = format!("W{}", "B".repeat(length - 1));
        let effects = bootstrap(
            &bootstrap_action(),
            &result(&team, &user, TimezoneEvidence::Explicit),
        );
        report(&effects, Outcome::Succeeded);
        count(
            &effects,
            "slack.sync.bootstrap.identity",
            Outcome::Succeeded,
        );
        for (action, _) in observe(&effects).into_iter().skip(1) {
            assert_eq!(action.subjects.len(), 1);
            assert_eq!(action.subjects[0].key.instance, team);
            assert_eq!(action.subjects[0].role, SubjectRole::Result);
        }
        assert!(
            !format!("{effects:?}").contains(&user),
            "even valid user IDs stay out of history"
        );
    }
    for (team, user, safe_team) in [
        ("T000ACME1\0private_control_canary", USER, false),
        (TEAM, "U000USER\tprivate_control_canary", true),
        ("xoxp-private_token_canary", USER, false),
        (TEAM, "xoxb-private_token_canary", true),
    ] {
        let effects = bootstrap(
            &bootstrap_action(),
            &result(team, user, TimezoneEvidence::Explicit),
        );
        report(&effects, Outcome::Partial);
        for (action, _) in observe(&effects).into_iter().skip(1) {
            assert_eq!(!action.subjects.is_empty(), safe_team);
        }
        private_payloads_are_absent(&format!("{effects:?}"));
    }
}

fn ok(body: Value) -> anyhow::Result<Reply> {
    Ok(Reply {
        status: 200,
        body: body.to_string(),
        ..Default::default()
    })
}

fn auth(team: &str) -> anyhow::Result<Reply> {
    ok(json!({"ok": true, "team_id": team, "user_id": USER,
        "team": "private_team_canary", "user": "private_user_canary",
        "url": "https://private_workspace_canary.example/"}))
}

fn sample(recorded: Arc<dyn crate::client::Transport>) -> SlackPlugin {
    let mut p = demo::sample();
    p.data.me = None;
    p.cfg.channels.clear();
    p.data.asked.clear();
    p.sync.started = None;
    p.sync.last_ok = None;
    p.cfg.token = "env:SLACK_PRIVATE_TOKEN".into();
    p.via = Via(Some(recorded));
    p
}

fn run(task: Task) -> Msg {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(task)
}

#[test]
fn recorded_bootstrap_dispatches_auth_then_profile_and_retains_legacy_utc_fallback() {
    for (profile, expected, timezone, offset) in [
        (
            json!({"ok": true, "user": {"id": USER, "tz_offset": 0}}),
            Outcome::Succeeded,
            TimezoneEvidence::Explicit,
            0,
        ),
        (
            json!({"ok": true, "user": {"id": USER, "tz_offset": 7200}}),
            Outcome::Succeeded,
            TimezoneEvidence::Explicit,
            7200,
        ),
        (
            json!({"ok": true, "user": {"id": USER}}),
            Outcome::Partial,
            TimezoneEvidence::Defaulted,
            0,
        ),
        (
            json!({"ok": false, "error": "private_profile_error_canary"}),
            Outcome::Partial,
            TimezoneEvidence::Failed,
            0,
        ),
    ] {
        let recorded = Recorded::new(vec![auth(TEAM), ok(profile)]);
        let mut p = sample(recorded.clone());
        let mut effects = p.read();
        assert_eq!(effects.len(), 1);
        let Effect::SpawnAction { action, task } = effects.remove(0) else {
            panic!("semantic bootstrap task");
        };
        assert_eq!(action.kind.as_str(), "slack.sync.bootstrap");
        assert!(action.subjects.is_empty());
        assert!(
            recorded.requests().is_empty(),
            "dispatch must not perform provider I/O"
        );
        let message = run(task).downcast::<SlackMsg>().ok().unwrap();
        let SlackMsg::Read { read, .. } = &*message else {
            panic!("read message");
        };
        let Read::Me {
            action: captured,
            result: Ok(got),
        } = &**read
        else {
            panic!("bootstrap result");
        };
        assert_eq!(format!("{captured:?}"), format!("{action:?}"));
        assert_eq!(got.timezone, timezone);
        assert_eq!(got.me.tz_offset, Some(offset));
        let requests = recorded.requests();
        assert_eq!(
            requests
                .iter()
                .map(|request| request.method)
                .collect::<Vec<_>>(),
            ["auth.test", "users.info"]
        );
        assert!(
            requests
                .iter()
                .all(|request| !request.write && request.body.is_none())
        );
        assert_eq!(requests[1].params, [("user".into(), USER.into())]);
        let applied = p.on_msg(message);
        report(&applied, expected);
        assert_eq!(p.data.me.as_ref().unwrap().tz_offset, Some(offset));
        assert_eq!(
            p.sync.reading, 1,
            "normal For-you follow-on remains in flight"
        );
        let history: Vec<_> = applied
            .iter()
            .filter(|effect| matches!(effect, Effect::Report(_) | Effect::Observe { .. }))
            .collect();
        private_payloads_are_absent(&format!("{history:?}"));
        let children: Vec<_> = applied
            .iter()
            .filter_map(|effect| match effect {
                Effect::SpawnAction { action, .. } => Some(action),
                _ => None,
            })
            .collect();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].kind.as_str(), "slack.sync.for_you");
        assert_eq!(children[0].subjects[0].key.instance, TEAM);
        assert_eq!(recorded.requests().len(), 2, "children are only dispatched");
    }
}

#[test]
fn recorded_auth_failure_skips_profile_and_has_no_bootstrap_children() {
    let recorded = Recorded::new(vec![ok(
        json!({"ok": false, "error": "private_auth_error_canary"}),
    )]);
    let mut p = sample(recorded.clone());
    let Effect::SpawnAction { task, .. } = p.read().remove(0) else {
        panic!("semantic bootstrap task");
    };
    let effects = p.on_msg(run(task));
    report(&effects, Outcome::Failed);
    count(&effects, "slack.sync.bootstrap.auth", Outcome::Failed);
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::Spawn(_) | Effect::SpawnAction { .. }))
    );
    assert_eq!(
        recorded
            .requests()
            .iter()
            .map(|request| request.method)
            .collect::<Vec<_>>(),
        ["auth.test"]
    );
    assert_eq!(p.data.me, None);
    assert_eq!(p.sync.reading, 0);
    assert_eq!(p.sync.last_ok, None);
}

#[test]
fn recorded_auth_with_missing_or_nonstring_identity_does_not_request_profile_or_install_account() {
    for (body, parsed_team, _parsed_user) in [
        (json!({"ok": true}), "", ""),
        (
            json!({"ok": true, "team_id": 123, "user_id": {"private_identity_canary": true}}),
            "",
            "",
        ),
        (json!({"ok": true, "team_id": TEAM}), TEAM, ""),
        (
            json!({"ok": true, "team_id": TEAM, "user_id": ["private_identity_canary"]}),
            TEAM,
            "",
        ),
        (
            json!({"ok": true, "team_id": null, "user_id": USER}),
            "",
            USER,
        ),
    ] {
        let recorded = Recorded::new(vec![
            ok(body),
            ok(json!({"ok": true, "user": {"tz_offset": 0}})),
        ]);
        let mut p = sample(recorded.clone());
        let Effect::SpawnAction { task, .. } = p.read().remove(0) else {
            panic!("semantic bootstrap task");
        };
        let effects = p.on_msg(run(task));
        report(&effects, Outcome::Partial);
        count(&effects, "slack.sync.bootstrap.auth", Outcome::Succeeded);
        count(&effects, "slack.sync.bootstrap.identity", Outcome::Partial);
        count(
            &effects,
            "slack.sync.bootstrap.timezone.not_requested",
            Outcome::Blocked,
        );
        assert!(p.data.me.is_none());
        assert!(!p.authenticated());
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Spawn(_) | Effect::SpawnAction { .. }))
        );
        let requests = recorded.requests();
        assert_eq!(
            requests
                .iter()
                .map(|request| request.method)
                .collect::<Vec<_>>(),
            ["auth.test"]
        );
        for (action, _) in observe(&effects).into_iter().skip(1) {
            assert_eq!(!action.subjects.is_empty(), !parsed_team.is_empty());
            for subject in &action.subjects {
                assert_eq!(subject.key.instance, TEAM);
                assert_eq!(subject.role, SubjectRole::Result);
            }
        }
        let history: Vec<_> = effects
            .iter()
            .filter(|effect| matches!(effect, Effect::Report(_) | Effect::Observe { .. }))
            .collect();
        private_payloads_are_absent(&format!("{history:?}"));
        assert_eq!(
            p.sync.reading, 0,
            "incomplete identity cannot dispatch account-scoped child reads"
        );
    }
}

enum Command {
    Start,
    Rotate,
}

struct BootstrapHost(Arc<Mutex<SlackPlugin>>);

impl Plugin for BootstrapHost {
    fn id(&self) -> &'static str {
        "slack"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn on_msg(&mut self, message: Msg) -> Vec<Effect> {
        let mut p = self.0.lock().unwrap();
        match message.downcast::<Command>() {
            Ok(command) => {
                if matches!(*command, Command::Rotate) {
                    p.replace_token("env:SLACK_NEXT_TOKEN".into());
                }
                p.read()
            }
            Err(message) => p.on_msg(message),
        }
    }
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

fn terminal(records: &[JournalRecord], target: &ActionId) -> Vec<Outcome> {
    records
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Outcome { id, outcome, .. } if id == target && outcome.is_terminal() => {
                Some(*outcome)
            }
            _ => None,
        })
        .collect()
}

fn start(host: &mut Host, command: Command) -> (ActionId, ActionId, Task, Vec<JournalRecord>) {
    let effects = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(command),
    });
    let journal = records(&effects);
    let semantic = journal
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. }
                if descriptor.kind.as_str() == "slack.sync.bootstrap" =>
            {
                assert!(descriptor.subjects.is_empty());
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    assert!(terminal(&journal, &semantic).is_empty());
    assert!(!journal.iter().any(
        |record| matches!(record, JournalRecord::Begin { descriptor, .. }
        if descriptor.kind.as_str().starts_with("slack.sync.bootstrap."))
    ));
    let (runtime, task) = effects
        .into_iter()
        .find_map(|effect| match effect {
            HostEffect::Tracked {
                action,
                semantic: Some(id),
                effect,
            } if id == semantic => match *effect {
                HostEffect::Spawn { task, .. } => Some((action, task)),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    (runtime, semantic, task, journal)
}

fn deliver(
    host: &mut Host,
    runtime: &ActionId,
    semantic: &ActionId,
    message: Msg,
) -> Vec<HostEffect> {
    host.update(Event::Tracked {
        action: runtime.clone(),
        semantic: Some(semantic.clone()),
        event: Box::new(Event::Msg {
            plugin: 0,
            msg: message,
        }),
    })
}

#[test]
fn two_bootstraps_complete_out_of_order_with_original_outcomes_and_stale_learned_scope() {
    let recorded = Recorded::new(vec![
        auth(TEAM),
        ok(json!({"ok": true, "user": {"id": USER}})),
        auth(NEXT_TEAM),
        ok(json!({"ok": true, "user": {"id": USER, "tz_offset": 0}})),
    ]);
    let shared = Arc::new(Mutex::new(sample(recorded.clone())));
    let mut host = Host::new(vec![Box::new(BootstrapHost(shared.clone()))], demo::NOW).unwrap();
    host.enable_journal();
    let (old_runtime, old_semantic, old_task, _) = start(&mut host, Command::Start);
    let old_message = run(old_task);
    let (new_runtime, new_semantic, new_task, _) = start(&mut host, Command::Rotate);
    let new_message = run(new_task);
    let new_effects = deliver(&mut host, &new_runtime, &new_semantic, new_message);
    let new_records = records(&new_effects);
    assert_eq!(terminal(&new_records, &new_semantic), [Outcome::Succeeded]);
    assert!(terminal(&new_records, &old_semantic).is_empty());
    let before = {
        let p = shared.lock().unwrap();
        (
            p.data.clone(),
            p.sync.reading,
            p.sync.last_ok,
            p.sync.errors.clone(),
        )
    };
    let old_effects = deliver(&mut host, &old_runtime, &old_semantic, old_message);
    assert!(
        old_effects
            .iter()
            .all(|effect| matches!(effect, HostEffect::Journal(_))),
        "{old_effects:?}"
    );
    let old_records = records(&old_effects);
    assert_eq!(terminal(&old_records, &old_runtime), [Outcome::Partial]);
    assert_eq!(terminal(&old_records, &old_semantic), [Outcome::Partial]);
    assert!(terminal(&old_records, &new_semantic).is_empty());
    let mut skip = None;
    let mut learned = 0;
    for record in &old_records {
        if let JournalRecord::Begin {
            id,
            descriptor,
            parent,
        } = record
        {
            assert_eq!(parent.as_ref(), Some(&old_runtime));
            if descriptor.kind.as_str() == "slack.read.apply.skipped.stale" {
                skip = Some(id.clone());
                assert_eq!(descriptor.metadata.reason, Some(Reason::Unspecified));
                assert_eq!(descriptor.subjects.len(), 1);
                assert_eq!(descriptor.subjects[0].key.instance, TEAM);
                assert_eq!(descriptor.subjects[0].role, SubjectRole::Result);
            } else {
                assert!(
                    descriptor
                        .kind
                        .as_str()
                        .starts_with("slack.sync.bootstrap.")
                );
                for subject in &descriptor.subjects {
                    learned += 1;
                    assert_eq!(subject.key.instance, TEAM);
                    assert_eq!(subject.role, SubjectRole::Result);
                }
            }
        }
    }
    assert!(learned > 0);
    assert_eq!(terminal(&old_records, &skip.unwrap()), [Outcome::Blocked]);
    let p = shared.lock().unwrap();
    assert_eq!(
        (
            p.data.clone(),
            p.sync.reading,
            p.sync.last_ok,
            p.sync.errors.clone()
        ),
        before
    );
    assert_eq!(p.data.me.as_ref().unwrap().team_id, NEXT_TEAM);
    private_payloads_are_absent(&format!("{old_records:?}{new_records:?}"));
    assert_eq!(recorded.requests().len(), 4);
}

#[derive(Default)]
struct PendingProfile {
    methods: Mutex<Vec<&'static str>>,
}

impl crate::client::Transport for PendingProfile {
    fn send<'a>(
        &'a self,
        request: &'a crate::client::Request,
    ) -> crate::client::Fut<'a, anyhow::Result<Reply>> {
        self.methods.lock().unwrap().push(request.method);
        match request.method {
            "auth.test" => Box::pin(async { auth(TEAM) }),
            "users.info" => Box::pin(std::future::pending()),
            _ => panic!("unexpected bootstrap request"),
        }
    }
}

#[test]
fn interrupted_bootstrap_has_unknown_history_without_fabricated_auth_or_profile_progress() {
    use std::task::{Context, Poll, Waker};

    let transport = Arc::new(PendingProfile::default());
    let shared = Arc::new(Mutex::new(sample(transport.clone())));
    let mut host = Host::new(vec![Box::new(BootstrapHost(shared.clone()))], demo::NOW).unwrap();
    host.enable_journal();
    let (runtime, semantic, mut task, journal) = start(&mut host, Command::Start);
    assert!(matches!(
        task.as_mut().poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    assert_eq!(
        *transport.methods.lock().unwrap(),
        ["auth.test", "users.info"]
    );
    drop(task);
    assert!(terminal(&journal, &runtime).is_empty());
    assert!(terminal(&journal, &semantic).is_empty());
    assert_eq!(
        shared.lock().unwrap().data.me,
        None,
        "partial task progress was never applied"
    );
    private_payloads_are_absent(&format!("{journal:?}"));

    #[cfg(unix)]
    {
        use radar_core::journal::{Journal, Query};
        let dir = std::env::temp_dir().join(format!(
            "radar-bootstrap-interrupted-{}",
            ActionId::new().as_str()
        ));
        let mut db = Journal::open(&dir.join("actions.sqlite3")).unwrap();
        let session = db.start_session().unwrap();
        for record in journal {
            match record {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                } => {
                    db.begin_with_id(&session, &id, descriptor, parent.as_ref())
                        .unwrap();
                }
                JournalRecord::Outcome {
                    id,
                    outcome,
                    metadata,
                } => {
                    db.record(&id, outcome, metadata).unwrap();
                }
                JournalRecord::Gap { .. } => panic!("no recording gap expected"),
            }
        }
        db.finish_session(&session).unwrap();
        let events = db
            .query(&Query {
                limit: 100,
                ..Default::default()
            })
            .unwrap();
        assert!(events.iter().any(|event| event.action_id == semantic
            && event.outcome == Outcome::Unknown
            && event.metadata.reason == Some(Reason::NoTerminalObservation)));
        assert!(events.iter().any(|event| event.action_id == runtime
            && event.outcome == Outcome::Unknown
            && event.metadata.reason == Some(Reason::Interrupted)));
        assert!(
            !events
                .iter()
                .any(|event| event.kind.as_str().starts_with("slack.sync.bootstrap."))
        );
        private_payloads_are_absent(&serde_json::to_string(&events).unwrap());
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
