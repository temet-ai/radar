//! Real settings mutations and production dispatch, with unpolled provider futures.
//! All completed reads below use recorded transport or explicit local failures.
use super::*;
use crate::client::{
    CheckError, CheckRead, Client,
    recorded::{Replay, block},
};
use crate::settings_checks::Request;
use crate::{demo, settings_history, write, write_recovery};
use radar_core::Plugin;
use radar_core::journal::{ActionDescriptor, Outcome as JournalOutcome};
use serde_json::json;

#[derive(Clone)]
struct Held {
    target: CheckTarget,
    request: Request,
    action: ActionDescriptor,
}
fn launch(p: &mut JiraPlugin, target: CheckTarget) -> Held {
    let i = p
        .cfg
        .sites
        .iter()
        .position(|s| s.name == target.site())
        .unwrap();
    let fx = match &target {
        CheckTarget::Site(_) => check_site(p, i),
        CheckTarget::Project { project, .. } => check_project(p, i, project),
    };
    let [Effect::SpawnAction { action, .. }] = &fx[..] else {
        panic!("real dispatch")
    };
    Held {
        request: p.check_owners.pending[&target],
        target,
        action: action.clone(),
    }
}
fn site() -> CheckTarget {
    CheckTarget::Site("acme".into())
}
fn project(key: &str) -> CheckTarget {
    CheckTarget::project("acme", key)
}
fn recorded(target: &CheckTarget, name: &str) -> Result<CheckRead<String>, CheckError> {
    let replay = Replay::new(vec![(
        200,
        json!({"accountId":"SECRET_RETURNED_ID", "displayName":name, "name":name}),
    )]);
    let result = match target {
        CheckTarget::Site(_) => block(settings_history::read_site(Ok(Client::over(
            replay.clone(),
        )))),
        CheckTarget::Project { project, .. } => block(settings_history::read_project(
            Ok(Client::over(replay.clone())),
            project,
        )),
    };
    assert!(replay.paths().len() <= 1);
    result
}
fn receive(
    p: &mut JiraPlugin,
    held: &Held,
    result: Result<CheckRead<String>, CheckError>,
) -> Vec<Effect> {
    p.on_msg(Box::new(JiraMsg::Checked {
        target: held.target.clone(),
        request: held.request,
        action: held.action.clone(),
        result,
    }))
}
fn success(p: &mut JiraPlugin, held: &Held, name: &str) -> Vec<Effect> {
    receive(p, held, recorded(&held.target, name))
}
fn skipped(fx: &[Effect], held: &Held, outcome: JournalOutcome) {
    assert!(matches!(fx.first(), Some(Effect::Report(got)) if *got == outcome));
    assert_eq!(
        fx.iter().filter(|e| matches!(e, Effect::Report(_))).count(),
        1
    );
    assert!(
        fx.iter()
            .all(|e| matches!(e, Effect::Report(_) | Effect::Observe { .. }))
    );
    let Some(Effect::Observe {
        action,
        outcome: JournalOutcome::Blocked,
    }) = fx.last()
    else {
        panic!("separate application skip")
    };
    assert_eq!(
        action.kind.as_str(),
        match held.target {
            CheckTarget::Site(_) => "jira.settings.check.site.application.skipped",
            _ => "jira.settings.check.project.application.skipped",
        }
    );
    assert_eq!(action.subjects, held.action.subjects);
    assert_eq!(action.metadata.count, Some(1));
    assert_eq!(action.metadata.redacted, held.action.metadata.redacted);
    assert_eq!(
        action.metadata.omitted_subjects,
        held.action.metadata.omitted_subjects
    );
    assert!(action.metadata.reason.is_none());
    assert!(!format!("{action:?}").contains("SECRET"));
}
fn accepted(fx: &[Effect]) {
    assert_eq!(
        fx.iter().filter(|e| matches!(e, Effect::Notify(_))).count(),
        1
    );
    assert!(!fx.iter().any(|e| matches!(e, Effect::Observe { action, .. } if action.kind.as_str().ends_with("application.skipped"))));
}
fn accepted_error(fx: &[Effect]) {
    let mut notices = fx.iter().filter(|e| matches!(e, Effect::Notify(_) | Effect::NotifyWith(_, _) | Effect::Notice(_)));
    assert!(matches!((notices.next(), notices.next()),
        (Some(Effect::Notice(message)), None)
            if message.tone == radar_core::view::chrome::NoticeTone::Error));
    assert!(!fx.iter().any(|e| matches!(e, Effect::Observe { action, .. } if action.kind.as_str().ends_with("application.skipped"))));
}
fn configure(p: &mut JiraPlugin, cfg: &config::Settings) {
    p.configure(Some(&toml::Value::try_from(cfg).unwrap()))
        .unwrap();
}
fn state(p: &JiraPlugin) -> String {
    format!(
        "{:?}",
        (
            p.config(),
            p.snapshot(),
            &p.checks,
            &p.check_owners.pending,
            &p.edit,
            &p.compose,
            &p.change,
            &p.recovery,
            &p.section_rows,
            &p.filter,
            (&p.details, &p.pages, &p.jql_checks)
        )
    )
}

