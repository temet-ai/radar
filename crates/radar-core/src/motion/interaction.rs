//! Pure composition for explicit tree folds and keyboard-owner transfers.
//!
//! The caller supplies a fresh clean buffer for every invocation. Identity,
//! input causality, effective motion level, elapsed time, retention and lifecycle
//! belong to the engine. These helpers never read the clock or keep state.

use ratatui::buffer::{Buffer, Cell, CellWidth};
use ratatui::layout::{Position, Rect};
use ratatui::style::Color;

use crate::design::theme::{Slot, Tier};
use crate::theme::{Theme, mix};
use crate::ui::frame::perimeter;

pub const FOLD_MS: u32 = 160;
pub const FOCUS_MS: u32 = 300;
/// A safety limit for an animation viewport, not for terminal dimensions.
pub const MAX_VIEWPORT_CELLS: usize = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldDirection {
    Open,
    Close,
}

/// Two row-major, equally sized snapshots of the affected viewport suffix.
/// `area` starts immediately below the unchanged parent anchor and excludes all
/// headers, notes, footers, titles and scrollbar columns. Both snapshots include
/// trailing blank cells/rows, so moving rows never reveals stale clean content.
/// Only the expanded snapshot's first `descendants` rows are descendants.
/// Descendants beyond this clipped suffix are never retained or rendered.
#[derive(Clone, Copy)]
pub struct FoldRows<'a> {
    pub area: Rect,
    pub expanded: &'a [Cell],
    pub compact: &'a [Cell],
    pub descendants: usize,
    pub direction: FoldDirection,
}

/// Integer, top-down unroll; close removes bottom-most descendants first.
/// The caller caps `descendants` to its visible suffix before calling this.
pub fn fold_visible(descendants: usize, elapsed_ms: u32, direction: FoldDirection) -> usize {
    let stepped =
        ((descendants as u128 * elapsed_ms.min(FOLD_MS) as u128) / u128::from(FOLD_MS)) as usize;
    match direction {
        FoldDirection::Open => stepped,
        FoldDirection::Close => descendants - stepped,
    }
}

/// Compose an active fold over clean cells, without touching the parent or any
/// cell outside `rows.area`. Returns false at the endpoint or for invalid input;
/// those cases leave the clean buffer entirely untouched.
///
/// Rectangles must already be clipped at the rendering boundary. Further
/// horizontal clipping here could cut a wide grapheme from its continuation,
/// and vertical clipping would change the causal parent anchor, so a changed
/// rectangle fails closed instead. Each source is bounded to one viewport.
pub fn compose_fold(buf: &mut Buffer, viewport: Rect, rows: FoldRows<'_>, elapsed_ms: u32) -> bool {
    if elapsed_ms >= FOLD_MS
        || !bounded(viewport)
        || !contained(viewport, buf.area)
        || !bounded(rows.area)
        || !contained(rows.area, viewport)
    {
        return false;
    }
    let width = usize::from(rows.area.width);
    let height = usize::from(rows.area.height);
    let count = width * height;
    let descendants = rows.descendants.min(height);
    if descendants == 0
        || rows.expanded.len() != count
        || rows.compact.len() != count
        || !whole_rows(rows.expanded, width)
        || !whole_rows(rows.compact, width)
    {
        return false;
    }
    let visible = fold_visible(descendants, elapsed_ms, rows.direction);
    for y in 0..height {
        let (source, row) = if y < visible {
            (rows.expanded, y)
        } else {
            (rows.compact, y - visible)
        };
        for x in 0..width {
            let pos = Position::new(rows.area.x + x as u16, rows.area.y + y as u16);
            buf[pos] = source[row * width + x].clone();
        }
    }
    true
}

fn whole_rows(cells: &[Cell], width: usize) -> bool {
    cells.iter().enumerate().all(|(i, cell)| {
        // Respect Ratatui's forced widths too. Continuation cells are cloned
        // verbatim; they are not reconstructed from strings or foregrounds.
        usize::from(cell.cell_width()) <= width - i % width
    })
}

/// Actual rendered frame geometry. `top_title` is protected automatically;
/// `keep` contains other protected regions, especially scrollbar cells and any
/// bottom title. Physical protected positions still count in the lap distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Border<'a> {
    pub area: Rect,
    pub top_title: Option<Rect>,
    pub keep: &'a [Rect],
}

impl Border<'_> {
    fn protected(self, pos: Position) -> bool {
        self.top_title.is_some_and(|title| title.contains(pos))
            || self.keep.iter().any(|region| region.contains(pos))
    }
}

/// Foregrounds in #31's physical clockwise perimeter order, including protected
/// cells. The engine owns this bounded baseline; no full-frame clone is needed.
/// Capture only a clean committed frame whose geometry/identity are established.
pub fn border_foregrounds(buf: &Buffer, border: Border<'_>) -> Option<Vec<Color>> {
    if !bounded(border.area) || !contained(border.area, buf.area) {
        return None;
    }
    Some(perimeter(border.area).map(|pos| buf[pos].fg).collect())
}

/// The title changes the animation origin, never the stationary #31 ramp.
/// With no top title, start at top-left. If the candidate is protected, continue
/// clockwise to the next drawable position; an entirely protected frame is idle.
pub fn lap_origin(border: Border<'_>) -> Option<usize> {
    if !bounded(border.area) {
        return None;
    }
    let count = perimeter(border.area).len();
    let after_title = border.top_title.and_then(|title| {
        perimeter(border.area)
            .enumerate()
            .filter(|(_, pos)| pos.y == border.area.y && title.contains(*pos))
            .map(|(i, _)| (i + 1) % count)
            .last()
    });
    let start = after_title.unwrap_or(0);
    perimeter(border.area)
        .enumerate()
        .filter(|(_, pos)| !border.protected(*pos))
        .min_by_key(|(i, _)| (i + count - start) % count)
        .map(|(i, _)| i)
}

