//! GitHub's chip on the status line: each repository, whether its last
//! read went through or failed, and GitHub's own wait (a rate limit, a
//! bucket running low, or a 5xx or the network backed off from), which
//! holds every repository.

use radar_core::health::{Health, Mark, Source, SourceState, failed};

use crate::{GithubPlugin, sync};

/// GitHub's octocat (Nerd Font `fa-github`), a ringed dot, an octopus.
pub(crate) const MARK: Mark = Mark { nerd: "\u{f09b}", unicode: "◉", emoji: "🐙" };

pub(crate) fn health(p: &GithubPlugin) -> Option<Health> {
    let wait = p.sync.wait_until(p.now).map(|until| {
        let left = sync::wait_words(until - p.now);
        if p.sync.failures > 0 {
            (SourceState::Failing, format!("GitHub unavailable or unreachable · trying again in {left}"))
        } else {
            (SourceState::Waiting, format!("GitHub asked radar to wait · reads resume in {left}"))
        }
    });
    let sources = p
        .repos
        .iter()
        .map(|r| {
            let name = r.id.full_name();
            match (&r.error, &wait) {
                (Some(e), Some((_, said))) => Source::new(name, failed(e), format!("{e} · {said}"), p.last_sync),
                (Some(e), None) => Source::new(name, failed(e), e.clone(), p.last_sync),
                (None, Some((state, said))) => Source::new(name, *state, said.clone(), p.last_sync),
                (None, None) => Source::ok(name, p.last_sync),
            }
        })
        .collect();
    Some(Health {
        mark: MARK,
        sources,
        syncing: p.sync.probing || p.repos.iter().any(|r| r.loading),
        every: sync::EVERY,
    })
}

#[cfg(test)]
mod tests {
    use radar_core::view::chrome::HealthState;

    use crate::demo::sample;
    use crate::model::{ApiError, ApiErrorKind};

    use super::*;

    fn read_now() -> GithubPlugin {
        let mut p = sample();
        p.last_sync = Some(p.now - 12);
        for r in &mut p.repos {
            r.error = None;
            r.loading = false;
        }
        p.sync.probing = false;
        p
    }

    fn state(p: &GithubPlugin) -> HealthState {
        health(p).unwrap().state(p.now)
    }

    #[test]
    fn repositories_read_lately_are_ok_and_say_when() {
        let p = read_now();
        assert!(p.repos.len() > 1, "the sample has several repositories");
        assert_eq!(state(&p), HealthState::Ok);
        let h = health(&p).unwrap();
        assert_eq!(radar_core::health::source_words(&h.sources[0], p.now), "ok · 12s ago");
    }

    #[test]
    fn one_repository_failing_is_partial() {
        let mut p = read_now();
        p.repos[0].error = Some("GitHub resource is missing or inaccessible".into());
        let h = health(&p).unwrap();
        assert_eq!(h.state(p.now), HealthState::Partial);
        assert_eq!(h.count(p.now), Some(format!("1/{}", p.repos.len())));
    }

    #[test]
    fn a_5xx_backed_off_is_failing_everywhere_and_says_when() {
        let mut p = read_now();
        p.sync.stop(&ApiError::new(ApiErrorKind::Unavailable), p.now);
        let h = health(&p).unwrap();
        assert_eq!(h.state(p.now), HealthState::Failing);
        assert_eq!(h.sources[0].words, "GitHub unavailable or unreachable · trying again in 1m");
    }

    #[test]
    fn a_rate_limit_is_waiting() {
        let mut p = read_now();
        p.sync.stop(&ApiError::new(ApiErrorKind::RateLimit), p.now);
        let h = health(&p).unwrap();
        assert_eq!(h.state(p.now), HealthState::Waiting);
        assert_eq!(h.sources[0].words, "GitHub asked radar to wait · reads resume in 1m");
    }

    #[test]
    fn a_token_turned_down_is_signed_out() {
        let mut p = read_now();
        for r in &mut p.repos {
            r.error = Some(ApiError::new(ApiErrorKind::Authentication).to_string());
        }
        assert_eq!(state(&p), HealthState::SignedOut);
    }

    #[test]
    fn a_round_under_way_is_syncing_and_none_for_long_is_stale() {
        let mut p = read_now();
        p.sync.probing = true;
        assert_eq!(state(&p), HealthState::Syncing);
        p.sync.probing = false;
        p.now += 3 * sync::EVERY;
        assert_eq!(state(&p), HealthState::Stale);
    }
}
