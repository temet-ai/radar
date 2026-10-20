//! The theme every plugin draws with, built from the `[theme]` config table
//! (TUI-C5: theming is infrastructure, not hex codes in source files).
//!
//! - `palette` picks an Opaline palette (`silkcircuit-neon` by default).
//! - `colors` picks the tier: `auto` (detect), `truecolor`, `256`, `16`, `none`.
//! - `slots` overrides any slot, core or plugin-declared, with `#rrggbb`.
//!
//! Plugins never hold colours: they declare domain slots with defaults
//! ([`crate::Plugin::slots`]) and name styles as specs like
//! `"status.warning bold"`, which the user can change in config.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use crate::design::theme::{DesignTheme, Modifier, Slot, SlotValue, Style, Tier};
use crate::ui::icon::{self, Glyph, Icons};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeConfig {
    #[serde(default = "default_palette")]
    pub palette: String,
    #[serde(default = "default_colors")]
    pub colors: String,
    /// Links the terminal opens on a click: `auto` (where the terminal is
    /// known to), `on`, `off`.
    #[serde(default = "default_colors", skip_serializing_if = "is_auto")]
    pub hyperlinks: String,
    /// Pictures drawn in the terminal: `auto` (where it answers for kitty's,
    /// sixel or iTerm2's graphics, not inside tmux), `on` (ask inside tmux
    /// too), `off`.
    #[serde(default = "default_colors", skip_serializing_if = "is_auto")]
    pub images: String,
    /// The mouse pointer's shape (OSC 22): a hand over what a click acts
    /// on. `auto` (where the terminal is known to take it, not inside
    /// tmux), `on`, `off`.
    #[serde(default = "default_colors", skip_serializing_if = "is_auto")]
    pub pointer: String,
    /// How much motion: `full`, `subtle` or `off` ([`crate::motion::Level`]).
    #[serde(default = "default_motion")]
    pub motion: String,
    /// The startup splash: `auto` (shown as motion allows) or `off`
    /// ([`crate::splash::Setting`]).
    #[serde(default = "default_colors", skip_serializing_if = "is_auto")]
    pub splash: String,
    /// The icon pack: `auto` (by the terminal), a built-in pack (`nerd`,
    /// `unicode`, `emoji`, `off`) or one of the user's ([`crate::ui::icon`]).
    #[serde(default = "default_colors", skip_serializing_if = "is_auto")]
    pub icons: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub slots: BTreeMap<String, String>,
    /// `[icons]` of config.toml, glyphs over the pack's. The host reads and
    /// writes it as its own table, not under `[theme]`.
    #[serde(skip)]
    pub icon_overrides: BTreeMap<String, Glyph>,
}

fn default_palette() -> String {
    "silkcircuit-neon".into()
}
fn default_colors() -> String {
    "auto".into()
}
fn is_auto(s: &String) -> bool {
    s == "auto"
}

/// The choices for a terminal feature (`hyperlinks`, `images`).
pub const SWITCHES: [&str; 3] = ["auto", "on", "off"];

/// What a switch's value means, for people.
pub fn switch_label(v: &str) -> &'static str {
    match v {
        "on" => "on",
        "off" => "off",
        _ => "where the terminal can",
    }
}
fn default_motion() -> String {
    crate::motion::Level::default().word().into()
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            palette: default_palette(),
            colors: default_colors(),
            hyperlinks: default_colors(),
            images: default_colors(),
            pointer: default_colors(),
            motion: default_motion(),
            splash: default_colors(),
            icons: default_colors(),
            slots: BTreeMap::new(),
            icon_overrides: BTreeMap::new(),
        }
    }
}

pub const COLOR_LEVELS: [&str; 5] = ["auto", "truecolor", "256", "16", "none"];

/// What a colour level means, for people.
pub fn color_level_label(level: &str) -> &'static str {
    match level {
        "truecolor" => "full colour",
        "256" => "256 colours",
        "16" => "16 colours (your terminal's own)",
        "none" => "no colour",
        _ => "match the terminal",
    }
}

/// A slot described for people: the heading it sits under, a short name and
/// where it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotInfo {
    pub group: &'static str,
    pub label: String,
    pub meaning: String,
}

/// The headings slots are listed under, in order.
pub const SLOT_GROUPS: [&str; 7] = [
    "backgrounds",
    "text",
    "accents",
    "status",
    "data",
    "glow",
    "plugins",
];

