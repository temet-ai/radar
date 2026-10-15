use super::*;
use crate::{GlMsg, client::RunsRead, model::fixtures, runs_history};
use radar_core::{Plugin, journal::Outcome, plugin::Effect};

const PROJECT: &str = "acme/team-a/api";
const OTHER: &str = "acme/team-a/web";
const SKIP: &str = "gitlab.page.runs.apply.skipped.stale";

fn launch(p: &mut GitlabPlugin, path: &str) -> Request {
    let branch = pages::branch_of(&pages::project(p, path));
    begin(p, path, &branch)
}
fn result(title: &str) -> Result<RunsRead, String> {
    let mut runs = fixtures::runs();
    runs[0].commit_title = title.into();
    Ok(RunsRead {
        runs,
        evidence: Default::default(),
    })
}
fn receive(
    p: &mut GitlabPlugin,
    request: &Request,
    result: Result<RunsRead, String>,
) -> Vec<Effect> {
    p.on_msg(Box::new(GlMsg::Runs {
        request: request.clone(),
        action: runs_history::action("https://original.example", &request.path, &request.branch),
        result,
    }))
}
fn branch(p: &mut GitlabPlugin, path: &str, value: &str) {
    for data in p.data.values_mut() {
        for project in &mut data.projects {
            if project.full_path == path {
                project.default_branch = value.into();
            }
        }
    }
}
fn state(p: &GitlabPlugin) -> String {
    format!("{:?}{:?}", p.runs, p.runs_scopes)
}
fn skipped(fx: &[Effect], expected: Outcome) {
    assert!(matches!(fx.first(), Some(Effect::Report(got)) if *got == expected));
    assert!(
        matches!(fx.last(), Some(Effect::Observe { action, outcome: Outcome::Blocked })
        if action.kind.as_str() == SKIP && action.metadata.count == Some(1))
    );
    assert!(!format!("{fx:?}").contains("SECRET"));
}

#[test]
fn reordered_forced_success_error_and_duplicate_keep_latest_exact_owner() {
    for old_error in [false, true] {
        let mut p = crate::demo::sample();
        let old = launch(&mut p, PROJECT);
        let new = launch(&mut p, PROJECT);
        let before = state(&p);
        skipped(
            &receive(
                &mut p,
                &old,
                if old_error {
                    Err("SECRET_OLD".into())
                } else {
                    result("SECRET_OLD")
                },
            ),
            if old_error {
                Outcome::Failed
            } else {
                Outcome::Succeeded
            },
        );
        assert_eq!(state(&p), before);
        receive(&mut p, &new, result("SECRET_NEW"));
        let accepted = state(&p);
        skipped(
            &receive(&mut p, &old, result("SECRET_OLD")),
            Outcome::Succeeded,
        );
        skipped(
            &receive(&mut p, &new, Err("SECRET_DUPLICATE".into())),
            Outcome::Failed,
        );
        assert_eq!(state(&p), accepted);
        assert_eq!(
            cached(&p, PROJECT).unwrap().value.as_ref().unwrap()[0].commit_title,
            "SECRET_NEW"
        );
    }
}

#[test]
fn current_refresh_error_preserves_rows_and_time_but_old_success_cannot_clear_it() {
    let mut p = crate::demo::sample();
    let first = launch(&mut p, PROJECT);
    receive(&mut p, &first, result("SECRET_HELD"));
    let at = p.runs[PROJECT].at;
    let old = launch(&mut p, PROJECT);
    let new = launch(&mut p, PROJECT);
    p.now += 5;
    receive(&mut p, &new, Err("SECRET_CURRENT_ERROR".into()));
    let state = state(&p);
    skipped(
        &receive(&mut p, &old, result("SECRET_OLD")),
        Outcome::Succeeded,
    );
    assert_eq!(self::state(&p), state);
    assert_eq!(p.runs[PROJECT].at, at);
    assert!(!p.runs[PROJECT].loading);
    assert_eq!(
        p.runs[PROJECT].error.as_deref(),
        Some("SECRET_CURRENT_ERROR")
    );
}

#[test]
fn connection_a_b_a_retires_pending_and_fresh_caches_without_resurrection() {
    for completed in [false, true] {
        let mut p = crate::demo::sample();
        let original = (p.cfg.url.clone(), p.cfg.token.clone());
        let old = launch(&mut p, PROJECT);
        if completed {
            receive(&mut p, &old, result("SECRET_HELD"));
        }
        p.replace_connection("https://other.example".into(), "SECRET_OTHER".into());
        assert!(p.runs.is_empty() && p.runs_scopes.is_empty());
        p.replace_connection(original.0, original.1);
        let before = state(&p);
        skipped(
            &receive(&mut p, &old, result("SECRET_OLD")),
            Outcome::Succeeded,
        );
        assert_eq!(state(&p), before);
        let fresh = launch(&mut p, PROJECT);
        receive(&mut p, &fresh, result("SECRET_FRESH"));
        assert_eq!(
            cached(&p, PROJECT).unwrap().value.as_ref().unwrap()[0].commit_title,
            "SECRET_FRESH"
        );
    }
}

