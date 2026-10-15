//! Jira's rate-limit headers, and when they ask radar to stay away.

use super::*;

fn headers(pairs: &[(&str, &str)]) -> Rate {
    Rate::from_headers(|name| pairs.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string()))
}

fn said(status: u16, retry_after: Option<u64>, rate: Rate) -> Resp {
    Resp { status, retry_after, rate, body: String::new() }
}

const AT: i64 = 1_800_000_000;

fn left(limit: u64, remaining: u64, reset: i64) -> Rate {
    Rate { limit: Some(limit), remaining: Some(remaining), reset: Some(reset), reason: None }
}

#[test]
fn the_headers_are_read_with_an_iso_or_epoch_reset_and_junk_is_left_out() {
    let r = headers(&[
        ("x-ratelimit-limit", " 500 "),
        ("x-ratelimit-remaining", "12"),
        ("x-ratelimit-reset", "2026-10-13T12:00:00.000Z"),
        ("ratelimit-reason", "jira-burst-based"),
    ]);
    assert_eq!((r.limit, r.remaining), (Some(500), Some(12)));
    assert_eq!(r.reset, time::parse_rfc3339("2026-10-13T12:00:00Z"));
    assert_eq!(r.reason.as_deref(), Some("jira-burst-based"));
    assert_eq!(headers(&[("x-ratelimit-reset", "1800000500")]).reset, Some(1_800_000_500));
    // Not numbers, a reset that is no time, a reason with anything odd in it.
    let junk = headers(&[
        ("x-ratelimit-limit", "lots"),
        ("x-ratelimit-remaining", "-3"),
        ("x-ratelimit-reset", "soon"),
        ("ratelimit-reason", "a b\nc"),
    ]);
    assert!(junk.is_empty(), "{junk:?}");
    assert!(headers(&[]).is_empty());
}

#[test]
fn under_a_tenth_left_a_site_is_let_alone_until_its_reset_and_not_after() {
    let limits = Limits::default();
    limits.note("acme", &said(200, None, left(1000, 101, AT + 600)));
    assert_eq!(limits.hold("acme", AT), None, "a tenth and a bit left is plenty");
    limits.note("acme", &said(200, None, left(1000, 99, AT + 600)));
    assert_eq!(limits.hold("acme", AT), Some(AT + 600), "under a tenth: until it refills");
    assert_eq!(limits.hold("acme", AT + 600), None, "refilled");
    assert_eq!(limits.hold("beta", AT), None, "each site has its own");
    // Nothing left at all is the same, an hour at most.
    limits.note("acme", &said(200, None, left(1000, 0, AT + 7200)));
    assert_eq!(limits.hold("acme", AT), Some(AT + HOLD_MAX_SECS));
}

#[test]
fn of_two_words_for_one_window_the_lower_remaining_stays_and_a_later_window_replaces() {
    let limits = Limits::default();
    limits.note("acme", &said(200, None, left(1000, 50, AT + 600)));
    // A slower answer from before: more left, same window.
    limits.note("acme", &said(200, None, left(1000, 400, AT + 600)));
    assert_eq!(limits.rate("acme").unwrap().remaining, Some(50));
    // An older window never replaces.
    limits.note("acme", &said(200, None, left(1000, 900, AT + 300)));
    assert_eq!(limits.rate("acme").unwrap().remaining, Some(50));
    limits.note("acme", &said(200, None, left(1000, 900, AT + 4200)));
    assert_eq!(limits.rate("acme").unwrap().remaining, Some(900));
}

#[test]
fn a_retry_after_is_dated_at_the_next_tick_and_a_good_answer_ends_it() {
    let limits = Limits::default();
    limits.note("acme", &said(429, Some(90), Rate { reason: Some("jira-burst-based".into()), ..Rate::default() }));
    assert_eq!(limits.hold("acme", AT), None, "not dated yet");
    limits.stamp(AT);
    assert_eq!(limits.hold("acme", AT), Some(AT + 90));
    assert_eq!(limits.hold("acme", AT + 90), None);
    // The later of the two reasons to wait wins.
    limits.note("acme", &said(503, Some(90), left(100, 1, AT + 300)));
    limits.stamp(AT);
    assert_eq!(limits.hold("acme", AT), Some(AT + 300));
    // Served again: the Retry-After is over, the allowance is what it says.
    limits.note("acme", &said(200, None, Rate::default()));
    assert_eq!(limits.hold("acme", AT), Some(AT + 300), "still low");
    limits.note("acme", &said(200, None, left(100, 90, AT + 900)));
    assert_eq!(limits.hold("acme", AT), None, "a later window, refilled");
    // A Retry-After is no more than an hour.
    limits.note("acme", &said(429, Some(86_400), Rate::default()));
    limits.stamp(AT);
    assert_eq!(limits.hold("acme", AT), Some(AT + HOLD_MAX_SECS));
}
