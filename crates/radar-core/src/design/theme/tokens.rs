//! The TUI-C4a slot table, carried as data.
//!
//! Every slot the normative style guide names appears here exactly once, with
//! its truecolor value, its 256-color fallback index, a sensible 16-color
//! name, and any modifiers the slot carries in its own right (only
//! `text.emphasis`, which is defined as `#ffffff bold`).
//!
//! This table is the **override source of truth**: where Opaline's builtin
//! `silkcircuit-neon` theme disagrees with TUI-C4a, the values below win and
//! are written back into the Opaline theme at construction
//! (see [`super::DesignTheme`]). No widget ever reads these constants
//! directly — everything resolves through `DesignTheme` so the capability
//! tier is honored (TUI-C4).

use super::style::{Color, Modifier};

/// A named semantic color slot from TUI-C4a.
///
/// Slot names match the style guide exactly ([`Slot::token_name`]), which is
/// also the key they are registered under in the wrapped Opaline theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Slot {
    /// `bg.base` — the app canvas.
    BgBase,
    /// `bg.surface` — panels and cards.
    BgSurface,
    /// `bg.overlay` — modals, overlays, and unfocused-panel selection.
    BgOverlay,
    /// `bg.selection` — the selected row in the focused panel.
    BgSelection,
    /// `bg.hover` — what is under the mouse pointer and would act on a
    /// click: quieter than the selection, so never taken for the cursor.
    BgHover,
    /// `text.primary` — main body text.
    TextPrimary,
    /// `text.secondary` — lavender; ledes and secondary emphasis.
    TextSecondary,
    /// `text.muted` — metadata, labels, timestamps, empty states.
    TextMuted,
    /// `text.emphasis` — headers and focused items. Bold by definition.
    TextEmphasis,
    /// `accent.primary` — brand; focused borders, interactive elements.
    AccentPrimary,
    /// `accent.muted` — non-focused accent structure (unfocused borders).
    AccentMuted,
    /// `accent.secondary` — interaction, paths, links.
    AccentSecondary,
    /// `accent.tertiary` — accents and hashes.
    AccentTertiary,
    /// `status.success` — confirmations, additions, online.
    StatusSuccess,
    /// `status.warning` — warnings, attention.
    StatusWarning,
    /// `status.error` — danger, offline.
    StatusError,
    /// `status.info` — informational.
    StatusInfo,
    /// `data.number` — numbers, counts, constants.
    DataNumber,
    /// `diff.added` — additions.
    DiffAdded,
    /// `diff.removed` — deletions.
    DiffRemoved,
    /// `glow.purple` — glow emphasis.
    GlowPurple,
    /// `glow.pink` — glow emphasis.
    GlowPink,
    /// `glow.cyan` — glow emphasis.
    GlowCyan,
}

impl Slot {
    /// Every slot TUI-C4a defines, in table order.
    ///
    /// The conformance test iterates this, so a slot added to the enum without
    /// being added here is caught by `slot_table_is_complete`.
    pub const ALL: [Self; 23] = [
        Self::BgBase,
        Self::BgSurface,
        Self::BgOverlay,
        Self::BgSelection,
        Self::BgHover,
        Self::TextPrimary,
        Self::TextSecondary,
        Self::TextMuted,
        Self::TextEmphasis,
        Self::AccentPrimary,
        Self::AccentMuted,
        Self::AccentSecondary,
        Self::AccentTertiary,
        Self::StatusSuccess,
        Self::StatusWarning,
        Self::StatusError,
        Self::StatusInfo,
        Self::DataNumber,
        Self::DiffAdded,
        Self::DiffRemoved,
        Self::GlowPurple,
        Self::GlowPink,
        Self::GlowCyan,
    ];

