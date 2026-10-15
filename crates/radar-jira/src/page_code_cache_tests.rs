//! Reducer-only coverage: optional code failure must not become fresh empty data.

use super::*;
use crate::client::{CodeEvidence, DetailEvidence};
use crate::{CODE_AT_ONCE, CodeEntry, JiraMsg, demo, tests};
use radar_core::Plugin;
use radar_core::journal::Outcome as JournalOutcome;

const SITE: &str = "acme";
const KEY: &str = "ABC-12";

fn id() -> String {
    detail_key(SITE, KEY)
}

fn plugin() -> JiraPlugin {
    let mut p = demo::sample();
    for data in p.data.values_mut() {
        for keys in data.found.values_mut() {
            keys.retain(|key| key == KEY);
        }
    }
    p.data
        .get_mut(SITE)
        .unwrap()
        .tickets
        .get_mut(KEY)
        .unwrap()
        .id = "10012".into();
    p
}

fn prior(p: &JiraPlugin, empty: bool) -> CodeEntry {
    CodeEntry {
        at: p.now - 10,
        updated: p.data[SITE].tickets[KEY].updated.clone(),
        links: if empty { vec![] } else { tests::detail().code },
    }
}

fn got(p: &JiraPlugin, failed: bool) -> DetailRead {
    let mut detail = tests::detail();
    detail.ticket = p.data[SITE].tickets[KEY].clone();
    detail.ticket.summary = "Current detail".into();
    if failed {
        detail.code.clear();
    }
    DetailRead {
        detail,
        evidence: DetailEvidence {
            code_failed: failed,
            ..Default::default()
        },
    }
}

fn receive(p: &mut JiraPlugin, read: Read, got: DetailRead) -> Vec<Effect> {
    p.on_msg(Box::new(JiraMsg::Detail {
        site: SITE.into(),
        key: KEY.into(),
        action: crate::page_history::action(p.cfg.site(SITE).unwrap(), KEY),
        read,
        result: Ok(Box::new(got)),
    }))
}

fn complete(p: &mut JiraPlugin, got: DetailRead) -> (Read, Vec<Effect>) {
    let read = tests::detail_read(p, SITE, KEY);
    (read, receive(p, read, got))
}

fn observed_only(fx: &[Effect], outcome: JournalOutcome) {
    assert!(matches!(fx.first(), Some(Effect::Report(got)) if *got == outcome));
    assert!(
        fx.iter()
            .all(|e| matches!(e, Effect::Report(_) | Effect::Observe { .. })),
        "detail completion adds no request, persistence, or retry effect"
    );
}

#[test]
fn failed_optional_code_preserves_exact_entry_or_absence_and_installs_fresh_detail() {
    for prior_empty in [None, Some(false), Some(true)] {
        let mut p = plugin();
        if let Some(empty) = prior_empty {
            let entry = prior(&p, empty);
            p.data.get_mut(SITE).unwrap().code.insert(KEY.into(), entry);
        }
        let cache = p.data[SITE].code.clone();
        let mut result = got(&p, true);
        result.detail.ticket.updated = "new-ticket-update".into();
        let detail = result.detail.clone();
        let (read, fx) = complete(&mut p, result);
        observed_only(&fx, JournalOutcome::Partial);
        assert_eq!(p.data[SITE].code, cache);
        assert_eq!(p.data[SITE].tickets[KEY], detail.ticket);
        let f = &p.details[&id()];
        assert_eq!(f.value.as_ref(), Some(&detail));
        assert_eq!(f.revision, read.id);
        assert_eq!(f.at, p.now);
        assert!(f.error.is_none() && f.request.is_none() && !f.loading);
        assert!(
            f.value.as_ref().unwrap().code.is_empty(),
            "no visible stale-link restoration"
        );
    }
}

