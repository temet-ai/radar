//! A ticket page is kept in jira.json against the ticket's `updated`: it
//! opens drawn after a restart and costs nothing while unchanged. A page of
//! a ticket outside the view is checked on a timer by its `updated` alone,
//! and read again only when that moved.

use super::*;
use crate::tests::{complete_detail, detail};

const SITE: &str = "acme";
const KEY: &str = "ABC-12";

fn page_reads(fx: &[Effect]) -> usize {
    fx.iter().filter(|e| matches!(e, Effect::SpawnAction { .. })).count()
}

fn checks(fx: &[Effect]) -> usize {
    fx.iter().filter(|e| matches!(e, Effect::Spawn(_))).count()
}

/// A plugin as after a restart: the cache of `p`, nothing else held.
fn restarted(p: &JiraPlugin) -> JiraPlugin {
    let mut q = sample();
    q.data.clear();
    q.restore(p.snapshot().unwrap());
    q
}

/// A page for a ticket that no view lists.
fn outside(key: &str) -> Detail {
    let mut d = detail();
    d.ticket.key = key.into();
    d.ticket.updated = "2026-10-01T09:00:00.000+0000".into();
    d
}

fn stamp(p: &mut JiraPlugin, key: &str, result: Result<String, String>) -> Vec<Effect> {
    p.on_msg(Box::new(JiraMsg::Stamp { site: SITE.into(), key: key.into(), result }))
}

#[test]
fn a_page_read_is_kept_with_the_cache_against_its_updated() {
    let mut p = sample();
    let d = detail();
    complete_detail(&mut p, SITE, KEY, Ok(d.clone()));
    let kept = &p.data[SITE].details[KEY];
    assert_eq!(kept.detail, d);
    assert_eq!(kept.updated, d.ticket.updated);
    assert_eq!(kept.at, p.now);
    // The cache round-trips it.
    let q = restarted(&p);
    assert_eq!(q.data[SITE].details[KEY], p.data[SITE].details[KEY]);
}

#[test]
fn a_kept_page_is_written_at_the_next_tick_once_no_list_is_out_and_only_if_it_changed() {
    let mut p = sample();
    complete_detail(&mut p, SITE, KEY, Ok(detail()));
    // A list read out: its answer writes the cache, so the tick does not.
    p.sync.in_flight.insert(job(SITE, "q"));
    assert!(!persists(&p.tick(NOW)));
    p.sync.in_flight.clear();
    assert!(persists(&p.tick(NOW)), "now it is written, once");
    assert!(!persists(&p.tick(NOW)));
    // The same page read again changes nothing worth a write.
    complete_detail(&mut p, SITE, KEY, Ok(detail()));
    assert!(!persists(&p.tick(NOW)));
    // One that changed does.
    let mut d = detail();
    d.ticket.summary = "Renamed".into();
    complete_detail(&mut p, SITE, KEY, Ok(d));
    assert!(persists(&p.tick(NOW)));
}

fn persists(fx: &[Effect]) -> bool {
    fx.iter().any(|e| matches!(e, Effect::PersistCache))
}

#[test]
fn a_connection_that_changed_takes_its_kept_pages_but_a_restart_keeps_them() {
    let mut p = sample();
    complete_detail(&mut p, SITE, KEY, Ok(detail()));
    // The site now points at another address (the cache had been written for the first).
    let mut q = restarted(&p);
    q.cfg.sites[0].url = "https://other.atlassian.net".into();
    let fx = page::open(&mut q, SITE, KEY);
    assert_eq!(page_reads(&fx), 1, "the old address's page is not drawn as this one's");
    assert!(q.details[&detail_key(SITE, KEY)].value.is_none());
    // The first look at a connection after a start is no change.
    let mut r = restarted(&p);
    assert!(!r.data[SITE].details.is_empty());
    page::open(&mut r, SITE, KEY);
    assert!(r.details[&detail_key(SITE, KEY)].value.is_some());
    // A change while radar runs is.
    r.cfg.sites[0].email = "someone@else.example".into();
    crate::write_context::reconcile(&mut r);
    assert!(r.data[SITE].details.is_empty());
}

