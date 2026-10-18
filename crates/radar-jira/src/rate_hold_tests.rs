//! Jira's rate-limit headers hold the background reads of a site that is
//! nearly out of allowance, alongside the backoff after failures: a list
//! sync, development panels and an open page's checks wait, `r` does not.

use super::*;
use crate::client::recorded::{Replay, block};
use crate::client::Resp;
use crate::rate::{HOLD_MAX_SECS, Rate as Headers};
use crate::tests::{complete_detail, detail};

fn acme_jql(p: &JiraPlugin) -> String {
    p.jql_for(0, &p.cfg.views[0].clone()).unwrap()
}

fn answer(status: u16, retry_after: Option<u64>, rate: Headers) -> Resp {
    Resp { status, retry_after, rate, body: String::new() }
}

/// Jira said acme has `remaining` of 1000 until `reset`.
fn running_low(p: &JiraPlugin, remaining: u64, reset: i64) {
    let rate = Headers { limit: Some(1000), remaining: Some(remaining), reset: Some(reset), reason: None };
    p.limits.note("acme", &answer(200, None, rate));
}

#[test]
fn a_site_under_a_tenth_of_its_allowance_is_not_synced_until_its_reset() {
    let mut p = sample_without_provider_ids();
    let (jql, site) = (acme_jql(&p), p.cfg.sites[0].clone());
    let every = p.cfg.refresh_secs as i64;
    running_low(&p, 80, NOW + 600);
    assert_eq!(p.plan(&site, &jql, false), None, "a tenth gone, and then some");
    assert!(p.plan(&site, &jql, true).is_some(), "r asks anyway");
    // The tick asks beta alone: one task, and it is not acme's.
    let fx = p.tick(NOW);
    assert_eq!(fx.len(), 1, "beta only");
    assert!(p.sync.in_flight.iter().all(|id| id.starts_with("beta\0")), "{:?}", p.sync.in_flight);
    // Still held a minute before the reset, however many refresh intervals have gone.
    p.now = NOW + 599;
    assert!(p.now - NOW > every);
    assert_eq!(p.plan(&site, &jql, false), None);
    p.now = NOW + 600;
    assert!(p.plan(&site, &jql, false).is_some(), "refilled");
}

#[test]
fn a_site_with_plenty_left_is_synced_as_always() {
    let p = sample();
    let (jql, site) = (acme_jql(&p), p.cfg.sites[0].clone());
    running_low(&p, 400, NOW + 600);
    assert!(p.plan(&site, &jql, false).is_some());
}

#[test]
fn a_retry_after_from_any_read_holds_the_site_from_the_next_tick() {
    let mut p = sample();
    let (jql, site) = (acme_jql(&p), p.cfg.sites[0].clone());
    // A page or a panel was told to wait 15 minutes: the list sync waits too,
    // although only a list sync's failure feeds the backoff.
    p.limits.note("acme", &answer(429, Some(900), Headers::default()));
    p.tick(NOW);
    assert_eq!(p.plan(&site, &jql, false), None);
    assert!(!p.sync.backoff.contains_key("acme"), "this is not the sync backoff");
    p.now = NOW + 899;
    assert_eq!(p.plan(&site, &jql, false), None);
    p.now = NOW + 900;
    assert!(p.plan(&site, &jql, false).is_some());
}

#[test]
fn what_a_client_hears_reaches_the_plugin_and_an_hour_is_the_longest_wait() {
    let p = sample();
    let reset = NOW + 10 * HOLD_MAX_SECS;
    let low = Headers { limit: Some(100), remaining: Some(2), reset: Some(reset), reason: Some("jira-quota-tenant-based".into()) };
    let r = Replay::saying(vec![answer(200, None, low)]);
    let client = Client::over(r).limited("acme", &p.limits);
    let req = crate::client::Req { method: crate::client::Method::Get, path: "/x".into(), body: None };
    block(client.request(req, "x", true)).unwrap();
    assert_eq!(p.limits.hold("acme", NOW), Some(NOW + HOLD_MAX_SECS));
    assert_eq!(p.limits.rate("acme").unwrap().reason.as_deref(), Some("jira-quota-tenant-based"));
}

#[test]
fn development_panels_of_a_held_site_wait_and_come_when_it_is_free() {
    let mut p = numbered();
    p.sync.in_flight.clear();
    running_low(&p, 10, NOW + 600);
    assert!(!p.sync_code().is_empty(), "beta's are asked");
    assert!(
        p.sync.code_in_flight.iter().all(|id| id.starts_with("beta\0")),
        "acme waits: {:?}",
        p.sync.code_in_flight
    );
    p.sync.code_in_flight.clear();
    p.sync.code_tried.clear();
    p.now = NOW + 600;
    p.sync_code();
    assert!(p.sync.code_in_flight.iter().any(|id| id.starts_with("acme\0")), "acme is free again");
}

#[test]
fn an_open_pages_timer_checks_wait_for_a_held_site_and_opening_does_not() {
    let mut p = sample();
    let every = p.cfg.refresh_secs as i64;
    let mut d = detail();
    d.ticket.key = "ABC-99".into();
    let fx = page::open(&mut p, "acme", "ABC-99");
    assert!(fx.iter().any(|e| matches!(e, Effect::SpawnAction { .. })), "opening reads");
    complete_detail(&mut p, "acme", "ABC-99", Ok(d));
    running_low(&p, 10, NOW + 3000);
    p.now = NOW + every;
    let fx = p.tick(p.now);
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::Spawn(_))),
        "no check of the page's ticket while acme is held"
    );
    p.now = NOW + 3000;
    let fx = p.tick(p.now);
    assert!(fx.iter().any(|e| matches!(e, Effect::Spawn(_))), "and one when it is free");
}
