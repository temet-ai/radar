use super::*;
use radar_core::Host;
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionEvent, ActionId, Journal, Query};
use radar_core::plugin::{ScreenSpec, Task};
use radar_core::view::{Chord, Key};
use std::sync::{Arc, Mutex};

enum Begin {
    Start,
    Reconfigure,
    Edit,
    Ask,
}
struct Harness(Arc<Mutex<GitlabPlugin>>);
impl Plugin for Harness {
    fn id(&self) -> &'static str {
        "gitlab"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        self.0.lock().unwrap().screens()
    }
    fn capturing(&self) -> bool {
        self.0.lock().unwrap().capturing()
    }
    fn paste(&mut self, text: &str) -> Vec<Effect> {
        self.0.lock().unwrap().paste(text)
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        settings::on_menu(&mut self.0.lock().unwrap(), menu, value)
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        match msg.downcast::<Begin>() {
            Ok(begin) => {
                let mut p = self.0.lock().unwrap();
                match *begin {
                    Begin::Start => {
                        let dispatched = settings::on_bulk(&mut p, "check-all");
                        assert_eq!(dispatched.len(), 4);
                        let mut effects = vec![Effect::Journal(ActionDescriptor::new(
                            "gitlab.test.parent",
                            Origin::Plugin("gitlab".into()),
                        ))];
                        // Keep the real dispatch's descriptor, replace only its I/O with
                        // recorded reads. Production provider tasks are never polled.
                        for (i, effect) in dispatched.into_iter().enumerate() {
                            let Effect::SpawnAction { action, .. } = effect else {
                                panic!("independent check-all spawn")
                            };
                            let (key, kind) = [
                                ("auth", Kind::Auth),
                                ("base", Kind::Base),
                                ("group:acme/team-b", Kind::Source),
                                ("group:acme/team-a", Kind::Source),
                            ][i];
                            let target = target(key);
                            let request = p.check_owners.pending[&target];
                            effects.push(spawn(action, target, request, async move {
                                match i {
                                    0 => Ok(read_async(
                                        Client::over(Replay::new(vec![(200, good(kind))])),
                                        kind,
                                    )
                                    .await
                                    .unwrap()),
                                    1 => Ok(read_async(
                                        Client::over(Replay::new(vec![(
                                            200,
                                            json!({"data":{"group":false}}),
                                        )])),
                                        kind,
                                    )
                                    .await
                                    .unwrap()),
                                    2 => {
                                        let attempt = Attempt::default();
                                        attempt.enter();
                                        attempt.finish(Err(anyhow::anyhow!("SECRET_READ_ERROR")))
                                    }
                                    _ => Attempt::default()
                                        .finish(Err(anyhow::anyhow!("SECRET_SETUP_ERROR"))),
                                }
                            }));
                        }
                        // A separately interrupted check and independent refresh siblings.
                        effects.extend(requested(&mut p, "group:acme/interrupted"));
                        effects.extend(p.refresh());
                        effects.push(Effect::Notify("SECRET_PARENT_NOTICE".into()));
                        effects.push(Effect::Report(Outcome::Succeeded));
                        effects
                    }
                    Begin::Reconfigure => {
                        let mut cfg = p.cfg.clone();
                        cfg.url = "https://changed.example".into();
                        cfg.base = "changed".into();
                        cfg.sources.clear();
                        cfg.token = "SECRET_CHANGED_TOKEN".into();
                        p.configure(Some(&toml::Value::try_from(cfg).unwrap()))
                            .unwrap();
                        vec![]
                    }
                    Begin::Edit => {
                        p.section_rows[0] = 0;
                        settings::on_menu(&mut p, "conn:server", "edit")
                    }
                    Begin::Ask => vec![Effect::Menu(radar_core::plugin::Menu::confirm(
                        crate::settings_checks::Target::Auth.menu(),
                        "SECRET_CHECK_TITLE",
                        "check it",
                        "leave it",
                    ))],
                }
            }
            Err(msg) => self.0.lock().unwrap().on_msg(msg),
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
fn tasks(effects: Vec<HostEffect>) -> Vec<(ActionId, ActionId, Task)> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            HostEffect::Tracked {
                action,
                semantic: Some(semantic),
                effect,
            } => match *effect {
                HostEffect::Spawn { task, .. } => Some((action, semantic, task)),
                _ => None,
            },
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
fn export(records: Vec<JournalRecord>) -> Vec<ActionEvent> {
    let dir = std::env::temp_dir().join(format!(
        "radar-gitlab-settings-history-{}",
        ActionId::new().as_str()
    ));
    let mut db = Journal::open(&dir.join("actions.sqlite3")).unwrap();
    let session = db.start_session().unwrap();
    for record in records {
        match record {
            JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } => db
                .begin_with_id(&session, &id, descriptor, parent.as_ref())
                .unwrap(),
            JournalRecord::Outcome {
                id,
                outcome,
                metadata,
            } => {
                db.record(&id, outcome, metadata).unwrap();
            }
            JournalRecord::Gap { .. } => panic!("unexpected gap"),
        }
    }
    db.finish_session(&session).unwrap();
    let events = db
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    let serialized = serde_json::to_string(&events).unwrap();
    for forbidden in [
        "SECRET",
        "token=",
        "username",
        "fullName",
        "nameWithNamespace",
        "env:",
        "ask-check:",
        "changed.example",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "{forbidden} leaked: {serialized}"
        );
    }
    drop(db);
    std::fs::remove_dir_all(dir).unwrap();
    events
}
fn harness() -> (Host, Arc<Mutex<GitlabPlugin>>) {
    let mut p = demo::sample();
    p.cfg.url = URL.into();
    p.cfg.token = "SECRET_TOKEN".into();
    let shared = Arc::new(Mutex::new(p));
    let mut host = Host::new(vec![Box::new(Harness(shared.clone()))], 1).unwrap();
    host.enable_journal();
    (host, shared)
}