#[test]
fn successful_empty_and_nonempty_reads_replace_code_including_existing_partial_policies() {
    for (empty, policy) in [
        (true, "complete"),
        (false, "complete"),
        (true, "partial"),
        (false, "partial"),
        (true, "invalid-id"),
    ] {
        let mut p = plugin();
        let entry = prior(&p, false);
        p.data.get_mut(SITE).unwrap().code.insert(KEY.into(), entry);
        let mut result = got(&p, false);
        result.detail.ticket.updated = "new-ticket-update".into();
        if empty {
            result.detail.code.clear();
        } else {
            result.detail.code[0].url = "https://code.example/new".into();
        }
        result.evidence.code = match policy {
            "partial" => CodeEvidence {
                omitted_rows: 1,
                incomplete_responses: 1,
                ..Default::default()
            },
            "invalid-id" => CodeEvidence {
                skipped_invalid_id: true,
                ..Default::default()
            },
            _ => CodeEvidence::default(),
        };
        let expected = CodeEntry {
            at: p.now,
            updated: result.detail.ticket.updated.clone(),
            links: result.detail.code.clone(),
        };
        let (_, fx) = complete(&mut p, result);
        observed_only(
            &fx,
            if policy == "complete" {
                JournalOutcome::Succeeded
            } else {
                JournalOutcome::Partial
            },
        );
        assert_eq!(p.data[SITE].code[KEY], expected, "{empty}/{policy}");
        assert!(p.sync_code().is_empty(), "successful empty is fresh too");
    }
}

#[test]
fn changed_ticket_after_code_failure_remains_eligible_for_normal_background_read() {
    let mut p = plugin();
    let old = prior(&p, false);
    p.data
        .get_mut(SITE)
        .unwrap()
        .code
        .insert(KEY.into(), old.clone());
    let mut result = got(&p, true);
    result.detail.ticket.updated = "new-ticket-update".into();
    let (_, fx) = complete(&mut p, result);
    observed_only(&fx, JournalOutcome::Partial);
    assert_eq!(p.data[SITE].code[KEY], old);
    assert_eq!(p.sync_code().len(), 1);
    assert!(p.sync.code_in_flight.contains(&id()));
    assert_eq!(p.sync.code_tried[&id()], p.now);
}

#[test]
fn unchanged_ticket_keeps_its_code_without_reading_it_again_until_the_slow_reread() {
    for empty in [false, true] {
        let mut p = plugin();
        let old = prior(&p, empty);
        p.data
            .get_mut(SITE)
            .unwrap()
            .code
            .insert(KEY.into(), old.clone());
        let result = got(&p, true);
        complete(&mut p, result);
        assert!(p.sync_code().is_empty());
        p.now = p.data[SITE].code[KEY].at + crate::CODE_REREAD_SECS - 1;
        assert!(
            p.sync_code().is_empty(),
            "an unchanged ticket costs no development-panel read by the refresh cycle"
        );
        assert_eq!(p.data[SITE].code[KEY], old);
        // A merge request can be linked without touching the ticket: half
        // an hour on, an open ticket in view is read again (code_reread_tests).
        p.now += 1;
        assert_eq!(p.sync_code().len(), 1);
    }
}

#[test]
fn failed_code_retry_retains_view_inflight_attempt_interval_and_capacity_guards() {
    for held_stale in [false, true] {
        for guard in ["view", "inflight", "attempt", "capacity"] {
            let mut p = plugin();
            if held_stale {
                // Held for an earlier version of the ticket.
                let mut old = prior(&p, false);
                old.updated = "2026-01-01T00:00:00.000+0000".into();
                p.data.get_mut(SITE).unwrap().code.insert(KEY.into(), old);
            }
            let result = got(&p, true);
            complete(&mut p, result);
            let cache = p.data[SITE].code.clone();
            let found = p.data[SITE].found.clone();
            match guard {
                "view" => p.data.get_mut(SITE).unwrap().found.clear(),
                "inflight" => {
                    p.sync.code_in_flight.insert(id());
                }
                "attempt" => {
                    p.sync.code_tried.insert(id(), p.now);
                }
                "capacity" => p
                    .sync
                    .code_in_flight
                    .extend((0..CODE_AT_ONCE).map(|n| detail_key(SITE, &format!("ABC-{n}")))),
                _ => unreachable!(),
            }
            assert!(p.sync_code().is_empty(), "{held_stale}/{guard}");
            match guard {
                "view" => p.data.get_mut(SITE).unwrap().found = found,
                "inflight" | "capacity" => p.sync.code_in_flight.clear(),
                "attempt" => {
                    p.now += p.cfg.refresh_secs as i64 - 1;
                    assert!(p.sync_code().is_empty(), "attempt interval still applies");
                    p.now += 1;
                }
                _ => unreachable!(),
            }
            assert_eq!(p.sync_code().len(), 1, "{held_stale}/{guard}");
            assert!(p.sync.code_in_flight.contains(&id()));
            assert_eq!(p.data[SITE].code, cache);
        }
    }
}

