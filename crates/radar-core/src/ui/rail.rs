//! The step rail: the steps of a run along one line, two rows tall. Each
//! stop sits on a track at its own place: a check when done, `»` when
//! skipped, a hollow dot ahead, and the current stop on the accent (a
//! square chip, never a pill). Under each, a line says how it went (a
//! result, `skipped`, `step 2 of 5`). The track is lit up to the current
//! stop. Labels are cut to their stop's room in display cells.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::symbols::line;

use super::icon::{self, Icon};
use crate::design::theme::Slot;
use crate::theme::Theme;
use crate::view::chrome::StopState;

/// The rows a rail takes.
pub const HEIGHT: u16 = 2;

/// One stop of the rail.
#[derive(Debug, Clone, Copy)]
pub struct Stop<'a> {
    pub label: &'a str,
    /// The step's own icon, shown on the current stop.
    pub icon: Option<Icon>,
    pub state: StopState,
    /// The line under it.
    pub note: Option<&'a str>,
}

/// `text` cut to `w` cells with an ellipsis, never padded.
fn cut(text: &str, w: usize) -> String {
    if super::width(text) <= w { text.to_string() } else { super::fit(text, w).trim_end().to_string() }
}

/// The mark before a stop's label.
fn mark(theme: &Theme, stop: &Stop<'_>) -> (String, Style) {
    let (icon, fallback, slot) = match stop.state {
        StopState::Done => (Some(Icon::RunPassed), "✓", Slot::StatusSuccess),
        StopState::Skipped => (Some(Icon::RunSkipped), "»", Slot::TextMuted),
        StopState::Upcoming => (Some(Icon::Todo), "○", Slot::TextMuted),
        StopState::Current => (stop.icon, "", Slot::BgBase),
    };
    let glyph = icon.map(|i| icon::text(theme, i)).unwrap_or_default();
    let glyph = if glyph.is_empty() && !fallback.is_empty() { format!("{fallback} ") } else { glyph };
    (glyph, theme.s(slot))
}

/// Draw `stops` across `area` (its first two rows) and return each stop's
/// area, for clicks.
pub fn render(theme: &Theme, stops: &[Stop<'_>], area: Rect, buf: &mut Buffer) -> Vec<(Rect, usize)> {
    if stops.is_empty() || area.height == 0 || area.width == 0 {
        return vec![];
    }
    let n = stops.len() as u16;
    let slot = (area.width / n).max(1);
    let center = |i: u16| area.x + i * slot + slot / 2;
    let y = area.y;
    // The track, from the first stop to the last; lit up to the current
    // stop (or the last one done).
    let lit_to = stops
        .iter()
        .rposition(|s| s.state == StopState::Current)
        .or_else(|| stops.iter().rposition(|s| matches!(s.state, StopState::Done | StopState::Skipped)));
    let (from, to) = (center(0), center(n - 1).min(area.right().saturating_sub(1)));
    for x in from..=to {
        let lit = lit_to.is_some_and(|i| x <= center(i as u16));
        let style = if lit { theme.s(Slot::StatusSuccess) } else { theme.s(Slot::TextMuted) };
        buf.set_string(x, y, line::HORIZONTAL, style);
    }
    let mut hits = Vec::new();
    for (i, stop) in stops.iter().enumerate() {
        let i16 = i as u16;
        let x0 = area.x + i16 * slot;
        let room = if i16 + 1 == n { area.right().saturating_sub(x0) } else { slot };
        // A cell of ground on each side breaks the track around the label.
        let inner = room.saturating_sub(2) as usize;
        if inner == 0 {
            continue;
        }
        let (glyph, mark_style) = mark(theme, stop);
        let current = stop.state == StopState::Current;
        let words = cut(&format!("{glyph}{}", stop.label), inner.saturating_sub(usize::from(current) * 2));
        let label = if current { format!(" {words} ") } else { words };
        let w = super::width(&label) as u16;
        let c = center(i16).min(x0 + room.saturating_sub(1));
        let lx = c.saturating_sub(w / 2).max(x0 + 1).min(x0 + room.saturating_sub(w + 1));
        let ground = theme.bg(Slot::BgBase);
        buf.set_string(lx.saturating_sub(1), y, " ", ground);
        buf.set_string(lx + w, y, " ", ground);
        if current {
            let chip = theme
                .bold(Slot::BgBase)
                .patch(Style::new().bg(theme.s(Slot::AccentPrimary).fg.unwrap_or_default()));
            buf.set_string(lx, y, &label, chip);
        } else {
            let glyph_w = super::width(&glyph).min(super::width(&label)) as u16;
            let label_style = match stop.state {
                StopState::Done => theme.s(Slot::TextPrimary),
                _ => theme.s(Slot::TextMuted),
            };
            buf.set_string(lx, y, &label, label_style);
            if glyph_w > 0 {
                let cut: String = label.chars().take(glyph.chars().count()).collect();
                buf.set_string(lx, y, cut, mark_style);
            }
        }
        if let Some(note) = stop.note
            && area.height > 1
        {
            let note = cut(note, inner);
            let nw = super::width(&note) as u16;
            let nx = c.saturating_sub(nw / 2).max(x0 + 1).min(x0 + room.saturating_sub(nw + 1));
            let style = if current { theme.s(Slot::TextSecondary) } else { theme.s(Slot::TextMuted) };
            buf.set_string(nx, y + 1, note, style);
        }
        hits.push((Rect::new(x0, y, room, area.height.min(HEIGHT)), i));
    }
    hits
}