#[test]
fn check_all_reversed_after_reconfiguration_preserves_scope_outcomes_and_refresh_siblings() {
    let (mut host, plugin) = harness();
    let original_me = plugin.lock().unwrap().me.clone();
    let original_data = plugin.lock().unwrap().data.clone();
    let start = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Start),
    });
    let mut all = records(&start);
    let parent = all
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. }
                if descriptor.kind.as_str() == "gitlab.test.parent" =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(terminal(&all, &parent), [Outcome::Succeeded]);
    let pending = tasks(start);
    assert_eq!(pending.len(), 7);
    let ids = pending
        .iter()
        .map(|(_, semantic, _)| semantic.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        7
    );
    all.extend(records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Reconfigure),
    })));
    for (i, (runtime, semantic, task)) in pending.into_iter().enumerate().rev() {
        if i >= 4 {
            drop(task);
            continue;
        }
        let effects = host.update(Event::Tracked {
            action: runtime.clone(),
            semantic: Some(semantic.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: block(task),
            }),
        });
        let got = records(&effects);
        let expected = [
            Outcome::Succeeded,
            Outcome::Partial,
            Outcome::Failed,
            Outcome::Blocked,
        ][i];
        assert_eq!(terminal(&got, &semantic), [expected]);
        assert_eq!(terminal(&got, &runtime), [expected]);
        assert!(
            ids.iter()
                .filter(|id| **id != semantic)
                .all(|id| terminal(&got, id).is_empty())
        );
        assert!(terminal(&got, &parent).is_empty());
        for record in &got {
            if let JournalRecord::Begin {
                descriptor, parent, ..
            } = record
                && descriptor
                    .kind
                    .as_str()
                    .starts_with("gitlab.settings.check.")
            {
                assert_eq!(parent.as_ref(), Some(&runtime));
                assert_eq!(descriptor.subjects[0].key.instance, "https://git.example/");
                assert_eq!(
                    descriptor.subjects[0].key.id,
                    ["server", "acme", "acme/team-b", "acme/team-a"][i]
                );
            }
        }
        assert!(got.iter().any(
            |record| matches!(record, JournalRecord::Begin { descriptor, .. }
            if descriptor.kind.as_str().ends_with(".application.skipped"))
        ));
        assert!(!effects.iter().any(|effect| matches!(
            effect,
            HostEffect::Save(_) | HostEffect::WriteCache { .. } | HostEffect::Spawn { .. }
        )));
        all.extend(got);
    }
    let p = plugin.lock().unwrap();
    assert!(
        p.checks.is_empty(),
        "retired settings replies do not recreate status"
    );
    assert_eq!(p.me, original_me);
    assert_eq!(p.data, original_data);
    drop(p);
    let events = export(all);
    for (i, id) in ids.iter().enumerate() {
        let terminal = events
            .iter()
            .filter(|event| event.action_id == *id && event.outcome.is_terminal())
            .map(|event| event.outcome)
            .collect::<Vec<_>>();
        assert_eq!(
            terminal,
            [if i < 4 {
                [
                    Outcome::Succeeded,
                    Outcome::Partial,
                    Outcome::Failed,
                    Outcome::Blocked,
                ][i]
            } else {
                Outcome::Unknown
            }]
        );
    }
}

#[test]
fn typing_and_declining_have_no_check_observations_but_confirmation_dispatches() {
    let (mut host, plugin) = harness();
    let mut all = records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Edit),
    }));
    for event in [
        Event::Paste("SECRET_TYPED_URL".into()),
        crate::key_event(Chord::new(Key::Esc)),
    ] {
        let fx = host.update(event);
        all.extend(records(&fx));
        assert!(tasks(fx).is_empty());
    }
    all.extend(records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Ask),
    })));
    let declined = host.update(crate::key_event(Chord::new(Key::Char('n'))));
    all.extend(records(&declined));
    assert!(tasks(declined).is_empty());
    assert!(plugin.lock().unwrap().checks.is_empty());
    assert!(!all.iter().any(|record|matches!(record,JournalRecord::Begin {descriptor,..} if descriptor.kind.as_str().starts_with("gitlab.settings.check."))));
    all.extend(records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Ask),
    })));
    let confirmed = host.update(crate::key_event(Chord::new(Key::Char('y'))));
    all.extend(records(&confirmed));
    assert_eq!(tasks(confirmed).len(), 1);
    assert_eq!(plugin.lock().unwrap().checks["auth"].0, Check::Running);
    let events = export(all);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind.as_str() == "gitlab.settings.check.auth"
                && event.outcome.is_terminal())
            .map(|event| event.outcome)
            .collect::<Vec<_>>(),
        [Outcome::Unknown]
    );
}
