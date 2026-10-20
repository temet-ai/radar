//! The screen-assemble moment: on a screen switch at `full`, the new screen
//! builds from its parts in the layout's order. Panels are found in the frame
//! just drawn (their rounded corners), left to right; each panel's frame
//! draws down the screen row by row, and each row inside reads in from the
//! left a moment after its frame row. Anything outside a panel (the sources
//! row) is there at once. Integer cell math, shared with Ink's
//! `motion/assemble.ts`, so both renderers draw the same cells.

use ratatui::buffer::Cell;
use ratatui::layout::Rect;

/// From the key to the finished screen.
pub const ASSEMBLE_MS: u32 = 450;
/// When the first panel's frame starts, and how far apart panels start.
const FRAME_START: u32 = 40;
const PANEL_GAP: u32 = 30;
const LAST_PANEL_START: u32 = 130;
/// A frame row draws at most this often, and a whole frame within `FRAME_SPAN`.
const FRAME_ROW: u32 = 8;
const FRAME_SPAN: u32 = 200;
/// A row inside starts this long after its frame row, and takes this long to read in.
const ROW_DELAY: u32 = 40;
const ROW_MS: u32 = 60;

const TOP_LEFT: [&str; 3] = ["╭", "┌", "┏"];
const TOP_RIGHT: [&str; 3] = ["╮", "┐", "┓"];
const BOTTOM_LEFT: [&str; 3] = ["╰", "└", "┗"];
const BOTTOM_RIGHT: [&str; 3] = ["╯", "┘", "┛"];
const SIDE: [&str; 4] = ["│", "┃", "├", "┣"];

/// The panels drawn inside `area`, by their corners, in the order they assemble (left to right, then down).
pub fn panels(cells: &[Cell], area: Rect) -> Vec<Rect> {
    let at = |x: u16, y: u16| cells[usize::from(y - area.y) * usize::from(area.width) + usize::from(x - area.x)].symbol();
    let mut found: Vec<Rect> = Vec::new();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if !TOP_LEFT.contains(&at(x, y)) || found.iter().any(|p| p.contains((x, y).into())) {
                continue;
            }
            let Some(right) = (x + 1..area.right()).find(|&r| TOP_RIGHT.contains(&at(r, y))) else {
                continue;
            };
            let mut bottom = None;
            for b in y + 1..area.bottom() {
                let s = at(x, b);
                if BOTTOM_LEFT.contains(&s) {
                    bottom = Some(b);
                    break;
                }
                if !SIDE.contains(&s) {
                    break;
                }
            }
            if let Some(bottom) = bottom.filter(|&b| BOTTOM_RIGHT.contains(&at(right, b))) {
                found.push(Rect::new(x, y, right - x + 1, bottom - y + 1));
            }
        }
    }
    found.sort_by_key(|p| (p.x, p.y));
    found
}

/// Whether the cell at `(x, y)` shows `elapsed` ms into the moment.
fn shows(panels: &[Rect], x: u16, y: u16, elapsed: u32) -> bool {
    let Some((i, p)) = panels.iter().enumerate().find(|(_, p)| p.contains((x, y).into())) else {
        return true;
    };
    let start = FRAME_START + (i as u32 * PANEL_GAP).min(LAST_PANEL_START - FRAME_START);
    let step = FRAME_ROW.min(FRAME_SPAN / u32::from(p.height.saturating_sub(1)).max(1));
    let row = u32::from(y - p.y);
    let framed = start + step * row;
    let edge = y == p.y || y == p.bottom() - 1 || x == p.x || x == p.right() - 1;
    if edge {
        return elapsed >= framed;
    }
    let from = framed + ROW_DELAY;
    if elapsed < from {
        return false;
    }
    let inner = u32::from(p.width - 2);
    let read = (inner * (elapsed - from).min(ROW_MS)).div_ceil(ROW_MS);
    u32::from(x - p.x - 1) < read
}

/// Hide what has not arrived yet: its cell keeps its ground and loses its glyph.
/// A wide glyph the reading edge would cut stays hidden until it is whole.
pub fn apply(cells: &mut [Cell], area: Rect, elapsed: u32) {
    if elapsed >= ASSEMBLE_MS || area.is_empty() {
        return;
    }
    let found = panels(cells, area);
    let width = usize::from(area.width);
    for y in area.y..area.bottom() {
        let row = usize::from(y - area.y) * width;
        let visible: Vec<bool> = (area.x..area.right()).map(|x| shows(&found, x, y, elapsed)).collect();
        for (i, &show) in visible.iter().enumerate() {
            let wide = crate::ui::width(cells[row + i].symbol()) > 1;
            let cut = wide && visible.get(i + 1).is_some_and(|next| !next);
            if !show || cut {
                let cell = &mut cells[row + i];
                cell.set_symbol(" ");
                cell.modifier = ratatui::style::Modifier::empty();
            }
        }
    }
}
