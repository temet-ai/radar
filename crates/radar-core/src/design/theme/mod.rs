//! The TUI Design theme: Opaline's SilkCircuit engine, conformed to TUI-C4a
//! and resolved through a terminal capability tier.
//!
//! [`DesignTheme`] wraps an [`opaline::Theme`] — by default the builtin
//! `silkcircuit-neon` — and is the only thing a widget is allowed to ask for a
//! color (TUI-C4: no raw color at a call site). Resolution follows the TUI-C5
//! pipeline: Opaline owns *palette → tokens*, this type owns *tokens →
//! styles*, and the capability tier decides which representation a token
//! resolves to.
//!
//! ## Where TUI-C4a and Opaline disagree, TUI-C4a wins
//!
//! Opaline's builtin `silkcircuit-neon` is a general-purpose editor theme; the
//! style guide's TUI-C4a table is a narrower, purple-tinted subset with slots
//! Opaline does not name at all (`bg.surface`, `bg.overlay`, `text.emphasis`,
//! `accent.muted`, `status.*`, `data.number`, `diff.*`, `glow.*`). At
//! construction every TUI-C4a slot is written into the Opaline theme with the
//! table's value, overwriting the builtin where it differs. Nothing is
//! silently absorbed: [`silkcircuit_neon_discrepancies`] reports exactly what
//! was overridden and what was added, and a unit test keeps that list honest.

pub mod capability;
pub mod style;
pub mod tokens;

use std::collections::BTreeMap;

use opaline::{OpalineColor, OpalineStyle};
pub use style::{Color, Modifier, Style};

pub use capability::{TerminalEnv, Tier};
pub use tokens::Slot;

/// A color slot's full definition across every capability tier.
///
/// Used to register application domain slots (TUI-C4's "domain slots, added
/// per domain, same pattern") so that a consumer can name its own semantic
/// colors without ever writing a raw color at a call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotValue {
    /// 24-bit value, used at [`Tier::TrueColor`].
    pub truecolor: (u8, u8, u8),
    /// xterm-256 index, used at [`Tier::Ansi256`].
    pub ansi256: u8,
    /// Named ANSI color, used at [`Tier::Ansi16`].
    pub ansi16: Color,
    /// Modifiers the slot carries at every tier, including monochrome.
    pub modifiers: Modifier,
}

impl SlotValue {
    /// A domain slot with no modifiers.
    #[must_use]
    pub fn new(truecolor: (u8, u8, u8), ansi256: u8, ansi16: impl Into<Color>) -> Self {
        Self {
            truecolor,
            ansi256,
            ansi16: ansi16.into(),
            modifiers: Modifier::empty(),
        }
    }

    /// Add modifiers that survive into the monochrome tier.
    #[must_use]
    pub const fn with_modifiers(mut self, modifiers: Modifier) -> Self {
        self.modifiers = modifiers;
        self
    }
}

impl From<Slot> for SlotValue {
    fn from(slot: Slot) -> Self {
        Self {
            truecolor: slot.truecolor(),
            ansi256: slot.ansi256(),
            ansi16: slot.ansi16(),
            modifiers: slot.modifiers(),
        }
    }
}

/// How a builtin Opaline value differs from the TUI-C4a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotConflict {
    /// The Opaline theme has no token under this slot's name at all; the
    /// TUI-C4a value is registered as a new token.
    Missing,
    /// The Opaline theme defines the slot with a different value; the TUI-C4a
    /// value overwrites it.
    Differs {
        /// What the builtin theme said before the override.
        opaline: (u8, u8, u8),
    },
}

/// One row of the Opaline-vs-TUI-C4a reconciliation report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotDiscrepancy {
    /// The slot in question.
    pub slot: Slot,
    /// The TUI-C4a value, which wins.
    pub guide: (u8, u8, u8),
    /// Why the slot needed an override.
    pub conflict: SlotConflict,
}

/// Compare Opaline's pristine builtin `silkcircuit-neon` against the TUI-C4a
/// table, slot by slot, matching on the slot's own name.
///
/// Slots where the builtin already agrees are omitted. This is the audit trail
/// behind the overrides [`DesignTheme::new`] applies.
#[must_use]
pub fn silkcircuit_neon_discrepancies() -> Vec<SlotDiscrepancy> {
    let builtin = opaline::builtins::silkcircuit_neon();
    Slot::ALL
        .into_iter()
        .filter_map(|slot| {
            let guide = slot.truecolor();
            let conflict = match builtin.try_color(slot.token_name()) {
                None => SlotConflict::Missing,
                Some(c) if (c.r, c.g, c.b) != guide => SlotConflict::Differs {
                    opaline: (c.r, c.g, c.b),
                },
                Some(_) => return None,
            };
            Some(SlotDiscrepancy {
                slot,
                guide,
                conflict,
            })
        })
        .collect()
}