#[test]
fn stale_recorded_site_success_after_credential_reference_edit_cannot_reinsert_state() {
    let mut p = demo::sample();
    let held = launch(&mut p, site());
    let result = recorded(&held.target, "SECRET_OLD_NAME");
    commit(
        &mut p,
        Target::SiteToken(0),
        "env:SECRET_CHANGED_REFERENCE".into(),
    );
    assert!(!p.data.contains_key("acme"));
    assert!(!p.checks.contains_key(&site()));
    assert!(!p.check_owners.pending.contains_key(&site()));
    let before = state(&p);
    let fx = receive(&mut p, &held, result);
    skipped(&fx, &held, JournalOutcome::Succeeded);
    assert_eq!(state(&p), before);
}

#[test]
fn configured_connection_mutations_and_aba_retire_only_affected_pending_checks() {
    for target in [site(), project("ABC")] {
        for field in ["url", "email", "token", "name", "remove"] {
            let mut p = demo::sample();
            let original = p.cfg.clone();
            let held = launch(&mut p, target.clone());
            let sibling = launch(&mut p, CheckTarget::Site("beta".into()));
            match field {
                "url" => {
                    commit(
                        &mut p,
                        Target::SiteUrl(0),
                        "https://replacement.example".into(),
                    );
                }
                "email" => {
                    commit(
                        &mut p,
                        Target::SiteEmail(0),
                        "env:SECRET_REPLACED_EMAIL".into(),
                    );
                }
                "token" => {
                    commit(
                        &mut p,
                        Target::SiteToken(0),
                        "env:SECRET_REPLACED_TOKEN".into(),
                    );
                }
                "name" => {
                    commit(&mut p, Target::SiteName(0), "renamed".into());
                }
                "remove" => {
                    on_menu(&mut p, "site-remove:0", "yes");
                }
                _ => unreachable!(),
            }
            assert!(!p.check_owners.pending.contains_key(&target), "{field}");
            assert!(!p.checks.contains_key(&target), "{field}");
            assert_eq!(
                p.check_owners.pending.get(&sibling.target),
                Some(&sibling.request)
            );
            configure(&mut p, &original);
            let before = state(&p);
            skipped(
                &success(&mut p, &held, "SECRET_LATE_NAME"),
                &held,
                JournalOutcome::Succeeded,
            );
            skipped(
                &receive(
                    &mut p,
                    &held,
                    Err(CheckError::Read(anyhow::anyhow!("SECRET_ERROR"))),
                ),
                &held,
                JournalOutcome::Failed,
            );
            assert_eq!(state(&p), before);
            accepted(&success(&mut p, &sibling, "current sibling"));
        }
    }
}

#[test]
fn configure_connection_changes_and_roundtrip_aba_do_not_revive_requests() {
    for field in ["url", "email", "token", "name", "remove"] {
        let mut p = demo::sample();
        let original = p.cfg.clone();
        let held = launch(&mut p, site());
        let mut changed = original.clone();
        match field {
            "url" => changed.sites[0].url = "https://changed.example".into(),
            "email" => changed.sites[0].email = "env:SECRET_EMAIL".into(),
            "token" => changed.sites[0].token = "env:SECRET_TOKEN".into(),
            "name" => changed.sites[0].name = "changed".into(),
            "remove" => {
                changed.sites.remove(0);
            }
            _ => unreachable!(),
        }
        configure(&mut p, &changed);
        assert!(!p.check_owners.pending.contains_key(&site()));
        assert!(!p.checks.contains_key(&site()));
        configure(&mut p, &original);
        let newer = launch(&mut p, site());
        assert_ne!(held.request, newer.request);
        skipped(
            &success(&mut p, &held, "old"),
            &held,
            JournalOutcome::Succeeded,
        );
        assert_eq!(p.check_owners.pending.get(&site()), Some(&newer.request));
        accepted(&success(&mut p, &newer, "new"));
    }
}