#[test]
fn after_a_restart_an_unchanged_ticket_opens_drawn_and_costs_nothing() {
    let mut p = sample();
    complete_detail(&mut p, SITE, KEY, Ok(detail()));
    let mut q = restarted(&p);
    q.now = NOW + 60;
    let fx = page::open(&mut q, SITE, KEY);
    assert_eq!(page_reads(&fx), 0, "the list's updated is the stored page's");
    assert_eq!(checks(&fx), 0);
    let f = &q.details[&detail_key(SITE, KEY)];
    assert_eq!(f.value.as_ref(), Some(&detail()), "drawn at once");
    assert!(!f.loading && f.error.is_none());
    // However long it stays open, nothing is asked while the list holds the same updated.
    q.now = NOW + 100 * q.cfg.refresh_secs as i64;
    assert_eq!(page_reads(&page::fetch(&mut q, SITE, KEY, false)), 0);
}

#[test]
fn after_a_restart_a_ticket_that_moved_is_drawn_from_the_cache_and_read_again() {
    let mut p = sample();
    complete_detail(&mut p, SITE, KEY, Ok(detail()));
    let mut q = restarted(&p);
    q.data.get_mut(SITE).unwrap().tickets.get_mut(KEY).unwrap().updated = "2026-10-12T09:00:00.000+0000".into();
    let fx = page::open(&mut q, SITE, KEY);
    assert_eq!(page_reads(&fx), 1);
    let f = &q.details[&detail_key(SITE, KEY)];
    assert!(f.loading && f.value.is_some(), "the stored page stays up while it is read");
}

#[test]
fn a_ticket_outside_the_view_is_checked_by_its_updated_on_the_cycle_and_read_only_when_it_moved() {
    let mut p = sample();
    let every = p.cfg.refresh_secs as i64;
    let id = detail_key(SITE, "ABC-99");
    page::open(&mut p, SITE, "ABC-99");
    complete_detail(&mut p, SITE, "ABC-99", Ok(outside("ABC-99")));
    assert!(p.tracked(SITE, "ABC-99").is_none());
    p.now = NOW + every - 1;
    let fx = page::fetch(&mut p, SITE, "ABC-99", false);
    assert!(fx.is_empty(), "fresh");
    // On the cycle: one small question, not the page.
    p.now = NOW + every;
    let fx = page::fetch(&mut p, SITE, "ABC-99", false);
    assert_eq!((checks(&fx), page_reads(&fx)), (1, 0));
    assert!(page::fetch(&mut p, SITE, "ABC-99", false).is_empty(), "never two at once");
    // Unchanged: nothing is read, and the next check is a cycle away.
    let fx = stamp(&mut p, "ABC-99", Ok("2026-10-01T09:00:00.000+0000".into()));
    assert_eq!(page_reads(&fx), 0);
    assert_eq!(p.details[&id].at, p.now);
    assert!(page::fetch(&mut p, SITE, "ABC-99", false).is_empty());
    // Moved: the page is read, and what was drawn stays until it comes.
    p.now += every;
    assert_eq!(checks(&page::fetch(&mut p, SITE, "ABC-99", false)), 1);
    let fx = stamp(&mut p, "ABC-99", Ok("2026-10-13T09:00:00.000+0000".into()));
    assert_eq!(page_reads(&fx), 1);
    assert!(p.details[&id].loading && p.details[&id].value.is_some());
    // A check that failed is tried again a cycle later, not at once.
    let read = p.details[&id].request.unwrap();
    assert!(page::on_detail(&mut p, SITE, "ABC-99", read, Ok(client::DetailRead { detail: outside("ABC-99"), ..Default::default() })));
    p.now += every;
    assert_eq!(checks(&page::fetch(&mut p, SITE, "ABC-99", false)), 1);
    assert!(stamp(&mut p, "ABC-99", Err("Jira answered 503".into())).is_empty());
    assert!(p.details[&id].error.is_none(), "a failed check is no reason to doubt the page");
    assert!(page::fetch(&mut p, SITE, "ABC-99", false).is_empty());
    p.now += every;
    assert_eq!(checks(&page::fetch(&mut p, SITE, "ABC-99", false)), 1);
}