/// The physical clockwise head index, relative to the lap origin. At 300 ms it
/// reaches the one-past-end index and the compositor leaves clean output alone.
pub fn lap_head(cells: usize, elapsed_ms: u32) -> usize {
    ((cells as u128 * u128::from(elapsed_ms.min(FOCUS_MS))) / u128::from(FOCUS_MS)) as usize
}

pub fn lap_width(cells: usize) -> usize {
    cells.min(7.max(cells.div_ceil(8)))
}

type Rgb = (u8, u8, u8);

/// Seven resolved status stops; there are no palette literals or fallbacks.
#[derive(Clone, Copy)]
pub struct FocusBand([Rgb; 7]);

impl FocusBand {
    pub fn from_theme(theme: &Theme) -> Option<Self> {
        if theme.tier() != Tier::TrueColor {
            return None;
        }
        Some(Self([
            rgb(theme.s(Slot::StatusError).fg?)?,
            rgb(theme.s(Slot::DataNumber).fg?)?,
            rgb(theme.s(Slot::StatusWarning).fg?)?,
            rgb(theme.s(Slot::StatusSuccess).fg?)?,
            rgb(theme.s(Slot::GlowCyan).fg?)?,
            rgb(theme.s(Slot::StatusInfo).fg?)?,
            rgb(theme.s(Slot::GlowPurple).fg?)?,
        ]))
    }

    /// Head (age zero) is red; cells age through orange to purple behind it.
    /// Quantize the six-segment position to integer per-mille exactly like
    /// #31's Ramp, then use its shared half-up theme mixer for each channel.
    pub fn at(self, age: usize, width: usize) -> Color {
        let last = width.saturating_sub(1);
        let phase = if last == 0 {
            0
        } else {
            ((age.min(last) as u128 * 6000) / last as u128) as u32
        };
        if phase == 6000 {
            let (r, g, b) = self.0[6];
            Color::Rgb(r, g, b)
        } else {
            let segment = (phase / 1000) as usize;
            mix(self.0[segment], self.0[segment + 1], phase % 1000)
        }
    }
}

/// One incoming lap over the exact current clean #31 ramp. Ahead of the head
/// the previous clean foreground is restored. The short trailing band uses the
/// seven theme stops; cells behind it remain byte-for-byte clean. The trail is
/// truncated at the start rather than wrapping onto unvisited positions.
///
/// `before` and `now` must match, including every protected region. Mismatch or
/// malformed prior foregrounds cancels the whole overlay before any cell changes.
pub fn focus_lap(
    buf: &mut Buffer,
    before: Border<'_>,
    now: Border<'_>,
    quiet: &[Color],
    band: FocusBand,
    elapsed_ms: u32,
) -> bool {
    if elapsed_ms >= FOCUS_MS || !matching_border(buf, before, now, quiet) {
        return false;
    }
    if perimeter(now.area)
        .enumerate()
        .any(|(i, pos)| !now.protected(pos) && rgb(quiet[i]).is_none())
    {
        return false;
    }
    let Some(origin) = lap_origin(now) else {
        return false;
    };
    let count = quiet.len();
    let head = lap_head(count, elapsed_ms);
    let width = lap_width(count);
    for (i, pos) in perimeter(now.area).enumerate() {
        if now.protected(pos) {
            continue;
        }
        let distance = (i + count - origin) % count;
        if distance > head {
            buf[pos].set_fg(quiet[i]);
        } else {
            let age = head - distance;
            if age < width {
                buf[pos].set_fg(band.at(age, width));
            }
        }
    }
    true
}

/// Drain the outgoing prior ramp toward each current clean border foreground.
/// This does not choose a new settled focus style; that belongs to the renderer.
/// Only foreground changes. Any geometry or RGB mismatch cancels atomically.
pub fn focus_drain(
    buf: &mut Buffer,
    before: Border<'_>,
    now: Border<'_>,
    prior: &[Color],
    elapsed_ms: u32,
) -> bool {
    if elapsed_ms >= FOCUS_MS || !matching_border(buf, before, now, prior) {
        return false;
    }
    let mut drawable = false;
    for (i, pos) in perimeter(now.area).enumerate() {
        if now.protected(pos) {
            continue;
        }
        drawable = true;
        if rgb(prior[i]).is_none() || rgb(buf[pos].fg).is_none() {
            return false;
        }
    }
    let per_mille = elapsed_ms * 1000 / FOCUS_MS;
    for (i, pos) in perimeter(now.area).enumerate() {
        if now.protected(pos) {
            continue;
        }
        if let (Some(from), Some(to)) = (rgb(prior[i]), rgb(buf[pos].fg)) {
            buf[pos].set_fg(mix(from, to, per_mille));
        }
    }
    drawable
}

fn matching_border(buf: &Buffer, before: Border<'_>, now: Border<'_>, prior: &[Color]) -> bool {
    before == now
        && bounded(now.area)
        && contained(now.area, buf.area)
        && prior.len() == perimeter(now.area).len()
}

fn rgb(color: Color) -> Option<Rgb> {
    match color {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        _ => None,
    }
}

fn bounded(area: Rect) -> bool {
    !area.is_empty()
        && usize::from(area.width) * usize::from(area.height) <= MAX_VIEWPORT_CELLS
        && area.right() - area.x == area.width
        && area.bottom() - area.y == area.height
}

fn contained(area: Rect, outer: Rect) -> bool {
    area.intersection(outer) == area
}

#[cfg(test)]
#[path = "interaction_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "interaction_parity.rs"]
mod parity;