#[test]
fn overlapping_site_and_project_requests_apply_only_latest_in_either_completion_order() {
    for target in [site(), project("ABC")] {
        for newer_first in [false, true] {
            let mut p = demo::sample();
            let old = launch(&mut p, target.clone());
            let new = launch(&mut p, target.clone());
            if newer_first {
                accepted(&success(&mut p, &new, "new"));
            }
            let before = state(&p);
            skipped(
                &success(&mut p, &old, "old"),
                &old,
                JournalOutcome::Succeeded,
            );
            assert_eq!(state(&p), before);
            if !newer_first {
                accepted(&success(&mut p, &new, "new"));
            }
            assert!(matches!(&p.checks[&target].0, Check::Passed(text) if text.contains("new")));
        }
    }
}

#[test]
fn newer_local_setup_failure_supersedes_older_success_before_any_client_work() {
    for target in [site(), project("ABC")] {
        let mut p = demo::sample();
        p.cfg.sites[0].email.clear();
        let old = launch(&mut p, target.clone());
        let fx = match &target {
            CheckTarget::Site(_) => check_site(&mut p, 0),
            CheckTarget::Project { project, .. } => check_project(&mut p, 0, project),
        };
        assert_ne!(p.check_owners.pending[&target], old.request);
        let Effect::SpawnAction { task, .. } = fx.into_iter().next().unwrap() else {
            unreachable!()
        };
        let msg = block(task); // Deterministic local no-email refusal, no HTTP.
        assert!(matches!(
            msg.downcast_ref::<JiraMsg>(),
            Some(JiraMsg::Checked {
                result: Err(CheckError::Setup(_)),
                ..
            })
        ));
        accepted_error(&p.on_msg(msg));
        let before = state(&p);
        skipped(
            &success(&mut p, &old, "old"),
            &old,
            JournalOutcome::Succeeded,
        );
        assert_eq!(state(&p), before);
        assert!(matches!(&p.checks[&target].0, Check::Failed(_)));
    }
}

#[test]
fn newer_invalid_project_failure_supersedes_older_owner_without_http() {
    let mut p = demo::sample();
    let target = project("SECRET invalid:key");
    let old = launch(&mut p, target.clone());
    let new = launch(&mut p, target.clone());
    let transport = Replay::new(vec![]);
    let invalid = block(settings_history::read_project(
        Ok(Client::over(transport.clone())),
        "SECRET invalid:key",
    ));
    assert!(matches!(&invalid, Err(CheckError::InvalidProject(_))));
    assert!(transport.paths().is_empty());
    accepted_error(&receive(&mut p, &new, invalid));
    let before = state(&p);
    skipped(
        &receive(
            &mut p,
            &old,
            Ok(CheckRead {
                value: "old".into(),
                evidence: Default::default(),
            }),
        ),
        &old,
        JournalOutcome::Succeeded,
    );
    assert_eq!(state(&p), before);
}

#[test]
fn duplicate_success_and_failure_before_and_after_new_owner_are_one_use() {
    for target in [site(), project("ABC")] {
        let mut p = demo::sample();
        let first = launch(&mut p, target.clone());
        accepted(&success(&mut p, &first, "first"));
        for newer in [false, true] {
            let new = newer.then(|| launch(&mut p, target.clone()));
            let before = state(&p);
            skipped(
                &success(&mut p, &first, "duplicate"),
                &first,
                JournalOutcome::Succeeded,
            );
            skipped(
                &receive(
                    &mut p,
                    &first,
                    Err(CheckError::Read(anyhow::anyhow!("duplicate error"))),
                ),
                &first,
                JournalOutcome::Failed,
            );
            assert_eq!(state(&p), before);
            if let Some(new) = new {
                accepted(&success(&mut p, &new, "new"));
            }
        }
    }
}

