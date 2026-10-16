//! Reducers and unpolled production dispatches only; no credentials or services.
use super::*;
use crate::{GlMsg, Settings, client::CheckRead, demo, settings, settings_history};
use radar_core::Plugin;
use radar_core::input::TextInput;
use radar_core::journal::{ActionDescriptor, Outcome as JournalOutcome};
use radar_core::plugin::{Effect, Outcome};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent};

#[derive(Clone)]
struct Pending {
    target: Target,
    request: Request,
    action: ActionDescriptor,
}
fn launch(p: &mut GitlabPlugin, target: Target) -> Pending {
    let effects = settings::on_menu(p, &target.menu(), "yes");
    let action = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::SpawnAction { action, .. } => Some(action),
            _ => None,
        })
        .expect("production check dispatch, dropped without polling");
    Pending {
        request: p.check_owners.pending[&target],
        target,
        action,
    }
}
fn result(value: &str) -> Result<CheckRead, settings_history::CheckError> {
    Ok(CheckRead {
        value: value.into(),
        evidence: Default::default(),
    })
}
fn failure(entered: bool) -> Result<CheckRead, settings_history::CheckError> {
    let attempt = settings_history::Attempt::default();
    if entered {
        attempt.enter();
    }
    attempt.finish(Err(anyhow::anyhow!("SECRET_ERROR")))
}
fn receive(
    p: &mut GitlabPlugin,
    pending: &Pending,
    result: Result<CheckRead, settings_history::CheckError>,
) -> Vec<Effect> {
    p.on_msg(Box::new(GlMsg::Checked {
        target: pending.target.clone(),
        request: pending.request,
        action: pending.action.clone(),
        result,
    }))
}
fn targets(p: &GitlabPlugin) -> Vec<Target> {
    [Target::Auth, Target::Base]
        .into_iter()
        .chain(
            p.cfg
                .sources
                .iter()
                .map(|s| Target::Source(p.cfg.group_path(s))),
        )
        .collect()
}
fn state(p: &GitlabPlugin) -> String {
    format!(
        "{:?}{:?}{:?}{:?}{:?}{:?}{:?}",
        p.cfg,
        p.snapshot(),
        p.checks,
        p.check_owners,
        p.edit,
        p.compose,
        p.recovery
    )
}
fn skip(fx: &[Effect], pending: &Pending, outcome: JournalOutcome) {
    assert!(matches!(fx.first(), Some(Effect::Report(got)) if *got == outcome));
    let Some(Effect::Observe {
        action,
        outcome: JournalOutcome::Blocked,
    }) = fx.last()
    else {
        panic!("independent skip after original result")
    };
    assert_eq!(
        action.kind.as_str(),
        format!("{}.application.skipped", pending.action.kind.as_str())
    );
    assert_eq!(action.subjects, pending.action.subjects);
    assert_eq!(action.metadata.count, Some(1));
    assert_eq!(action.metadata.redacted, pending.action.metadata.redacted);
    assert_eq!(
        action.metadata.omitted_subjects,
        pending.action.metadata.omitted_subjects
    );
    assert!(!fx.iter().any(|effect| matches!(
        effect,
        Effect::Notify(_)
            | Effect::Spawn(_)
            | Effect::SpawnAction { .. }
            | Effect::PersistCache
            | Effect::Emit { .. }
    )));
    assert!(!format!("{fx:?}").contains("SECRET"));
}
fn configure(p: &mut GitlabPlugin, cfg: Settings) {
    p.configure(Some(&toml::Value::try_from(cfg).unwrap()))
        .unwrap();
}
fn edit(p: &mut GitlabPlugin, target: crate::Target, text: &str) -> Vec<Effect> {
    p.edit = Some((target, TextInput::new(text)));
    let Outcome::Handled(fx) = settings::key(p, "connection", KeyEvent::from(KeyCode::Enter))
    else {
        panic!()
    };
    assert!(p.edit.is_none());
    fx
}

