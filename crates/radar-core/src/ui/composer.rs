//! The composer: text typed for a write (a comment, a reply, a new item's
//! description), in every part the same way. `⏎` starts a new line,
//! `ctrl-s` reviews it (the part shows what will be sent and asks), `esc`
//! stops: at once when nothing is typed, else after asking ([`stop_menu`]).
//! Backspace at the start of an empty line goes back up a line; pasted
//! lines stay lines. A one-line composer (a title) reviews on `⏎`.
//!
//! The renderer draws it from the view data a part builds out of
//! [`Composer::lines`] and the line being typed.

use crate::design::help_bar::KeyHint;
use crate::input::{Edit, TextInput};
use crate::plugin::Menu;
use crate::view::{Chord, Key};

/// What a key did in the composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Typed {
    /// Keep typing.
    Typing,
    /// `ctrl-s` (or `⏎` in a one-line composer): show what will be sent.
    Review,
    /// `esc` with nothing typed: close it.
    Stop,
    /// `esc` with text: ask first ([`stop_menu`]).
    AskStop,
}

/// Text being typed: the lines kept, and the one being edited.
#[derive(Debug)]
pub struct Composer {
    pub lines: Vec<String>,
    pub input: TextInput,
    one_line: bool,
}

impl Composer {
    /// A composer of many lines, holding `text` (a kept draft, or nothing).
    pub fn new(text: &str) -> Self {
        let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        let last = lines.pop().unwrap_or_default();
        Self {
            lines,
            input: TextInput::new(&last),
            one_line: false,
        }
    }

    /// A composer of one line (a title): `⏎` reviews it.
    pub fn one_line(text: &str) -> Self {
        Self {
            lines: vec![],
            input: TextInput::new(text),
            one_line: true,
        }
    }

    pub fn is_one_line(&self) -> bool {
        self.one_line
    }

    /// Everything typed, the line being edited included.
    pub fn text(&self) -> String {
        let mut all = self.lines.clone();
        all.push(self.input.value());
        all.join("\n")
    }

    /// Whether nothing but spaces is typed.
    pub fn is_empty(&self) -> bool {
        self.text().trim().is_empty()
    }

    pub fn key(&mut self, key: Chord) -> Typed {
        if key.ctrl && key.key == Key::Char('s') {
            return Typed::Review;
        }
        if !self.one_line && key.key == Key::Backspace && self.input.value().is_empty() {
            if let Some(last) = self.lines.pop() {
                self.input = TextInput::new(&last);
            }
            return Typed::Typing;
        }
        match self.input.key(key) {
            Edit::Submit(_) if self.one_line => Typed::Review,
            Edit::Submit(line) => {
                self.lines.push(line);
                self.input = TextInput::new("");
                Typed::Typing
            }
            Edit::Cancel if self.is_empty() => Typed::Stop,
            Edit::Cancel => Typed::AskStop,
            Edit::Typing => Typed::Typing,
        }
    }

    /// Pasted text: its lines stay lines (one line in a one-line composer).
    pub fn paste(&mut self, text: &str) {
        if self.one_line {
            self.input.paste(text);
            return;
        }
        let mut parts = text.split('\n').map(|l| l.trim_end_matches('\r'));
        if let Some(first) = parts.next() {
            self.input.paste(first);
        }
        for line in parts {
            self.lines.push(self.input.value());
            self.input = TextInput::new(line);
        }
    }

    /// The status line's keys while typing.
    pub fn hints(&self) -> Vec<KeyHint<'static>> {
        if self.one_line {
            vec![KeyHint::new("⏎", "next"), KeyHint::new("esc", "stop")]
        } else {
            vec![
                KeyHint::new("⏎", "new line"),
                KeyHint::new("ctrl-s", "review and send"),
                KeyHint::new("esc", "stop"),
            ]
        }
    }
}

/// The question `esc` asks when text is typed: `stop: drop the {what}?`;
/// `y` answers `yes` (the text is thrown away), `n` `no` (keep writing).
pub fn stop_menu(id: impl Into<String>, what: &str) -> Menu {
    Menu::confirm(id, format!("stop: drop the {what}?"), "throw the text away", "keep writing")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(c: &mut Composer, code: Key) -> Typed {
        c.key(Chord::new(code))
    }

    #[test]
    fn enter_starts_a_line_ctrl_s_reviews_and_backspace_joins_lines_again() {
        let mut c = Composer::new("one\ntwo");
        assert_eq!((c.lines.clone(), c.input.value()), (vec!["one".to_string()], "two".to_string()));
        assert_eq!(press(&mut c, Key::Enter), Typed::Typing);
        assert_eq!(c.text(), "one\ntwo\n");
        assert_eq!(press(&mut c, Key::Backspace), Typed::Typing);
        assert_eq!(c.text(), "one\ntwo");
        assert_eq!(c.key(Chord::ctrl('s')), Typed::Review);
        c.paste(" more\nnext line\nlast");
        assert_eq!(c.text(), "one\ntwo more\nnext line\nlast");
        assert_eq!(press(&mut c, Key::Esc), Typed::AskStop);
        assert_eq!(press(&mut Composer::new(" "), Key::Esc), Typed::Stop);
    }

    #[test]
    fn a_one_line_composer_reviews_on_enter_and_keeps_a_paste_on_its_line() {
        let mut c = Composer::one_line("a title");
        c.paste(" and\nmore");
        assert_eq!(c.text(), "a title and more");
        assert_eq!(press(&mut c, Key::Enter), Typed::Review);
        assert_eq!(c.hints()[0].action, "next");
        assert_eq!(Composer::new("").hints()[1].keys, "ctrl-s");
    }

    #[test]
    fn stopping_with_text_asks_first() {
        let m = stop_menu("drop", "comment");
        assert_eq!(m.title, "stop: drop the comment?");
        assert_eq!(m.items[0].hint, "yes, throw the text away");
        assert_eq!(m.items[1].hint, "no, keep writing");
    }
}
