//! The one text input every part of radar uses, so editing behaves the same
//! everywhere and like a normal line editor: ← → home end move; alt-← alt-→
//! (or ctrl-←/→, alt-b/alt-f) jump words; backspace/delete; ctrl-u clears to
//! the start, ctrl-k to the end, ctrl-w deletes the word before the cursor,
//! ctrl-a/ctrl-e jump to start/end; shift with a movement key selects;
//! ctrl-z undoes, ctrl-shift-z/ctrl-r redoes; pasting inserts the text.
//!
//! This is radar's own one-line editor. It holds text, caret and selection;
//! Ink draws the field from the view data they feed (a menu's typed text and
//! caret), so nothing here draws.

use crate::view::Key;

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

/// One undo step: the text, the caret and the selection anchor before an edit.
#[derive(Clone, PartialEq, Eq)]
struct Snapshot {
    text: Vec<char>,
    caret: usize,
    anchor: Option<usize>,
}

/// A single line of text with a caret, an optional selection and undo/redo.
/// Positions count characters from the start.
pub struct TextInput {
    text: Vec<char>,
    caret: usize,
    /// The other end of the selection; the caret is the extent.
    anchor: Option<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl std::fmt::Debug for TextInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TextInput").field(&self.value()).finish()
    }
}

/// A word character: what a word jump or a word delete stops at.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A line has no line breaks: they become spaces.
fn one_line(text: &str) -> Vec<char> {
    text.chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect()
}