#[test]
fn reversed_overlaps_and_duplicate_success_or_failure_cannot_replace_newer_owners() {
    let mut p = demo::sample();
    for target in targets(&p) {
        let old = launch(&mut p, target.clone());
        let new = launch(&mut p, target.clone());
        p.edit = Some((
            crate::Target::SourceAdd,
            TextInput::new("SECRET_LOCAL_TEXT"),
        ));
        let held = state(&p);
        for (answer, outcome) in [
            (result("SECRET_OLD"), JournalOutcome::Succeeded),
            (failure(true), JournalOutcome::Failed),
        ] {
            skip(&receive(&mut p, &old, answer), &old, outcome);
        }
        assert_eq!(state(&p), held);
        let fx = receive(&mut p, &new, result("new"));
        assert!(matches!(fx.last(), Some(Effect::Notify(_))));
        assert_eq!(p.checks[&target.key()].0, Check::Passed("new".into()));
        let held = state(&p);
        for (answer, outcome) in [
            (result("SECRET_DUPLICATE"), JournalOutcome::Succeeded),
            (failure(true), JournalOutcome::Failed),
        ] {
            skip(&receive(&mut p, &new, answer), &new, outcome);
            skip(
                &receive(&mut p, &old, result("SECRET_OLD")),
                &old,
                JournalOutcome::Succeeded,
            );
            assert_eq!(state(&p), held);
        }
        let newer = launch(&mut p, target.clone());
        let held = state(&p);
        skip(
            &receive(&mut p, &new, failure(true)),
            &new,
            JournalOutcome::Failed,
        );
        assert_eq!(state(&p), held);
        receive(&mut p, &newer, failure(true));
        assert_eq!(
            p.checks[&target.key()].0,
            Check::Failed("SECRET_ERROR".into())
        );
    }
}

#[test]
fn original_success_partial_failure_and_setup_refusal_survive_stale_application() {
    for target in targets(&demo::sample()) {
        for n in 0..4 {
            let mut p = demo::sample();
            p.cfg.url =
                "https://SECRET_USER:SECRET_PASSWORD@git.example/?token=SECRET#SECRET".into();
            let old = launch(&mut p, target.clone());
            let mut answer = result("SECRET_RESULT");
            let outcome = match n {
                0 => JournalOutcome::Succeeded,
                1 => {
                    answer.as_mut().unwrap().evidence.incomplete_fields = 1;
                    JournalOutcome::Partial
                }
                2 => {
                    answer = failure(true);
                    JournalOutcome::Failed
                }
                _ => {
                    answer = failure(false);
                    JournalOutcome::Blocked
                }
            };
            p.replace_connection("https://new.example".into(), "SECRET_NEW_TOKEN".into());
            let held = state(&p);
            let fx = receive(&mut p, &old, answer);
            skip(&fx, &old, outcome);
            assert_eq!(state(&p), held);
        }
    }
}

#[test]
fn connection_url_token_and_configure_aba_retire_immediately_without_new_check() {
    for mode in 0..4 {
        let mut p = demo::sample();
        let cfg = p.cfg.clone();
        let pending: Vec<_> = targets(&p).into_iter().map(|t| launch(&mut p, t)).collect();
        if mode < 2 {
            p.replace_connection(
                if mode == 0 {
                    "https://other.example".into()
                } else {
                    cfg.url.clone()
                },
                if mode == 1 {
                    "env:OTHER".into()
                } else {
                    cfg.token.clone()
                },
            );
        } else {
            let mut changed = cfg.clone();
            if mode == 2 {
                changed.url = "https://other.example".into();
            } else {
                changed.token = "glab".into();
            }
            configure(&mut p, changed);
        }
        assert!(p.check_owners.pending.is_empty() && p.checks.is_empty());
        configure(&mut p, cfg);
        let held = state(&p);
        for old in pending {
            skip(
                &receive(&mut p, &old, result("SECRET_OLD")),
                &old,
                JournalOutcome::Succeeded,
            );
        }
        assert_eq!(state(&p), held);
    }
}

