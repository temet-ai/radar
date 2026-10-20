//! The one text input every part of radar uses, so editing behaves the same
//! everywhere and like a normal line editor: the terminal's own cursor sits
//! where you type; ← → home end move; alt-← alt-→ (or ctrl-←/→) jump words;
//! backspace/delete; ctrl-u clears to the start, ctrl-k to the end, ctrl-w
//! deletes the word before the cursor, ctrl-a/ctrl-e jump to start/end;
//! ctrl-z undoes, ctrl-shift-z/ctrl-r redoes; pasting inserts the text.
//! Editing comes from `ratatui-textarea`; drawing is ours, on one row, with
//! the text scrolled so the cursor is always visible.

use std::cell::Cell;

use ratatui::buffer::Buffer;
use crate::view::{Chord, Key};
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui_textarea::{CursorMove, TextArea};
use crate::design::theme::Slot;

use crate::theme::Theme;

/// What a key did to the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// `⏎`: the text as typed.
    Submit(String),
    /// `esc`: give up.
    Cancel,
    /// Anything else: keep editing.
    Typing,
}

pub struct TextInput {
    area: TextArea<'static>,
}

impl std::fmt::Debug for TextInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TextInput").field(&self.value()).finish()
    }
}

impl TextInput {
    /// Start with `value`, cursor at the end.
    pub fn new(value: &str) -> Self {
        let mut area = TextArea::new(vec![value.to_string()]);
        area.move_cursor(CursorMove::End);
        Self { area }
    }

    pub fn value(&self) -> String {
        self.area.lines().first().cloned().unwrap_or_default()
    }

    /// The cursor's position in characters from the start.
    pub fn cursor(&self) -> usize {
        self.area.cursor().1
    }

    /// Put the cursor `at` characters from the start.
    pub fn set_cursor(&mut self, at: usize) {
        self.area
            .move_cursor(CursorMove::Jump(0, at.min(u16::MAX as usize) as u16));
    }

    /// The selected text as `(anchor, extent)`, counted in characters like
    /// [`TextInput::cursor`], which is the extent; `None` when nothing is
    /// selected.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let ((_, from), (_, to)) = self.area.selection_range()?;
        let caret = self.cursor();
        let anchor = if from == caret { to } else { from };
        (anchor != caret).then_some((anchor, caret))
    }

    /// Select from `anchor` to `extent` (the caret ends at `extent`); equal
    /// ends select nothing. Typing then replaces the selection and
    /// backspace removes it, as in any line editor.
    pub fn select(&mut self, anchor: usize, extent: usize) {
        self.area.cancel_selection();
        if anchor == extent {
            self.set_cursor(extent);
            return;
        }
        self.set_cursor(anchor);
        self.area.start_selection();
        self.set_cursor(extent);
    }

    /// Drop the selection, keeping the caret.
    pub fn unselect(&mut self) {
        self.area.cancel_selection();
    }

    /// A key from a view's intent ([`crate::view::Chord`]): as [`TextInput::key`].
    pub fn chord(&mut self, chord: crate::view::Chord) -> Edit {
        self.key(chord)
    }

    pub fn key(&mut self, key: impl crate::view::scroll::AsChord) -> Edit {
        let Some(key) = key.as_chord() else {
            return Edit::Typing;
        };
        let ctrl = key.ctrl;
        match key.key {
            Key::Enter => return Edit::Submit(self.value()),
            Key::Esc => return Edit::Cancel,
            // Readline, not the library's defaults (where ctrl-u is undo).
            Key::Char('u') if ctrl => {
                self.area.delete_line_by_head();
            }
            Key::Char('Z') if ctrl => {
                self.area.redo();
            }
            Key::Char('z') if ctrl => {
                self.area.undo();
            }
            Key::Char('r') if ctrl => {
                self.area.redo();
            }
            // One line only: keys that would start a new line do nothing.
            Key::Char('m' | 'j') if ctrl => {}
            _ => {
                self.area.input(crate::key_compat::key_of(key));
            }
        }
        Edit::Typing
    }

    /// Insert pasted text at the cursor; line breaks become spaces.
    pub fn paste(&mut self, text: &str) {
        let one_line: String = text
            .split(['\r', '\n'])
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        self.area.insert_str(one_line);
    }

    /// Draw on one row, scrolled to keep the cursor in view, and say where
    /// the terminal cursor goes.
    pub fn render(
        &self,
        theme: &Theme,
        area: Rect,
        buf: &mut Buffer,
        cursor: &Cell<Option<Position>>,
    ) {
        render_field(&self.value(), self.cursor(), theme, area, buf, cursor);
    }
}