pub fn slot_info(name: &str) -> SlotInfo {
    let (group, label, meaning) = match name {
        "bg.base" => ("backgrounds", "canvas", "the screen behind every panel"),
        "bg.surface" => ("backgrounds", "panels", "inside each panel"),
        "bg.overlay" => (
            "backgrounds",
            "overlays",
            "menus, help, and the selected row of a panel without focus",
        ),
        "bg.selection" => (
            "backgrounds",
            "selection",
            "the selected row of the focused panel",
        ),
        "bg.hover" => (
            "backgrounds",
            "hover",
            "what a click would act on, under the mouse pointer",
        ),
        "text.primary" => ("text", "main text", "body text everywhere"),
        "text.secondary" => ("text", "secondary text", "repo names and secondary values"),
        "text.muted" => ("text", "muted text", "labels, times, empty states"),
        "text.emphasis" => ("text", "emphasis", "titles and the selected row"),
        "accent.primary" => (
            "accents",
            "accent",
            "focused panel borders and shortcut letters",
        ),
        "accent.muted" => ("accents", "quiet accent", "borders of panels without focus"),
        "accent.secondary" => (
            "accents",
            "links and keys",
            "links, branches, keys in the help bar",
        ),
        "accent.tertiary" => ("accents", "third accent", "spare accent for plugins"),
        "status.success" => (
            "status",
            "success",
            "things that are fine: passed, ready, shown",
        ),
        "status.warning" => ("status", "warning", "things that want you: review, unsaved"),
        "status.error" => ("status", "error", "things that are broken: failed, fix"),
        "status.info" => (
            "status",
            "info",
            "things in motion: running, waiting, loading",
        ),
        "data.number" => ("data", "numbers", "counts and numbers"),
        "diff.added" => ("data", "added", "lines added"),
        "diff.removed" => ("data", "removed", "lines removed"),
        "glow.purple" => ("glow", "glow purple", "strong emphasis"),
        "glow.pink" => ("glow", "glow pink", "strong emphasis"),
        "glow.cyan" => ("glow", "glow cyan", "strong emphasis"),
        other => {
            let (owner, rest) = other.split_once('.').unwrap_or(("plugin", other));
            return SlotInfo {
                group: "plugins",
                label: format!("{owner} {rest}"),
                meaning: format!("declared by a plugin: {owner} colour \"{rest}\""),
            };
        }
    };
    SlotInfo {
        group,
        label: label.into(),
        meaning: meaning.into(),
    }
}

/// Every palette Opaline can load.
pub fn palettes() -> Vec<String> {
    let mut names: Vec<String> = opaline::builtins::list_available_themes()
        .into_iter()
        .map(|t| t.name)
        .collect();
    names.sort();
    names
}

impl ThemeConfig {
    pub fn tier(&self, detected: Tier) -> Tier {
        match self.colors.as_str() {
            "truecolor" => Tier::TrueColor,
            "256" => Tier::Ansi256,
            "16" => Tier::Ansi16,
            "none" => Tier::Mono,
            _ => detected,
        }
    }
}

/// `#rrggbb` → `(r, g, b)`.
pub fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

/// The nearest xterm-256 colour-cube index, so an override also degrades.
fn nearest_256((r, g, b): (u8, u8, u8)) -> u8 {
    let q = |v: u8| {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            (v - 35) / 40
        }
    };
    16 + 36 * q(r) + 6 * q(g) + q(b)
}

pub struct Theme {
    pub d: DesignTheme,
    /// Every domain slot, after overrides.
    domain: Vec<String>,
    /// The icons in use: the chosen pack with the overrides on top.
    icons: Icons,
}

impl Theme {
    /// The built-in look, for tests and for the moment before config loads.
    pub fn new(tier: Tier) -> Self {
        Self::build(&ThemeConfig::default(), tier, &[])
    }

    pub fn build(cfg: &ThemeConfig, detected: Tier, declared: &[(String, SlotValue)]) -> Self {
        let tier = cfg.tier(detected);
        // The default palette goes through the design layer's guide-exact table
        // (TUI-C4a); any other palette keeps its own values.
        let mut d = if cfg.palette == default_palette() {
            DesignTheme::with_tier(tier)
        } else {
            DesignTheme::from_builtin(&cfg.palette, tier)
                .unwrap_or_else(|| DesignTheme::with_tier(tier))
        };
        let mut domain = Vec::new();
        for (name, value) in declared {
            let value = match cfg.slots.get(name).and_then(|h| parse_hex(h)) {
                Some(rgb) => SlotValue {
                    truecolor: rgb,
                    ansi256: nearest_256(rgb),
                    ..*value
                },
                None => *value,
            };
            d.register_domain_slot(name.clone(), value);
            domain.push(name.clone());
        }
        // Core slots: the override replaces the palette's value.
        for slot in Slot::ALL {
            if let Some(rgb) = cfg.slots.get(slot.token_name()).and_then(|h| parse_hex(h)) {
                let base = SlotValue::from(slot);
                d.register_domain_slot(
                    slot.token_name(),
                    SlotValue {
                        truecolor: rgb,
                        ansi256: nearest_256(rgb),
                        ..base
                    },
                );
            }
        }
        // The off pack ignores custom packs and overrides. Keep that explicit
        // path independent of the runner's thread-local icon configuration.
        let icons = if cfg.icons == "off" {
            Icons::off()
        } else {
            icon::resolve(&cfg.icons, &icon::user_packs(), &cfg.icon_overrides).0
        };
        Self { d, domain, icons }
    }

