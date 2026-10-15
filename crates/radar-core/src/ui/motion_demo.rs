//! The gallery's motion tab: every effect of [`crate::motion`] on sample
//! rows, each started by a key, so it can be seen live and judged
//! (`cargo run -p radar-core --example gallery`, the last tab).

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Widget};

use super::card::panel;
use super::states::loading;
use crate::design::theme::Slot;
use crate::motion::{self, Changes, Cue, Level, Region};
use crate::theme::Theme;
use crate::widgets;

const ITEMS: [(&str, &str); 7] = [
    ("Atlas of the north", "on loan"),
    ("Birds of the coast", "shelved"),
    ("Clocks and their makers", "on loan"),
    ("Dyes from plants", "shelved"),
    ("Engines, a history", "missing"),
    ("Ferns of the valleys", "shelved"),
    ("Glass and its uses", "on loan"),
];

/// What each key shows, and at which level.
const KEYS: [(&str, &str, &str); 8] = [
    ("s", "a screen switch: the body fades in", "subtle"),
    ("m", "a menu or confirm sweeps open (again: closes)", "subtle"),
    ("g", "rows a sync changed glow and fade", "subtle"),
    ("p", "an item's page slides in", "full"),
    ("n", "a new row sweeps in at the top", "full"),
    ("t", "a message fades in (again: fades out)", "full"),
    ("l", "a panel waiting for data shimmers (again: stops)", "full"),
    ("f", "full, subtle, off", ""),
];

/// The tab's state: its rows, what is open, and the cues its keys made.
pub struct MotionDemo {
    pub level: Level,
    rows: Vec<(String, String, &'static str)>,
    added: usize,
    popup: bool,
    message: Option<String>,
    waiting: bool,
    cues: Vec<Cue>,
}

impl Default for MotionDemo {
    fn default() -> Self {
        Self {
            level: Level::Full,
            rows: ITEMS
                .iter()
                .enumerate()
                .map(|(i, (name, state))| (format!("item:{i}"), name.to_string(), *state))
                .collect(),
            added: 0,
            popup: false,
            message: None,
            waiting: false,
            cues: Vec::new(),
        }
    }
}

impl MotionDemo {
    /// Something on screen changed: show it.
    pub fn cue(&mut self, cue: Cue) {
        self.cues.push(cue);
    }

    /// What the keys started since the last call, for the engine.
    pub fn take_cues(&mut self) -> Vec<Cue> {
        std::mem::take(&mut self.cues)
    }

    /// A key on the tab; false when it is not one of the tab's.
    pub fn key(&mut self, key: &KeyEvent) -> bool {
        let KeyCode::Char(c) = key.code else {
            return false;
        };
        match c {
            's' => self.cue(Cue::Screen),
            'p' => self.cue(Cue::Page),
            'm' => {
                self.popup = !self.popup;
                if self.popup {
                    self.cue(Cue::Popup);
                }
            }
            'g' => {
                let mut changes = Changes::default();
                for i in [1, 3] {
                    if let Some((k, ..)) = self.rows.get(i) {
                        changes.changed.insert(k.clone());
                    }
                }
                self.cue(Cue::Changed(changes));
            }
            'n' => {
                self.added += 1;
                let key = format!("item:new{}", self.added);
                self.rows
                    .insert(0, (key.clone(), format!("New arrival {}", self.added), "shelved"));
                let mut changes = Changes::default();
                changes.added.insert(key);
                self.cue(Cue::Changed(changes));
            }
            't' => {
                if self.message.take().is_some() {
                    self.cue(Cue::MessageGone);
                } else {
                    self.message = Some("copied 42 characters".into());
                    self.cue(Cue::Message);
                }
            }
            'l' => self.waiting = !self.waiting,
            'f' => {
                let at = Level::ALL.iter().position(|l| *l == self.level).unwrap_or(0);
                self.level = Level::ALL[(at + 1) % Level::ALL.len()];
            }
            _ => return false,
        }
        true
    }

