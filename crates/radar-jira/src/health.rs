//! Jira's chip on the status line: each site the current view asks, whether
//! its last read went through, failed (and when it is asked again), or was
//! turned down.

use radar_core::health::{Health, Mark, Source, failed};

use crate::JiraPlugin;

/// Jira's mark (Nerd Font `md-jira`), a diamond, a blue diamond.
pub(crate) const MARK: Mark = Mark { nerd: "\u{f0303}", unicode: "◆", emoji: "🔷" };

pub(crate) fn health(p: &JiraPlugin) -> Option<Health> {
    let view = p.current_view();
    let sources = p
        .cfg
        .sites
        .iter()
        .enumerate()
        .filter_map(|(i, site)| {
            // A site the view asks nothing of is not read.
            let jql = match view {
                Some(v) => Some(p.jql_for(i, v)?),
                None => None,
            };
            let data = p.data.get(&site.name);
            let at = match &jql {
                Some(jql) => data.and_then(|d| d.synced.get(jql)).map(|pt| pt.at),
                None => data.and_then(|d| d.synced.values().map(|pt| pt.at).max()),
            };
            Some(match p.sync.errors.get(&site.name) {
                Some(e) => {
                    let why = e.lines().next().unwrap_or_default();
                    let words = match p.sync.backoff.get(&site.name) {
                        Some((until, _)) if *until > p.now => {
                            format!("{why} · trying again in {}", in_words(until - p.now))
                        }
                        _ => why.to_string(),
                    };
                    Source::new(&site.name, failed(e), words, at)
                }
                None => Source::ok(&site.name, at),
            })
        })
        .collect();
    Some(Health {
        mark: MARK,
        sources,
        syncing: !p.sync.in_flight.is_empty(),
        every: p.cfg.refresh_secs as i64,
    })
}

/// `45s`, `4m`: how long, rounded up to the minute from a minute on.
fn in_words(secs: i64) -> String {
    if secs < 60 { format!("{}s", secs.max(1)) } else { format!("{}m", (secs + 59) / 60) }
}

#[cfg(test)]
mod tests {
    use radar_core::view::chrome::HealthState;

    use crate::SyncPoint;
    use crate::demo::sample;

    use super::*;

    /// The sample with each site's view read 12 s ago.
    fn read_now() -> JiraPlugin {
        let mut p = sample();
        let v = p.current_view().cloned().expect("a view");
        for i in 0..p.cfg.sites.len() {
            let jql = p.jql_for(i, &v).expect("each site is asked");
            let name = p.cfg.sites[i].name.clone();
            let at = p.now - 12;
            p.data.entry(name).or_default().synced.insert(jql, SyncPoint { at, swept: at, fields: String::new() });
        }
        p
    }

    fn state(p: &JiraPlugin) -> HealthState {
        health(p).unwrap().state(p.now)
    }

    fn fail(p: &mut JiraPlugin, site: &str, why: &str) {
        p.sync.errors.insert(site.into(), why.into());
        p.sync.backoff.insert(site.into(), (p.now + 230, 1));
    }

    #[test]
    fn sites_read_lately_are_ok_and_say_when() {
        let p = read_now();
        assert_eq!(state(&p), HealthState::Ok);
        let h = health(&p).unwrap();
        assert_eq!(radar_core::health::source_words(&h.sources[0], p.now), "ok · 12s ago");
    }

    #[test]
    fn one_site_failing_is_partial_and_says_when_it_is_asked_again() {
        let mut p = read_now();
        fail(&mut p, "beta", "Jira answered 503 Service Unavailable");
        let h = health(&p).unwrap();
        assert_eq!(h.state(p.now), HealthState::Partial);
        assert_eq!(h.count(p.now).as_deref(), Some("1/2"));
        assert_eq!(h.sources[1].words, "Jira answered 503 Service Unavailable · trying again in 4m");
    }

    #[test]
    fn every_site_failing_is_failing_and_a_429_is_waiting() {
        let mut p = read_now();
        fail(&mut p, "acme", "Jira answered 503 Service Unavailable");
        fail(&mut p, "beta", "Jira answered 503 Service Unavailable");
        assert_eq!(state(&p), HealthState::Failing);
        fail(&mut p, "acme", "Jira answered 429 Too Many Requests");
        fail(&mut p, "beta", "Jira answered 429 Too Many Requests");
        assert_eq!(state(&p), HealthState::Waiting);
    }

    #[test]
    fn credentials_turned_down_are_signed_out() {
        let mut p = read_now();
        for s in ["acme", "beta"] {
            fail(&mut p, s, "Jira turned the email and API token down (401): check them under settings, Jira, connection");
        }
        assert_eq!(state(&p), HealthState::SignedOut);
    }

    #[test]
    fn a_read_under_way_is_syncing_and_none_for_long_is_stale() {
        let mut p = read_now();
        p.sync.in_flight.insert("acme\0jql".into());
        assert_eq!(state(&p), HealthState::Syncing);
        p.sync.in_flight.clear();
        p.now += 3 * p.cfg.refresh_secs as i64;
        assert_eq!(state(&p), HealthState::Stale);
    }
}