    /// The icons in use ([`crate::ui::icon`] draws them).
    pub fn icons(&self) -> &Icons {
        &self.icons
    }

    pub fn tier(&self) -> Tier {
        self.d.tier()
    }

    /// A slot's style by name: core first (with any override), then domain.
    pub fn resolve(&self, name: &str) -> Style {
        if self.d.has_domain_slot(name) {
            // An override of a core slot is registered under its own name.
            let mut s = self.d.domain_style(name).unwrap_or_default();
            if let Some(slot) = Slot::from_token_name(name) {
                s = s.add_modifier(self.d.style(slot).add_modifier);
            }
            return s;
        }
        self.d
            .style_named(name)
            .unwrap_or_else(|| self.d.style(Slot::TextPrimary))
    }

    /// A style spec: a slot name then modifiers, e.g. `"status.error bold"`.
    pub fn resolve_spec(&self, spec: &str) -> Style {
        let mut words = spec.split_whitespace();
        let mut style = words.next().map_or_else(
            || self.d.style(Slot::TextPrimary),
            |slot| self.resolve(slot),
        );
        for w in words {
            style = style.add_modifier(match w {
                "bold" => Modifier::BOLD,
                "italic" => Modifier::ITALIC,
                "underline" => Modifier::UNDERLINED,
                "dim" => Modifier::DIM,
                "reversed" => Modifier::REVERSED,
                "strike" => Modifier::CROSSED_OUT,
                _ => Modifier::empty(),
            });
        }
        style
    }

    /// Every slot name a user can override, core then domain.
    pub fn slot_names(&self) -> Vec<String> {
        Slot::ALL
            .iter()
            .map(|s| s.token_name().to_string())
            .chain(self.domain.iter().cloned())
            .collect()
    }

    /// A slot's current truecolor value as `#rrggbb`.
    pub fn hex(&self, name: &str) -> String {
        let color = self.d.opaline().color(name);
        format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::theme::Color;

    #[test]
    fn overrides_reach_core_and_domain_slots() {
        let declared = vec![(
            "tenant.cyan".to_string(),
            SlotValue::new((0, 255, 255), 51, Color::Cyan),
        )];
        let mut cfg = ThemeConfig {
            colors: "truecolor".into(),
            ..Default::default()
        };
        cfg.slots.insert("accent.primary".into(), "#112233".into());
        cfg.slots.insert("tenant.cyan".into(), "#445566".into());
        let t = Theme::build(&cfg, Tier::Mono, &declared);
        assert_eq!(
            t.resolve(Slot::AccentPrimary.token_name()).fg,
            Some(Color::Rgb(0x11, 0x22, 0x33))
        );
        assert_eq!(
            t.resolve("tenant.cyan").fg,
            Some(Color::Rgb(0x44, 0x55, 0x66))
        );
        assert!(t.slot_names().contains(&"tenant.cyan".to_string()));
    }

    #[test]
    fn colors_none_strips_everything_and_auto_uses_detection() {
        let cfg = ThemeConfig {
            colors: "none".into(),
            ..Default::default()
        };
        assert_eq!(
            Theme::build(&cfg, Tier::TrueColor, &[])
                .resolve(Slot::StatusError.token_name())
                .fg,
            None
        );
        let auto = ThemeConfig::default();
        let sixteen = Theme::build(&auto, Tier::Ansi16, &[])
            .resolve(Slot::StatusError.token_name())
            .fg;
        assert_eq!(sixteen, Some(Slot::StatusError.ansi16()));
    }

    #[test]
    fn specs_add_modifiers_and_unknown_names_fall_back() {
        let t = Theme::new(Tier::TrueColor);
        let s = t.resolve_spec("status.warning bold underline");
        assert!(
            s.add_modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );
        assert_eq!(t.resolve_spec("no.such.slot").fg, t.resolve(Slot::TextPrimary.token_name()).fg);
    }

    #[test]
    fn hex_parsing_and_256_fallback() {
        assert_eq!(parse_hex("#e135ff"), Some((0xe1, 0x35, 0xff)));
        assert_eq!(parse_hex("e135ff"), None);
        assert_eq!(nearest_256((255, 255, 255)), 231);
        assert_eq!(nearest_256((0, 0, 0)), 16);
    }

    #[test]
    fn the_default_palette_uses_the_style_guide_values() {
        let t = Theme::new(Tier::TrueColor);
        assert_eq!(t.hex("text.muted"), "#6272a4");
        assert_eq!(t.hex("bg.selection"), "#44475a");
    }

    #[test]
    fn the_default_palette_is_listed() {
        assert!(palettes().contains(&"silkcircuit-neon".to_string()));
    }
}