#[test]
fn the_tick_checks_the_open_page_of_a_ticket_outside_the_view() {
    let mut p = sample();
    let every = p.cfg.refresh_secs as i64;
    page::open(&mut p, SITE, "ABC-99");
    complete_detail(&mut p, SITE, "ABC-99", Ok(outside("ABC-99")));
    p.now = NOW + every;
    let fx = p.tick(p.now);
    assert_eq!(checks(&fx), 1, "the page's check beside the views' syncs");
    assert_eq!(page_reads(&fx), p.cfg.sites.len(), "only the lists, no page read");
}

#[test]
fn a_stored_page_of_a_ticket_outside_the_view_is_checked_when_opened_after_a_restart() {
    let mut p = sample();
    complete_detail(&mut p, SITE, "ABC-99", Ok(outside("ABC-99")));
    let mut q = restarted(&p);
    q.now = NOW + 3 * q.cfg.refresh_secs as i64;
    let fx = page::open(&mut q, SITE, "ABC-99");
    assert_eq!((checks(&fx), page_reads(&fx)), (1, 0), "drawn from the cache, one small question");
    assert!(q.details[&detail_key(SITE, "ABC-99")].value.is_some());
    // Within the cycle of its reading, nothing at all.
    let mut r = restarted(&p);
    r.now = NOW + 1;
    let fx = page::open(&mut r, SITE, "ABC-99");
    assert_eq!((checks(&fx), page_reads(&fx)), (0, 0));
}

#[test]
fn a_new_cycle_check_from_a_page_that_was_closed_or_a_site_removed_is_let_go() {
    let mut p = sample();
    let every = p.cfg.refresh_secs as i64;
    page::open(&mut p, SITE, "ABC-99");
    complete_detail(&mut p, SITE, "ABC-99", Ok(outside("ABC-99")));
    p.now = NOW + every;
    page::fetch(&mut p, SITE, "ABC-99", false);
    p.details.clear();
    assert!(stamp(&mut p, "ABC-99", Ok("2027-01-01T00:00:00.000+0000".into())).is_empty());
    assert!(p.details.is_empty());
}

#[test]
fn only_the_latest_pages_are_kept_and_only_the_first_comments_of_each() {
    let mut p = sample();
    for n in 0..MAX_STORED_PAGES + 5 {
        p.now = NOW + n as i64;
        complete_detail(&mut p, SITE, &format!("ABC-{}", 100 + n), Ok(outside(&format!("ABC-{}", 100 + n))));
    }
    let kept = &p.data[SITE].details;
    assert_eq!(kept.len(), MAX_STORED_PAGES);
    assert!(!kept.contains_key("ABC-100") && kept.contains_key(&format!("ABC-{}", 100 + MAX_STORED_PAGES + 4)));
    // A long thread paged in is kept as far as the first page of it.
    let mut d = outside("ABC-500");
    d.comments = (0..client::COMMENTS_PAGE + 10)
        .map(|n| model::Comment { author: format!("a{n}"), created: String::new(), body: Default::default() })
        .collect();
    d.comments_total = d.comments.len() + 30;
    complete_detail(&mut p, SITE, "ABC-500", Ok(d.clone()));
    let stored = &p.data[SITE].details["ABC-500"].detail;
    assert_eq!(stored.comments.len(), client::COMMENTS_PAGE);
    assert_eq!(stored.comments_total, d.comments_total, "the rest are still known to be there");
    assert_eq!(p.details[&detail_key(SITE, "ABC-500")].value.as_ref().unwrap().comments.len(), d.comments.len());
}

#[test]
fn a_stored_page_in_a_shape_radar_cannot_read_is_let_go_and_not_the_site() {
    let mut p = sample();
    complete_detail(&mut p, SITE, KEY, Ok(detail()));
    let mut snap = p.snapshot().unwrap();
    snap["data"][SITE]["details"]["ABC-77"] = json!({ "at": "yesterday", "detail": 3 });
    let mut q = sample();
    q.data.clear();
    q.restore(snap);
    assert_eq!(q.data[SITE].details.keys().collect::<Vec<_>>(), [KEY]);
    assert_eq!(q.data[SITE].tickets, p.data[SITE].tickets, "the site stays");
    // A cache from before pages were kept reads as none.
    let mut old = p.snapshot().unwrap();
    old["data"][SITE].as_object_mut().unwrap().remove("details");
    let mut q = sample();
    q.data.clear();
    q.restore(old);
    assert!(q.data[SITE].details.is_empty());
}
