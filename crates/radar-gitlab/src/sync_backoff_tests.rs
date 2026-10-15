//! A tenancy that fails, or that GitLab is limiting, is left alone for a
//! while: through both ticks, not through `r`.

use radar_core::Plugin;
use radar_core::plugin::Effect;

use crate::backoff::{self, Net, Pacing};
use crate::client::{Synced, Want};
use crate::demo::sample;
use crate::{GitlabPlugin, GlMsg, sync_history};

const A: &str = "acme/team-a";
const B: &str = "acme/team-b";
const DOWN: &str = "GitLab answered 503 Service Unavailable";

fn deliver(p: &mut GitlabPlugin, group: &str, result: Result<Synced, String>, net: Net) {
    p.sync.in_flight = p.sync.in_flight.max(1);
    // As the plan asks while a view wants merged and closed ones.
    let want = Want {
        ended_since: Some("2026-09-22T00:00:00Z".into()),
        ..Want::default()
    };
    p.on_msg(Box::new(GlMsg::Loaded {
        group: group.into(),
        action: sync_history::action(&p.cfg.url, group, &want),
        want,
        result,
        net,
    }));
}

fn fail(p: &mut GitlabPlugin, group: &str, pacing: Pacing) {
    let net = Net {
        transient: true,
        pacing,
    };
    deliver(p, group, Err(DOWN.into()), net);
}

fn succeed(p: &mut GitlabPlugin, group: &str, pacing: Pacing) {
    let net = Net {
        transient: false,
        pacing,
    };
    deliver(p, group, Ok(Synced::default()), net);
}

/// The tenancies a tick sent a sync for.
fn synced(fx: &[Effect]) -> Vec<String> {
    let mut groups: Vec<String> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::SpawnAction { action, .. } => Some(action.subjects[0].key.id.clone()),
            _ => None,
        })
        .collect();
    groups.sort();
    groups
}

fn plugin() -> GitlabPlugin {
    let mut p = sample();
    p.cfg.refresh_secs = 10;
    p
}

#[test]
fn the_wait_after_each_failure_doubles_from_a_minute_and_stops_at_fifteen() {
    let mut p = plugin();
    for minutes in [1, 2, 4, 8, 15, 15, 15] {
        fail(&mut p, A, Pacing::default());
        let b = p.sync.backoff[A];
        let (left, full) = (b.left(p.now), minutes * 60);
        assert!(
            left <= full && left >= full - full / 8,
            "{} failures: {left}s, not about {minutes}m",
            b.failures
        );
        p.now = b.until;
    }
    assert_eq!(p.sync.backoff[A].failures, 7);
    assert!(!p.sync.backoff.contains_key(B), "a tenancy of its own");
}

#[test]
fn a_success_starts_the_count_over() {
    let mut p = plugin();
    for _ in 0..3 {
        fail(&mut p, A, Pacing::default());
        p.now = p.sync.backoff[A].until;
    }
    succeed(&mut p, A, Pacing::default());
    assert_eq!(p.sync.backoff[A].failures, 0);
    assert!(!p.backing_off(A));
    fail(&mut p, A, Pacing::default());
    assert_eq!(p.sync.backoff[A].failures, 1);
    assert!(p.sync.backoff[A].left(p.now) <= 60);
}

#[test]
fn a_failure_that_is_not_waited_out_changes_nothing() {
    let mut p = plugin();
    deliver(
        &mut p,
        A,
        Err("GitLab: no such field".into()),
        Net::default(),
    );
    assert!(!p.sync.backoff.contains_key(A));
    assert_eq!(p.sync.errors.len(), 1, "still said, as before");
    assert!(!synced(&p.tick(p.now)).is_empty(), "and tried again");
}

#[test]
fn a_tenancy_that_failed_is_not_sent_again_until_its_wait_is_over() {
    let mut p = plugin();
    let t0 = p.now;
    assert_eq!(synced(&p.tick(t0)), [A, B]);
    fail(&mut p, A, Pacing::default());
    deliver(&mut p, B, Ok(Synced::default()), Net::default());
    // Due again after 10 s, but A waits at least 52 s.
    assert_eq!(synced(&p.tick(t0 + 20)), [B]);
    deliver(&mut p, B, Ok(Synced::default()), Net::default());
    assert_eq!(synced(&p.tick(t0 + 40)), [B]);
    deliver(&mut p, B, Ok(Synced::default()), Net::default());
    let until = p.sync.backoff[A].until;
    assert!(until > t0 + 50);
    assert_eq!(synced(&p.tick(until)), [A, B]);
}

#[test]
fn when_every_tenancy_waits_a_tick_sends_nothing() {
    let mut p = plugin();
    let t0 = p.now;
    p.tick(t0);
    fail(&mut p, A, Pacing::default());
    fail(&mut p, B, Pacing::default());
    for later in [1, 11, 30] {
        assert!(p.tick(t0 + later).is_empty());
    }
    assert!(p.sync.in_flight == 0);
}

