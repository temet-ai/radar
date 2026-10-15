//! How each service a part syncs with is doing: one chip per ready part at
//! the far right of the status line, on every screen, in the colour of its
//! state; a click opens a menu with each source's state and a `refresh now`.
//!
//! A part says what it holds ([`crate::Plugin::health`]): its service's own
//! mark, each source (a tenancy, a site, a repository, a channel), whether
//! its last sync went through, failed, was turned down or is held back, and
//! when it last synced well. The host turns that into one state
//! ([`Health::state`]) with the host's clock, so every part's chip means the
//! same thing.

use crate::plugin::MenuItem;
use crate::ui::icon::{Icon, Icons};
use crate::view::Ink;
use crate::view::chrome::HealthState;

/// The health menu's id; the part's id follows a `:`.
pub const MENU: &str = "radar-health";

/// No good sync for this many refresh intervals is stale.
pub const STALE_AFTER: i64 = 3;

/// The ink of a service where some sources fail and others are fine. Amber
/// with the count of failing sources; `Ink::Bad` would show any failure red.
pub const PARTIAL: Ink = Ink::Warn;

/// A service's own mark, in each kind of icon pack: a Nerd Font glyph (one
/// cell), a plain symbol every font has (one cell) and an emoji (two).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    pub nerd: &'static str,
    pub unicode: &'static str,
    pub emoji: &'static str,
}

impl Mark {
    /// The mark as `icons` would draw it: none with icons off, the emoji in
    /// a pack of two-cell glyphs, the Nerd Font glyph in a pack of Nerd
    /// Font symbols, else the plain symbol.
    pub fn in_pack(&self, icons: &Icons) -> String {
        match icons.width {
            0 => String::new(),
            2 => self.emoji.into(),
            _ if icons.nerd() => self.nerd.into(),
            _ => self.unicode.into(),
        }
    }
}

/// How one source stands after its last sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceState {
    Ok,
    /// Held back without a failure: the service's rate limit is nearly
    /// spent, or it asked radar to wait.
    Waiting,
    /// Its last sync failed (and may be waiting to try again).
    Failing,
    /// The service turned the credentials down, or there are none.
    SignedOut,
}

/// One source of a part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// What the part calls it (`acme`, `team-a`, `owner/repo`).
    pub name: String,
    pub state: SourceState,
    /// What went wrong and when it is tried again
    /// (`503 Service Unavailable · trying again in 4m`); empty when ok.
    pub words: String,
    /// When it last synced well.
    pub at: Option<i64>,
}

impl Source {
    pub fn ok(name: impl Into<String>, at: Option<i64>) -> Self {
        Self { name: name.into(), state: SourceState::Ok, words: String::new(), at }
    }

    pub fn new(name: impl Into<String>, state: SourceState, words: impl Into<String>, at: Option<i64>) -> Self {
        Self { name: name.into(), state, words: words.into(), at }
    }
}

/// What a part says about its syncing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    /// The service's mark.
    pub mark: Mark,
    pub sources: Vec<Source>,
    /// A sync is under way.
    pub syncing: bool,
    /// How often it syncs, in seconds.
    pub every: i64,
}

impl Health {
    /// The service's state at `now`: a failure first (all sources, or some),
    /// then syncing, then stale.
    pub fn state(&self, now: i64) -> HealthState {
        let n = self.sources.len();
        let bad: Vec<SourceState> =
            self.sources.iter().map(|s| s.state).filter(|s| *s != SourceState::Ok).collect();
        if n > 0 && bad.len() == n {
            return if bad.iter().all(|s| *s == SourceState::SignedOut) {
                HealthState::SignedOut
            } else if bad.iter().all(|s| *s == SourceState::Waiting) {
                HealthState::Waiting
            } else {
                HealthState::Failing
            };
        }
        if !bad.is_empty() {
            return HealthState::Partial;
        }
        if self.syncing {
            return HealthState::Syncing;
        }
        let stale = |s: &Source| s.at.is_none_or(|t| now - t > STALE_AFTER * self.every.max(1));
        if self.sources.iter().any(stale) { HealthState::Stale } else { HealthState::Ok }
    }

