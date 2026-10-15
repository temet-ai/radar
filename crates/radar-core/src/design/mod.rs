//! radar's design layer: the theme's named colour slots on Opaline palettes,
//! the terminal's colour depth, and the small widgets drawn with them (the
//! key hints of the status line, selected-row styles).
//!
//! First explored in the tui-kit sandbox; radar owns and changes it here.

pub mod help_bar;
pub mod select;
pub mod theme;

pub use help_bar::KeyHint;
pub use theme::{DesignTheme, Slot, SlotValue, TerminalEnv, Tier};