#[test]
fn base_aba_including_empty_retires_base_and_changed_paths_but_keeps_auth() {
    for via_ui in [false, true] {
        let mut p = demo::sample();
        let pending: Vec<_> = targets(&p).into_iter().map(|t| launch(&mut p, t)).collect();
        for base in ["", "acme"] {
            if via_ui {
                edit(&mut p, crate::Target::Conn(settings::Field::Base), base);
            } else {
                let mut cfg = p.cfg.clone();
                cfg.base = base.into();
                configure(&mut p, cfg);
            }
            assert_eq!(p.check_owners.pending.len(), 1);
            assert_eq!(p.checks.len(), 1);
            assert_eq!(p.checks["auth"].0, Check::Running);
        }
        receive(&mut p, &pending[0], result("current auth"));
        for old in &pending[1..] {
            skip(
                &receive(&mut p, old, result("SECRET_OLD")),
                old,
                JournalOutcome::Succeeded,
            );
        }
    }
}

#[test]
fn source_rename_and_remove_readd_retire_only_changed_membership_in_ui_and_configure() {
    for mode in 0..4 {
        let mut p = demo::sample();
        let cfg = p.cfg.clone();
        let pending: Vec<_> = targets(&p).into_iter().map(|t| launch(&mut p, t)).collect();
        match mode {
            0 => {
                edit(&mut p, crate::Target::SourceGroup(0), "renamed");
                edit(&mut p, crate::Target::SourceGroup(0), &cfg.sources[0].group);
            }
            1 => {
                settings::on_menu(&mut p, "tenancy-remove:0", "yes");
                edit(&mut p, crate::Target::SourceAdd, &cfg.sources[0].group);
            }
            2 => {
                let mut next = cfg.clone();
                next.sources[0].group = "renamed".into();
                configure(&mut p, next);
                configure(&mut p, cfg);
            }
            _ => {
                let mut next = cfg.clone();
                next.sources.remove(0);
                configure(&mut p, next);
                configure(&mut p, cfg);
            }
        }
        assert!(!p.check_owners.pending.contains_key(&pending[2].target));
        assert!(!p.checks.contains_key(&pending[2].target.key()));
        skip(
            &receive(&mut p, &pending[2], result("SECRET_OLD")),
            &pending[2],
            JournalOutcome::Succeeded,
        );
        for current in [&pending[0], &pending[1], &pending[3]] {
            let fx = receive(&mut p, current, result("current"));
            assert!(matches!(fx.last(), Some(Effect::Notify(_))));
        }
    }
}

#[test]
fn optional_unwatched_absent_present_absent_transition_is_not_revived() {
    for via_ui in [false, true] {
        let mut p = demo::sample();
        let old = launch(&mut p, Target::Source("acme/optional".into()));
        let cfg = p.cfg.clone();
        if via_ui {
            edit(&mut p, crate::Target::SourceAdd, "optional");
            assert!(!p.check_owners.pending.contains_key(&old.target));
            settings::on_menu(&mut p, "tenancy-remove:2", "yes");
        } else {
            let mut changed = cfg.clone();
            changed.sources.push(crate::config::Source {
                group: "optional".into(),
                code: "o".into(),
                color: "cyan".into(),
                hide: vec![],
            });
            configure(&mut p, changed);
            assert!(!p.check_owners.pending.contains_key(&old.target));
            configure(&mut p, cfg);
        }
        let held = state(&p);
        skip(
            &receive(&mut p, &old, result("SECRET_OLD")),
            &old,
            JournalOutcome::Succeeded,
        );
        assert_eq!(state(&p), held);
    }
}

#[test]
fn unchanged_and_nonidentity_configuration_preserve_pending_checks() {
    let mut p = demo::sample();
    let pending: Vec<_> = targets(&p).into_iter().map(|t| launch(&mut p, t)).collect();
    let mut cfg = p.cfg.clone();
    configure(&mut p, cfg.clone());
    cfg.sources.reverse();
    cfg.sources[0].code = "label".into();
    cfg.sources[0].color = "blue".into();
    cfg.sources[0].hide.push("hidden".into());
    cfg.refresh_secs += 10;
    cfg.history_days += 1;
    cfg.views[0].name = "renamed".into();
    cfg.views[0].filter = "is:review".into();
    configure(&mut p, cfg);
    settings::on_menu(&mut p, "tenancy:0", "down");
    settings::on_bulk(&mut p, "unhide");
    for current in pending {
        assert_eq!(p.check_owners.pending[&current.target], current.request);
        let fx = receive(&mut p, &current, result("current"));
        assert!(matches!(fx.last(), Some(Effect::Notify(_))));
    }
}

