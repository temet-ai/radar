//! Backing off from a GitLab that is down or limiting us.
//!
//! A tenancy that fails with a 5xx, a 429 or a connection error is not tried
//! again at once: it waits 1, 2, 4, 8 … minutes (at most 15, a little less by
//! a jitter that is the same for the same tenancy and failure, so tenancies
//! do not retry in step), and longer when GitLab said so (`Retry-After`,
//! `RateLimit-Reset`). A success resets the count. A server that is about to
//! run out of requests (`RateLimit-Remaining` under 5% of `RateLimit-Limit`)
//! holds the next sync until the window resets. Nothing here reads a clock:
//! the caller passes `now`, in seconds since the epoch.

use radar_core::time;

/// The first wait, and what doubles.
pub const BASE_SECS: i64 = 60;
/// The longest the count alone makes a tenancy wait.
pub const CAP_SECS: i64 = 15 * 60;
/// The longest GitLab's own words (`Retry-After` and the like) are obeyed, so
/// a wrong header cannot silence a tenancy for a day.
pub const MOST_HOLD_SECS: i64 = 60 * 60;
/// Without a reset time, how long a nearly spent allowance is waited out.
pub const WINDOW_SECS: i64 = 60;

/// The text of the headers that say how fast GitLab wants to be asked.
#[derive(Debug, Clone, Copy, Default)]
pub struct Headers<'a> {
    pub retry_after: Option<&'a str>,
    pub limit: Option<&'a str>,
    pub remaining: Option<&'a str>,
    pub reset: Option<&'a str>,
}

/// What an answer said about pacing, as times in seconds since the epoch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pacing {
    /// Not before this (`Retry-After`).
    pub retry_at: Option<i64>,
    /// When the allowance is renewed (`RateLimit-Reset`).
    pub reset_at: Option<i64>,
    pub remaining: Option<u64>,
    pub limit: Option<u64>,
}

impl Pacing {
    pub fn read(h: &Headers, now: i64) -> Self {
        let number = |v: Option<&str>| v.and_then(|v| v.trim().parse::<u64>().ok());
        Self {
            retry_at: h.retry_after.and_then(|v| retry_at(v, now)),
            // GitLab sends the time of the reset; a draft of the header sends
            // the seconds left. Nothing near the present is a time of 1970.
            reset_at: number(h.reset).map(|v| {
                if v > 1_000_000_000 {
                    i64::try_from(v).unwrap_or(i64::MAX)
                } else {
                    now + v as i64
                }
            }),
            remaining: number(h.remaining),
            limit: number(h.limit),
        }
    }

    /// Under 5% of the allowance is left.
    pub fn low(&self) -> bool {
        match (self.remaining, self.limit) {
            (Some(r), Some(l)) if l > 0 => r.saturating_mul(20) < l,
            (Some(0), _) => true,
            _ => false,
        }
    }

    /// Until when an allowance that is nearly spent holds the next sync.
    fn hold_until(&self, now: i64) -> Option<i64> {
        self.low().then(|| {
            self.reset_at
                .unwrap_or(now + WINDOW_SECS)
                .min(now + MOST_HOLD_SECS)
        })
    }

    /// When GitLab itself said to come back at the earliest.
    fn said_until(&self, now: i64) -> Option<i64> {
        [self.retry_at, self.reset_at]
            .into_iter()
            .flatten()
            .max()
            .map(|t| t.min(now + MOST_HOLD_SECS))
    }
}

/// A `Retry-After`: seconds, or an HTTP date.
fn retry_at(v: &str, now: i64) -> Option<i64> {
    let v = v.trim();
    match v.parse::<u64>() {
        Ok(secs) => Some(now.saturating_add(i64::try_from(secs).unwrap_or(i64::MAX))),
        Err(_) => http_date(v).map(|at| at.max(now)),
    }
}

/// `Sun, 06 Nov 1994 08:49:37 GMT`, in seconds since the epoch.
pub fn http_date(s: &str) -> Option<i64> {
    let mut parts = s.split_whitespace();
    let _day_of_week = parts.next()?;
    let day: u32 = parts.next()?.parse().ok()?;
    let month = parts.next()?;
    let year: u32 = parts.next()?.parse().ok()?;
    let clock = parts.next()?;
    let zone = parts.next()?;
    if parts.next().is_some() || !zone.eq_ignore_ascii_case("GMT") {
        return None;
    }
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| m.eq_ignore_ascii_case(month))?
        + 1;
    time::parse_rfc3339(&format!("{year:04}-{month:02}-{day:02}T{clock}Z"))
}