#[test]
fn project_remove_and_rewatch_retire_only_that_membership_without_connection_change() {
    let mut p = demo::sample();
    let old = launch(&mut p, project("ABC"));
    let same_site = launch(&mut p, site());
    let other_project = launch(&mut p, project("DEF"));
    let other_site = launch(&mut p, CheckTarget::Site("beta".into()));
    let generation = p.connections.generation("acme");
    on_menu(&mut p, "project-remove:0:0", "yes");
    assert_eq!(p.connections.generation("acme"), generation);
    assert!(!p.checks.contains_key(&old.target));
    assert!(!p.check_owners.pending.contains_key(&old.target));
    watch(&mut p, 0, "ABC");
    assert_eq!(p.connections.generation("acme"), generation);
    let before = state(&p);
    skipped(
        &success(&mut p, &old, "old"),
        &old,
        JournalOutcome::Succeeded,
    );
    assert_eq!(state(&p), before);
    for held in [same_site, other_project, other_site] {
        accepted(&success(&mut p, &held, "sibling"));
    }
    let new = launch(&mut p, project("ABC"));
    assert_ne!(new.request, old.request);
    accepted(&success(&mut p, &new, "new membership"));
}

#[test]
fn watched_and_unwatched_membership_aba_via_configure_cannot_revive_old_requests() {
    for key in ["ABC", "UNWATCHED"] {
        let mut p = demo::sample();
        let old = launch(&mut p, project(key));
        let original = p.cfg.clone();
        let mut changed = original.clone();
        if key == "ABC" {
            changed.sites[0].projects.remove(0);
        } else {
            changed.sites[0].projects.push(Project {
                key: key.into(),
                ac_field: String::new(),
            });
        }
        configure(&mut p, &changed);
        assert!(!p.check_owners.pending.contains_key(&old.target));
        assert!(!p.checks.contains_key(&old.target));
        configure(&mut p, &original);
        skipped(
            &success(&mut p, &old, "old"),
            &old,
            JournalOutcome::Succeeded,
        );
        let new = launch(&mut p, project(key));
        accepted(&success(&mut p, &new, "new"));
    }
}

#[test]
fn unwatched_project_checks_accept_but_watch_unwatch_aba_retires_them_immediately() {
    let mut p = demo::sample();
    let valid = launch(&mut p, project("UNWATCHED"));
    accepted(&success(&mut p, &valid, "accessible unwatched"));
    let old = launch(&mut p, project("UNWATCHED"));
    watch(&mut p, 0, "UNWATCHED");
    assert!(!p.check_owners.pending.contains_key(&old.target));
    assert!(!p.checks.contains_key(&old.target));
    on_menu(&mut p, "project-remove:0:2", "yes");
    assert!(!p.cfg.sites[0].watches("UNWATCHED"));
    skipped(
        &success(&mut p, &old, "old"),
        &old,
        JournalOutcome::Succeeded,
    );
}

#[test]
fn identical_and_unrelated_settings_project_order_and_ac_edits_preserve_owners() {
    let mut p = demo::sample();
    let held = [
        launch(&mut p, site()),
        launch(&mut p, project("ABC")),
        launch(&mut p, project("UNWATCHED")),
    ];
    let original = p.cfg.clone();
    configure(&mut p, &original);
    let mut changed = original.clone();
    changed.refresh_secs += 10;
    changed.idle_minutes += 10;
    changed.views[0].name = "changed view".into();
    changed.sites[0].color = "yellow".into();
    changed.sites[0].from = "SECRET_IMPORT".into();
    changed.sites[0].projects[0].ac_field = "different".into();
    changed.sites[0].projects.reverse();
    changed.sites[1].token = "env:SECRET_OTHER_REFERENCE".into();
    changed.sites.reverse();
    configure(&mut p, &changed);
    on_menu(&mut p, "ac:1:ABC", "customfield_2");
    for h in held {
        assert_eq!(p.check_owners.pending.get(&h.target), Some(&h.request));
        accepted(&success(&mut p, &h, "current"));
    }
}

