//! GitLab's chip on the status line: each tenancy, whether its last sync
//! went through, failed (and when it is tried again), was turned down, or
//! is held while GitLab's rate limit is nearly spent.

use radar_core::health::{Health, Mark, Source, SourceState, failed};

use crate::{GitlabPlugin, backoff};

/// GitLab's tanuki (Nerd Font `fa-gitlab`), a triangle, a fox.
pub(crate) const MARK: Mark = Mark { nerd: "\u{f296}", unicode: "▼", emoji: "🦊" };

pub(crate) fn health(p: &GitlabPlugin) -> Option<Health> {
    let sources = p
        .cfg
        .sources
        .iter()
        .map(|s| {
            let path = p.cfg.group_path(s);
            let at = p.sync.synced_at.get(&path).copied();
            match (p.sync.errors.get(&path), p.sync.backoff.get(&path)) {
                (Some(e), _) => Source::new(&s.group, failed(e), p.sync_failure(&path, e), at),
                (None, Some(b)) if b.waiting(p.now) => Source::new(
                    &s.group,
                    SourceState::Waiting,
                    format!(
                        "rate limit nearly spent · next sync in {}",
                        backoff::in_words(b.left(p.now))
                    ),
                    at,
                ),
                (None, _) => Source::ok(&s.group, at),
            }
        })
        .collect();
    Some(Health {
        mark: MARK,
        sources,
        syncing: p.sync.in_flight > 0,
        every: p.cfg.refresh_secs as i64,
    })
}

#[cfg(test)]
mod tests {
    use radar_core::view::chrome::HealthState;

    use crate::backoff::Backoff;
    use crate::demo::sample;

    use super::*;

    const A: &str = "acme/team-a";
    const B: &str = "acme/team-b";

    fn synced_now() -> GitlabPlugin {
        let mut p = sample();
        for g in [A, B] {
            p.sync.synced_at.insert(g.into(), p.now - 12);
        }
        p
    }

    fn state(p: &GitlabPlugin) -> HealthState {
        health(p).unwrap().state(p.now)
    }

    #[test]
    fn tenancies_synced_lately_are_ok_and_say_when() {
        let p = synced_now();
        assert_eq!(state(&p), HealthState::Ok);
        let h = health(&p).unwrap();
        let names: Vec<&str> = h.sources.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["team-b", "team-a"]);
        assert_eq!(radar_core::health::source_words(&h.sources[0], p.now), "ok · 12s ago");
    }

    #[test]
    fn one_tenancy_failing_is_partial_and_says_when_it_is_tried_again() {
        let mut p = synced_now();
        p.sync.errors.insert(A.into(), "GitLab answered 503 Service Unavailable".into());
        p.sync.backoff.insert(A.into(), Backoff { failures: 1, until: p.now + 230 });
        let h = health(&p).unwrap();
        assert_eq!(h.state(p.now), HealthState::Partial);
        assert_eq!(h.count(p.now).as_deref(), Some("1/2"));
        assert_eq!(h.sources[1].words, "GitLab answered 503 Service Unavailable · trying again in 4m");
    }

    #[test]
    fn every_tenancy_failing_is_failing() {
        let mut p = synced_now();
        for g in [A, B] {
            p.sync.errors.insert(g.into(), "GitLab answered 502 Bad Gateway".into());
        }
        assert_eq!(state(&p), HealthState::Failing);
    }

    #[test]
    fn a_rate_limit_nearly_spent_is_waiting() {
        let mut p = synced_now();
        for g in [A, B] {
            p.sync.backoff.insert(g.into(), Backoff { failures: 0, until: p.now + 90 });
        }
        assert_eq!(state(&p), HealthState::Waiting);
        assert_eq!(health(&p).unwrap().sources[0].words, "rate limit nearly spent · next sync in 2m");
    }

    #[test]
    fn a_token_turned_down_or_not_set_is_signed_out() {
        let mut p = synced_now();
        p.sync.errors.insert(A.into(), "GitLab rejected the token (401)".into());
        p.sync.errors.insert(B.into(), "GITLAB_TOKEN is not set".into());
        assert_eq!(state(&p), HealthState::SignedOut);
    }

    #[test]
    fn a_sync_under_way_is_syncing_and_none_for_long_is_stale() {
        let mut p = synced_now();
        p.sync.in_flight = 1;
        assert_eq!(state(&p), HealthState::Syncing);
        p.sync.in_flight = 0;
        p.now += 3 * p.cfg.refresh_secs as i64 + 13;
        assert_eq!(state(&p), HealthState::Stale);
    }
}
