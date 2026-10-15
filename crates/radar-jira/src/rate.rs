//! What Jira Cloud says about its rate limits, and when radar should stay away.
//!
//! Every response may carry `X-RateLimit-Limit`, `-Remaining` and `-Reset`
//! (an ISO 8601 time; seconds since 1970 are read too), a 429 `RateLimit-Reason`
//! (which quota ran out), and 429 and 503 a `Retry-After`. A [`Limits`] handle
//! is given to each client the plugin starts; it keeps the latest word per
//! site, and the plugin holds its background reads (list syncs, development
//! panels, an open page's checks) while a site is under a tenth of its
//! allowance until its reset, or until its `Retry-After` has passed. `r` and
//! opening a page ask anyway.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use radar_core::time;

use crate::client::Resp;

/// A site under this share of its allowance (one in so many) is let alone.
pub(crate) const LOW_SHARE: u64 = 10;
/// The longest a hold lasts, whatever the reset or `Retry-After` says.
pub(crate) const HOLD_MAX_SECS: i64 = 3600;

/// The rate-limit headers of one response; empty when it had none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rate {
    pub limit: Option<u64>,
    pub remaining: Option<u64>,
    /// Unix seconds when the allowance refills.
    pub reset: Option<i64>,
    /// Which quota a refusal ran into (`jira-quota-tenant-based`, ...).
    pub reason: Option<String>,
}

impl Rate {
    /// Read the headers through `get` (a header name, lower case, to its value).
    pub fn from_headers(get: impl Fn(&str) -> Option<String>) -> Self {
        let number = |name: &str| get(name).and_then(|v| v.trim().parse::<u64>().ok());
        Self {
            limit: number("x-ratelimit-limit"),
            remaining: number("x-ratelimit-remaining"),
            reset: get("x-ratelimit-reset").and_then(|v| reset_at(&v)),
            // Shown in the log: short and plain.
            reason: get("ratelimit-reason")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty() && v.len() <= 64)
                .filter(|v| v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))),
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Whether the allowance is nearly gone at `now`: under a tenth left,
    /// and not yet refilled.
    fn low(&self, now: i64) -> Option<i64> {
        match (self.limit, self.remaining, self.reset) {
            (Some(limit), Some(left), Some(reset)) if limit > 0 && left * LOW_SHARE < limit && reset > now => {
                Some(reset)
            }
            _ => None,
        }
    }

    /// Fold in a later word. Of two for one window the lower remaining wins
    /// (concurrent answers arrive in any order); a later window replaces.
    fn absorb(&mut self, new: &Rate) {
        let newer = match (self.reset, new.reset) {
            (Some(held), Some(reset)) => {
                reset > held || (reset == held && new.remaining.zip(self.remaining).is_none_or(|(n, h)| n <= h))
            }
            _ => true,
        };
        if newer {
            *self = new.clone();
        }
    }
}

/// An ISO 8601 time, or seconds since 1970.
fn reset_at(v: &str) -> Option<i64> {
    let v = v.trim();
    time::parse_rfc3339(v).or_else(|| v.parse::<i64>().ok().filter(|n| *n > 1_000_000_000))
}

#[derive(Debug, Default)]
struct Held {
    rate: Rate,
    /// A `Retry-After` not yet dated: the next tick says what time it is.
    wait: Option<u64>,
    /// Until when a `Retry-After` asked radar to stay away.
    until: Option<i64>,
}

/// The latest rate-limit word per site, shared by the clients the plugin
/// starts and the plugin that reads it.
#[derive(Debug, Clone, Default)]
pub struct Limits(Arc<Mutex<BTreeMap<String, Held>>>);

impl Limits {
    fn held(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Held>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What one response from `site` said.
    pub(crate) fn note(&self, site: &str, resp: &Resp) {
        let mut all = self.held();
        if resp.retry_after.is_some() || !resp.rate.is_empty() {
            let held = all.entry(site.to_string()).or_default();
            if let Some(s) = resp.retry_after {
                held.wait = Some(s);
                held.until = None;
            } else if (200..300).contains(&resp.status) {
                held.wait = None;
                held.until = None;
            }
            if !resp.rate.is_empty() {
                held.rate.absorb(&resp.rate);
            }
        } else if (200..300).contains(&resp.status)
            && let Some(held) = all.get_mut(site)
        {
            // Served again: a wait that was asked for has passed.
            held.wait = None;
            held.until = None;
        }
    }

    /// Date the `Retry-After`s heard since the last tick.
    pub(crate) fn stamp(&self, now: i64) {
        for held in self.held().values_mut() {
            if let Some(s) = held.wait.take() {
                held.until = Some(now.saturating_add(i64::try_from(s).unwrap_or(i64::MAX)));
            }
        }
    }

    /// Until when background reads of `site` should wait at `now`, if they
    /// should: its `Retry-After`, or the reset of an allowance under a tenth
    /// left, whichever is later, at most an hour away.
    pub(crate) fn hold(&self, site: &str, now: i64) -> Option<i64> {
        let all = self.held();
        let held = all.get(site)?;
        held.until
            .filter(|u| *u > now)
            .into_iter()
            .chain(held.rate.low(now))
            .map(|t| t.min(now.saturating_add(HOLD_MAX_SECS)))
            .max()
    }

    /// The latest headers heard from `site`.
    pub(crate) fn rate(&self, site: &str) -> Option<Rate> {
        self.held().get(site).map(|h| h.rate.clone()).filter(|r| !r.is_empty())
    }
}

#[cfg(test)]
mod tests;