#[test]
fn initial_setup_failure_owns_newer_request_before_any_async_setup() {
    let mut p = demo::sample();
    p.cfg.token.clear();
    let old = launch(&mut p, Target::Auth);
    let mut fx = settings::on_menu(&mut p, &Target::Auth.menu(), "yes");
    let Effect::SpawnAction { task, .. } = fx.remove(0) else {
        panic!()
    };
    let msg = crate::client::recorded::block(task); // Empty token fails locally, no I/O.
    let effects = p.on_msg(msg);
    assert!(matches!(
        effects.first(),
        Some(Effect::Report(JournalOutcome::Blocked))
    ));
    assert_eq!(
        p.checks["auth"].0,
        Check::Failed("no GitLab token configured".into())
    );
    let held = state(&p);
    skip(
        &receive(&mut p, &old, result("SECRET_OLD")),
        &old,
        JournalOutcome::Succeeded,
    );
    assert_eq!(state(&p), held);
}

#[test]
fn identity_retirement_preserves_terminal_labels_and_unrelated_local_state() {
    let mut p = demo::sample();
    let auth = launch(&mut p, Target::Auth);
    receive(&mut p, &auth, result("old terminal"));
    launch(&mut p, Target::Base);
    p.edit = Some((
        crate::Target::SourceAdd,
        TextInput::new("SECRET_LOCAL_TEXT"),
    ));
    let before = p.snapshot();
    let local = format!("{:?}", p.edit);
    p.replace_connection("https://other.example".into(), "env:OTHER".into());
    assert_eq!(p.checks.len(), 1);
    assert_eq!(p.checks["auth"].0, Check::Passed("old terminal".into()));
    assert_eq!(p.snapshot(), before);
    assert_eq!(format!("{:?}", p.edit), local);
}

#[test]
fn token_offer_replacement_retires_old_owners_and_starts_new_auth() {
    let mut p = demo::sample();
    p.me.clear();
    p.cfg.token = "glab".into();
    p.env = Some(vec![(
        "GITLAB_TOKEN".into(),
        "fake-token-long-enough-for-offer".into(),
    )]);
    let old = launch(&mut p, Target::Auth);
    let base = launch(&mut p, Target::Base);
    settings::on_menu(&mut p, "offer:0", "use");
    assert_ne!(p.check_owners.pending[&Target::Auth], old.request);
    assert!(!p.check_owners.pending.contains_key(&Target::Base));
    let held = state(&p);
    skip(
        &receive(&mut p, &old, result("SECRET_OLD")),
        &old,
        JournalOutcome::Succeeded,
    );
    skip(
        &receive(&mut p, &base, failure(true)),
        &base,
        JournalOutcome::Failed,
    );
    assert_eq!(state(&p), held);
}

#[test]
fn typed_confirmations_capture_exact_source_across_reorder_and_cancel_has_no_owner() {
    let mut p = demo::sample();
    let fx = edit(&mut p, crate::Target::SourceGroup(0), "new/path");
    let menu = fx
        .into_iter()
        .find_map(|f| {
            if let Effect::Menu(m) = f {
                Some(m)
            } else {
                None
            }
        })
        .unwrap();
    let expected = Target::Source("acme/new/path".into());
    assert_eq!(menu.id, expected.menu());
    settings::on_menu(&mut p, "tenancy:0", "down");
    assert!(settings::on_menu(&mut p, &menu.id, "no").is_empty());
    assert!(p.check_owners.pending.is_empty());
    let fx = settings::on_menu(&mut p, &menu.id, "yes");
    assert!(
        matches!(&fx[..], [Effect::SpawnAction { action, .. }] if action.subjects[0].key.id == "acme/new/path")
    );
    assert!(p.check_owners.pending.contains_key(&expected));
    assert!(settings::on_menu(&mut p, "ask-check:arbitrary", "yes").is_empty());
}

