use super::*;
use radar_core::Host;
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionEvent, ActionId, Journal, Query};
use radar_core::plugin::Task;

enum Begin {
    Start,
    Remove,
    RemoveProject,
    Ask(crate::settings_checks::Target),
}
struct Harness {
    plugin: Arc<Mutex<JiraPlugin>>,
}
impl Plugin for Harness {
    fn id(&self) -> &'static str {
        "jira"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        settings::on_menu(&mut self.plugin.lock().unwrap(), menu, value)
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        match msg.downcast::<Begin>() {
            Ok(begin) => match *begin {
                Begin::Start => {
                    let mut p = self.plugin.lock().unwrap();
                    // Production check-all constructs and captures all sibling scopes.
                    let dispatched = settings::on_bulk(&mut p, "s-check-all");
                    assert_eq!(dispatched.len(), 4);
                    let mut fx = vec![Effect::Journal(ActionDescriptor::new(
                        "jira.test.parent",
                        Origin::Plugin("jira".into()),
                    ))];
                    for (i, effect) in dispatched.into_iter().enumerate() {
                        let Effect::SpawnAction { action, .. } = effect else {
                            panic!("check-all independent spawn")
                        };
                        let kind = if i < 2 { Kind::Site } else { Kind::Project };
                        let alias = if i % 2 == 0 { "first" } else { "second" };
                        let target = match kind {
                            Kind::Site => crate::settings_checks::Target::Site(alias.into()),
                            Kind::Project => crate::settings_checks::Target::project(alias, "ABC"),
                        };
                        let owner = p.check_owners.pending[&target];
                        fx.push(spawn(target, owner, action, async move {
                            match i {
                                0 => read_async(kind, Recorded::body(good(kind))).await,
                                1 => read_async(kind, Recorded::body(Value::Null)).await,
                                2 => read_async(kind, Recorded::new(vec![Reply::Network])).await,
                                _ => {
                                    read_project(Err(anyhow::anyhow!("SECRET_SETUP_ERROR")), "ABC")
                                        .await
                                }
                            }
                        }));
                    }
                    fx.extend(settings::check_site(&mut p, usize::MAX));
                    fx.push(Effect::Notify("SECRET_PARENT_NOTICE".into()));
                    fx.push(Effect::Report(Outcome::Succeeded));
                    fx
                }
                Begin::Ask(target) => vec![Effect::Menu(radar_core::plugin::Menu::confirm(
                    target.menu(),
                    "SECRET_MENU_TITLE",
                    "check",
                    "leave unchecked",
                ))],
                Begin::RemoveProject => settings::on_menu(
                    &mut self.plugin.lock().unwrap(),
                    "project-remove:0:0",
                    "yes",
                ),
                Begin::Remove => {
                    let mut p = self.plugin.lock().unwrap();
                    p.configure(None).unwrap();
                    p.data.clear();
                    vec![]
                }
            },
            Err(msg) => self.plugin.lock().unwrap().on_msg(msg),
        }
    }
}
fn records(fx: &[HostEffect]) -> Vec<JournalRecord> {
    fx.iter()
        .filter_map(|effect| match effect {
            HostEffect::Journal(record) => Some(record.clone()),
            _ => None,
        })
        .collect()
}
fn tasks(fx: Vec<HostEffect>) -> Vec<(ActionId, ActionId, Task)> {
    fx.into_iter()
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
        "radar-jira-settings-history-{}",
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
            JournalRecord::Gap { .. } => panic!("unexpected journal gap"),
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
        "emailAddress",
        "accountId",
        "displayName",
        "scopes",
        "env:",
        "jql",
        "generation",
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

#[test]
fn check_all_reversed_removed_config_keeps_original_scopes_outcomes_and_siblings_isolated() {
    let mut p = demo::sample();
    p.cfg.sites = ["first", "second"].into_iter().map(|name| {
        let mut site = site();
        site.name = name.into();
        site.url = format!("https://SECRET_USER:SECRET_PASSWORD@{name}.example/?token=SECRET_TOKEN#SECRET_FRAGMENT");
        site.projects = vec![crate::config::Project { key: "ABC".into(), ac_field: "SECRET_AC_FIELD".into() }];
        site
    }).collect();
    p.cfg.views[0].jql = "SECRET_JQL".into();
    let plugin = Arc::new(Mutex::new(p));
    let mut host = Host::new(
        vec![Box::new(Harness {
            plugin: plugin.clone(),
        })],
        1,
    )
    .unwrap();
    host.enable_journal();
    let start = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Start),
    });
    let mut all = records(&start);
    let parent = all
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. }
                if descriptor.kind.as_str() == "jira.test.parent" =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(terminal(&all, &parent), [Outcome::Succeeded]);
    let notice_parent = all
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin {
                descriptor, parent, ..
            } if descriptor.kind.as_str() == "ui.notice" => parent.clone(),
            _ => None,
        })
        .unwrap();
    assert_eq!(notice_parent, parent);
    let unavailable = all
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. }
                if descriptor.kind.as_str() == "jira.settings.check.site.unavailable" =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(terminal(&all, &unavailable), [Outcome::Blocked]);
    let pending = tasks(start);
    assert_eq!(pending.len(), 4);
    let semantics: Vec<_> = pending
        .iter()
        .map(|(_, semantic, _)| semantic.clone())
        .collect();
    assert!(semantics.iter().all(|id| terminal(&all, id).is_empty()));
    all.extend(records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Remove),
    })));
    for (i, (runtime, semantic, task)) in pending.into_iter().enumerate().rev() {
        let got = records(&host.update(Event::Tracked {
            action: runtime.clone(),
            semantic: Some(semantic.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: block(task),
            }),
        }));
        let expected = [
            Outcome::Succeeded,
            Outcome::Partial,
            Outcome::Failed,
            Outcome::Blocked,
        ][i];
        assert_eq!(terminal(&got, &semantic), [expected]);
        assert_eq!(terminal(&got, &runtime), [expected]);
        assert!(terminal(&got, &parent).is_empty());
        assert!(
            semantics
                .iter()
                .filter(|id| *id != &semantic)
                .all(|id| terminal(&got, id).is_empty())
        );
        all.extend(got);
    }
    let events = export(all);
    for (i, semantic) in semantics.iter().enumerate() {
        let expected_instance = if i % 2 == 0 {
            "https://first.example/"
        } else {
            "https://second.example/"
        };
        let scoped: Vec<_> = events
            .iter()
            .filter(|event| &event.action_id == semantic)
            .collect();
        assert!(!scoped.is_empty());
        for event in scoped {
            assert_eq!(event.subjects.len(), 1);
            assert_eq!(event.subjects[0].key.instance, expected_instance);
            assert_eq!(event.subjects[0].key.id, if i < 2 { "site" } else { "ABC" });
        }
    }
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind.as_str().ends_with("application.skipped")
                && e.outcome == Outcome::Blocked)
            .count(),
        4
    );
    let p = plugin.lock().unwrap();
    assert!(p.cfg.sites.is_empty());
    assert!(p.checks.is_empty());
    assert!(p.data.is_empty());
}

