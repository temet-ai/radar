//! radar's own colour and style types: what a theme slot resolves to.
//!
//! These name no terminal or renderer library. A renderer turns them into its
//! own: `ipc/look.rs` writes them as the colour words Ink takes.

use std::ops::{BitOr, BitOrAssign};

/// A colour: the terminal's own (`Reset`), one of the 16 named ANSI colours,
/// an xterm-256 index, or 24-bit RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Color {
    #[default]
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    /// ANSI white (the light grey one).
    Gray,
    /// ANSI bright black.
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    /// ANSI bright white.
    White,
    Rgb(u8, u8, u8),
    Indexed(u8),
}

/// Text attributes, as a set of flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifier(u16);

impl Modifier {
    pub const BOLD: Self = Self(1 << 0);
    pub const DIM: Self = Self(1 << 1);
    pub const ITALIC: Self = Self(1 << 2);
    pub const UNDERLINED: Self = Self(1 << 3);
    pub const SLOW_BLINK: Self = Self(1 << 4);
    pub const RAPID_BLINK: Self = Self(1 << 5);
    pub const REVERSED: Self = Self(1 << 6);
    pub const HIDDEN: Self = Self(1 << 7);
    pub const CROSSED_OUT: Self = Self(1 << 8);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn from_bits_retain(bits: u16) -> Self {
        Self(bits)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Every flag of `other` is set in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}

impl BitOr for Modifier {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl BitOrAssign for Modifier {
    fn bitor_assign(&mut self, rhs: Self) {
        self.insert(rhs);
    }
}

/// Foreground, background and the modifiers added and removed. `None` leaves
/// what is underneath alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub add_modifier: Modifier,
    pub sub_modifier: Modifier,
}

impl Style {
    pub const fn new() -> Self {
        Self {
            fg: None,
            bg: None,
            add_modifier: Modifier::empty(),
            sub_modifier: Modifier::empty(),
        }
    }

    #[must_use]
    pub const fn fg(mut self, c: Color) -> Self {
        self.fg = Some(c);
        self
    }

    #[must_use]
    pub const fn bg(mut self, c: Color) -> Self {
        self.bg = Some(c);
        self
    }

    #[must_use]
    pub const fn add_modifier(mut self, m: Modifier) -> Self {
        self.sub_modifier = self.sub_modifier.difference(m);
        self.add_modifier = self.add_modifier.union(m);
        self
    }

    #[must_use]
    pub const fn remove_modifier(mut self, m: Modifier) -> Self {
        self.add_modifier = self.add_modifier.difference(m);
        self.sub_modifier = self.sub_modifier.union(m);
        self
    }

    /// `other` laid over `self`: its colours win where set, its modifiers
    /// are applied after ours.
    #[must_use]
    pub const fn patch(mut self, other: Self) -> Self {
        if other.fg.is_some() {
            self.fg = other.fg;
        }
        if other.bg.is_some() {
            self.bg = other.bg;
        }
        self.add_modifier = self
            .add_modifier
            .difference(other.sub_modifier)
            .union(other.add_modifier);
        self.sub_modifier = self
            .sub_modifier
            .difference(other.add_modifier)
            .union(other.sub_modifier);
        self
    }
}