/// The design theme: semantic slots in, tier-correct styles out.
///
/// Construct one at startup and pass it by reference (`&DesignTheme`) — this
/// crate keeps no statics and no thread-locals, so a test can hold a
/// monochrome theme and a truecolor theme side by side in the same process.
#[derive(Debug, Clone)]
pub struct DesignTheme {
    theme: opaline::Theme,
    tier: Tier,
    domain: BTreeMap<String, SlotValue>,
}

impl DesignTheme {
    /// SilkCircuit Neon conformed to TUI-C4a, with the tier detected from the
    /// process environment **once** (`NO_COLOR`, `COLORTERM`, `TERM`).
    ///
    /// Build this at startup and keep it; do not rebuild it per frame.
    #[must_use]
    pub fn new() -> Self {
        Self::with_tier(Tier::detect())
    }

    /// SilkCircuit Neon conformed to TUI-C4a at an explicit tier.
    ///
    /// This is the constructor tests use — the environment is never read and
    /// never written.
    #[must_use]
    pub fn with_tier(tier: Tier) -> Self {
        let mut theme = opaline::builtins::silkcircuit_neon();
        apply_guide_slots(&mut theme, Override::Always);
        Self {
            theme,
            tier,
            domain: BTreeMap::new(),
        }
    }

    /// Wrap a caller-supplied Opaline theme — a different SilkCircuit variant
    /// (TUI-C4b) or a user's own TOML.
    ///
    /// The supplied theme's values are kept; TUI-C4a is used only to fill in
    /// slots the theme does not define, so no slot half-resolves (TUI-C5).
    /// 256- and 16-color fallbacks always come from the TUI-C4a table.
    #[must_use]
    pub fn from_opaline(mut theme: opaline::Theme, tier: Tier) -> Self {
        apply_guide_slots(&mut theme, Override::OnlyMissing);
        Self {
            theme,
            tier,
            domain: BTreeMap::new(),
        }
    }

    /// Load one of Opaline's builtin themes by its kebab-case id (for example
    /// `"silkcircuit-soft"`), filling any slots it lacks from TUI-C4a.
    ///
    /// Returns `None` when no builtin carries that id.
    #[must_use]
    pub fn from_builtin(name: &str, tier: Tier) -> Option<Self> {
        opaline::builtins::load_by_name(name).map(|theme| Self::from_opaline(theme, tier))
    }

    /// The capability tier every lookup on this theme is resolved through.
    #[must_use]
    pub const fn tier(&self) -> Tier {
        self.tier
    }

    /// Change the tier — used when a caller detects capability later than
    /// theme construction, not per frame.
    pub fn set_tier(&mut self, tier: Tier) {
        self.tier = tier;
    }

    /// The wrapped Opaline theme, for gradients, metadata, or token names this
    /// layer does not model.
    #[must_use]
    pub const fn opaline(&self) -> &opaline::Theme {
        &self.theme
    }

    /// The color for a slot, or `None` at [`Tier::Mono`] where there is none.
    ///
    /// At [`Tier::TrueColor`] the value comes from the wrapped Opaline theme,
    /// so a swapped variant flows through. At the lower tiers it comes from
    /// the TUI-C4a fallback columns, which are fixed by the style guide.
    #[must_use]
    pub fn color(&self, slot: Slot) -> Option<Color> {
        self.resolve(slot.token_name(), slot.ansi256(), slot.ansi16())
    }

    /// A foreground style for a slot: its color, plus any modifiers the slot
    /// carries in its own right.
    ///
    /// At [`Tier::Mono`] the color is dropped and only the modifiers remain —
    /// which is why `text.emphasis` stays legible with no color at all
    /// (TUI-P4, TUI-P7 tier 1).
    #[must_use]
    pub fn style(&self, slot: Slot) -> Style {
        let mut style = Style::new();
        if let Some(color) = self.color(slot) {
            style = style.fg(color);
        }
        let modifiers = slot.modifiers();
        if !modifiers.is_empty() {
            style = style.add_modifier(modifiers);
        }
        style
    }

    /// A background-only style for a slot.
    #[must_use]
    pub fn bg_style(&self, slot: Slot) -> Style {
        match self.color(slot) {
            Some(color) => Style::new().bg(color),
            None => Style::new(),
        }
    }