#[test]
fn exact_default_ref_changes_hide_fresh_or_pending_rows_and_do_not_block_new_read() {
    for completed in [false, true] {
        let mut p = crate::demo::sample();
        let old = launch(&mut p, PROJECT);
        if completed {
            receive(&mut p, &old, result("SECRET_HELD"));
        }
        branch(&mut p, PROJECT, "feature/A");
        assert!(cached(&p, PROJECT).is_none());
        let fx = pages::push(&mut p, pages::Kind::Repo(PROJECT.into()));
        assert!(fx.iter().any(|e| matches!(e, Effect::SpawnAction { action, .. } if action.kind.as_str() == "gitlab.page.runs")));
        assert!(p.runs[PROJECT].value.is_none() && p.runs[PROJECT].loading);
        let new = pending(&p, PROJECT);
        let before = state(&p);
        skipped(
            &receive(&mut p, &old, result("SECRET_OLD")),
            Outcome::Succeeded,
        );
        assert_eq!(state(&p), before);
        receive(&mut p, &new, result("SECRET_NEW"));
        assert!(cached(&p, PROJECT).is_some());
        branch(&mut p, PROJECT, "feature/a");
        reconcile(&mut p);
        branch(&mut p, PROJECT, "feature/A");
        reconcile(&mut p);
        assert!(cached(&p, PROJECT).is_none());
        skipped(
            &receive(&mut p, &new, result("SECRET_DUPLICATE")),
            Outcome::Succeeded,
        );
    }
}

#[test]
fn retired_cache_never_recreated_and_other_projects_keep_their_owners() {
    let mut p = crate::demo::sample();
    let retired = launch(&mut p, PROJECT);
    let other = launch(&mut p, OTHER);
    p.runs.remove(PROJECT);
    skipped(
        &receive(&mut p, &retired, result("SECRET_RETIRED")),
        Outcome::Succeeded,
    );
    assert!(!p.runs.contains_key(PROJECT));
    receive(&mut p, &other, result("SECRET_OTHER"));
    assert!(cached(&p, OTHER).is_some());
    let latest = launch(&mut p, PROJECT);
    let before = state(&p);
    skipped(
        &receive(&mut p, &retired, result("SECRET_RETIRED")),
        Outcome::Succeeded,
    );
    assert_eq!(state(&p), before);
    receive(&mut p, &latest, result("SECRET_LATEST"));
    let snapshot = p.snapshot().unwrap().to_string();
    for value in ["SECRET", "runs_scopes", "pending", "connection", SKIP] {
        assert!(!snapshot.contains(value), "{value}");
    }
}

#[test]
fn accepted_project_sync_retires_a_b_a_ref_owner_without_page_visit() {
    let mut p = crate::demo::sample();
    let old = launch(&mut p, PROJECT);
    let other = launch(&mut p, OTHER);
    let group = p
        .data
        .iter()
        .find(|(_, data)| data.projects.iter().any(|pr| pr.full_path == PROJECT))
        .unwrap()
        .0
        .clone();
    let original = pages::branch_of(&pages::project(&p, PROJECT));
    for target in ["release/new", original.as_str()] {
        let mut projects = p.data[&group].projects.clone();
        projects
            .iter_mut()
            .find(|pr| pr.full_path == PROJECT)
            .unwrap()
            .default_branch = target.into();
        let want = crate::client::Want {
            projects: true,
            ..Default::default()
        };
        let action = crate::sync_history::action(&p.cfg.url, &group, &want);
        p.on_msg(Box::new(GlMsg::Loaded {
            group: group.clone(),
            want,
            action,
            result: Ok(crate::client::Synced {
                projects: Some(projects),
                ..Default::default()
            }),
            net: Default::default(),
        }));
    }
    assert!(!p.runs.contains_key(PROJECT) && !p.runs_scopes.contains_key(PROJECT));
    skipped(
        &receive(&mut p, &old, result("SECRET_OLD")),
        Outcome::Succeeded,
    );
    receive(&mut p, &other, result("SECRET_OTHER"));
    assert!(cached(&p, OTHER).is_some());
}

#[test]
fn source_removal_effective_ref_fallback_cannot_revive_on_source_return() {
    for completed in [false, true] {
        let mut p = crate::demo::sample();
        branch(&mut p, PROJECT, "feature/A");
        let old = launch(&mut p, PROJECT);
        if completed {
            receive(&mut p, &old, result("SECRET_HELD"));
        }
        let snapshot = p.snapshot().unwrap();
        let group = p
            .data
            .iter()
            .find(|(_, data)| data.projects.iter().any(|pr| pr.full_path == PROJECT))
            .unwrap()
            .0
            .clone();
        let source = p
            .cfg
            .sources
            .iter()
            .position(|s| p.cfg.group_path(s) == group)
            .unwrap();
        crate::settings::on_menu(&mut p, &format!("tenancy-remove:{source}"), "yes");
        assert_eq!(pages::branch_of(&pages::project(&p, PROJECT)), "main");
        assert!(!p.runs.contains_key(PROJECT) && !p.runs_scopes.contains_key(PROJECT));
        p.restore(snapshot);
        assert_eq!(pages::branch_of(&pages::project(&p, PROJECT)), "feature/A");
        assert!(cached(&p, PROJECT).is_none());
        skipped(
            &receive(&mut p, &old, result("SECRET_OLD")),
            Outcome::Succeeded,
        );
        assert!(!p.runs.contains_key(PROJECT));
    }
}