#[test]
fn page_only_tickets_still_do_not_create_general_code_cache_entries() {
    for failed in [false, true] {
        let mut p = plugin();
        let result = got(&p, failed);
        p.data.get_mut(SITE).unwrap().tickets.remove(KEY);
        complete(&mut p, result);
        assert!(!p.data[SITE].code.contains_key(KEY));
        assert!(!p.data[SITE].tickets.contains_key(KEY));
        assert!(p.details[&id()].value.is_some());
    }
}

#[test]
fn rejected_success_and_optional_failure_never_mutate_code_or_new_detail_owner() {
    for transition in [
        "duplicate",
        "new-owner",
        "connection",
        "evicted",
        "wrong-owner",
    ] {
        for failed in [false, true] {
            let mut p = plugin();
            let result = got(&p, false);
            let (old, _) = complete(&mut p, result);
            let mut stale = got(&p, failed);
            stale.detail.ticket.summary = "STALE_DETAIL".into();
            stale.detail.ticket.updated = "STALE_UPDATE".into();
            let mut owner = old;
            match transition {
                "duplicate" => {}
                "new-owner" => {
                    tests::detail_read(&mut p, SITE, KEY);
                }
                "connection" => {
                    p.cfg.sites[0].token = "env:OTHER_TOKEN".into();
                    crate::write_context::reconcile(&mut p);
                    tests::detail_read(&mut p, SITE, KEY);
                }
                "evicted" => {
                    p.details.remove(&id());
                }
                "wrong-owner" => {
                    owner = tests::detail_read(&mut p, SITE, KEY);
                    owner.id += 1;
                }
                _ => unreachable!(),
            }
            let snapshot = p.snapshot();
            let details = format!("{:?}", p.details);
            let fx = receive(&mut p, owner, stale);
            observed_only(
                &fx,
                if failed {
                    JournalOutcome::Partial
                } else {
                    JournalOutcome::Succeeded
                },
            );
            assert!(
                matches!(fx.last(), Some(Effect::Observe { action, outcome: JournalOutcome::Blocked })
                if action.kind.as_str() == "jira.page.detail.application.skipped")
            );
            assert_eq!(p.snapshot(), snapshot, "{transition}/{failed}");
            assert_eq!(format!("{:?}", p.details), details, "{transition}/{failed}");
        }
    }
}

#[test]
fn preserved_code_roundtrips_with_unchanged_schema_and_no_transient_evidence() {
    let mut p = plugin();
    let old = prior(&p, false);
    p.data
        .get_mut(SITE)
        .unwrap()
        .code
        .insert(KEY.into(), old.clone());
    let config = p.config();
    let result = got(&p, true);
    let (_, fx) = complete(&mut p, result);
    let snapshot = p.snapshot().unwrap();
    assert_eq!(
        snapshot["data"][SITE]["code"][KEY],
        serde_json::to_value(&old).unwrap()
    );
    assert_eq!(
        snapshot["data"][SITE]["code"][KEY]
            .as_object()
            .unwrap()
            .len(),
        3
    );
    let serialized = snapshot.to_string();
    for forbidden in [
        "code_failed",
        "DetailEvidence",
        "\"request\"",
        "\"generation\"",
        "jira.page.detail",
        "OTHER_TOKEN",
    ] {
        assert!(!serialized.contains(forbidden), "{forbidden}");
    }
    for effect in fx {
        if let Effect::Observe { action, .. } = effect {
            let serialized = format!("{action:?}");
            for forbidden in [
                "Current detail",
                "abc-12-stats",
                "merge_requests",
                "alice@example.com",
                "RADAR_JIRA_TEST_TOKEN",
            ] {
                assert!(!serialized.contains(forbidden), "{forbidden}");
            }
        }
    }
    let mut restored = JiraPlugin::new();
    restored.restore(snapshot);
    assert_eq!(restored.data[SITE].code[KEY], old);
    assert_eq!(p.config(), config);
}
