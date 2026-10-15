//! What is held is the reference: a refresh asks only what changed since
//! the last read, a sweep now and then lets go of tickets that left a view,
//! a cache from before (or one radar cannot read) is read safely, and a failing
//! site backs off.

use super::*;
use crate::client::{Ask, TicketsRead};

fn acme_jql(p: &JiraPlugin) -> String {
    p.jql_for(0, &p.cfg.views[0].clone()).unwrap()
}

fn answer(p: &mut JiraPlugin, jql: &str, tickets: Vec<Ticket>, changed_only: bool, swept: Option<Vec<String>>) -> Vec<Effect> {
    p.on_msg(Box::new(JiraMsg::Loaded {
        site: "acme".into(),
        jql: jql.into(),
        action: sync_history::action(&p.cfg.sites[0]),
        result: Ok(sync_history::Synced {
            list: TicketsRead { tickets, ..Default::default() },
            changed_only,
            swept,
            ..Default::default()
        }),
    }))
}

fn fail(p: &mut JiraPlugin, jql: &str, retry_after: Option<u64>) {
    p.on_msg(Box::new(JiraMsg::Loaded {
        site: "acme".into(),
        jql: jql.into(),
        action: sync_history::action(&p.cfg.sites[0]),
        result: Err(sync_history::Failed { text: "Jira answered 503".into(), retry_after }),
    }));
}

fn persists(fx: &[Effect]) -> bool {
    fx.iter().any(|e| matches!(e, Effect::PersistCache))
}

fn point(p: &JiraPlugin, at: i64, swept: i64) -> SyncPoint {
    SyncPoint { at, swept, fields: fields_of(&p.cfg.sites[0]) }
}

#[test]
fn after_one_whole_read_a_refresh_asks_only_what_changed_since() {
    let mut p = sample();
    let (jql, site) = (acme_jql(&p), p.cfg.sites[0].clone());
    p.sync_view(false);
    assert_eq!(p.sync.asking[&job("acme", &jql)], Ask::Full, "a cache from before holds no sync point");
    let held: Vec<Ticket> = p.data["acme"].found[&jql].iter().map(|k| p.data["acme"].tickets[k].clone()).collect();
    assert!(!persists(&answer(&mut p, &jql, held, false, None)), "not while beta is out");
    // beta answers with nothing new: acme's whole read is still written.
    let beta = p.jql_for(1, &p.cfg.views[0].clone()).unwrap();
    let fx = p.on_msg(Box::new(JiraMsg::Loaded {
        site: "beta".into(),
        jql: beta.clone(),
        action: sync_history::action(&p.cfg.sites[1]),
        result: Ok(sync_history::Synced { changed_only: true, ..Default::default() }),
    }));
    assert!(persists(&fx), "a whole read is kept, whoever answers last");
    assert_eq!(p.data["acme"].synced[&jql], point(&p, NOW, NOW));
    assert!(p.sync.asking.is_empty());
    let every = p.cfg.refresh_secs as i64;
    p.now = NOW + every - 1;
    assert_eq!(p.plan(&site, &jql, false), None, "fresh");
    // The window is the time since the last read, two minutes more, in whole minutes.
    p.now = NOW + every;
    assert_eq!(p.plan(&site, &jql, false), Some(Ask::Changed { minutes: 4, sweep: false }));
    p.sync_view(false);
    assert_eq!(p.sync.asking[&job("acme", &jql)], Ask::Changed { minutes: 4, sweep: false });
    assert_eq!(p.plan(&site, &jql, true), None, "never twice at once");
    // Nothing changed: what is shown stays, nothing is written, the point moves.
    let before = p.data["acme"].found[&jql].clone();
    let fx = answer(&mut p, &jql, vec![], true, None);
    assert!(!persists(&fx), "a quiet read writes nothing");
    assert!(!fx.iter().any(|e| matches!(e, Effect::Changed(_))));
    assert_eq!(p.data["acme"].found[&jql], before);
    assert_eq!(p.data["acme"].synced[&jql], point(&p, NOW + every, NOW));
}