#[test]
fn structured_check_menu_ids_titles_and_invalid_targets_never_enter_host_journal() {
    use crate::settings_checks::Target;
use radar_core::view::{Chord, Key};
    let mut p = demo::sample();
    p.cfg.sites[0].name = "SECRET:alias:α\"\\".into();
    let alias = p.cfg.sites[0].name.clone();
    let plugin = Arc::new(Mutex::new(p));
    let mut host = Host::new(
        vec![Box::new(Harness {
            plugin: plugin.clone(),
        })],
        1,
    )
    .unwrap();
    host.enable_journal();
    let mut all = vec![];
    for target in [
        Target::Site(alias.clone()),
        Target::project(&alias, "SECRET invalid:key"),
    ] {
        all.extend(records(&host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(Begin::Ask(target.clone())),
        })));
        assert_eq!(host.menu.as_ref().unwrap().menu.id, target.menu());
        let fx = host.update(crate::key_event(Chord::new(Key::Char('y'))));
        all.extend(records(&fx));
        // Production dispatch is reached, but no live provider task is polled.
        assert_eq!(tasks(fx).len(), 1);
        assert!(
            plugin
                .lock()
                .unwrap()
                .check_owners
                .pending
                .contains_key(&target)
        );
    }
    let events = export(all);
    let json = serde_json::to_string(&events).unwrap();
    assert!(!json.contains("ask-check"));
    assert!(events.iter().any(|e| e.kind.as_str() == "ui.menu"));
    assert!(
        events
            .iter()
            .any(|e| e.kind.as_str() == "jira.settings.check.project" && e.metadata.redacted)
    );
}

#[test]
fn one_removed_project_check_does_not_finish_or_cancel_check_all_siblings() {
    let mut p = demo::sample();
    p.cfg.sites = ["first", "second"]
        .into_iter()
        .map(|name| {
            let mut site = site();
            site.name = name.into();
            site.url = format!("https://{name}.example");
            site.projects = vec![crate::config::Project {
                key: "ABC".into(),
                ac_field: String::new(),
            }];
            site
        })
        .collect();
    let plugin = Arc::new(Mutex::new(p));
    let mut host = Host::new(
        vec![Box::new(Harness {
            plugin: plugin.clone(),
        })],
        1,
    )
    .unwrap();
    host.enable_journal();
    let fx = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Start),
    });
    let mut all = records(&fx);
    let pending = tasks(fx);
    let semantics: Vec<_> = pending
        .iter()
        .map(|(_, semantic, _)| semantic.clone())
        .collect();
    all.extend(records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::RemoveProject),
    })));
    assert_eq!(plugin.lock().unwrap().check_owners.pending.len(), 3);
    for (i, (runtime, semantic, task)) in pending.into_iter().enumerate().rev() {
        let fx = host.update(Event::Tracked {
            action: runtime.clone(),
            semantic: Some(semantic.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: block(task),
            }),
        });
        fn no_external(effect: &HostEffect) -> bool {
            match effect {
                HostEffect::Journal(_) => true,
                HostEffect::Tracked { effect, .. } => no_external(effect),
                _ => false,
            }
        }
        assert!(fx.iter().all(no_external));
        let got = records(&fx);
        let expected = [
            Outcome::Succeeded,
            Outcome::Partial,
            Outcome::Failed,
            Outcome::Blocked,
        ][i];
        assert_eq!(terminal(&got, &semantic), [expected]);
        assert_eq!(terminal(&got, &runtime), [expected]);
        assert!(
            semantics
                .iter()
                .filter(|id| *id != &semantic)
                .all(|id| terminal(&got, id).is_empty())
        );
        let skips: Vec<_> = got
            .iter()
            .filter_map(|record| match record {
                JournalRecord::Begin { descriptor, .. }
                    if descriptor.kind.as_str().ends_with("application.skipped") =>
                {
                    Some(descriptor)
                }
                _ => None,
            })
            .collect();
        assert_eq!(skips.len(), usize::from(i == 2));
        all.extend(got);
    }
    let p = plugin.lock().unwrap();
    assert_eq!(p.checks.len(), 3);
    assert!(p.check_owners.pending.is_empty());
    assert!(
        !p.checks
            .contains_key(&crate::settings_checks::Target::project("first", "ABC"))
    );
    drop(p);
    export(all);
}
