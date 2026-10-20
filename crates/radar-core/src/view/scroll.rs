//! How far a panel is scrolled, and how far it can be: the state a panel
//! keeps while it is drawn through `&self`. Drawing it (a scroll view, the
//! scrollbar) is the painter's, in `ui::scroll`.

use std::cell::Cell;

use super::{Chord, Key};

/// What can be read as a [`Chord`]: a chord itself, or a renderer's own key
/// type through the painter that knows it. `Scroll::key` takes either.
pub trait AsChord {
    fn as_chord(&self) -> Option<Chord>;
}

impl AsChord for Chord {
    fn as_chord(&self) -> Option<Chord> {
        Some(*self)
    }
}

/// How far a panel is scrolled, and how far it can be: kept in `Cell`s so it
/// can be read and set while drawing (`Plugin::render` takes `&self`). The
/// room is only known after a frame was drawn.
#[derive(Debug, Default)]
pub struct Scroll {
    at: Cell<u16>,
    /// (visible rows, furthest offset), from the last frame.
    room: Cell<(u16, u16)>,
}

impl Scroll {
    pub fn new() -> Self {
        Self::default()
    }

    /// The first row shown.
    pub fn at(&self) -> u16 {
        self.at.get()
    }

    /// The furthest it can scroll, as of the last frame.
    pub fn max(&self) -> u16 {
        self.room.get().1
    }

    /// Rows visible, as of the last frame.
    pub fn height(&self) -> u16 {
        self.room.get().0
    }

    /// Whether there is more than fits.
    pub fn more(&self) -> bool {
        self.max() > 0
    }

    /// Back to the top: a new selection starts there.
    pub fn top(&self) {
        self.at.set(0);
    }

    pub fn set(&self, at: u16) {
        self.at.set(at);
    }

    /// Move by `rows`, within what the last frame allowed; whether it
    /// moved.
    pub fn by(&self, rows: i32) -> bool {
        let at = (i32::from(self.at()) + rows).clamp(0, i32::from(self.max()));
        let moved = at as u16 != self.at();
        self.at.set(at as u16);
        moved
    }

    /// A page down or up, keeping one row of the last page; whether it
    /// moved (a part reports `PgUp`/`PgDn` not handled when it did not, and
    /// the host says there is nothing to scroll).
    pub fn page(&self, down: bool) -> bool {
        let step = i32::from(self.height().saturating_sub(1).max(1));
        self.by(if down { step } else { -step })
    }

    /// Scrolls for `↑↓ j k PgUp PgDn g G Home End`; false for any other key,
    /// and for `↑` at the top (the host takes it up to the top bar).
    pub fn key(&self, key: &impl AsChord) -> bool {
        let Some(chord) = key.as_chord() else {
            return false;
        };
        match chord.key {
            Key::Down | Key::Char('j') => {
                self.by(1);
            }
            // At the top `↑` is not the panel's: the host takes it up to
            // the top bar.
            Key::Up | Key::Char('k') if self.at() == 0 => return false,
            Key::Up | Key::Char('k') => {
                self.by(-1);
            }
            Key::PageDown => return self.page(true),
            Key::PageUp => return self.page(false),
            Key::Home | Key::Char('g') => self.top(),
            Key::End | Key::Char('G') => self.at.set(self.max()),
            _ => return false,
        }
        true
    }

    /// What a renderer laid out: `rows` of the body show, the content took
    /// `content` rows, and the first shown is `at` (kept within them). How
    /// a part's scroll learns its room from whichever renderer draws
    /// ([`crate::Host::laid_out`]); no frame of the host's is needed.
    pub fn laid_out(&self, rows: u16, content: u16, at: u16) {
        let max = content.saturating_sub(rows);
        self.room.set((rows, max));
        self.at.set(at.min(max));
    }

    /// Record what this frame can show and keep the offset within it.
    pub(crate) fn fit(&self, height: u16, total: u16) -> u16 {
        let max = total.saturating_sub(height);
        let at = self.at().min(max);
        self.at.set(at);
        self.room.set((height, max));
        at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_scroll_within_the_room_the_last_frame_gave() {
        let s = Scroll::new();
        s.laid_out(10, 30, 0);
        assert_eq!((s.height(), s.max()), (10, 20));
        assert!(!s.key(&Chord::char('k')), "up at the top is the bar's");
        assert!(s.key(&Chord::char('j')));
        assert_eq!(s.at(), 1);
        assert!(s.key(&Chord::new(Key::PageDown)));
        assert_eq!(s.at(), 10, "a page keeps one row");
        assert!(s.key(&Chord::char('G')));
        assert_eq!(s.at(), 20);
        assert!(s.key(&Chord::new(Key::Home)));
        assert_eq!(s.at(), 0);
        assert!(!s.key(&Chord::char('z')));
    }
}
