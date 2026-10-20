//! Motion: what happened, as data for the renderer to show as short effects.
//!
//! The host says what happened, as a [`Cue`]: a screen switched, an item's
//! page opened, a menu or confirm appeared, a message came or went, a part's
//! rows changed in a sync. The renderer (Ink) runs the effects, fitted to the
//! level the host sends ([`Level`], [`effective`]).
//!
//! Parts never draw motion themselves. A part reports what a sync changed with
//! `Effect::Changed(Changes)` (see [`Changes::between`]), keyed by the row's
//! subject (`kind:id`, [`key`]).

pub mod transition;

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use crate::design::theme::Tier;

/// How much motion: `[theme] motion = "full" | "subtle" | "off"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    /// Every effect.
    Full,
    /// Screen switches, menus and confirms, and the glow of synced rows.
    #[default]
    Subtle,
    /// None at all: radar draws as it does without motion.
    Off,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Full, Level::Subtle, Level::Off];

    /// The config word; anything unknown is the default, `subtle`.
    pub fn parse(word: &str) -> Self {
        match word.trim() {
            "full" => Self::Full,
            "off" | "none" => Self::Off,
            _ => Self::Subtle,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Subtle => "subtle",
            Self::Off => "off",
        }
    }

    /// What it shows, in a few words.
    pub fn about(self) -> &'static str {
        match self {
            Self::Full => "everything: also pages, new rows, messages and loading",
            Self::Subtle => "switching screens, menus, and rows a sync changed",
            Self::Off => "no motion at all",
        }
    }
}

/// Why motion is off here whatever the setting says: over SSH (every frame
/// crosses the network), or a terminal not drawing in full colour (the
/// effects blend colours, which only full colour shows as meant).
pub fn held_back(tier: Tier, ssh: bool) -> Option<&'static str> {
    if ssh {
        Some("over SSH")
    } else if tier != Tier::TrueColor {
        Some("the terminal is not drawing in full colour")
    } else {
        None
    }
}

/// The level that runs: the setting, unless [`held_back`].
pub fn effective(level: Level, tier: Tier, ssh: bool) -> Level {
    if held_back(tier, ssh).is_some() {
        Level::Off
    } else {
        level
    }
}

/// The rows a sync changed, by key: ones that were there and differ, and
/// ones that were not there. Keys are the row's subject, `kind:id`, so two
/// parts' keys never meet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub changed: BTreeSet<String>,
    pub added: BTreeSet<String>,
}

impl Changes {
    /// Compare rows before and after a sync. With nothing before (the first
    /// data), nothing is new: a list that fills is not a list that changed.
    pub fn between<T: PartialEq>(
        before: impl IntoIterator<Item = (String, T)>,
        after: impl IntoIterator<Item = (String, T)>,
    ) -> Self {
        let before: HashMap<String, T> = before.into_iter().collect();
        let mut changes = Self::default();
        if before.is_empty() {
            return changes;
        }
        for (key, now) in after {
            match before.get(&key) {
                Some(was) if *was == now => {}
                Some(_) => {
                    changes.changed.insert(key);
                }
                None => {
                    changes.added.insert(key);
                }
            }
        }
        changes
    }

    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty()
    }
}

/// What happened, for the renderer to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cue {
    /// Another screen (a tab's key, settings, a show across parts).
    Screen,
    /// An item's page opened on the same screen.
    Page,
    /// A menu or a confirm appeared.
    Popup,
    /// A message appeared on the status line.
    Message,
    /// The message left the status line.
    MessageGone,
    /// A part's sync changed these rows.
    Changed(Changes),
}

impl Cue {
    /// Whether it shows at `level`: `subtle` has screen switches, popups
    /// and the sync glow; `full` everything.
    pub fn runs_at(&self, level: Level) -> bool {
        match level {
            Level::Off => false,
            Level::Full => true,
            Level::Subtle => matches!(self, Self::Screen | Self::Popup | Self::Changed(_)),
        }
    }
}

/// The key a row goes by in [`Changes`]: its subject's kind and id,
/// `kind:id` (`shelf:A-12`), so two parts' keys never meet.
pub fn key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

/// How long a screen switch runs, in ms. It is the shortest effect: it
/// happens most, and the new screen's words read from its first frame.
pub const SCREEN_MS: u32 = 100;
/// How long Ink's effects run, in ms; the host sends them in
/// [`crate::ipc::MotionTimings`].
pub const PAGE_MS: u32 = 300;
pub const POPUP_MS: u32 = 180;
pub const MESSAGE_MS: u32 = 250;
pub const MESSAGE_GONE_MS: u32 = 200;
pub const GLOW_MS: u32 = 450;
pub const ADDED_MS: u32 = 320;
/// One pass of the loading shimmer; it repeats while the panel waits.
pub const SHIMMER_MS: u32 = 1400;
/// How often a frame is drawn while an effect runs.
pub const FRAME: Duration = Duration::from_millis(30);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_compare_rows_and_the_first_data_is_not_a_change() {
        let rows = |r: &[(&str, i32)]| -> Vec<(String, i32)> {
            r.iter().map(|(k, v)| (k.to_string(), *v)).collect()
        };
        assert!(Changes::between(rows(&[]), rows(&[("a", 1)])).is_empty());
        let c = Changes::between(
            rows(&[("a", 1), ("b", 2), ("gone", 0)]),
            rows(&[("a", 1), ("b", 3), ("c", 4)]),
        );
        assert_eq!(c.changed.iter().collect::<Vec<_>>(), ["b"]);
        assert_eq!(c.added.iter().collect::<Vec<_>>(), ["c"]);
    }

    #[test]
    fn motion_is_held_back_over_ssh_and_below_full_colour() {
        assert_eq!(effective(Level::Full, Tier::TrueColor, false), Level::Full);
        assert_eq!(effective(Level::Full, Tier::TrueColor, true), Level::Off);
        for tier in [Tier::Ansi256, Tier::Ansi16, Tier::Mono] {
            assert_eq!(effective(Level::Subtle, tier, false), Level::Off, "{tier:?}");
        }
        assert_eq!(Level::parse("full"), Level::Full);
        assert_eq!(Level::parse("off"), Level::Off);
        assert_eq!(Level::parse("whatever"), Level::Subtle);
        for l in Level::ALL {
            assert_eq!(Level::parse(l.word()), l);
        }
    }

    #[test]
    fn subtle_has_switches_popups_and_the_glow_only() {
        let mut changes = Changes::default();
        changes.changed.insert("row:1".into());
        let cues = [
            Cue::Screen,
            Cue::Page,
            Cue::Popup,
            Cue::Message,
            Cue::MessageGone,
            Cue::Changed(changes),
        ];
        for cue in cues {
            let shows = matches!(cue, Cue::Screen | Cue::Popup | Cue::Changed(_));
            assert_eq!(cue.runs_at(Level::Subtle), shows, "{cue:?} at subtle");
            assert!(cue.runs_at(Level::Full), "{cue:?} at full");
            assert!(!cue.runs_at(Level::Off), "{cue:?} at off");
        }
    }
}