/// A failure the tenancy should back off from: GitLab answered 429 or 5xx,
/// or could not be reached. It carries what the answer said about pacing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trouble {
    pub status: Option<u16>,
    pub pacing: Pacing,
    message: String,
}

impl Trouble {
    pub fn answered(status: u16, message: String, pacing: Pacing) -> Self {
        Self {
            status: Some(status),
            pacing,
            message,
        }
    }

    pub fn unreachable(message: String) -> Self {
        Self {
            status: None,
            pacing: Pacing::default(),
            message,
        }
    }
}

impl std::fmt::Display for Trouble {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Trouble {}

/// The [`Trouble`] behind `e`, if it is one.
pub fn trouble_of(e: &anyhow::Error) -> Option<&Trouble> {
    e.chain().find_map(|c| c.downcast_ref::<Trouble>())
}

/// What one tenancy's sync said about backing off, beside its data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Net {
    /// It failed in a way that is waited out.
    pub transient: bool,
    pub pacing: Pacing,
}

/// A tenancy's wait: how many times in a row it failed, and until when it
/// is left alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Backoff {
    pub failures: u32,
    pub until: i64,
}

impl Backoff {
    pub fn waiting(&self, now: i64) -> bool {
        self.until > now
    }

    pub fn left(&self, now: i64) -> i64 {
        (self.until - now).max(0)
    }

    /// After one more failure.
    pub fn failed(self, group: &str, now: i64, pacing: &Pacing) -> Self {
        let failures = self.failures.saturating_add(1);
        let mut until = now + delay(failures, group);
        for at in [pacing.said_until(now), pacing.hold_until(now)]
            .into_iter()
            .flatten()
        {
            until = until.max(at);
        }
        Self { failures, until }
    }

    /// After a success: the count starts over, and a nearly spent allowance
    /// holds the next sync until it is renewed.
    pub fn succeeded(now: i64, pacing: &Pacing) -> Self {
        Self {
            failures: 0,
            until: pacing.hold_until(now).unwrap_or(0),
        }
    }
}

/// What the count alone makes a tenancy wait: 1, 2, 4, 8 … minutes, at most
/// 15, less by a jitter of up to an eighth.
pub fn delay(failures: u32, group: &str) -> i64 {
    let base = (BASE_SECS << failures.saturating_sub(1).min(10)).min(CAP_SECS);
    base - jitter(group, failures) % (base / 8 + 1)
}

/// The same for a tenancy and a count, and different between tenancies.
fn jitter(group: &str, failures: u32) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in group.bytes().chain(failures.to_le_bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    (h >> 1) as i64
}

