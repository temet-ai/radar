//! The look the host resolved, for a renderer in another process: every
//! theme slot's colour at the active tier, the modifiers a slot carries of
//! its own, and the icons in use. The Ratatui renderer reads these from
//! [`Theme`] as it draws; the Ink renderer is sent them
//! ([`super::ToRenderer::Look`]) and chooses no colour or glyph of its own,
//! so a palette, a colour tier, a slot override, an icon pack or an
//! `[icons]` override reaches both renderers the same.

use std::collections::BTreeMap;

use ratatui::style::{Color, Modifier};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::design::theme::Tier;
use crate::theme::Theme;
use crate::ui::icon::Icon;

/// The colours a terminal draws, as the host chose them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ColourTier {
    #[serde(rename = "truecolor")]
    TrueColor,
    #[serde(rename = "256")]
    Ansi256,
    #[serde(rename = "16")]
    Ansi16,
    #[serde(rename = "mono")]
    Mono,
}

impl From<Tier> for ColourTier {
    fn from(t: Tier) -> Self {
        match t {
            Tier::TrueColor => ColourTier::TrueColor,
            Tier::Ansi256 => ColourTier::Ansi256,
            Tier::Ansi16 => ColourTier::Ansi16,
            Tier::Mono => ColourTier::Mono,
        }
    }
}

/// One icon as it is drawn: the glyph (padded to the pack's width; empty
/// when icons are off or the pack has none) and its style spec
/// (`status.error`, `accent.primary bold`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LookIcon {
    pub glyph: String,
    pub style: String,
}

/// The resolved look, whole. Bounded by what it names: one entry per theme
/// slot (core and the parts' declared ones) and one per [`Icon`]; a glyph
/// is at most a few cells ([`crate::ui::icon::check`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Look {
    pub tier: ColourTier,
    /// Every slot name → its colour as Ratatui's `Theme::style(name)`
    /// resolves it at `tier`, written as Ink's `color` takes it: `#rrggbb`
    /// at truecolor, `ansi256(n)` at 256, a terminal colour's name at 16
    /// (`red`, `redBright`, `blackBright` …), and `""` (no colour, the
    /// terminal's own) at mono.
    pub slots: BTreeMap<String, String>,
    /// Slots whose own style carries modifiers, as `Theme::spec`'s words
    /// (`text.emphasis` → `bold`). A slot not listed carries none.
    pub modifiers: BTreeMap<String, String>,
    /// Every icon by its name (`run_failed`) → glyph and style.
    pub icons: BTreeMap<String, LookIcon>,
    /// Cells a glyph takes: 0 when icons are off, 1, or 2 (emoji).
    pub icon_width: u8,
    /// The pack in use (`nerd`, `unicode`, `emoji`, `off`, or a user's).
    pub pack: String,
    /// Links are written as OSC 8 hyperlinks (`[theme] hyperlinks`, as
    /// `ui::link::wanted` decides for this terminal).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hyperlinks: bool,
    /// The pointer's shape follows what it is over, by OSC 22 (`[theme]
    /// pointer`, as `ui::pointer::wanted` decides for this terminal).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pointer: bool,
}

impl Look {
    /// The look `theme` draws with.
    pub fn of(theme: &Theme) -> Self {
        let mut slots = BTreeMap::new();
        let mut modifiers = BTreeMap::new();
        for name in theme.slot_names() {
            let style = theme.style(&name);
            slots.insert(name.clone(), colour_word(style.fg));
            let words = modifier_words(style.add_modifier);
            if !words.is_empty() {
                modifiers.insert(name, words);
            }
        }
        let icons = theme.icons();
        Look {
            tier: theme.tier().into(),
            slots,
            modifiers,
            icons: Icon::ALL
                .iter()
                .map(|&i| {
                    let r = icons.get(i);
                    (
                        i.name().to_string(),
                        LookIcon {
                            glyph: r.glyph.clone(),
                            style: r.style.clone(),
                        },
                    )
                })
                .collect(),
            icon_width: icons.width.min(u8::MAX as usize) as u8,
            pack: icons.pack.clone(),
            hyperlinks: false,
            pointer: false,
        }
    }
}

/// A colour as Ink's `color` prop takes it; `""` for none.
pub fn colour_word(c: Option<Color>) -> String {
    let Some(c) = c else {
        return String::new();
    };
    let named = match c {
        Color::Reset => "",
        Color::Black => "black",
        Color::Red => "red",
        Color::Green => "green",
        Color::Yellow => "yellow",
        Color::Blue => "blue",
        Color::Magenta => "magenta",
        Color::Cyan => "cyan",
        Color::Gray => "white",
        Color::DarkGray => "blackBright",
        Color::LightRed => "redBright",
        Color::LightGreen => "greenBright",
        Color::LightYellow => "yellowBright",
        Color::LightBlue => "blueBright",
        Color::LightMagenta => "magentaBright",
        Color::LightCyan => "cyanBright",
        Color::White => "whiteBright",
        Color::Rgb(r, g, b) => return format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(n) => return format!("ansi256({n})"),
    };
    named.to_string()
}

/// A slot's own modifiers as `Theme::spec`'s words, in its order.
fn modifier_words(m: Modifier) -> String {
    [
        (Modifier::BOLD, "bold"),
        (Modifier::ITALIC, "italic"),
        (Modifier::UNDERLINED, "underline"),
        (Modifier::DIM, "dim"),
        (Modifier::REVERSED, "reversed"),
        (Modifier::CROSSED_OUT, "strike"),
    ]
    .iter()
    .filter(|(f, _)| m.contains(*f))
    .map(|(_, w)| *w)
    .collect::<Vec<_>>()
    .join(" ")
}
