//! The services' chips on the status line: the host asks each ready part
//! for its health, turns it into one chip each, and a click on a chip opens
//! the part's health menu, whose `refresh now` asks the part to sync.

use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::health::{Health, Mark, Source, SourceState};
use crate::host::tests::probe;
use crate::host::Event;
use crate::plugin::{Effect, Plugin};
use crate::view::chrome::{HealthState, targets};
use crate::view::{Ink, Intent};
use crate::{Host, Setup};

const NOW: i64 = 1_800_000_000;
const MARK: Mark = Mark { nerd: "\u{f0ac}", unicode: "◆", emoji: "🌐" };

/// A part that syncs with a service: what it says of its sources, and how
/// often it was asked to refresh.
struct Svc {
    id: &'static str,
    ready: bool,
    sources: Vec<Source>,
    refreshed: Arc<Mutex<usize>>,
}

impl Plugin for Svc {
    fn id(&self) -> &'static str {
        self.id
    }
    fn set_up(&self) -> Setup {
        if self.ready { Setup::Ready } else { Setup::Needed("a token".into()) }
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn health(&self) -> Option<Health> {
        Some(Health { mark: MARK, sources: self.sources.clone(), syncing: false, every: 60 })
    }
    fn refresh_now(&mut self) -> Vec<Effect> {
        *self.refreshed.lock().unwrap() += 1;
        vec![]
    }
}

fn svc(id: &'static str, ready: bool, sources: Vec<Source>) -> (Box<dyn Plugin>, Arc<Mutex<usize>>) {
    let refreshed = Arc::new(Mutex::new(0));
    (Box::new(Svc { id, ready, sources, refreshed: refreshed.clone() }), refreshed)
}

fn failing(name: &str) -> Source {
    Source::new(name, SourceState::Failing, "503 Service Unavailable · trying again in 4m", Some(NOW - 600))
}

/// A screen part without health, `lab` with one of two sources failing,
/// `vault` signed out, and `later` not set up yet.
fn host() -> (Host, Arc<Mutex<usize>>) {
    let (lab, refreshed) = svc("lab", true, vec![failing("wecm"), Source::ok("acme", Some(NOW - 12))]);
    let (vault, _) = svc(
        "vault",
        true,
        vec![Source::new("main", SourceState::SignedOut, "VAULT_TOKEN is not set", None)],
    );
    let (later, _) = svc("later", false, vec![Source::ok("x", Some(NOW))]);
    let host = Host::new(vec![probe("alpha", 'A', "a").0, lab, vault, later], NOW).unwrap();
    (host, refreshed)
}

#[test]
fn each_ready_part_with_health_has_a_chip_in_the_colour_of_its_state() {
    let (host, _) = host();
    let chips = host.chrome_status().health;
    let got: Vec<(&str, HealthState, Ink, Option<&str>)> =
        chips.iter().map(|c| (c.part.as_str(), c.state, c.ink, c.count.as_deref())).collect();
    assert_eq!(
        got,
        [
            ("lab", HealthState::Partial, crate::health::PARTIAL, Some("1/2")),
            ("vault", HealthState::SignedOut, Ink::AccentSecondary, None),
        ]
    );
    assert_eq!(chips[0].target, targets::health("lab"));
    // Icons are off in tests: no glyph, the part's name is drawn.
    assert_eq!((chips[0].glyph.as_str(), chips[0].label.as_str()), ("", "lab"));
    // With a pack, the part's own mark, as the pack draws marks.
    let mut host = host;
    host.colours.cfg.icons = "unicode".into();
    let theme = host.build_theme(crate::design::theme::Tier::Mono);
    assert_eq!(host.chrome_view_with(120, 30, &theme).status.health[0].glyph, "◆");
    host.colours.cfg.icons = "nerd".into();
    let theme = host.build_theme(crate::design::theme::Tier::Mono);
    assert_eq!(host.chrome_view_with(120, 30, &theme).status.health[0].glyph, "\u{f0ac}");
    let last = crate::headless::text_of(&host, 120, 24).lines().nth(23).unwrap().to_string();
    assert!(last.ends_with("\u{f0ac} 1/2 \u{f0ac}"), "{last:?}");
}

#[test]
fn a_chip_opens_its_menu_and_refresh_now_asks_the_part_to_sync() {
    let (mut host, refreshed) = host();
    host.update(Event::Intent(Intent::Activate { target: targets::health("lab") }));
    let menu = host.chrome_view(120, 30).menu.expect("the health menu");
    assert_eq!(menu.title, "lab · 1 of 2 failing");
    let rows: Vec<(&str, &str)> = menu.items.iter().map(|c| (c.label.as_str(), c.hint.as_str())).collect();
    assert_eq!(
        rows,
        [
            ("refresh now", "ask every source again, waiting or not"),
            ("sources", ""),
            ("wecm", "503 Service Unavailable · trying again in 4m"),
            ("acme", "ok · 12s ago"),
        ]
    );
    host.update(Event::Intent(Intent::Activate { target: targets::menu_item(0) }));
    assert!(host.menu.is_none());
    assert_eq!(*refreshed.lock().unwrap(), 1);
    // A target for no part does nothing.
    assert!(host.update(Event::Intent(Intent::Activate { target: targets::health("nope") })).is_empty());
    assert!(host.menu.is_none());
}

#[test]
fn the_chips_are_view_data_in_order_and_activating_one_opens_its_menu() {
    let (mut host, _) = host();
    let status = host.chrome_view(120, 24).status;
    let names: Vec<&str> = status.health.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(names, ["lab", "vault"], "{status:?}");
    host.update(Event::Intent(Intent::Activate { target: targets::health("vault") }));
    let open = host.menu.as_ref().expect("activating the chip opens its menu");
    assert_eq!(open.menu.id, "radar-health:vault");
    assert_eq!(open.menu.title, "vault · signed out");
}

#[test]
fn the_chips_give_way_last_when_the_line_is_narrow() {
    let (host, _) = host();
    // Room for the keys and the chips.
    let wide = crate::headless::text_of(&host, 80, 24);
    assert!(wide.lines().nth(23).unwrap().ends_with("lab 1/2 vault"), "{wide}");
    // Too narrow: the chips go, the extra-actions word stays.
    let narrow = crate::headless::text_of(&host, 30, 24);
    let last = narrow.lines().nth(23).unwrap();
    assert!(!last.contains("vault"), "{last:?}");
    assert!(last.contains("extra actions"), "{last:?}");
}
