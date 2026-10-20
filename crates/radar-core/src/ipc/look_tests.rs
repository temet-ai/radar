//! The look sent to a renderer is what the theme resolves: every slot's colour at the tier, a slot's own modifiers, every icon's
//! every slot's colour at the tier, a slot's own modifiers, every icon's
//! glyph and style, for the same configuration.

use super::fixture;
use super::look::colour_word;
use super::*;
use crate::design::theme::{Color, Modifier, SlotValue, Tier};
use crate::theme::Theme;
use crate::ui::icon::{Glyph, Icon};

/// A colour word back to the colour it was written from.
fn colour_of(word: &str) -> Option<Color> {
    if word.is_empty() {
        return None;
    }
    if let Some(hex) = word.strip_prefix('#') {
        let n = u32::from_str_radix(hex, 16).unwrap();
        return Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8));
    }
    if let Some(n) = word.strip_prefix("ansi256(").and_then(|r| r.strip_suffix(')')) {
        return Some(Color::Indexed(n.parse().unwrap()));
    }
    let all = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];
    Some(*all.iter().find(|c| colour_word(Some(**c)) == word).unwrap_or_else(|| panic!("unknown colour word {word}")))
}

/// A host with a part's declared slot, two overrides, a pack and an
/// `[icons]` glyph over it.
fn configured() -> Bridge {
    let mut host = fixture::host();
    host.colours
        .declared
        .push(("tenant.cyan".into(), SlotValue::new((0, 255, 255), 51, Color::Cyan)));
    let cfg = &mut host.colours.cfg;
    cfg.slots.insert("accent.primary".into(), "#112233".into());
    cfg.slots.insert("tenant.cyan".into(), "#445566".into());
    cfg.icons = "nerd".into();
    cfg.icon_overrides.insert("run_failed".into(), Glyph::new("X"));
    Bridge::new(host, 80, 24)
}

fn welcomed(bridge: &mut Bridge) -> Vec<ToRenderer> {
    bridge.receive(ToHost::Hello {
        protocol: PROTOCOL,
        view: crate::view::VERSION,
        capabilities: Capabilities {
            columns: 80,
            rows: 24,
            mouse: true,
            true_colour: true,
            image: ImageProtocol::None,
            plain_text: false,
        },
    });
    bridge.outgoing()
}

fn looks(out: &[ToRenderer]) -> Vec<&Look> {
    out.iter()
        .filter_map(|m| match m {
            ToRenderer::Look(l) => Some(&**l),
            _ => None,
        })
        .collect()
}

fn assert_same_as_theme(look: &Look, theme: &Theme) {
    assert_eq!(look.tier, theme.tier().into());
    assert_eq!(look.slots.len(), theme.slot_names().len());
    for name in theme.slot_names() {
        let style = theme.resolve(&name);
        assert_eq!(colour_of(&look.slots[&name]), style.fg, "slot {name} at {:?}", theme.tier());
        let own = look.modifiers.get(&name).map_or("", String::as_str);
        let m = own.split_whitespace().fold(Modifier::empty(), |m, w| {
            m | match w {
                "bold" => Modifier::BOLD,
                "italic" => Modifier::ITALIC,
                "underline" => Modifier::UNDERLINED,
                "dim" => Modifier::DIM,
                "reversed" => Modifier::REVERSED,
                "strike" => Modifier::CROSSED_OUT,
                other => panic!("not a spec word: {other}"),
            }
        });
        assert_eq!(m, style.add_modifier, "modifiers of {name}");
    }
    assert_eq!(look.icons.len(), Icon::ALL.len());
    for &i in Icon::ALL {
        let r = theme.icons().get(i);
        assert_eq!(look.icons[i.name()].glyph, r.glyph, "glyph of {}", i.name());
        assert_eq!(look.icons[i.name()].style, r.style, "style of {}", i.name());
    }
    assert_eq!(look.icon_width as usize, theme.icons().width);
    assert_eq!(look.pack, theme.icons().pack);
}

#[test]
fn the_look_sent_is_what_theme_resolves_at_every_tier() {
    for tier in [Tier::TrueColor, Tier::Ansi256, Tier::Ansi16, Tier::Mono] {
        let mut bridge = configured();
        bridge.set_tier(tier);
        let out = welcomed(&mut bridge);
        let sent = looks(&out);
        assert_eq!(sent.len(), 1, "one look after the welcome at {tier:?}");
        // What the theme resolves for the same host and tier.
        let theme = bridge.host().build_theme(tier);
        assert_same_as_theme(sent[0], &theme);
    }
}

#[test]
fn overrides_and_the_pack_reach_the_look() {
    let mut bridge = configured();
    let out = welcomed(&mut bridge);
    let look = looks(&out)[0];
    assert_eq!(look.slots["accent.primary"], "#112233");
    assert_eq!(look.slots["tenant.cyan"], "#445566");
    assert_eq!(look.slots["bg.selection"], "#44475a");
    assert_eq!(look.modifiers.get("text.emphasis").map(String::as_str), Some("bold"));
    assert_eq!(look.icons["run_failed"].glyph, "X");
    assert_eq!(look.pack, "nerd");
    assert_eq!(look.icon_width, 1);
}

#[test]
fn the_look_comes_before_the_first_snapshot_and_again_when_the_theme_changes() {
    let mut bridge = configured();
    let out = welcomed(&mut bridge);
    let kinds: Vec<&str> = out
        .iter()
        .map(|m| match m {
            ToRenderer::Welcome { .. } => "welcome",
            ToRenderer::Look(_) => "look",
            ToRenderer::Snapshot(_) => "snapshot",
            _ => "other",
        })
        .filter(|k| *k != "other")
        .collect();
    assert_eq!(kinds, ["welcome", "look", "snapshot"]);
    // Nothing changed: no look again.
    assert!(looks(&bridge.outgoing()).is_empty());
    // The user picks another pack in settings: the new look is sent, even
    // while a snapshot waits for its acknowledgement.
    assert!(bridge.in_flight().is_some());
    bridge.host_mut().colours.cfg.icons = "off".into();
    let out = bridge.outgoing();
    let again = looks(&out);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].icon_width, 0);
    assert_eq!(again[0].icons["repo"].glyph, "");
    // A restarted renderer is sent the look again.
    bridge.reconnect();
    assert_eq!(looks(&welcomed(&mut bridge)).len(), 1);
}