    /// A foreground slot painted over a background slot.
    #[must_use]
    pub fn on(&self, fg: Slot, bg: Slot) -> Style {
        let style = self.style(fg);
        match self.color(bg) {
            Some(color) => style.bg(color),
            None => style,
        }
    }

    /// Resolve a slot by its style-guide name, falling back to a registered
    /// domain slot. `None` means the name is not defined anywhere.
    #[must_use]
    pub fn style_named(&self, name: &str) -> Option<Style> {
        if let Some(slot) = Slot::from_token_name(name) {
            return Some(self.style(slot));
        }
        self.domain_style(name)
    }

    /// Register an application domain slot — `session.live`, `bind.down`, and
    /// the like — so a consumer names meaning, never a color (TUI-C4).
    ///
    /// Re-registering the same name replaces it.
    pub fn register_domain_slot(&mut self, name: impl Into<String>, value: SlotValue) {
        let key = name.into();
        let (r, g, b) = value.truecolor;
        self.theme
            .register_token(key.clone(), OpalineColor::new(r, g, b));
        self.domain.insert(key, value);
    }

    /// Whether a domain slot with this name has been registered.
    #[must_use]
    pub fn has_domain_slot(&self, name: &str) -> bool {
        self.domain.contains_key(name)
    }

    /// The names of every registered domain slot, sorted.
    #[must_use]
    pub fn domain_slot_names(&self) -> Vec<&str> {
        self.domain.keys().map(String::as_str).collect()
    }

    /// A foreground style for a registered domain slot.
    ///
    /// `None` means the slot was never registered — a monochrome tier yields
    /// `Some` with a colorless style, not `None`.
    #[must_use]
    pub fn domain_style(&self, name: &str) -> Option<Style> {
        let value = self.domain.get(name)?;
        let mut style = Style::new();
        if let Some(color) = self.resolve(name, value.ansi256, value.ansi16) {
            style = style.fg(color);
        }
        if !value.modifiers.is_empty() {
            style = style.add_modifier(value.modifiers);
        }
        Some(style)
    }

    fn resolve(&self, token: &str, ansi256: u8, ansi16: Color) -> Option<Color> {
        match self.tier {
            Tier::Mono => None,
            Tier::Ansi16 => Some(ansi16),
            Tier::Ansi256 => Some(Color::Indexed(ansi256)),
            Tier::TrueColor => {
                let color = self.theme.color(token);
                Some(Color::Rgb(color.r, color.g, color.b))
            }
        }
    }
}

impl Default for DesignTheme {
    /// Same as [`DesignTheme::new`] — SilkCircuit Neon at the detected tier.
    fn default() -> Self {
        Self::new()
    }
}

/// Whether [`apply_guide_slots`] overwrites values the theme already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Override {
    /// TUI-C4a wins outright (the conformance path).
    Always,
    /// The theme's own values win; TUI-C4a only fills gaps.
    OnlyMissing,
}