/// `45s`, `4m`: how long, rounded up to the minute from a minute on.
pub fn in_words(secs: i64) -> String {
    if secs < 60 {
        format!("{}s", secs.max(1))
    } else {
        format!("{}m", (secs + 59) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn same_minutes(secs: i64, minutes: i64) -> bool {
        // Up to an eighth less than the minutes asked for.
        let full = minutes * 60;
        secs <= full && secs >= full - full / 8
    }

    #[test]
    fn the_wait_doubles_from_one_minute_and_stops_at_fifteen() {
        let waits: Vec<i64> = (1..=8).map(|n| delay(n, "acme/team-a")).collect();
        for (wait, minutes) in waits.iter().zip([1, 2, 4, 8, 15, 15, 15, 15]) {
            assert!(same_minutes(*wait, minutes), "{waits:?}");
        }
        assert!(delay(u32::MAX, "acme/team-a") <= CAP_SECS);
    }

    #[test]
    fn the_jitter_is_small_stable_and_differs_between_tenancies() {
        assert_eq!(delay(3, "acme/team-a"), delay(3, "acme/team-a"));
        let waits: std::collections::BTreeSet<i64> = ["a", "b", "c", "d", "e", "f", "g", "h"]
            .into_iter()
            .map(|g| delay(4, g))
            .collect();
        assert!(waits.len() > 1, "{waits:?}");
        assert!(waits.iter().all(|w| same_minutes(*w, 8)), "{waits:?}");
    }

    #[test]
    fn failures_count_up_and_a_success_starts_over() {
        let mut b = Backoff::default();
        for n in 1..=3 {
            b = b.failed("g", NOW, &Pacing::default());
            assert_eq!(b.failures, n);
            assert!(b.waiting(NOW) && !b.waiting(b.until));
        }
        assert!(same_minutes(b.left(NOW), 4));
        let b = Backoff::succeeded(NOW, &Pacing::default());
        assert_eq!((b.failures, b.waiting(NOW)), (0, false));
    }

    #[test]
    fn what_gitlab_asked_for_is_a_floor_under_the_count() {
        let retry = Pacing {
            retry_at: Some(NOW + 600),
            ..Pacing::default()
        };
        let b = Backoff::default().failed("g", NOW, &retry);
        assert_eq!(b.until, NOW + 600, "ten minutes asked, a minute counted");
        let short = Pacing {
            retry_at: Some(NOW + 5),
            ..Pacing::default()
        };
        assert!(Backoff::default().failed("g", NOW, &short).left(NOW) > 5);
        let reset = Pacing {
            reset_at: Some(NOW + 90),
            ..Pacing::default()
        };
        assert_eq!(Backoff::default().failed("g", NOW, &reset).until, NOW + 90);
        let absurd = Pacing {
            retry_at: Some(NOW + 86_400),
            ..Pacing::default()
        };
        assert_eq!(
            Backoff::default().failed("g", NOW, &absurd).until,
            NOW + MOST_HOLD_SECS
        );
    }

    #[test]
    fn retry_after_is_seconds_or_an_http_date() {
        let read = |v: &str| {
            Pacing::read(
                &Headers {
                    retry_after: Some(v),
                    ..Headers::default()
                },
                NOW,
            )
            .retry_at
        };
        assert_eq!(read("120"), Some(NOW + 120));
        assert_eq!(read(" 7 "), Some(NOW + 7));
        let at = http_date("Fri, 15 Jan 2027 08:00:00 GMT").unwrap();
        assert_eq!(at, time::parse_rfc3339("2027-01-15T08:00:00Z").unwrap());
        assert_eq!(read("Fri, 15 Jan 2027 08:00:00 GMT"), Some(at));
        assert_eq!(
            read("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(NOW),
            "past: now"
        );
        assert_eq!(read("soon"), None);
        assert_eq!(http_date("Sun, 06 Nov 1994 08:49:37 PST"), None);
    }

    #[test]
    fn the_ratelimit_headers_are_read_as_a_time_and_a_share() {
        let pacing = Pacing::read(
            &Headers {
                limit: Some("2000"),
                remaining: Some("1999"),
                reset: Some("1800000042"),
                ..Headers::default()
            },
            NOW,
        );
        assert_eq!(
            pacing,
            Pacing {
                retry_at: None,
                reset_at: Some(1_800_000_042),
                remaining: Some(1999),
                limit: Some(2000),
            }
        );
        assert!(!pacing.low());
        let seconds_left = Pacing::read(
            &Headers {
                reset: Some("30"),
                ..Headers::default()
            },
            NOW,
        );
        assert_eq!(seconds_left.reset_at, Some(NOW + 30));
    }

    #[test]
    fn under_five_percent_left_holds_the_next_sync_until_the_reset() {
        let pace = |remaining, limit| Pacing {
            remaining: Some(remaining),
            limit: Some(limit),
            reset_at: Some(NOW + 45),
            ..Pacing::default()
        };
        assert!(!pace(100, 2000).low(), "5% is not under 5%");
        assert!(pace(99, 2000).low());
        let held = Backoff::succeeded(NOW, &pace(99, 2000));
        assert_eq!((held.failures, held.until), (0, NOW + 45));
        assert!(!Backoff::succeeded(NOW, &pace(500, 2000)).waiting(NOW));
        let no_reset = Pacing {
            remaining: Some(0),
            ..Pacing::default()
        };
        assert_eq!(Backoff::succeeded(NOW, &no_reset).until, NOW + WINDOW_SECS);
    }

    #[test]
    fn a_wait_reads_as_seconds_then_minutes() {
        assert_eq!(in_words(0), "1s");
        assert_eq!(in_words(45), "45s");
        assert_eq!(in_words(60), "1m");
        assert_eq!(in_words(61), "2m");
        assert_eq!(in_words(240), "4m");
    }
}