    /// `1/4`: how many of its sources fail, of all, when some do.
    pub fn count(&self, now: i64) -> Option<String> {
        (self.state(now) == HealthState::Partial).then(|| format!("{}/{}", self.failing(), self.sources.len()))
    }

    fn failing(&self) -> usize {
        self.sources.iter().filter(|s| s.state != SourceState::Ok).count()
    }
}

/// The colour of a state.
pub fn ink(state: HealthState) -> Ink {
    match state {
        HealthState::Ok => Ink::Good,
        HealthState::Syncing => Ink::Info,
        HealthState::Stale => Ink::Muted,
        HealthState::Partial => PARTIAL,
        HealthState::Failing => Ink::Bad,
        HealthState::SignedOut => Ink::AccentSecondary,
        HealthState::Waiting => Ink::Warn,
    }
}

/// The state in words, for the menu's title.
pub fn words(health: &Health, now: i64) -> String {
    match health.state(now) {
        HealthState::Ok => "ok".into(),
        HealthState::Syncing => "syncing".into(),
        HealthState::Stale => format!("stale: no good sync for {STALE_AFTER} refreshes"),
        HealthState::Partial => format!("{} of {} failing", health.failing(), health.sources.len()),
        HealthState::Failing => "failing".into(),
        HealthState::SignedOut => "signed out".into(),
        HealthState::Waiting => "waiting on a rate limit".into(),
    }
}

/// What a source's row says: `ok · 12s ago`, or what went wrong.
pub fn source_words(s: &Source, now: i64) -> String {
    match (s.state, s.at) {
        (SourceState::Ok, Some(t)) => format!("ok · {} ago", crate::time::ago(t, now)),
        (SourceState::Ok, None) => "ok · not synced yet".into(),
        _ if s.words.is_empty() => match s.state {
            SourceState::SignedOut => "signed out".into(),
            SourceState::Waiting => "waiting".into(),
            _ => "failed".into(),
        },
        _ => s.words.clone(),
    }
}

/// The menu's rows: `refresh now`, then each source with its state.
pub fn menu_items(health: &Health, now: i64) -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem::new('r', "refresh now", "ask every source again, waiting or not", "refresh").icon(Icon::Updated),
        MenuItem::heading("sources"),
    ];
    for (i, s) in health.sources.iter().enumerate() {
        let icon = match s.state {
            SourceState::Ok => Icon::RunPassed,
            SourceState::Waiting => Icon::Time,
            SourceState::Failing => Icon::RunFailed,
            SourceState::SignedOut => Icon::Connection,
        };
        items.push(MenuItem::pick(s.name.clone(), source_words(s, now), format!("source:{i}")).icon(icon));
    }
    items
}

/// The state of a source whose last sync failed with these words: signed
/// out when the credentials were turned down or are missing, waiting when
/// the service said to slow down (a 429, a rate limit), else failing.
pub fn failed(words: &str) -> SourceState {
    let w = words.to_lowercase();
    if signed_out_words(words) {
        SourceState::SignedOut
    } else if ["(429)", " 429 ", "429 too many", "rate limit", "ratelimited"].iter().any(|s| w.contains(s)) {
        SourceState::Waiting
    } else {
        SourceState::Failing
    }
}