#[test]
fn typed_targets_prevent_colon_status_or_owner_collisions() {
    let mut p = demo::sample();
    p.cfg.sites[0].name = "a:B".into();
    p.cfg.sites[1].name = "a".into();
    let good = CheckTarget::project("a:B", "C");
    let invalid = CheckTarget::project("a", "B:C");
    let a = launch(&mut p, good.clone());
    let b = launch(&mut p, invalid.clone());
    assert_ne!(a.request, b.request);
    assert_eq!(p.check_owners.pending.len(), 2);
    accepted_error(&receive(&mut p, &b, recorded(&invalid, "ignored")));
    assert!(matches!(p.checks[&good].0, Check::Running));
    assert!(matches!(p.checks[&invalid].0, Check::Failed(_)));
    accepted(&success(&mut p, &a, "good"));
    assert!(matches!(&p.checks[&good].0, Check::Passed(text) if text == "found good"));
}

#[test]
fn check_confirmation_roundtrips_exact_targets_and_preserves_cancel_and_unwatched_paths() {
    for target in [
        CheckTarget::Site("SECRET:a:α\"\\\n".into()),
        CheckTarget::project("SECRET:a:α\"\\\n", "B:C"),
        CheckTarget::project("SECRET:a:α\"\\\n", "ABC"),
    ] {
        let mut p = demo::sample();
        p.cfg.sites[0].name = target.site().into();
        p.cfg.sites[0].projects.clear();
        let Effect::Menu(menu) = ask_to_check(&target, "private target") else {
            unreachable!()
        };
        assert_eq!(
            CheckTarget::from_menu(menu.id.strip_prefix("ask-check:").unwrap()),
            Some(target.clone())
        );
        assert!(on_menu(&mut p, &menu.id, "no").is_empty());
        assert!(p.check_owners.pending.is_empty());
        let fx = on_menu(&mut p, &menu.id, "yes");
        assert!(matches!(&fx[..], [Effect::SpawnAction { .. }]));
        assert!(p.check_owners.pending.contains_key(&target));
        assert!(matches!(p.checks[&target].0, Check::Running));
        assert!(on_menu(&mut p, "ask-check:{not-json", "yes").is_empty());
    }
}

#[test]
fn stale_results_preserve_drafts_recovery_selection_cache_and_unrelated_checks() {
    let mut p = demo::sample();
    let old = launch(&mut p, site());
    crate::page::open(&mut p, "acme", "ABC-12");
    assert!(!p.details.is_empty());
    let change = write::Change::Comment {
        site: "acme".into(),
        key: "ABC-12".into(),
        text: "SECRET_RECOVERY".into(),
    };
    let record = write_recovery::Record::capture(&mut p, &change).unwrap();
    p.recovery.reserve(record).unwrap();
    write::open_compose(
        &mut p,
        "acme",
        write::Draft::Comment {
            key: "ABC-12".into(),
        },
        "SECRET_DRAFT",
        Default::default(),
        Default::default(),
        String::new(),
    );
    p.change = Some(change);
    p.edit = Some((Target::ViewName(0), TextInput::new("SECRET_TYPING")));
    p.filter = "SECRET_FILTER".into();
    p.section_rows = [1, 2, 1, 0];
    let sibling = launch(&mut p, CheckTarget::Site("beta".into()));
    commit(
        &mut p,
        Target::SiteToken(0),
        "env:SECRET_CHANGED_REFERENCE".into(),
    );
    assert!(
        p.details.is_empty(),
        "existing page invalidation stays active"
    );
    let before = state(&p);
    skipped(
        &success(&mut p, &old, "SECRET_OLD"),
        &old,
        JournalOutcome::Succeeded,
    );
    assert_eq!(state(&p), before);
    assert_eq!(
        p.check_owners.pending.get(&sibling.target),
        Some(&sibling.request)
    );
}

#[test]
fn retirement_preserves_terminal_results_and_only_removes_owned_running_statuses() {
    let mut p = demo::sample();
    let terminal = launch(&mut p, project("ABC"));
    accepted(&success(&mut p, &terminal, "old terminal"));
    let running = launch(&mut p, project("DEF"));
    let beta = launch(&mut p, CheckTarget::Site("beta".into()));
    let before_terminal = format!("{:?}", p.checks[&terminal.target]);
    let mut cfg = p.cfg.clone();
    cfg.sites[0].email = "env:SECRET_NEW".into();
    configure(&mut p, &cfg);
    assert_eq!(format!("{:?}", p.checks[&terminal.target]), before_terminal);
    assert!(!p.checks.contains_key(&running.target));
    assert!(matches!(p.checks[&beta.target].0, Check::Running));
    assert_eq!(
        p.check_owners.pending.get(&beta.target),
        Some(&beta.request)
    );
}