impl TextInput {
    /// Start with `value` exactly as given (a restored draft keeps its line
    /// breaks), cursor at the end. Typed keys and pastes still never add one.
    pub fn new(value: &str) -> Self {
        let text: Vec<char> = value.chars().collect();
        Self {
            caret: text.len(),
            text,
            anchor: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn value(&self) -> String {
        self.text.iter().collect()
    }

    /// The cursor's position in characters from the start.
    pub fn cursor(&self) -> usize {
        self.caret
    }

    /// Put the cursor `at` characters from the start (clamped to the text);
    /// any selection is dropped.
    pub fn set_cursor(&mut self, at: usize) {
        self.caret = at.min(self.text.len());
        self.anchor = None;
    }

    /// The selected text as `(anchor, extent)`, counted in characters like
    /// [`TextInput::cursor`], which is the extent; `None` when nothing is
    /// selected.
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.anchor
            .filter(|a| *a != self.caret)
            .map(|a| (a, self.caret))
    }

    /// Select from `anchor` to `extent` (the caret ends at `extent`); equal
    /// ends select nothing. Typing then replaces the selection and
    /// backspace removes it, as in any line editor.
    pub fn select(&mut self, anchor: usize, extent: usize) {
        let len = self.text.len();
        let (anchor, extent) = (anchor.min(len), extent.min(len));
        self.caret = extent;
        self.anchor = (anchor != extent).then_some(anchor);
    }

    /// Drop the selection, keeping the caret.
    pub fn unselect(&mut self) {
        self.anchor = None;
    }

    /// A key from a view's intent ([`crate::view::Chord`]): as [`TextInput::key`].
    pub fn chord(&mut self, chord: crate::view::Chord) -> Edit {
        self.key(chord)
    }

    pub fn key(&mut self, key: impl crate::view::scroll::AsChord) -> Edit {
        let Some(key) = key.as_chord() else {
            return Edit::Typing;
        };
        let (ctrl, alt) = (key.ctrl, key.alt);
        // Shift on a movement key extends the selection; a capital letter is
        // already a different char.
        let shift = key.shift || matches!(key.key, Key::Char(c) if ctrl && c.is_uppercase());
        match key.key {
            Key::Enter => return Edit::Submit(self.value()),
            Key::Esc => return Edit::Cancel,
            Key::Left if ctrl || alt => self.travel(shift, Self::word_start),
            Key::Right if ctrl || alt => self.travel(shift, Self::word_end),
            Key::Left => self.travel(shift, |s| s.caret.saturating_sub(1)),
            Key::Right => self.travel(shift, |s| (s.caret + 1).min(s.text.len())),
            Key::Home => self.travel(shift, |_| 0),
            Key::End => self.travel(shift, |s| s.text.len()),
            Key::Backspace if alt => self.delete_to(Self::word_start),
            Key::Backspace => self.delete_to(|s| s.caret.saturating_sub(1)),
            Key::Delete => self.delete_to(|s| (s.caret + 1).min(s.text.len())),
            Key::Char(c) if ctrl => match c.to_ascii_lowercase() {
                'z' if shift => self.redo(),
                'z' => self.undo(),
                'r' => self.redo(),
                'a' => self.travel(false, |_| 0),
                'e' => self.travel(false, |s| s.text.len()),
                'b' => self.travel(false, |s| s.caret.saturating_sub(1)),
                'f' => self.travel(false, |s| (s.caret + 1).min(s.text.len())),
                'u' => self.delete_to(|_| 0),
                'k' => self.delete_to(|s| s.text.len()),
                'w' => self.delete_to(Self::word_start),
                'h' => self.delete_to(|s| s.caret.saturating_sub(1)),
                'd' => self.delete_to(|s| (s.caret + 1).min(s.text.len())),
                // One line only, and no other control key types anything.
                _ => {}
            },
            Key::Char('b') if alt => self.travel(false, Self::word_start),
            Key::Char('f') if alt => self.travel(false, Self::word_end),
            Key::Char('d') if alt => self.delete_to(Self::word_end),
            Key::Char(_) if alt => {}
            Key::Char(c) => self.insert_str(&c.to_string()),
            _ => {}
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
        self.insert_str(&one_line);
    }

    /// Insert `text` at the caret, replacing the selection if there is one.
    pub fn insert_str(&mut self, text: &str) {
        let chars = one_line(text);
        if chars.is_empty() && self.selection().is_none() {
            return;
        }
        self.record();
        self.remove_selected();
        let at = self.caret;
        self.caret += chars.len();
        self.text.splice(at..at, chars);
    }

    // --- movement ---------------------------------------------------------

    /// Move the caret to where `to` says; with `select` the selection grows
    /// from where it was, otherwise it is dropped.
    fn travel(&mut self, select: bool, to: impl Fn(&Self) -> usize) {
        let target = to(self);
        if select {
            self.anchor.get_or_insert(self.caret);
        } else {
            self.anchor = None;
        }
        self.caret = target;
        if self.anchor == Some(self.caret) {
            self.anchor = None;
        }
    }

    /// The start of the word before the caret.
    fn word_start(&self) -> usize {
        let mut i = self.caret;
        while i > 0 && !is_word(self.text[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(self.text[i - 1]) {
            i -= 1;
        }
        i
    }

    /// The end of the word after the caret.
    fn word_end(&self) -> usize {
        let len = self.text.len();
        let mut i = self.caret;
        while i < len && !is_word(self.text[i]) {
            i += 1;
        }
        while i < len && is_word(self.text[i]) {
            i += 1;
        }
        i
    }

    // --- deleting and history ----------------------------------------------

    /// Delete between the caret and where `to` says (or the selection, if
    /// there is one). Nothing to delete records nothing.
    fn delete_to(&mut self, to: impl Fn(&Self) -> usize) {
        if self.selection().is_some() {
            self.record();
            self.remove_selected();
            return;
        }
        let target = to(self);
        let (from, upto) = (self.caret.min(target), self.caret.max(target));
        if from == upto {
            return;
        }
        self.record();
        self.text.drain(from..upto);
        self.caret = from;
        self.anchor = None;
    }

    fn remove_selected(&mut self) {
        if let Some((a, b)) = self.selection() {
            let (from, to) = (a.min(b), a.max(b));
            self.text.drain(from..to);
            self.caret = from;
        }
        self.anchor = None;
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            caret: self.caret,
            anchor: self.anchor,
        }
    }

    /// Remember the state before an edit; a new edit forgets what was undone.
    fn record(&mut self) {
        self.undo.push(self.snapshot());
        self.redo.clear();
    }

    fn restore(&mut self, s: Snapshot) {
        self.text = s.text;
        self.caret = s.caret;
        self.anchor = s.anchor;
    }

    fn undo(&mut self) {
        if let Some(before) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(before);
            self.anchor = None;
        }
    }

    fn redo(&mut self) {
        if let Some(after) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(after);
            self.anchor = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Chord;

    fn k(code: Key) -> Chord {
        Chord::new(code)
    }
    fn ctrl(c: char) -> Chord {
        Chord::ctrl(c)
    }
    fn alt(key: Key) -> Chord {
        Chord {
            alt: true,
            ..Chord::new(key)
        }
    }
    fn shift(key: Key) -> Chord {
        Chord {
            shift: true,
            ..Chord::new(key)
        }
    }
    fn type_in(i: &mut TextInput, text: &str) {
        for c in text.chars() {
            i.key(k(Key::Char(c)));
        }
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
    fn a_new_input_has_its_cursor_at_the_end() {
        let i = TextInput::new("Sam Vimes");
        assert_eq!(i.cursor(), 9);
        assert_eq!(i.selection(), None);
    }

    #[test]
    fn a_restored_value_keeps_its_line_breaks_but_typing_and_paste_add_none() {
        let mut i = TextInput::new("  Sam\nVimes  ");
        assert_eq!(i.value(), "  Sam\nVimes  ");
        assert_eq!(i.cursor(), 13);
        assert_eq!(i.key(k(Key::Enter)), Edit::Submit("  Sam\nVimes  ".into()));
        assert_eq!(i.value(), "  Sam\nVimes  ", "enter inserts nothing");
        i.paste("a\nb");
        assert_eq!(i.value(), "  Sam\nVimes  a b");
    }

    #[test]
    fn arrows_home_end_and_delete_move_and_remove() {
        let mut i = TextInput::new("vimes");
        i.key(k(Key::Left));
        i.key(k(Key::Left));
        assert_eq!(i.cursor(), 3);
        i.key(k(Key::Delete));
        assert_eq!(i.value(), "vims");
        i.key(k(Key::Right));
        i.key(k(Key::Right));
        i.key(k(Key::Right));
        assert_eq!(i.cursor(), 4, "right stops at the end");
        i.key(k(Key::Delete));
        assert_eq!(i.value(), "vims", "delete at the end does nothing");
        i.key(k(Key::Home));
        i.key(k(Key::Left));
        assert_eq!(i.cursor(), 0, "left stops at the start");
        i.key(k(Key::Backspace));
        assert_eq!(i.value(), "vims", "backspace at the start does nothing");
        i.key(k(Key::End));
        assert_eq!(i.cursor(), 4);
    }

    #[test]
    fn editing_is_by_character_not_byte() {
        let mut i = TextInput::new("Ankh-Morpork ☕");
        i.key(k(Key::Backspace));
        assert_eq!(i.value(), "Ankh-Morpork ");
        type_in(&mut i, "é☕");
        assert_eq!(i.cursor(), 15);
        i.key(k(Key::Left));
        i.key(k(Key::Backspace));
        assert_eq!(i.value(), "Ankh-Morpork ☕");
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
    fn ctrl_u_k_w_a_e_cut_and_move_around_the_cursor() {
        let mut i = TextInput::new("night watch patrol");
        i.set_cursor(11);
        i.key(ctrl('u'));
        assert_eq!(
            i.value(),
            " patrol",
            "ctrl-u removes everything before the caret"
        );
        assert_eq!(i.cursor(), 0);
        i.key(ctrl('e'));
        assert_eq!(i.cursor(), 7);
        i.key(ctrl('w'));
        assert_eq!(i.value(), " ", "ctrl-w deletes the word before the caret");
        let mut j = TextInput::new("night watch");
        j.set_cursor(5);
        j.key(ctrl('k'));
        assert_eq!(
            j.value(),
            "night",
            "ctrl-k removes from the caret to the end"
        );
        j.key(ctrl('w'));
        assert_eq!(j.value(), "");
        j.key(ctrl('w'));
        assert_eq!(j.value(), "", "ctrl-w on an empty line does nothing");
    }

    #[test]
    fn ctrl_w_skips_spaces_then_takes_the_word() {
        let mut i = TextInput::new("one two   ");
        i.key(ctrl('w'));
        assert_eq!(i.value(), "one ");
    }

    #[test]
    fn word_jumps_with_alt_and_ctrl_arrows_and_alt_b_f() {
        let mut i = TextInput::new("ankh morpork watch");
        i.key(alt(Key::Left));
        assert_eq!(i.cursor(), 13);
        i.key(Chord {
            ctrl: true,
            ..Chord::new(Key::Left)
        });
        assert_eq!(i.cursor(), 5);
        i.key(alt(Key::Char('b')));
        assert_eq!(i.cursor(), 0);
        i.key(alt(Key::Char('f')));
        assert_eq!(i.cursor(), 4);
        i.key(alt(Key::Right));
        assert_eq!(i.cursor(), 12);
        i.key(Chord {
            ctrl: true,
            ..Chord::new(Key::Right)
        });
        assert_eq!(i.cursor(), 18);
        i.key(alt(Key::Right));
        assert_eq!(i.cursor(), 18, "a word jump stops at the end");
    }

    #[test]
    fn alt_d_and_alt_backspace_delete_a_word_either_way() {
        let mut i = TextInput::new("ankh morpork");
        i.set_cursor(0);
        i.key(alt(Key::Char('d')));
        assert_eq!(i.value(), " morpork");
        i.key(ctrl('e'));
        i.key(alt(Key::Backspace));
        assert_eq!(i.value(), " ");
    }

    #[test]
    fn ctrl_b_f_h_d_are_the_emacs_arrows_and_deletes() {
        let mut i = TextInput::new("abc");
        i.key(ctrl('b'));
        assert_eq!(i.cursor(), 2);
        i.key(ctrl('h'));
        assert_eq!(i.value(), "ac");
        i.key(ctrl('d'));
        assert_eq!(i.value(), "a");
        i.key(ctrl('f'));
        assert_eq!(i.cursor(), 1);
    }

    #[test]
    fn other_control_and_alt_keys_type_nothing() {
        let mut i = TextInput::new("ab");
        i.key(ctrl('q'));
        i.key(alt(Key::Char('x')));
        i.key(k(Key::Tab));
        i.key(k(Key::PageUp));
        assert_eq!(i.value(), "ab");
        assert_eq!(i.cursor(), 2);
        assert_eq!(i.key(ctrl('m')), Edit::Typing);
        assert_eq!(i.key(ctrl('j')), Edit::Typing);
        assert_eq!(i.value(), "ab");
    }

    #[test]
    fn undo_and_redo_walk_the_edits() {
        let mut i = TextInput::new("");
        type_in(&mut i, "ab");
        i.key(ctrl('z'));
        assert_eq!(i.value(), "a");
        i.key(ctrl('z'));
        assert_eq!(i.value(), "");
        i.key(ctrl('z'));
        assert_eq!(i.value(), "", "undo past the start does nothing");
        i.key(ctrl('r'));
        assert_eq!(i.value(), "a");
        i.key(Chord {
            shift: true,
            ..Chord::ctrl('z')
        });
        assert_eq!(i.value(), "ab", "ctrl-shift-z redoes too");
        i.key(ctrl('Z'));
        assert_eq!(i.value(), "ab", "nothing left to redo");
        i.key(ctrl('z'));
        type_in(&mut i, "x");
        i.key(ctrl('r'));
        assert_eq!(i.value(), "ax", "a new edit forgets what was undone");
    }

    #[test]
    fn undo_restores_the_caret_and_a_cut_line() {
        let mut i = TextInput::new("night watch");
        i.set_cursor(5);
        i.key(ctrl('k'));
        i.key(ctrl('z'));
        assert_eq!(i.value(), "night watch");
        assert_eq!(i.cursor(), 5);
    }

    #[test]
    fn paste_inserts_at_the_caret_and_joins_lines_with_spaces() {
        let mut i = TextInput::new("ad");
        i.set_cursor(1);
        i.paste("b\r\nc");
        assert_eq!(i.value(), "ab cd");
        assert_eq!(i.cursor(), 4);
        i.paste("\n\n");
        assert_eq!(i.value(), "ab cd", "only breaks pastes nothing");
        i.key(ctrl('z'));
        assert_eq!(i.value(), "ad", "a paste undoes in one step");
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
    fn a_selection_is_clamped_and_delete_and_cut_keys_remove_it() {
        let mut i = TextInput::new("abc");
        i.select(1, 99);
        assert_eq!(i.selection(), Some((1, 3)));
        i.key(k(Key::Delete));
        assert_eq!(i.value(), "a");
        let mut j = TextInput::new("watchman");
        j.select(0, 5);
        j.key(ctrl('k'));
        assert_eq!(j.value(), "man", "a cut key removes just the selection");
        let mut m = TextInput::new("watchman");
        m.select(5, 8);
        m.key(ctrl('w'));
        assert_eq!(m.value(), "watch");
        let mut n = TextInput::new("watchman");
        n.select(0, 5);
        n.paste("guard");
        assert_eq!(n.value(), "guardman", "a paste replaces the selection");
    }

    #[test]
    fn shift_with_a_movement_key_selects() {
        let mut i = TextInput::new("ankh morpork");
        i.key(k(Key::Home));
        i.key(shift(Key::Right));
        i.key(shift(Key::Right));
        assert_eq!(i.selection(), Some((0, 2)));
        i.key(Chord {
            shift: true,
            ctrl: true,
            ..Chord::new(Key::Right)
        });
        assert_eq!(
            i.selection(),
            Some((0, 4)),
            "shift-ctrl-right selects a word"
        );
        i.key(shift(Key::End));
        assert_eq!(i.selection(), Some((0, 12)));
        i.key(shift(Key::Home));
        assert_eq!(
            i.selection(),
            None,
            "selecting back to the anchor leaves nothing"
        );
        i.key(shift(Key::Left));
        assert_eq!(i.selection(), None, "the start stops it");
        i.key(shift(Key::End));
        i.key(k(Key::Char('!')));
        assert_eq!(i.value(), "!");
    }
}
