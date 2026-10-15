//! The kit's recurring lines as view text, so every part words and marks
//! them alike without a terminal: headings, labelled values, the `+ add`
//! row, a check's verdict, a failed load. A frontend paints them like any
//! other [`Text`].

use super::{Ink, Seg, Text};
use crate::ui::icon::Icon;

/// `DESCRIPTION`: a small heading inside a panel, in muted capitals.
pub fn heading(text: &str) -> Text {
    Text::new().push(Seg::new(text.to_uppercase(), Ink::Muted).strong())
}

/// `label` in a muted column `width` cells wide, then `value`.
pub fn labelled(label: &str, width: usize, value: Text) -> Text {
    let mut line = Text::of(crate::ui::pad(label, width), Ink::Muted);
    line.extend(value);
    line
}

/// The `+ …` row that starts adding: an accent `+`, the action, and what
/// it takes when that fits in `width`.
pub fn add_line(action: &str, takes: &str, width: u16) -> Text {
    let mut line = Text::new()
        .push(Seg::new(" + ", Ink::Accent).strong())
        .push(Seg::new(action, Ink::AccentSecondary).strong());
    let hint = format!("   {takes}");
    if 3 + crate::ui::width(action) + crate::ui::width(&hint) + 2 <= usize::from(width) {
        line.add(Seg::new(hint, Ink::Muted));
    }
    line
}

/// `✓ said` or `⚠ said`: the icon where the pack has one, else the symbol.
pub fn check(good: bool, said: &str) -> Text {
    let (icon, symbol, ink) = if good {
        (Icon::RunPassed, "✓", Ink::Good)
    } else {
        (Icon::Warning, "⚠", Ink::Warn)
    };
    Text::new()
        .push(Seg::icon_or(icon, symbol, ink))
        .push(Seg::new(format!(" {said}"), ink))
}

/// The verdict on what is being typed: `✓ …` taken, `⚠ …` taken but may
/// not work, `✗ …` refused.
pub fn as_you_type(verdict: Result<&str, &str>) -> Text {
    match verdict {
        Ok(m) => Text::of(format!("✓ {m}"), Ink::Good),
        Err(m) => Text::of(format!("✗ {m}"), Ink::Warn),
    }
}

pub fn as_you_type_unsure(why: &str) -> Text {
    Text::of(format!("⚠ {why}"), Ink::Warn)
}

/// `loading…`
pub fn loading() -> Text {
    Text::of("loading…", Ink::Muted)
}

/// `✗ words`, with the error icon when icons are on.
pub fn error(words: &str) -> Text {
    Text::new()
        .push(Seg::icon_or(Icon::Error, "✗", Ink::Bad))
        .push(Seg::new(format!(" {words}"), Ink::Bad))
}

/// `✗ could not load: <why>`.
pub fn failed(why: &str) -> Text {
    Text::new()
        .push(Seg::icon_or(Icon::Error, "✗", Ink::Bad))
        .push(Seg::new(" could not load: ", Ink::Bad))
        .push(Seg::new(why, Ink::Bad).strong())
}

/// `✗ <source> could not sync: <why>`.
pub fn sync_failed(source: &str, why: &str) -> Text {
    Text::new()
        .push(Seg::icon_or(Icon::Error, "✗", Ink::Bad))
        .push(Seg::new(format!(" {source} could not sync: "), Ink::Bad))
        .push(Seg::new(why, Ink::Bad).strong())
}

/// The line for something fetched: what went wrong, or `loading…` while
/// there is nothing to show yet; nothing once there is.
pub fn fetching(loading: bool, shown: bool, error: Option<&str>) -> Option<Text> {
    if let Some(e) = error {
        return Some(failed(e));
    }
    (loading && !shown).then(self::loading)
}