    pub fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let [main, status] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
        let wide = main.width >= 100;
        let [left, right] = if wide {
            Layout::horizontal([Constraint::Percentage(45), Constraint::Fill(1)]).areas(main)
        } else {
            Layout::vertical([Constraint::Fill(1), Constraint::Fill(1)]).areas(main)
        };
        let [about, wait] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(right);

        let inner = panel(theme, left, buf, "items", true);
        for (i, (key, name, state)) in self.rows.iter().enumerate().take(inner.height as usize) {
            let y = inner.y + i as u16;
            let rect = Rect { y, height: 1, ..inner };
            let tone = match *state {
                "missing" => Slot::StatusError,
                "on loan" => Slot::StatusWarning,
                _ => Slot::TextMuted,
            };
            widgets::row(
                buf,
                inner,
                y,
                widgets::row_style(theme, true, i == 1),
                Line::styled(format!(" {name}"), theme.s(Slot::TextPrimary)),
                Some(Span::styled(*state, theme.s(tone))),
            );
            motion::row(key.clone(), rect);
        }

        let inner = panel(theme, about, buf, "motion", false);
        let mut lines = vec![
            Line::from(vec![
                Span::styled("level  ", theme.s(Slot::TextMuted)),
                Span::styled(self.level.word(), theme.bold(Slot::TextEmphasis)),
                Span::styled(format!("  {}", self.level.about()), theme.s(Slot::TextMuted)),
            ]),
            Line::default(),
        ];
        for (k, what, from) in KEYS {
            let shown = match from {
                "subtle" => self.level != Level::Off,
                "full" => self.level == Level::Full,
                _ => true,
            };
            let style = if shown {
                theme.s(Slot::TextPrimary)
            } else {
                theme.s(Slot::TextMuted)
            };
            let mut spans = vec![
                Span::styled(format!(" {k}  "), theme.bold(Slot::AccentSecondary)),
                Span::styled(what, style),
            ];
            if !from.is_empty() {
                spans.push(Span::styled(format!("  · {from}"), theme.s(Slot::TextMuted)));
            }
            lines.push(Line::from(spans));
        }
        widgets::paragraph(buf, inner, lines);

        let inner = panel(theme, wait, buf, "waiting", false);
        if self.waiting {
            buf.set_line(inner.x + 1, inner.y, &loading(theme), inner.width.saturating_sub(1));
            motion::loading(inner);
        } else {
            widgets::empty(buf, inner, theme, "Nothing to wait for.");
        }

        buf.set_style(status, theme.bg(Slot::BgOverlay));
        if let Some(m) = &self.message {
            let rect = Rect {
                x: status.x + 1,
                width: (super::width(m) as u16).min(status.width.saturating_sub(1)),
                ..status
            };
            buf.set_string(rect.x, rect.y, m, theme.s(Slot::StatusInfo));
            motion::region(Region::Message, rect);
        }

        if self.popup {
            self.confirm(theme, main, buf);
        }
    }

    fn confirm(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        // As the host draws a question: what will happen, a blank line, the
        // buttons against the right, `y` highlighted.
        let (three, _) = widgets::buttons(
            theme,
            &[
                widgets::Button { keys: "y".into(), words: "shelve it".into(), focused: true, ..Default::default() },
                widgets::Button { keys: "n".into(), words: "leave it".into(), ..Default::default() },
            ],
            5,
        );
        let mut lines = vec![
            Line::from(Span::styled(" puts the book back on its shelf", theme.s(Slot::TextPrimary))),
            Line::raw(""),
        ];
        lines.extend(three);
        let w = (lines.iter().map(Line::width).max().unwrap_or(0) as u16 + 2).min(area.width);
        let h = (lines.len() as u16 + 2).min(area.height);
        let r = Rect {
            x: area.x + (area.width - w) / 2,
            y: area.y + (area.height - h) / 2,
            width: w,
            height: h,
        };
        Clear.render(r, buf);
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_set(ratatui::symbols::border::ROUNDED)
                    .border_style(theme.s(Slot::AccentPrimary))
                    .title(Line::styled(" shelve it? ", theme.bold(Slot::TextEmphasis))),
            )
            .style(theme.bg(Slot::BgOverlay))
            .render(r, buf);
        motion::region(Region::Popup, r);
    }
}
