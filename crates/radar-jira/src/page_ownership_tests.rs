use super::*;
use crate::{JiraMsg, client::DetailRead, demo, tests, write_context};
use radar_core::Plugin;
use radar_core::journal::{ActionDescriptor, Outcome as JournalOutcome};
use radar_core::ratatui::crossterm::event::KeyModifiers;

const SITE: &str = "acme";
const KEY: &str = "ABC-12";

fn id() -> String {
    detail_key(SITE, KEY)
}
fn event(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn launch(p: &mut JiraPlugin) -> (Read, ActionDescriptor) {
    let fx = fetch(p, SITE, KEY, true);
    let [Effect::SpawnAction { action, .. }] = &fx[..] else {
        panic!("one independent read")
    };
    (p.details[&id()].request.unwrap(), action.clone())
}

fn receive(
    p: &mut JiraPlugin,
    read: Read,
    action: &ActionDescriptor,
    result: Result<Detail, String>,
) -> Vec<Effect> {
    p.on_msg(Box::new(JiraMsg::Detail {
        site: SITE.into(),
        key: KEY.into(),
        read,
        action: action.clone(),
        result: result.map(|detail| {
            Box::new(DetailRead {
                detail,
                ..Default::default()
            })
        }),
    }))
}

fn installed() -> JiraPlugin {
    let mut p = demo::sample();
    open(&mut p, SITE, KEY);
    tests::complete_detail(&mut p, SITE, KEY, Ok(tests::detail()));
    p
}

fn more(p: &mut JiraPlugin) -> More {
    assert!(matches!(
        &fetch_more(p, SITE, KEY)[..],
        [Effect::SpawnAction { .. }]
    ));
    p.details[&id()].more.unwrap()
}

fn older() -> Result<(Vec<crate::model::Comment>, usize), String> {
    Ok((
        vec![crate::model::Comment {
            author: "Older".into(),
            created: "then".into(),
            body: demo::doc("old body"),
        }],
        3,
    ))
}

fn state(p: &JiraPlugin) -> String {
    format!("{:?}", p.details)
}

fn skipped(fx: &[Effect], scope: &ActionDescriptor, outcome: JournalOutcome) {
    assert!(matches!(fx.first(), Some(Effect::Report(got)) if *got == outcome));
    let Some(Effect::Observe {
        action,
        outcome: JournalOutcome::Blocked,
    }) = fx.last()
    else {
        panic!("separate skipped observation follows original evidence")
    };
    assert_eq!(action.kind.as_str(), "jira.page.detail.application.skipped");
    assert_eq!(action.subjects, scope.subjects);
    assert_eq!(action.metadata.count, Some(1));
    assert_eq!(action.metadata.redacted, scope.metadata.redacted);
    assert_eq!(
        action.metadata.omitted_subjects,
        scope.metadata.omitted_subjects
    );
    assert!(action.metadata.reason.is_none());
    assert!(!format!("{action:?}").contains("SECRET"));
}

#[test]
fn current_detail_success_and_failure_preserve_the_successful_revision_and_freshness() {
    let mut p = demo::sample();
    let (read, scope) = launch(&mut p);
    let fx = receive(&mut p, read, &scope, Ok(tests::detail()));
    assert!(matches!(
        fx.first(),
        Some(Effect::Report(JournalOutcome::Succeeded))
    ));
    assert_eq!(p.details[&id()].revision, read.id);
    assert!(!p.details[&id()].loading);
    assert!(p.details[&id()].request.is_none());
    let (value, at, revision) = (
        p.details[&id()].value.clone(),
        p.details[&id()].at,
        p.details[&id()].revision,
    );
    let snapshot = p.snapshot();
    let (next, scope) = launch(&mut p);
    p.now += 5;
    let fx = receive(&mut p, next, &scope, Err("current failure".into()));
    assert!(matches!(
        fx.first(),
        Some(Effect::Report(JournalOutcome::Failed))
    ));
    let f = &p.details[&id()];
    assert_eq!((f.value.clone(), f.at, f.revision), (value, at, revision));
    assert_eq!(f.error.as_deref(), Some("current failure"));
    assert!(!f.loading);
    assert!(f.request.is_none());
    assert_eq!(p.snapshot(), snapshot);
}

#[test]
fn duplicate_detail_success_and_error_never_apply_or_clear_a_new_owner() {
    for newer_pending in [false, true] {
        for failure in [false, true] {
            let mut p = demo::sample();
            let (old, scope) = launch(&mut p);
            receive(&mut p, old, &scope, Ok(tests::detail()));
            if newer_pending {
                launch(&mut p);
            }
            let before = state(&p);
            let snapshot = p.snapshot();
            let mut stale = tests::detail();
            stale.ticket.summary = "SECRET_STALE".into();
            let fx = receive(
                &mut p,
                old,
                &scope,
                if failure {
                    Err("SECRET_FAILURE".into())
                } else {
                    Ok(stale)
                },
            );
            skipped(
                &fx,
                &scope,
                if failure {
                    JournalOutcome::Failed
                } else {
                    JournalOutcome::Succeeded
                },
            );
            assert_eq!(state(&p), before);
            assert_eq!(p.snapshot(), snapshot);
        }
    }
}

#[test]
fn configured_connection_changes_reject_late_detail_success_and_error() {
    for transition in [
        "url",
        "email",
        "token",
        "rename",
        "remove",
        "aba",
        "readd",
        "rename_back",
    ] {
        for failure in [false, true] {
            let mut p = installed();
            let (old, scope) = launch(&mut p);
            let original = p.cfg.sites[0].clone();
            match transition {
                "url" | "aba" => p.cfg.sites[0].url = "https://other.example".into(),
                "email" => p.cfg.sites[0].email = "other@example.com".into(),
                "token" => p.cfg.sites[0].token = "env:OTHER_TOKEN".into(),
                "rename" | "rename_back" => p.cfg.sites[0].name = "renamed".into(),
                "remove" | "readd" => {
                    p.cfg.sites.remove(0);
                }
                _ => unreachable!(),
            }
            write_context::reconcile(&mut p);
            assert!(!p.details.contains_key(&id()), "{transition}");
            if matches!(transition, "aba" | "rename_back") {
                p.cfg.sites[0] = original;
                write_context::reconcile(&mut p);
            } else if transition == "readd" {
                p.cfg.sites.insert(0, original);
                write_context::reconcile(&mut p);
            }
            if p.cfg.site(SITE).is_some() {
                launch(&mut p);
            }
            let before = state(&p);
            let snapshot = p.snapshot();
            let mut stale = tests::detail();
            stale.ticket.summary = "STALE ORIGINAL CONNECTION RESPONSE".into();
            let fx = receive(
                &mut p,
                old,
                &scope,
                if failure {
                    Err("SECRET_FAILURE".into())
                } else {
                    Ok(stale)
                },
            );
            skipped(
                &fx,
                &scope,
                if failure {
                    JournalOutcome::Failed
                } else {
                    JournalOutcome::Succeeded
                },
            );
            assert_eq!(state(&p), before, "{transition}");
            assert_eq!(p.snapshot(), snapshot, "{transition}");
            assert!(p.next_page_read >= old.id);
            assert_eq!(p.pages.len(), 1);
            assert_eq!(
                (&p.pages[0].site, &p.pages[0].key),
                (&SITE.to_string(), &KEY.to_string())
            );
        }
    }
}

#[test]
fn other_sites_and_non_connection_settings_keep_owners_and_cache() {
    let mut p = installed();
    let read = more(&mut p);
    let before = state(&p);
    let generation = p.connections.generation(SITE);
    p.cfg.sites[0].color = "green".into();
    p.cfg.sites[0].projects.clear();
    p.cfg.sites[1].url = "https://beta-new.example".into();
    p.cfg.refresh_secs += 60;
    write_context::reconcile(&mut p);
    assert_eq!(p.connections.generation(SITE), generation);
    assert_eq!(state(&p), before);
    assert!(on_more(&mut p, SITE, KEY, read, older()));
}

#[test]
fn current_more_success_and_failure_are_one_use_without_changing_full_revision() {
    for failure in [false, true] {
        let mut p = installed();
        let read = more(&mut p);
        let revision = p.details[&id()].revision;
        assert!(on_more(
            &mut p,
            SITE,
            KEY,
            read,
            if failure {
                Err("current paging failure".into())
            } else {
                older()
            }
        ));
        assert_eq!(p.details[&id()].revision, revision);
        assert!(p.details[&id()].more.is_none());
        assert_eq!(
            p.details[&id()].value.as_ref().unwrap().comments.len(),
            if failure { 2 } else { 3 }
        );
        assert_eq!(
            p.details[&id()].error.as_deref(),
            failure.then_some("current paging failure")
        );
        let before = state(&p);
        assert!(!on_more(&mut p, SITE, KEY, read, older()));
        assert_eq!(state(&p), before);
    }
}

#[test]
fn duplicate_more_success_and_failure_cannot_clear_or_append_into_new_more() {
    for failure in [false, true] {
        let mut p = installed();
        p.details
            .get_mut(&id())
            .unwrap()
            .value
            .as_mut()
            .unwrap()
            .comments_total = 9;
        let old = more(&mut p);
        assert!(on_more(&mut p, SITE, KEY, old, Ok((vec![], 9))));
        let next = more(&mut p);
        assert_ne!(old.read.id, next.read.id);
        let before = state(&p);
        assert!(!on_more(
            &mut p,
            SITE,
            KEY,
            old,
            if failure {
                Err("stale paging failure".into())
            } else {
                older()
            }
        ));
        assert_eq!(state(&p), before);
    }
}

#[test]
fn full_refresh_supersedes_more_in_both_completion_orders() {
    for detail_first in [false, true] {
        for more_failure in [false, true] {
            let mut p = installed();
            let old = more(&mut p);
            let (full, scope) = launch(&mut p);
            assert!(p.details[&id()].more.is_none());
            assert!(fetch_more(&mut p, SITE, KEY).is_empty());
            if detail_first {
                receive(&mut p, full, &scope, Ok(tests::detail()));
            }
            let before = state(&p);
            assert!(!on_more(
                &mut p,
                SITE,
                KEY,
                old,
                if more_failure {
                    Err("late".into())
                } else {
                    older()
                }
            ));
            assert_eq!(state(&p), before);
            if !detail_first {
                receive(&mut p, full, &scope, Ok(tests::detail()));
            }
            assert_eq!(p.details[&id()].value.as_ref().unwrap().comments.len(), 2);
            assert_eq!(p.details[&id()].revision, full.id);
            assert!(!loading(&p));
        }
    }
}

#[test]
fn completed_more_then_full_refresh_replaces_its_prefix() {
    let mut p = installed();
    let old = more(&mut p);
    assert!(on_more(&mut p, SITE, KEY, old, older()));
    let (full, scope) = launch(&mut p);
    receive(&mut p, full, &scope, Ok(tests::detail()));
    assert_eq!(p.details[&id()].value.as_ref().unwrap().comments.len(), 2);
    assert_eq!(p.details[&id()].revision, full.id);
}

#[test]
fn failed_refresh_releases_new_more_without_reviving_retired_more() {
    let mut p = installed();
    let old = more(&mut p);
    let (value, revision, at) = (
        p.details[&id()].value.clone(),
        p.details[&id()].revision,
        p.details[&id()].at,
    );
    let (full, scope) = launch(&mut p);
    p.now += p.cfg.refresh_secs as i64;
    receive(&mut p, full, &scope, Err("refresh failed".into()));
    let f = &p.details[&id()];
    assert_eq!((f.value.clone(), f.revision, f.at), (value, revision, at));
    let new = more(&mut p);
    assert_eq!((new.revision, new.start), (old.revision, old.start));
    assert_ne!(new.read.id, old.read.id);
    let before = state(&p);
    assert!(!on_more(&mut p, SITE, KEY, old, older()));
    assert!(!on_more(&mut p, SITE, KEY, old, Err("late error".into())));
    assert_eq!(state(&p), before);
    assert!(on_more(&mut p, SITE, KEY, new, older()));
    assert!(
        matches!(
            &fetch(&mut p, SITE, KEY, false)[..],
            [Effect::SpawnAction { .. }]
        ),
        "existing stale failed-refresh cadence is unchanged"
    );
}

#[test]
fn more_requires_unchanged_local_revision_offset_and_generation() {
    for drift in ["revision", "offset", "generation"] {
        for failure in [false, true] {
            let mut p = installed();
            let read = more(&mut p);
            let f = p.details.get_mut(&id()).unwrap();
            match drift {
                "revision" => f.revision += 1,
                "offset" => {
                    f.value.as_mut().unwrap().comments.pop();
                }
                "generation" => {
                    p.cfg.sites[0].token = "env:NEW_TOKEN".into();
                    write_context::reconcile(&mut p);
                    launch(&mut p);
                }
                _ => unreachable!(),
            }
            let before = state(&p);
            let held = p.details[&id()].value.clone();
            let revision = p.details[&id()].revision;
            let at = p.details[&id()].at;
            let error = p.details[&id()].error.clone();
            assert!(!on_more(
                &mut p,
                SITE,
                KEY,
                read,
                if failure { Err("late".into()) } else { older() }
            ));
            if drift == "generation" {
                assert_eq!(state(&p), before, "a newer connection's owner is untouched");
            } else {
                let f = &p.details[&id()];
                assert_eq!(
                    (&f.value, f.revision, f.at, &f.error),
                    (&held, revision, at, &error)
                );
                assert!(
                    f.more.is_none(),
                    "only this exact invalidated owner was retired"
                );
                assert!(!loading(&p));
                let replacement = more(&mut p);
                assert_ne!(replacement.read.id, read.read.id);
                assert_eq!(replacement.revision, revision);
                assert_eq!(replacement.start, held.as_ref().unwrap().comments.len());
            }
        }
    }
}

#[test]
fn mismatched_exact_ids_leave_both_current_owners_untouched() {
    let mut p = installed();
    let read = more(&mut p);
    let before = state(&p);
    let forged = More {
        read: Read {
            id: read.read.id + 1,
            ..read.read
        },
        ..read
    };
    assert!(!on_more(&mut p, SITE, KEY, forged, older()));
    assert_eq!(state(&p), before);
    let (read, scope) = launch(&mut p);
    let before = state(&p);
    let forged = Read {
        id: read.id + 1,
        ..read
    };
    skipped(
        &receive(&mut p, forged, &scope, Err("wrong owner".into())),
        &scope,
        JournalOutcome::Failed,
    );
    assert_eq!(state(&p), before);
}

#[test]
fn settings_immediately_clear_stacked_held_extras_and_enter_before_redraw_is_inert() {
    for target in [
        crate::Target::SiteUrl(0),
        crate::Target::SiteEmail(0),
        crate::Target::SiteToken(0),
        crate::Target::SiteName(0),
    ] {
        let mut host = tests::hosted();
        let p = tests::jira_mut(&mut host);
        open(p, SITE, KEY);
        tests::complete_detail(p, SITE, KEY, Ok(tests::detail()));
        let _ = radar_core::shell::to_text(&host, 160, 40);
        let p = tests::jira_mut(&mut host);
        assert!(
            !p.pages[0].extra.borrow().is_empty(),
            "actual page render holds actionable Code rows"
        );
        p.pages[0].tab = TABS.len();
        p.pages[0].row.set(7);
        open(p, SITE, KEY);
        let held = p.pages[0].extra.borrow().clone();
        *p.pages[1].extra.borrow_mut() = held;
        p.pages[1].tab = TABS.len();
        let value = match target {
            crate::Target::SiteUrl(_) => "https://other.example",
            crate::Target::SiteEmail(_) => "other@example.com",
            crate::Target::SiteToken(_) => "TEST_ONLY_NEW_TOKEN_123",
            _ => "renamed",
        };
        p.edit = Some((target, radar_core::input::TextInput::new(value)));
        crate::settings::key(p, "connection", event(KeyCode::Enter));
        assert!(p.details.is_empty());
        assert_eq!(p.pages.len(), 2);
        for page in &p.pages {
            assert!(page.extra.borrow().is_empty());
            assert!(page.tab < TABS.len());
            assert_eq!(page.row.get(), 0);
        }
        assert!(matches!(key(p, event(KeyCode::Enter)), Outcome::Handled(fx) if fx.is_empty()));
        key(p, event(KeyCode::Esc));
        assert!(matches!(key(p, event(KeyCode::Enter)), Outcome::Handled(fx) if fx.is_empty()));
    }
}

#[test]
fn configure_removal_rename_and_readd_do_not_migrate_page_cache() {
    let mut p = installed();
    let original = p.cfg.clone();
    let mut changed = original.clone();
    changed.sites[0].name = "renamed".into();
    p.configure(Some(&toml::Value::try_from(&changed).unwrap()))
        .unwrap();
    assert!(p.details.is_empty());
    assert_eq!(p.pages.len(), 1);
    p.configure(None).unwrap();
    p.configure(Some(&toml::Value::try_from(&original).unwrap()))
        .unwrap();
    assert!(p.details.is_empty());
    assert!(matches!(
        &fetch(&mut p, SITE, KEY, false)[..],
        [Effect::SpawnAction { .. }]
    ));
}

#[test]
fn invalidation_preserves_all_composer_recovery_and_write_state() {
    use crate::write::{Change, Compose, Draft, NewTicket, Sent};
    use crate::write_recovery::{Context, Original, Record, Status};
    let mut p = installed();
    let context = Context::capture(&mut p, SITE);
    let change = Change::Comment {
        site: SITE.into(),
        key: KEY.into(),
        text: "exact  \noriginal\n".into(),
    };
    p.compose = Some(Compose {
        site: SITE.into(),
        draft: Draft::Comment { key: KEY.into() },
        composer: radar_core::ui::composer::Composer::new("exact  \noriginal\n"),
        context: context.clone(),
        status: Status::Uncertain,
        saved_description: "saved  \n".into(),
    });
    p.change = Some(change.clone());
    p.moving = Some((SITE.into(), KEY.into(), vec![]));
    p.new_ticket = Some(NewTicket {
        site: SITE.into(),
        project: "ABC".into(),
        types: vec![],
    });
    p.asking.insert("pending".into());
    let action = crate::page_history::action(p.cfg.site(SITE).unwrap(), KEY);
    let dispatch = write_context::Dispatch {
        id: 31,
        generation: context.generation.unwrap(),
        action: action.clone(),
    };
    p.sent.insert(
        id(),
        Sent {
            dispatch: Some(dispatch.clone()),
            words: "pending comment".into(),
            result: None,
        },
    );
    let original = p
        .recovery
        .reserve(Record {
            context,
            change: change.clone(),
            original: Original::Comment("exact  \noriginal\n".into()),
            status: Status::Unsent,
        })
        .unwrap();
    p.recovery.accept(original, &dispatch);
    p.starting = Some(crate::start::Start {
        site: SITE.into(),
        key: KEY.into(),
        assign: Some(change.clone()),
        mv: None,
    });
    p.start_next = Some(crate::start::Pending {
        owner: dispatch,
        change,
        action: action.clone(),
    });
    p.dialogs.read(action);
    p.next_write = 31;
    let preserved = |p: &JiraPlugin| {
        format!(
            "{:?}",
            (
                &p.compose,
                &p.change,
                &p.moving,
                &p.new_ticket,
                &p.asking,
                &p.sent,
                &p.recovery,
                &p.starting,
                &p.start_next,
                &p.dialogs,
                p.next_write
            )
        )
    };
    let before = preserved(&p);
    p.cfg.sites[0].token = "env:DIFFERENT_TOKEN".into();
    write_context::reconcile(&mut p);
    assert_eq!(preserved(&p), before);
    assert_eq!(p.compose.as_ref().unwrap().text(), "exact  \noriginal\n");
}

#[test]
fn dedicated_cache_invalidation_deliberately_retains_unowned_list_fallback() {
    let mut p = installed();
    let snapshot = p.snapshot();
    let summary = ticket(&p, &p.pages[0]).unwrap().summary.clone();
    p.cfg.sites[0].url = "https://other.example".into();
    write_context::reconcile(&mut p);
    assert!(p.details.is_empty());
    // Only the pages kept in the cache go with the connection.
    let mut expected = snapshot.unwrap();
    expected["data"][SITE].as_object_mut().unwrap().remove("details");
    assert_eq!(
        p.snapshot().unwrap(),
        expected,
        "general SiteData ownership is a separate task"
    );
    assert_eq!(ticket(&p, &p.pages[0]).unwrap().summary, summary);
    assert_eq!(crate::join::held(&p, SITE, KEY).unwrap().summary, summary);
    assert!(!p.data[SITE].code[KEY].links.is_empty());
}

#[test]
fn automatic_noops_stay_empty_and_missing_site_manual_refresh_is_accurate() {
    let mut p = installed();
    assert!(fetch(&mut p, "missing", KEY, false).is_empty());
    assert!(fetch(&mut p, SITE, KEY, false).is_empty());
    launch(&mut p);
    for force in [false, true] {
        assert!(fetch(&mut p, SITE, KEY, force).is_empty());
    }
    assert!(
        matches!(key(&mut p, event(KeyCode::Char('r'))), Outcome::Handled(fx) if matches!(&fx[..], [Effect::Notify(message)] if message == "already asking Jira"))
    );
    p.cfg.sites.clear();
    p.cfg.views.clear();
    write_context::reconcile(&mut p);
    let now = p.now;
    // The page read at the start is written once, then nothing more.
    assert!(matches!(&p.tick(now)[..], [Effect::PersistCache]));
    assert!(p.tick(now).is_empty());
    assert!(p.tick(now + 1).is_empty());
    assert!(
        matches!(key(&mut p, event(KeyCode::Char('r'))), Outcome::Handled(fx) if matches!(&fx[..], [Effect::Notify(message)] if message == "Jira site is no longer configured"))
    );
}

#[test]
fn dispatched_more_carries_exact_owner_without_resolving_client_until_polled() {
    let mut p = demo::sample();
    p.cfg.sites[0].email.clear();
    tests::complete_detail(&mut p, SITE, KEY, Ok(tests::detail()));
    let fx = fetch_more(&mut p, SITE, KEY);
    let owner = p.details[&id()].more.unwrap();
    assert_eq!(owner.start, 2);
    assert_eq!(owner.revision, p.details[&id()].revision);
    let task = fx
        .into_iter()
        .find_map(|fx| match fx {
            Effect::SpawnAction { task, .. } => Some(task),
            _ => None,
        })
        .unwrap();
    let msg = crate::client::recorded::block(task);
    let JiraMsg::More {
        read,
        site,
        key,
        result,
        ..
    } = *msg.downcast::<JiraMsg>().unwrap()
    else {
        panic!("More reply")
    };
    assert_eq!((read, site.as_str(), key.as_str()), (owner, SITE, KEY));
    assert!(result.unwrap_err().contains("no email set"));
}

#[test]
fn site_removal_clears_empty_cache_page_extras_and_preserves_other_sites() {
    let mut p = installed();
    tests::complete_detail(&mut p, "beta", "XYZ-7", Ok(tests::detail()));
    let untouched = format!("{:?}", p.details[&detail_key("beta", "XYZ-7")]);
    p.details.remove(&id());
    *p.pages[0].extra.borrow_mut() = vec![("third-party tab".into(), vec![])];
    p.pages[0].tab = TABS.len();
    crate::settings::on_menu(&mut p, "site-remove:0", "yes");
    assert!(p.pages[0].extra.borrow().is_empty());
    assert_eq!(p.pages[0].tab, LINKS);
    assert_eq!(
        format!("{:?}", p.details[&detail_key("beta", "XYZ-7")]),
        untouched
    );
}

#[test]
fn rejected_more_reports_actual_result_then_scoped_local_skip() {
    for failure in [false, true] {
        let mut p = installed();
        let read = more(&mut p);
        let scope = crate::comments_history::action(p.cfg.site(SITE).unwrap(), KEY, read.start);
        p.cfg.sites[0].url = "https://other.example".into();
        write_context::reconcile(&mut p);
        launch(&mut p);
        let before = state(&p);
        let fx = p.on_msg(Box::new(JiraMsg::More {
            site: SITE.into(),
            key: KEY.into(),
            action: scope.clone(),
            read,
            result: if failure {
                Err("SECRET_FAILURE".into())
            } else {
                older().map(|(comments, total)| crate::client::CommentsRead {
                    comments,
                    total,
                    evidence: Default::default(),
                })
            },
        }));
        let expected = if failure {
            JournalOutcome::Failed
        } else {
            JournalOutcome::Succeeded
        };
        assert!(matches!(fx.first(), Some(Effect::Report(outcome)) if *outcome == expected));
        let Some(Effect::Observe {
            action,
            outcome: JournalOutcome::Blocked,
        }) = fx.last()
        else {
            panic!("local refusal follows actual outcome")
        };
        assert_eq!(action.kind.as_str(), "jira.page.comments.application.skipped");
        assert_eq!(action.metadata.count, Some(1));
        assert_eq!(action.subjects, scope.subjects);
        assert!(!format!("{action:?}").contains("SECRET"));
        assert_eq!(state(&p), before);
    }
}

#[test]
fn old_more_cannot_apply_after_connection_aba_or_readd_even_with_identical_prefix() {
    for transition in ["url", "email", "token", "aba", "remove", "readd"] {
        for failure in [false, true] {
            let mut p = installed();
            let read = more(&mut p);
            let original = p.cfg.sites[0].clone();
            match transition {
                "url" | "aba" => p.cfg.sites[0].url = "https://replacement.example".into(),
                "email" => p.cfg.sites[0].email = "replacement@example.com".into(),
                "token" => p.cfg.sites[0].token = "env:NEW_TOKEN".into(),
                _ => {
                    p.cfg.sites.remove(0);
                }
            }
            write_context::reconcile(&mut p);
            if transition == "aba" {
                p.cfg.sites[0] = original;
            } else if transition == "readd" {
                p.cfg.sites.insert(0, original);
            }
            write_context::reconcile(&mut p);
            if p.cfg.site(SITE).is_some() {
                tests::complete_detail(&mut p, SITE, KEY, Ok(tests::detail()));
                let next = more(&mut p);
                assert_eq!(next.start, read.start);
                assert_ne!(next.read.generation, read.read.generation);
                assert_ne!(next.read.id, read.read.id);
            }
            let before = state(&p);
            assert!(!on_more(
                &mut p,
                SITE,
                KEY,
                read,
                if failure {
                    Err("stale error".into())
                } else {
                    older()
                }
            ));
            assert_eq!(state(&p), before, "{transition}");
        }
    }
}

#[test]
fn exact_more_rejected_during_full_read_retires_only_more_owner() {
    let mut p = installed();
    let old = more(&mut p);
    let (full, _) = launch(&mut p);
    // Defensive invariant: even if an external mutation reintroduced this owner,
    // its completion cannot release or mutate the current full read.
    p.details.get_mut(&id()).unwrap().more = Some(old);
    assert!(!on_more(&mut p, SITE, KEY, old, Err("stale error".into())));
    let f = &p.details[&id()];
    assert!(f.more.is_none());
    assert_eq!(f.request, Some(full));
    assert!(f.loading);
    assert!(f.error.is_none());
    assert_eq!(f.value, Some(tests::detail()));
    assert_eq!(f.at, demo::NOW);
}