#[test]
fn a_changed_ticket_is_merged_first_and_one_that_left_goes_with_the_sweep() {
    let mut p = sample();
    let (jql, site) = (acme_jql(&p), p.cfg.sites[0].clone());
    let keys = p.data["acme"].found[&jql].clone();
    assert!(keys.len() >= 2, "{keys:?}");
    let at = point(&p, NOW, NOW);
    p.data.get_mut("acme").unwrap().synced.insert(jql.clone(), at);
    let every = p.cfg.refresh_secs as i64;
    // One ticket changed: it comes first (most recently updated), the rest stay.
    p.now = NOW + every;
    let mut t = p.data["acme"].tickets[&keys[1]].clone();
    t.summary = "Renamed".into();
    t.updated = "2026-10-06T10:00:00.000+0000".into();
    let fx = answer(&mut p, &jql, vec![t], true, None);
    assert!(persists(&fx), "a change is kept");
    let found = p.data["acme"].found[&jql].clone();
    assert_eq!(found[0], keys[1]);
    assert_eq!(found.len(), keys.len());
    assert_eq!(p.data["acme"].tickets[&keys[1]].summary, "Renamed");
    assert_eq!(p.data["acme"].synced[&jql].swept, NOW, "not read whole");
    // keys[0] was resolved out of the view: a changed-only read cannot see
    // that. The sweep, every few intervals or on `r`, can.
    p.now = NOW + every * (SWEEP_REFRESHES - 1);
    assert!(matches!(p.plan(&site, &jql, false), Some(Ask::Changed { sweep: false, .. })));
    p.now = NOW + every * SWEEP_REFRESHES;
    assert!(matches!(p.plan(&site, &jql, false), Some(Ask::Changed { sweep: true, .. })));
    p.now = NOW + every * (SWEEP_REFRESHES - 1);
    assert!(matches!(p.plan(&site, &jql, true), Some(Ask::Changed { sweep: true, .. })), "r sweeps");
    let rest: Vec<String> = found.iter().filter(|k| **k != keys[0]).cloned().collect();
    answer(&mut p, &jql, vec![], true, Some(rest.clone()));
    assert_eq!(p.data["acme"].found[&jql], rest);
    assert!(!p.visible().iter().any(|it| it.site.name == "acme" && it.t.key == keys[0]));
    assert_eq!(p.data["acme"].synced[&jql].swept, p.now, "read whole by its keys");
}

#[test]
fn an_old_or_unreadable_cache_is_read_safely_or_let_go_once() {
    let p = sample();
    let (jql, site) = (acme_jql(&p), p.cfg.sites[0].clone());
    // From before sync points: drawn at once; the first refresh reads whole.
    let mut old = p.snapshot().unwrap();
    assert!(old["data"]["acme"].get("synced").is_none());
    let mut q = sample();
    q.data.clear();
    q.restore(old.clone());
    assert_eq!(q.data, p.data);
    assert_eq!(q.plan(&site, &jql, false), Some(Ask::Full));
    // A site radar can no longer read is let go; the others stay.
    old["data"]["beta"] = json!({ "tickets": "not a map" });
    let mut q = sample();
    q.restore(old);
    assert!(!q.data.contains_key("beta"));
    assert_eq!(q.data["acme"], p.data["acme"]);
    // Nothing usable at all: nothing held, nothing broken.
    let mut q = JiraPlugin::new();
    q.restore(json!([1, 2]));
    assert!(q.data.is_empty());
    // A point from a clock ahead, read with other fields, or not read whole
    // for a week reads the view whole.
    let mut q = sample();
    q.now = NOW + 200;
    let fields = point(&q, 0, 0).fields;
    for bad in [
        SyncPoint { at: NOW + 500, swept: NOW, fields: fields.clone() },
        SyncPoint { at: NOW, swept: NOW, fields: "summary".into() },
        SyncPoint { at: NOW, swept: NOW - WHOLE_AFTER_SECS, fields: fields.clone() },
    ] {
        q.data.get_mut("acme").unwrap().synced.insert(jql.clone(), bad.clone());
        assert_eq!(q.plan(&site, &jql, false), Some(Ask::Full), "{bad:?}");
    }
    // A good point: nothing asked at start while it is fresh, and the top
    // bar says when it was read.
    let good = point(&q, NOW + 150, NOW + 150);
    q.data.get_mut("acme").unwrap().synced.insert(jql.clone(), good);
    let mut r = JiraPlugin::new();
    r.cfg = q.cfg.clone();
    r.now = NOW + 200;
    r.restore(q.snapshot().unwrap());
    assert_eq!(r.sync.last_ok, Some(NOW + 150));
    assert_eq!(r.plan(&site, &jql, false), None);
    r.now = NOW + 150 + r.cfg.refresh_secs as i64;
    assert_eq!(r.plan(&site, &jql, false), Some(Ask::Changed { minutes: 4, sweep: false }));
}

#[test]
fn a_failing_site_backs_off_and_waits_as_long_as_jira_asks() {
    let mut p = sample();
    let (jql, site) = (acme_jql(&p), p.cfg.sites[0].clone());
    let every = p.cfg.refresh_secs as i64;
    fail(&mut p, &jql, None);
    assert_eq!(p.sync.backoff["acme"], (NOW + every, 1));
    p.now = NOW + every;
    fail(&mut p, &jql, None);
    assert_eq!(p.sync.backoff["acme"], (NOW + 3 * every, 2), "twice as long");
    p.now += every;
    assert_eq!(p.plan(&site, &jql, false), None, "backing off");
    assert!(p.plan(&site, &jql, true).is_some(), "r asks anyway");
    fail(&mut p, &jql, Some(3600));
    assert_eq!(p.sync.backoff["acme"], (p.now + 3600, 3), "as long as Jira asked");
    for _ in 0..20 {
        fail(&mut p, &jql, None);
    }
    assert_eq!(p.sync.backoff["acme"].0, p.now + LONGEST_BACKOFF_SECS);
    answer(&mut p, &jql, vec![], false, None);
    assert!(p.sync.backoff.is_empty() && p.sync.errors.is_empty(), "an answer ends it");
}
