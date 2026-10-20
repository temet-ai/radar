//! Movement along selectable rows: where the cursor goes for a key, skipping
//! the rows that cannot be chosen (headings).

use crate::view::{Chord, Key};

/// A way the cursor moves in a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Down,
    Up,
    First,
    Last,
    /// By this many rows (a page).
    PageDown(usize),
    PageUp(usize),
}

impl Move {
    /// The move a key asks for: `↓ j`, `↑ k`, `Home g`, `End G`, and
    /// `PgDn`/`PgUp` by `page` rows (`Scroll::height`). `None` for any
    /// other key.
    pub fn from_key(key: Chord, page: u16) -> Option<Move> {
        let page = usize::from(page.saturating_sub(1).max(1));
        Some(match key.key {
            Key::Down | Key::Char('j') => Move::Down,
            Key::Up | Key::Char('k') => Move::Up,
            Key::Home | Key::Char('g') => Move::First,
            Key::End | Key::Char('G') => Move::Last,
            Key::PageDown => Move::PageDown(page),
            Key::PageUp => Move::PageUp(page),
            _ => return None,
        })
    }
}

/// The row after (`down`) or before `i` among `n`, staying at the ends.
pub fn step(i: usize, n: usize, down: bool) -> usize {
    if n == 0 {
        0
    } else if down {
        (i + 1).min(n - 1)
    } else {
        i.saturating_sub(1)
    }
}

/// The first row at or after `from` (going down, or up) that is
/// selectable, below `n`.
fn seek(n: usize, from: usize, down: bool, selectable: &impl Fn(usize) -> bool) -> Option<usize> {
    if down {
        (from..n).find(|&i| selectable(i))
    } else {
        (0..=from.min(n.saturating_sub(1))).rev().find(|&i| selectable(i))
    }
}

/// Where the cursor goes from `from` in a list of `n` rows for `mv`,
/// skipping rows that are not `selectable`. It stays at `from` when there
/// is nowhere to go that way, and a page that ends on a heading goes on to
/// the next row that can be selected (or back to the last one before it).
pub fn select(n: usize, from: usize, mv: Move, selectable: impl Fn(usize) -> bool) -> usize {
    if n == 0 {
        return 0;
    }
    let last = n - 1;
    let from = from.min(last);
    let (target, down) = match mv {
        Move::Down => (from + 1, true),
        Move::Up => (from.wrapping_sub(1), false),
        Move::First => (0, true),
        Move::Last => (last, false),
        Move::PageDown(k) => ((from + k).min(last), true),
        Move::PageUp(k) => (from.saturating_sub(k), false),
    };
    if target > last {
        // Up from the first row wraps to a huge number: nowhere to go.
        return from;
    }
    let found = seek(n, target, down, &selectable).or_else(|| match mv {
        // A page that ran out: the last row the other way that moves on.
        Move::PageDown(_) => seek(n, last, false, &selectable).filter(|&i| i > from),
        Move::PageUp(_) => seek(n, 0, true, &selectable).filter(|&i| i < from),
        _ => None,
    });
    found.unwrap_or(from)
}

/// `cursor` made one a list can rest on: itself when selectable, else the
/// next selectable row, else the nearest one before it.
pub fn settle(n: usize, cursor: usize, selectable: impl Fn(usize) -> bool) -> usize {
    if n == 0 {
        return 0;
    }
    let cursor = cursor.min(n - 1);
    if selectable(cursor) {
        return cursor;
    }
    seek(n, cursor, true, &selectable)
        .or_else(|| seek(n, cursor, false, &selectable))
        .unwrap_or(cursor)
}