#[test]
fn captured_connection_menu_field_is_used_after_selection_moves() {
    let mut p = demo::sample();
    p.section_rows[0] = 2; // Token; menu still names Base.
    let fx = settings::on_menu(&mut p, "conn:base group", "check");
    assert!(
        matches!(&fx[..], [Effect::SpawnAction { action, .. }] if action.kind.as_str() == "gitlab.settings.check.base")
    );
    settings::on_menu(&mut p, "conn:base group", "edit");
    assert!(matches!(
        p.edit,
        Some((crate::Target::Conn(settings::Field::Base), _))
    ));
    assert!(settings::on_menu(&mut p, "conn:invalid", "check").is_empty());
}

#[test]
fn base_spelling_change_retires_base_but_preserves_identical_effective_source_paths() {
    let mut p = demo::sample();
    let pending: Vec<_> = targets(&p).into_iter().map(|t| launch(&mut p, t)).collect();
    let mut cfg = p.cfg.clone();
    cfg.base = "/acme/".into();
    configure(&mut p, cfg);
    assert!(!p.check_owners.pending.contains_key(&Target::Base));
    for current in [&pending[0], &pending[2], &pending[3]] {
        assert_eq!(p.check_owners.pending[&current.target], current.request);
        assert!(matches!(
            receive(&mut p, current, result("current")).last(),
            Some(Effect::Notify(_))
        ));
    }
    skip(
        &receive(&mut p, &pending[1], result("SECRET_OLD")),
        &pending[1],
        JournalOutcome::Succeeded,
    );
}

#[test]
fn overlapping_bulk_groups_leave_auth_and_base_as_independent_siblings() {
    let mut p = demo::sample();
    let effects = settings::on_bulk(&mut p, "check-all");
    let all: Vec<_> = targets(&p)
        .into_iter()
        .zip(effects)
        .map(|(target, effect)| {
            let Effect::SpawnAction { action, .. } = effect else {
                panic!()
            };
            Pending {
                request: p.check_owners.pending[&target],
                target,
                action,
            }
        })
        .collect();
    assert_eq!(all.len(), 4);
    let group_effects = settings::on_bulk(&mut p, "check-groups");
    assert_eq!(group_effects.len(), 2);
    let newer: Vec<_> = all[2..]
        .iter()
        .zip(group_effects)
        .map(|(old, effect)| {
            let Effect::SpawnAction { action, .. } = effect else {
                panic!()
            };
            Pending {
                target: old.target.clone(),
                request: p.check_owners.pending[&old.target],
                action,
            }
        })
        .collect();
    for old in all.iter().rev() {
        if matches!(old.target, Target::Source(_)) {
            let held = state(&p);
            skip(
                &receive(&mut p, old, failure(true)),
                old,
                JournalOutcome::Failed,
            );
            assert_eq!(state(&p), held);
        } else {
            assert!(matches!(
                receive(&mut p, old, result("current")).last(),
                Some(Effect::Notify(_))
            ));
        }
    }
    for current in newer.iter().rev() {
        assert!(matches!(
            receive(&mut p, current, result("current")).last(),
            Some(Effect::Notify(_))
        ));
    }
    assert!(p.check_owners.pending.is_empty());
}

#[test]
fn defensive_begin_and_claim_reconcile_changed_base_and_membership() {
    let mut p = demo::sample();
    let base = launch(&mut p, Target::Base);
    let source = launch(&mut p, Target::Source("acme/team-b".into()));
    p.cfg.base = "other".into(); // Defensive check at entry, outside normal mutation hooks.
    let auth = launch(&mut p, Target::Auth);
    assert!(!p.check_owners.pending.contains_key(&base.target));
    assert!(!p.check_owners.pending.contains_key(&source.target));
    let now = launch(&mut p, Target::Source("other/team-a".into()));
    p.cfg.sources.pop();
    skip(
        &receive(&mut p, &now, result("SECRET_OLD")),
        &now,
        JournalOutcome::Succeeded,
    );
    assert!(!p.checks.contains_key(&now.target.key()));
    assert_eq!(p.check_owners.pending[&Target::Auth], auth.request);
}

mod host_tests;