/// Whether an error's words say the service turned the credentials down,
/// or that there are none to send (a token's variable not set).
pub fn signed_out_words(words: &str) -> bool {
    let w = words.to_lowercase();
    [
        "is not set",
        "rejected the token",
        "api token down",
        "token was refused",
        "token no longer works",
        "authentication failed",
        "token is invalid",
        "token environment variable",
        "(401)",
        "invalid_auth",
        "not_authed",
    ]
    .iter()
    .any(|s| w.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;
    const MARK: Mark = Mark { nerd: "\u{f0ac}", unicode: "◆", emoji: "🌐" };

    fn health(sources: Vec<Source>) -> Health {
        Health { mark: MARK, sources, syncing: false, every: 60 }
    }

    fn failing(name: &str) -> Source {
        Source::new(name, SourceState::Failing, "503 Service Unavailable · trying again in 4m", Some(NOW - 600))
    }

    #[test]
    fn every_source_synced_lately_is_ok_and_one_old_sync_is_stale() {
        let h = health(vec![Source::ok("a", Some(NOW - 12)), Source::ok("b", Some(NOW - 170))]);
        assert_eq!(h.state(NOW), HealthState::Ok);
        let h = health(vec![Source::ok("a", Some(NOW - 12)), Source::ok("b", Some(NOW - 181))]);
        assert_eq!(h.state(NOW), HealthState::Stale);
        assert_eq!(health(vec![Source::ok("a", None)]).state(NOW), HealthState::Stale);
    }

    #[test]
    fn syncing_shows_while_nothing_fails() {
        let mut h = health(vec![Source::ok("a", None)]);
        h.syncing = true;
        assert_eq!(h.state(NOW), HealthState::Syncing);
        h.sources.push(failing("b"));
        assert_eq!(h.state(NOW), HealthState::Partial);
    }

    #[test]
    fn some_failing_is_partial_with_a_count_and_all_failing_is_failing() {
        let h = health(vec![
            failing("wecm"),
            Source::ok("acme", Some(NOW - 12)),
            Source::ok("c", Some(NOW)),
            Source::ok("d", Some(NOW)),
        ]);
        assert_eq!(h.state(NOW), HealthState::Partial);
        assert_eq!(h.count(NOW).as_deref(), Some("1/4"));
        assert_eq!(ink(h.state(NOW)), PARTIAL);
        let h = health(vec![failing("a"), failing("b")]);
        assert_eq!(h.state(NOW), HealthState::Failing);
        assert_eq!(h.count(NOW), None);
        assert_eq!(ink(h.state(NOW)), Ink::Bad);
    }

    #[test]
    fn all_signed_out_or_all_waiting_say_so_and_a_mix_is_failing() {
        let out = || Source::new("a", SourceState::SignedOut, "the service rejected the token (401)", None);
        let wait = || Source::new("b", SourceState::Waiting, "rate limit nearly spent · next sync in 2m", Some(NOW));
        assert_eq!(health(vec![out(), out()]).state(NOW), HealthState::SignedOut);
        assert_eq!(health(vec![wait()]).state(NOW), HealthState::Waiting);
        assert_eq!(health(vec![out(), wait()]).state(NOW), HealthState::Failing);
        assert_eq!(health(vec![out(), Source::ok("c", Some(NOW))]).state(NOW), HealthState::Partial);
    }

    #[test]
    fn the_menu_has_refresh_first_then_each_source_and_when() {
        let h = health(vec![failing("wecm"), Source::ok("acme", Some(NOW - 12))]);
        let items = menu_items(&h, NOW);
        assert_eq!(items[0].value, "refresh");
        assert!(items[1].heading);
        let rows: Vec<(String, String)> = items[2..].iter().map(|i| (i.label.clone(), i.hint.clone())).collect();
        assert_eq!(
            rows,
            vec![
                ("wecm".into(), "503 Service Unavailable · trying again in 4m".into()),
                ("acme".into(), "ok · 12s ago".into()),
            ]
        );
        assert_eq!(words(&h, NOW), "1 of 2 failing");
    }

    #[test]
    fn credentials_turned_down_or_missing_read_as_signed_out_and_a_429_as_waiting() {
        for w in [
            "SOME_TOKEN is not set",
            "the service rejected the token (401)",
            "the service turned the email and API token down (401): check them under settings",
            "the token was refused (invalid_auth)",
            "authentication failed; check the token",
        ] {
            assert_eq!(failed(w), SourceState::SignedOut, "{w}");
        }
        assert_eq!(failed("the service answered 503 Service Unavailable"), SourceState::Failing);
        assert_eq!(failed("the service answered 429 Too Many Requests"), SourceState::Waiting);
        assert_eq!(failed("rate limit reached; wait before refreshing"), SourceState::Waiting);
        assert_eq!(failed("rate limited"), SourceState::Waiting);
    }

    #[test]
    fn the_mark_follows_the_pack() {
        let icons = |name: &str| crate::ui::icon::resolve(name, &[], &Default::default()).0;
        assert_eq!(MARK.in_pack(&icons("nerd")), "\u{f0ac}");
        assert_eq!(MARK.in_pack(&icons("unicode")), "◆");
        assert_eq!(MARK.in_pack(&icons("emoji")), "🌐");
        assert_eq!(MARK.in_pack(&Icons::off()), "");
    }
}
