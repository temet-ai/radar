//! Slack's chip on the status line: the workspace while radar cannot sign in
//! (the token refused or not set); once signed in, each watched channel and
//! For you, whether its last read went through.

use radar_core::health::{Health, Mark, Source, failed};

use crate::{SlackPlugin, SyncErrorKey};

/// Slack's mark (Nerd Font `fa-slack`), a hash, a speech balloon.
pub(crate) const MARK: Mark = Mark { nerd: "\u{f198}", unicode: "#", emoji: "💬" };

pub(crate) fn health(p: &SlackPlugin) -> Option<Health> {
    let at = p.sync.last_ok;
    let source = |name: &str, key: SyncErrorKey| match p.sync.errors.get(&key) {
        Some(e) => {
            let why = e.lines().next().unwrap_or_default();
            Source::new(name, failed(e), why, at)
        }
        None => Source::ok(name, at),
    };
    let sources = if p.sync.errors.contains_key(&SyncErrorKey::Bootstrap) {
        vec![source("workspace", SyncErrorKey::Bootstrap)]
    } else {
        let mut out: Vec<Source> =
            p.cfg.watched().map(|c| source(&c.alias, SyncErrorKey::Channel(c.alias.clone()))).collect();
        out.push(source("for you", SyncErrorKey::ForYou));
        if p.sync.errors.contains_key(&SyncErrorKey::Followed) {
            out.push(source("followed threads", SyncErrorKey::Followed));
        }
        out
    };
    Some(Health { mark: MARK, sources, syncing: p.sync.reading > 0, every: p.cfg.refresh_secs as i64 })
}

#[cfg(test)]
mod tests {
    use radar_core::view::chrome::HealthState;

    use crate::demo::sample;

    use super::*;

    fn read_now() -> SlackPlugin {
        let mut p = sample();
        p.sync.errors.clear();
        p.sync.reading = 0;
        p.sync.last_ok = Some(p.now - 12);
        p
    }

    fn state(p: &SlackPlugin) -> HealthState {
        health(p).unwrap().state(p.now)
    }

    #[test]
    fn watched_channels_and_for_you_read_lately_are_ok() {
        let p = read_now();
        let h = health(&p).unwrap();
        let names: Vec<&str> = h.sources.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["team", "svc", "for you"]);
        assert_eq!(h.state(p.now), HealthState::Ok);
        assert_eq!(radar_core::health::source_words(&h.sources[0], p.now), "ok · 12s ago");
    }

    #[test]
    fn one_channel_failing_is_partial() {
        let mut p = read_now();
        p.sync.errors.insert(SyncErrorKey::Channel("svc".into()), "Slack answered 503".into());
        let h = health(&p).unwrap();
        assert_eq!(h.state(p.now), HealthState::Partial);
        assert_eq!(h.count(p.now).as_deref(), Some("1/3"));
    }

    #[test]
    fn every_read_failing_is_failing_and_rate_limited_is_waiting() {
        let mut p = read_now();
        for k in [SyncErrorKey::Channel("team".into()), SyncErrorKey::Channel("svc".into()), SyncErrorKey::ForYou] {
            p.sync.errors.insert(k, "Slack answered 503".into());
        }
        assert_eq!(state(&p), HealthState::Failing);
        for k in [SyncErrorKey::Channel("team".into()), SyncErrorKey::Channel("svc".into()), SyncErrorKey::ForYou] {
            p.sync.errors.insert(k, "rate limited".into());
        }
        assert_eq!(state(&p), HealthState::Waiting);
    }

    #[test]
    fn a_token_refused_is_signed_out_for_the_whole_workspace() {
        let mut p = read_now();
        p.sync.errors.insert(SyncErrorKey::Bootstrap, "the token was refused (invalid_auth)".into());
        let h = health(&p).unwrap();
        assert_eq!(h.sources.len(), 1);
        assert_eq!(h.state(p.now), HealthState::SignedOut);
    }

    #[test]
    fn a_read_under_way_is_syncing_and_none_for_long_is_stale() {
        let mut p = read_now();
        p.sync.reading = 2;
        assert_eq!(state(&p), HealthState::Syncing);
        p.sync.reading = 0;
        p.now += 3 * p.cfg.refresh_secs as i64;
        assert_eq!(state(&p), HealthState::Stale);
    }
}