#[test]
fn retry_after_is_obeyed_beyond_the_count() {
    let mut p = plugin();
    let t0 = p.now;
    p.tick(t0);
    fail(
        &mut p,
        A,
        Pacing {
            retry_at: Some(t0 + 600),
            ..Pacing::default()
        },
    );
    deliver(&mut p, B, Ok(Synced::default()), Net::default());
    assert_eq!(p.sync.backoff[A].until, t0 + 600);
    assert_eq!(synced(&p.tick(t0 + 599)), [B]);
    deliver(&mut p, B, Ok(Synced::default()), Net::default());
    assert_eq!(
        synced(&p.tick(t0 + 610)),
        [A, B],
        "due, and A's wait is over"
    );
}

#[test]
fn a_429_waits_for_the_ratelimit_reset() {
    let mut p = plugin();
    let t0 = p.now;
    fail(
        &mut p,
        A,
        Pacing {
            reset_at: Some(t0 + 100),
            remaining: Some(0),
            limit: Some(2000),
            ..Pacing::default()
        },
    );
    assert_eq!(p.sync.backoff[A].until, t0 + 100);
}

#[test]
fn the_merged_and_closed_tick_waits_after_a_failure_too() {
    let mut p = plugin();
    p.filter = "state:merged".into();
    p.data.get_mut(A).unwrap().ended_since = None;
    p.cfg.refresh_secs = 3600;
    p.sync.last_started = Some(p.now);
    let t0 = p.now;
    assert_eq!(
        synced(&p.tick(t0)),
        [A, B],
        "a view wants them and A has none"
    );
    fail(&mut p, A, Pacing::default());
    fail(&mut p, B, Pacing::default());
    // Without the wait this is sent again on the next tick, and the next.
    for later in [1, 2, 30] {
        assert!(p.tick(t0 + later).is_empty(), "{later}");
    }
    let until = p.sync.backoff[A].until;
    assert_eq!(synced(&p.tick(until)), [A, B]);
}

#[test]
fn the_merged_and_closed_tick_does_not_keep_syncing_the_others_for_a_waiting_tenancy() {
    let mut p = plugin();
    p.filter = "state:merged".into();
    p.data.get_mut(A).unwrap().ended_since = None;
    p.cfg.refresh_secs = 3600;
    p.sync.last_started = Some(p.now);
    let t0 = p.now;
    p.tick(t0);
    fail(&mut p, A, Pacing::default());
    deliver(&mut p, B, Ok(Synced::default()), Net::default());
    assert!(p.tick(t0 + 1).is_empty(), "A waits, and B is not due");
}

#[test]
fn a_nearly_spent_ratelimit_holds_the_syncs_until_the_reset() {
    let mut p = plugin();
    let t0 = p.now;
    p.tick(t0);
    let low = Pacing {
        remaining: Some(99),
        limit: Some(2000),
        reset_at: Some(t0 + 45),
        ..Pacing::default()
    };
    succeed(&mut p, A, low);
    succeed(&mut p, B, Pacing::default());
    assert_eq!(p.sync.backoff[A].failures, 0, "not a failure");
    assert_eq!(synced(&p.tick(t0 + 20)), [B], "due, but A is held");
    succeed(&mut p, B, Pacing::default());
    assert_eq!(synced(&p.tick(t0 + 45)), [A, B]);
}

#[test]
fn plenty_of_ratelimit_left_holds_nothing() {
    let mut p = plugin();
    let t0 = p.now;
    p.tick(t0);
    let plenty = Pacing {
        remaining: Some(100),
        limit: Some(2000),
        reset_at: Some(t0 + 45),
        ..Pacing::default()
    };
    succeed(&mut p, A, plenty);
    succeed(&mut p, B, plenty);
    assert_eq!(synced(&p.tick(t0 + 11)), [A, B]);
}

#[test]
fn r_asks_a_waiting_tenancy_once_and_does_not_forgive_its_failures() {
    let mut p = plugin();
    for _ in 0..3 {
        fail(&mut p, A, Pacing::default());
        p.now = p.sync.backoff[A].until - 1;
    }
    p.sync.in_flight = 0;
    assert!(p.backing_off(A));
    assert!(
        synced(&p.tick(p.now)).iter().all(|g| g != A),
        "the tick waits"
    );
    deliver(&mut p, B, Ok(Synced::default()), Net::default());
    let fx = p.refresh();
    assert_eq!(synced(&fx), [A, B], "asked for, wait or not");
    assert_eq!(
        p.sync.backoff[A].failures, 3,
        "the count stands until a sync succeeds"
    );
    fail(&mut p, A, Pacing::default());
    assert_eq!(p.sync.backoff[A].failures, 4);
    deliver(&mut p, B, Ok(Synced::default()), Net::default());
    assert!(
        synced(&p.tick(p.now + 1)).iter().all(|g| g != A),
        "once, not for good"
    );
}

#[test]
fn the_failure_line_says_when_the_tenancy_is_tried_again() {
    let mut p = plugin();
    for _ in 0..3 {
        fail(&mut p, A, Pacing::default());
        p.now = p.sync.backoff[A].until - 1;
    }
    p.now = p.sync.backoff[A].until - 235;
    let why = &p.sync.errors[A];
    assert_eq!(
        p.sync_failure(A, why),
        format!("{DOWN} · trying again in {}", backoff::in_words(235))
    );
    p.now = p.sync.backoff[A].until;
    assert_eq!(
        p.sync_failure(A, why),
        DOWN,
        "nothing once the wait is over"
    );
}