    /// The style guide's name for this slot — also its Opaline token key.
    #[must_use]
    pub const fn token_name(self) -> &'static str {
        match self {
            Self::BgBase => "bg.base",
            Self::BgSurface => "bg.surface",
            Self::BgOverlay => "bg.overlay",
            Self::BgSelection => "bg.selection",
            Self::BgHover => "bg.hover",
            Self::TextPrimary => "text.primary",
            Self::TextSecondary => "text.secondary",
            Self::TextMuted => "text.muted",
            Self::TextEmphasis => "text.emphasis",
            Self::AccentPrimary => "accent.primary",
            Self::AccentMuted => "accent.muted",
            Self::AccentSecondary => "accent.secondary",
            Self::AccentTertiary => "accent.tertiary",
            Self::StatusSuccess => "status.success",
            Self::StatusWarning => "status.warning",
            Self::StatusError => "status.error",
            Self::StatusInfo => "status.info",
            Self::DataNumber => "data.number",
            Self::DiffAdded => "diff.added",
            Self::DiffRemoved => "diff.removed",
            Self::GlowPurple => "glow.purple",
            Self::GlowPink => "glow.pink",
            Self::GlowCyan => "glow.cyan",
        }
    }

    /// Look a slot up by its style-guide name.
    #[must_use]
    pub fn from_token_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.token_name() == name)
    }

    /// The TUI-C4a truecolor value as `(r, g, b)`.
    #[must_use]
    pub const fn truecolor(self) -> (u8, u8, u8) {
        match self {
            Self::BgBase => (0x0a, 0x08, 0x12),
            Self::BgSurface => (0x12, 0x10, 0x1a),
            Self::BgOverlay => (0x1a, 0x16, 0x2a),
            Self::BgSelection => (0x44, 0x47, 0x5a),
            Self::BgHover => (0x2a, 0x26, 0x3e),
            Self::TextPrimary => (0xf8, 0xf8, 0xf2),
            Self::TextSecondary => (0xe9, 0xd5, 0xff),
            Self::TextMuted => (0x62, 0x72, 0xa4),
            Self::TextEmphasis => (0xff, 0xff, 0xff),
            Self::AccentPrimary => (0xe1, 0x35, 0xff),
            Self::AccentMuted => (0x95, 0x80, 0xff),
            Self::AccentSecondary => (0x80, 0xff, 0xea),
            Self::AccentTertiary => (0xff, 0x6a, 0xc1),
            Self::StatusSuccess => (0x50, 0xfa, 0x7b),
            Self::StatusWarning => (0xf1, 0xfa, 0x8c),
            Self::StatusError => (0xff, 0x63, 0x63),
            Self::StatusInfo => (0x82, 0xaa, 0xff),
            Self::DataNumber => (0xff, 0xb8, 0x6c),
            Self::DiffAdded => (0x50, 0xfa, 0x7b),
            Self::DiffRemoved => (0xff, 0x63, 0x63),
            Self::GlowPurple => (0xe1, 0x35, 0xff),
            Self::GlowPink => (0xff, 0x00, 0xff),
            Self::GlowCyan => (0x80, 0xff, 0xea),
        }
    }

    /// The TUI-C4a truecolor value as a `#rrggbb` string.
    #[must_use]
    pub fn hex(self) -> String {
        let (r, g, b) = self.truecolor();
        format!("#{r:02x}{g:02x}{b:02x}")
    }

    /// The TUI-C4a 256-color fallback column — the nearest xterm-256 index.
    #[must_use]
    pub const fn ansi256(self) -> u8 {
        match self {
            Self::BgBase => 232,
            Self::BgSurface => 233,
            Self::BgOverlay => 235,
            Self::BgSelection => 239,
            Self::BgHover => 236,
            Self::TextPrimary => 255,
            Self::TextSecondary => 189,
            Self::TextMuted => 61,
            Self::TextEmphasis => 231,
            Self::AccentPrimary | Self::GlowPurple => 171,
            Self::AccentMuted => 105,
            Self::AccentSecondary | Self::GlowCyan => 122,
            Self::AccentTertiary => 205,
            Self::StatusSuccess | Self::DiffAdded => 84,
            Self::StatusWarning => 228,
            Self::StatusError | Self::DiffRemoved => 203,
            Self::StatusInfo => 111,
            Self::DataNumber => 215,
            Self::GlowPink => 201,
        }
    }

    /// The 16-color name for this slot.
    ///
    /// TUI-C2 makes 16 colors the foundation rather than a lossy afterthought,
    /// and named ANSI colors deliberately inherit the user's terminal theme —
    /// so this column is chosen for *role separation* (nothing that must be
    /// told apart collapses onto the same name) rather than hue fidelity.
    /// The style guide does not publish a 16-color column; this mapping is a
    /// design-layer choice, documented here rather than presented as a rule.
    #[must_use]
    pub const fn ansi16(self) -> Color {
        match self {
            // Hover has no colour of its own at 16 colours: it is drawn
            // underlined there (`ui::hover`), never in the selection's grey.
            Self::BgBase | Self::BgSurface | Self::BgHover => Color::Black,
            Self::BgOverlay | Self::BgSelection | Self::TextMuted => Color::DarkGray,
            Self::TextPrimary => Color::White,
            Self::TextSecondary => Color::Gray,
            Self::TextEmphasis => Color::White,
            Self::AccentPrimary | Self::GlowPurple => Color::LightMagenta,
            Self::AccentMuted => Color::Blue,
            Self::AccentSecondary | Self::GlowCyan => Color::LightCyan,
            Self::AccentTertiary | Self::GlowPink => Color::Magenta,
            Self::StatusSuccess | Self::DiffAdded => Color::LightGreen,
            Self::StatusWarning => Color::LightYellow,
            Self::StatusError | Self::DiffRemoved => Color::LightRed,
            Self::StatusInfo => Color::LightBlue,
            Self::DataNumber => Color::Yellow,
        }
    }

    /// Modifiers the slot carries by definition, independent of tier.
    ///
    /// Only `text.emphasis` has one: TUI-C4a defines it as `#ffffff bold`, so
    /// the bold survives all the way down to the monochrome tier and is what
    /// keeps headers legible there.
    #[must_use]
    pub const fn modifiers(self) -> Modifier {
        match self {
            Self::TextEmphasis => Modifier::BOLD,
            _ => Modifier::empty(),
        }
    }

    /// Whether this slot is a background layer (`bg.*`).
    ///
    /// Backgrounds are the one family that must **not** be painted at the
    /// 16-color tier as a foreground by accident, so callers that build
    /// generic tooling can branch on this.
    #[must_use]
    pub const fn is_background(self) -> bool {
        matches!(
            self,
            Self::BgBase | Self::BgSurface | Self::BgOverlay | Self::BgSelection | Self::BgHover
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_table_is_complete_and_unique() {
        assert_eq!(Slot::ALL.len(), 23);
        let mut names: Vec<&str> = Slot::ALL.iter().map(|s| s.token_name()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "duplicate slot names in the table");
    }

    #[test]
    fn every_slot_round_trips_through_its_name() {
        for slot in Slot::ALL {
            assert_eq!(Slot::from_token_name(slot.token_name()), Some(slot));
        }
        assert_eq!(Slot::from_token_name("not.a.slot"), None);
    }

    #[test]
    fn hex_matches_the_style_guide_spelling() {
        assert_eq!(Slot::BgBase.hex(), "#0a0812");
        assert_eq!(Slot::AccentPrimary.hex(), "#e135ff");
        assert_eq!(Slot::TextMuted.hex(), "#6272a4");
        assert_eq!(Slot::DataNumber.hex(), "#ffb86c");
    }

    #[test]
    fn only_text_emphasis_carries_a_modifier() {
        for slot in Slot::ALL {
            let expected = if slot == Slot::TextEmphasis {
                Modifier::BOLD
            } else {
                Modifier::empty()
            };
            assert_eq!(slot.modifiers(), expected, "{}", slot.token_name());
        }
    }

    #[test]
    fn background_family_is_exactly_the_bg_slots() {
        for slot in Slot::ALL {
            assert_eq!(
                slot.is_background(),
                slot.token_name().starts_with("bg."),
                "{}",
                slot.token_name()
            );
        }
    }
}