/// Draw a one-line field holding `text` with the caret `caret` characters
/// in, scrolled to keep the caret in view, and say where the terminal
/// cursor goes. What [`TextInput::render`] draws, from view data (a menu's
/// [`crate::view::MenuView::typed`] and its caret).
pub fn render_field(
    text: &str,
    caret: usize,
    theme: &Theme,
    area: Rect,
    buf: &mut Buffer,
    cursor: &Cell<Option<Position>>,
) {
    {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let row = Rect { height: 1, ..area };
        let bg = theme.bg(Slot::BgOverlay);
        for x in row.x..row.x + row.width {
            buf[(x, row.y)].reset();
        }
        buf.set_style(row, bg);

        let chars: Vec<char> = text.chars().collect();
        let col = caret.min(chars.len());
        let width = row.width as usize;
        // Keep one cell for the cursor at the end of the text.
        let offset = (col + 1).saturating_sub(width);
        let visible: String = chars.iter().skip(offset).take(width).collect();
        buf.set_stringn(
            row.x,
            row.y,
            &visible,
            width,
            theme.bold(Slot::TextEmphasis).patch(bg),
        );

        let x = row.x + (col - offset) as u16;
        // A block cursor in the buffer too, for terminals and snapshots that
        // do not show the real one.
        buf[(x, row.y)].set_style(theme.s(Slot::TextEmphasis).add_modifier(Modifier::REVERSED));
        cursor.set(Some(Position { x, y: row.y }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: Key) -> Chord {
        Chord::new(code)
    }
    fn ctrl(c: char) -> Chord {
        Chord::ctrl(c)
    }

    #[test]
    fn typing_moving_and_submitting() {
        let mut i = TextInput::new("env:TOKEN");
        i.key(k(Key::Backspace));
        i.key(k(Key::Char('X')));
        assert_eq!(i.value(), "env:TOKEX");
        i.key(k(Key::Home));
        i.key(k(Key::Char('_')));
        assert_eq!(i.cursor(), 1);
        assert_eq!(i.key(k(Key::Enter)), Edit::Submit("_env:TOKEX".into()));
        assert_eq!(i.key(k(Key::Esc)), Edit::Cancel);
    }

    #[test]
    fn readline_keys_work() {
        let mut i = TextInput::new("tenants/old-group");
        i.key(ctrl('u'));
        assert_eq!(i.value(), "", "ctrl-u clears to the start");
        i.paste("https://example.com/acme/team-a/\n");
        assert_eq!(i.value(), "https://example.com/acme/team-a/");
        i.key(ctrl('w'));
        assert!(
            i.value().len() < "https://example.com/acme/team-a/".len(),
            "ctrl-w deletes a word"
        );
        i.key(ctrl('a'));
        assert_eq!(i.cursor(), 0);
        i.key(ctrl('k'));
        assert_eq!(i.value(), "", "ctrl-k clears to the end");
        i.key(ctrl('z'));
        assert!(!i.value().is_empty(), "ctrl-z undoes");
        i.key(ctrl('m'));
        assert_eq!(i.value().lines().count(), 1, "never more than one line");
    }

    #[test]
    fn a_selection_is_set_read_replaced_removed_and_undone() {
        let mut i = TextInput::new("hello world");
        assert_eq!(i.selection(), None);
        i.select(6, 11);
        assert_eq!(i.selection(), Some((6, 11)));
        assert_eq!(i.cursor(), 11);
        i.select(11, 6);
        assert_eq!(i.selection(), Some((11, 6)), "the caret ends at the extent");
        i.unselect();
        assert_eq!(i.selection(), None);
        assert_eq!(i.cursor(), 6, "unselecting keeps the caret");
        i.select(0, 5);
        i.key(k(Key::Char('J')));
        assert_eq!(i.value(), "J world", "typing replaces the selection");
        assert_eq!(i.selection(), None);
        i.select(0, 2);
        i.key(k(Key::Backspace));
        assert_eq!(i.value(), "world", "backspace removes it");
        i.key(ctrl('z'));
        assert_eq!(i.value(), "J world", "undo restores the removed text");
        i.select(2, 2);
        assert_eq!(i.selection(), None, "equal ends select nothing");
        i.select(1, 4);
        i.key(k(Key::Left));
        assert_eq!(i.selection(), None, "a plain movement key clears it");
    }

    #[test]
    fn long_text_scrolls_so_the_cursor_stays_visible() {
        let theme = Theme::new(crate::design::theme::Tier::Mono);
        let i = TextInput::new("abcdefghijklmnopqrstuvwxyz");
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        let cursor = Cell::new(None);
        i.render(&theme, Rect::new(0, 0, 10, 1), &mut buf, &cursor);
        let shown: String = (0..10).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert_eq!(shown, "rstuvwxyz ");
        assert_eq!(cursor.get(), Some(Position { x: 9, y: 0 }));
    }
}
