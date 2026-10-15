//! The composer: text typed for a write (a comment, a reply, a new item's
//! description), in every part the same way. `⏎` starts a new line,
//! `ctrl-s` reviews it (the part shows what will be sent and asks), `esc`
//! stops: at once when nothing is typed, else after asking ([`stop_menu`]).
//! Backspace at the start of an empty line goes back up a line; pasted
//! lines stay lines. A one-line composer (a title) reviews on `⏎`.
//!
//! It draws over the foot of a body ([`Composer::draw_foot`]) or as lines
//! under a row of another element ([`Composer::block`] and
//! [`Composer::draw_input`], e.g. under a line of a diff).

use std::cell::Cell;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use crate::design::help_bar::KeyHint;
use crate::design::theme::Slot;
use crate::input::{Edit, TextInput};
use crate::plugin::Menu;
use crate::theme::Theme;
use crate::widgets;

use super::card::panel;

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

    pub fn key(&mut self, key: KeyEvent) -> Typed {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            return Typed::Review;
        }
        if !self.one_line && key.code == KeyCode::Backspace && self.input.value().is_empty() {
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

    /// Its lines when it sits under a row of another element: the title,
    /// the lines kept, and a row for the line being typed
    /// ([`Composer::draw_input`] draws it).
    pub fn block(&self, theme: &Theme, title: &str) -> Vec<Line<'static>> {
        let mut out = vec![Line::styled(title.to_string(), theme.bold(Slot::AccentPrimary))];
        out.extend(self.lines.iter().map(|l| Line::from(Span::styled(l.clone(), theme.s(Slot::TextPrimary)))));
        out.push(Line::default());
        out
    }

    /// The line being typed, on the last row of `area` (its [`Composer::block`]).
    pub fn draw_input(&self, theme: &Theme, area: Rect, buf: &mut Buffer, cursor: &Cell<Option<Position>>) {
        if area.height == 0 {
            return;
        }
        let row = Rect {
            y: area.y + area.height - 1,
            height: 1,
            ..area
        };
        self.input.render(theme, row, buf, cursor);
    }

    /// A titled panel over the foot of `area`, as tall as its lines (up to
    /// two thirds of it); the last lines when they do not all fit.
    pub fn draw_foot(&self, theme: &Theme, title: &str, area: Rect, buf: &mut Buffer, cursor: &Cell<Option<Position>>) {
        if area.height == 0 {
            return;
        }
        let want = self.lines.len() as u16 + 4;
        let height = want.clamp(5, (area.height * 2 / 3).max(5)).min(area.height);
        let r = Rect {
            y: area.y + area.height - height,
            height,
            ..area
        };
        Clear.render(r, buf);
        let inner = widgets::padded(panel(theme, r, buf, title, true));
        if inner.height == 0 {
            return;
        }
        let room = inner.height.saturating_sub(1) as usize;
        let skip = self.lines.len().saturating_sub(room);
        for (i, l) in self.lines.iter().skip(skip).enumerate() {
            buf.set_line(
                inner.x,
                inner.y + i as u16,
                &Line::from(Span::styled(l.clone(), theme.s(Slot::TextPrimary))),
                inner.width,
            );
        }
        let y = inner.y + (self.lines.len() - skip) as u16;
        self.input.render(theme, Rect { y, height: 1, ..inner }, buf, cursor);
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

    fn press(c: &mut Composer, code: KeyCode) -> Typed {
        c.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn enter_starts_a_line_ctrl_s_reviews_and_backspace_joins_lines_again() {
        let mut c = Composer::new("one\ntwo");
        assert_eq!((c.lines.clone(), c.input.value()), (vec!["one".to_string()], "two".to_string()));
        assert_eq!(press(&mut c, KeyCode::Enter), Typed::Typing);
        assert_eq!(c.text(), "one\ntwo\n");
        assert_eq!(press(&mut c, KeyCode::Backspace), Typed::Typing);
        assert_eq!(c.text(), "one\ntwo");
        assert_eq!(c.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)), Typed::Review);
        c.paste(" more\nnext line\nlast");
        assert_eq!(c.text(), "one\ntwo more\nnext line\nlast");
        assert_eq!(press(&mut c, KeyCode::Esc), Typed::AskStop);
        assert_eq!(press(&mut Composer::new(" "), KeyCode::Esc), Typed::Stop);
    }

    #[test]
    fn a_one_line_composer_reviews_on_enter_and_keeps_a_paste_on_its_line() {
        let mut c = Composer::one_line("a title");
        c.paste(" and\nmore");
        assert_eq!(c.text(), "a title and more");
        assert_eq!(press(&mut c, KeyCode::Enter), Typed::Review);
        assert_eq!(c.hints()[0].action, "next");
        assert_eq!(Composer::new("").hints()[1].keys, "ctrl-s");
    }

    #[test]
    fn stopping_with_text_asks_first() {
        let m = stop_menu("drop", "comment");
        assert_eq!(m.title, "stop: drop the comment?");
        assert_eq!(m.items[0].hint, "throw the text away");
        assert_eq!(m.items[1].hint, "keep writing");
    }
}