/// Write the TUI-C4a table into an Opaline theme as tokens *and* composed
/// styles, keeping the TUI-C5 pipeline intact rather than short-circuiting it.
fn apply_guide_slots(theme: &mut opaline::Theme, mode: Override) {
    // A palette without a hover colour gets one of its own: halfway from
    // its canvas to its selection, so it reads as quieter than the cursor
    // in a light palette as in a dark one.
    let hover = Slot::BgHover.token_name();
    if mode == Override::OnlyMissing
        && !theme.has_token(hover)
        && let (Some(a), Some(b)) = (theme.try_color(Slot::BgBase.token_name()), theme.try_color(Slot::BgSelection.token_name()))
    {
        let mid = |x: u8, y: u8| ((u16::from(x) + u16::from(y)) / 2) as u8;
        let color = OpalineColor::new(mid(a.r, b.r), mid(a.g, b.g), mid(a.b, b.b));
        theme.register_token(hover, color);
        theme.register_style(hover, OpalineStyle::fg(color));
    }
    for slot in Slot::ALL {
        let (r, g, b) = slot.truecolor();
        let color = OpalineColor::new(r, g, b);
        let mut style = OpalineStyle::fg(color);
        if slot.modifiers().contains(Modifier::BOLD) {
            style = style.bold();
        }
        match mode {
            Override::Always => {
                theme.register_token(slot.token_name(), color);
                theme.register_style(slot.token_name(), style);
            }
            Override::OnlyMissing => {
                theme.register_default_token(slot.token_name(), color);
                theme.register_default_style(slot.token_name(), style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(slot: Slot) -> Color {
        let (r, g, b) = slot.truecolor();
        Color::Rgb(r, g, b)
    }

    #[test]
    fn every_guide_slot_resolves_to_its_truecolor_value() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        for slot in Slot::ALL {
            assert_eq!(
                theme.color(slot),
                Some(rgb(slot)),
                "slot {} did not resolve to its TUI-C4a value",
                slot.token_name()
            );
            assert_eq!(
                theme.style(slot).fg,
                Some(rgb(slot)),
                "slot {} style fg",
                slot.token_name()
            );
        }
    }

    #[test]
    fn every_guide_slot_is_present_in_the_wrapped_opaline_theme() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        for slot in Slot::ALL {
            assert!(
                theme.opaline().has_token(slot.token_name()),
                "opaline theme is missing token {}",
                slot.token_name()
            );
            assert!(
                theme.opaline().has_style(slot.token_name()),
                "opaline theme is missing style {}",
                slot.token_name()
            );
        }
    }

    #[test]
    fn text_emphasis_is_bold_at_every_tier() {
        for tier in [Tier::Mono, Tier::Ansi16, Tier::Ansi256, Tier::TrueColor] {
            let theme = DesignTheme::with_tier(tier);
            assert!(
                theme
                    .style(Slot::TextEmphasis)
                    .add_modifier
                    .contains(Modifier::BOLD),
                "text.emphasis lost its bold at {tier:?}"
            );
        }
    }

    #[test]
    fn ansi256_uses_the_guide_fallback_column() {
        let theme = DesignTheme::with_tier(Tier::Ansi256);
        assert_eq!(theme.color(Slot::BgBase), Some(Color::Indexed(232)));
        assert_eq!(theme.color(Slot::AccentPrimary), Some(Color::Indexed(171)));
        assert_eq!(theme.color(Slot::StatusError), Some(Color::Indexed(203)));
        for slot in Slot::ALL {
            assert_eq!(
                theme.color(slot),
                Some(Color::Indexed(slot.ansi256())),
                "{}",
                slot.token_name()
            );
        }
    }

    #[test]
    fn ansi16_uses_named_colors_only() {
        let theme = DesignTheme::with_tier(Tier::Ansi16);
        for slot in Slot::ALL {
            let color = theme.color(slot).expect("ansi16 tier is colored");
            assert_eq!(color, slot.ansi16(), "{}", slot.token_name());
            assert!(
                !matches!(color, Color::Rgb(..) | Color::Indexed(_)),
                "{} resolved to a non-named color at the 16-color tier",
                slot.token_name()
            );
        }
    }

    #[test]
    fn mono_renders_zero_color() {
        let theme = DesignTheme::with_tier(Tier::Mono);
        for slot in Slot::ALL {
            assert_eq!(theme.color(slot), None, "{}", slot.token_name());

            let style = theme.style(slot);
            assert_eq!(style.fg, None, "{} fg", slot.token_name());
            assert_eq!(style.bg, None, "{} bg", slot.token_name());

            let bg = theme.bg_style(slot);
            assert_eq!(bg.fg, None, "{} bg_style fg", slot.token_name());
            assert_eq!(bg.bg, None, "{} bg_style bg", slot.token_name());

            let on = theme.on(slot, Slot::BgSelection);
            assert_eq!(on.fg, None, "{} on fg", slot.token_name());
            assert_eq!(on.bg, None, "{} on bg", slot.token_name());
        }
    }

    #[test]
    fn degradation_narrows_as_the_tier_drops() {
        let slot = Slot::AccentPrimary;
        assert!(matches!(
            DesignTheme::with_tier(Tier::TrueColor).color(slot),
            Some(Color::Rgb(0xe1, 0x35, 0xff))
        ));
        assert!(matches!(
            DesignTheme::with_tier(Tier::Ansi256).color(slot),
            Some(Color::Indexed(171))
        ));
        assert_eq!(
            DesignTheme::with_tier(Tier::Ansi16).color(slot),
            Some(Color::LightMagenta)
        );
        assert_eq!(DesignTheme::with_tier(Tier::Mono).color(slot), None);
    }

    #[test]
    fn on_composes_foreground_over_background() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        let style = theme.on(Slot::TextEmphasis, Slot::BgSelection);
        assert_eq!(style.fg, Some(rgb(Slot::TextEmphasis)));
        assert_eq!(style.bg, Some(rgb(Slot::BgSelection)));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn domain_slots_register_and_resolve_per_tier() {
        let mut theme = DesignTheme::with_tier(Tier::TrueColor);
        assert_eq!(theme.domain_style("session.live"), None);

        theme.register_domain_slot(
            "session.live",
            SlotValue::new((0x50, 0xfa, 0x7b), 84, Color::LightGreen)
                .with_modifiers(Modifier::BOLD),
        );
        assert!(theme.has_domain_slot("session.live"));
        assert_eq!(theme.domain_slot_names(), vec!["session.live"]);

        let style = theme.domain_style("session.live").expect("registered");
        assert_eq!(style.fg, Some(Color::Rgb(0x50, 0xfa, 0x7b)));
        assert!(style.add_modifier.contains(Modifier::BOLD));

        theme.set_tier(Tier::Ansi256);
        assert_eq!(
            theme.domain_style("session.live").and_then(|s| s.fg),
            Some(Color::Indexed(84))
        );

        theme.set_tier(Tier::Mono);
        let mono = theme
            .domain_style("session.live")
            .expect("still registered");
        assert_eq!(mono.fg, None);
        assert!(mono.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn style_named_covers_guide_and_domain_slots() {
        let mut theme = DesignTheme::with_tier(Tier::TrueColor);
        assert_eq!(
            theme.style_named("accent.primary"),
            Some(theme.style(Slot::AccentPrimary))
        );
        assert_eq!(theme.style_named("bind.down"), None);
        theme.register_domain_slot(
            "bind.down",
            SlotValue::new((0xff, 0x63, 0x63), 203, Color::LightRed),
        );
        assert_eq!(
            theme.style_named("bind.down").and_then(|s| s.fg),
            Some(Color::Rgb(0xff, 0x63, 0x63))
        );
    }

    #[test]
    fn a_variant_theme_keeps_its_own_values_and_gains_the_missing_slots() {
        let soft = DesignTheme::from_builtin("silkcircuit-soft", Tier::TrueColor)
            .expect("silkcircuit-soft ships builtin");
        // The variant defines bg.base itself, so it must NOT be Neon's value.
        assert_ne!(soft.color(Slot::BgBase), Some(rgb(Slot::BgBase)));
        // It does not define status.error, so TUI-C4a fills that gap.
        assert_eq!(soft.color(Slot::StatusError), Some(rgb(Slot::StatusError)));
        // Every slot still resolves — nothing half-resolves (TUI-C5).
        for slot in Slot::ALL {
            assert!(soft.color(slot).is_some(), "{}", slot.token_name());
        }
        assert!(DesignTheme::from_builtin("not-a-theme", Tier::Mono).is_none());
    }

    #[test]
    fn all_five_silkcircuit_variants_ship_builtin() {
        for name in [
            "silkcircuit-neon",
            "silkcircuit-vibrant",
            "silkcircuit-soft",
            "silkcircuit-glow",
            "silkcircuit-dawn",
        ] {
            assert!(
                opaline::builtins::load_by_name(name).is_some(),
                "{name} is not a builtin"
            );
        }
    }

    #[test]
    fn discrepancy_report_matches_what_the_overrides_actually_change() {
        let builtin = opaline::builtins::silkcircuit_neon();
        let report = silkcircuit_neon_discrepancies();

        for row in &report {
            match row.conflict {
                SlotConflict::Missing => assert!(
                    !builtin.has_token(row.slot.token_name()),
                    "{} reported missing but the builtin defines it",
                    row.slot.token_name()
                ),
                SlotConflict::Differs { opaline } => {
                    assert_ne!(opaline, row.guide);
                    let actual = builtin
                        .try_color(row.slot.token_name())
                        .expect("reported as differing, so it exists");
                    assert_eq!((actual.r, actual.g, actual.b), opaline);
                }
            }
        }

        // Slots absent from the report are the ones Opaline already got right.
        for slot in Slot::ALL {
            if report.iter().any(|r| r.slot == slot) {
                continue;
            }
            let actual = builtin
                .try_color(slot.token_name())
                .expect("agreeing slots must exist");
            assert_eq!((actual.r, actual.g, actual.b), slot.truecolor());
        }

        // The four slots Opaline names identically and values identically.
        let agreed: Vec<&str> = Slot::ALL
            .into_iter()
            .filter(|s| !report.iter().any(|r| r.slot == *s))
            .map(Slot::token_name)
            .collect();
        assert_eq!(
            agreed,
            vec![
                "text.primary",
                "accent.primary",
                "accent.secondary",
                "accent.tertiary"
            ]
        );
    }
}
