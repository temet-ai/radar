//! The tour: five stops over the real app, after setup (`yes, show me
//! around`) or with `radar setup tour`. Each stop is a card over the body
//! ([`TourCard`]); the app under it keeps working, so what the card says to
//! try can be tried. `→` goes on, `←` back, `esc` ends it; the key a stop
//! says to try does what it always does and goes on too.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::host::{Host, HostEffect, View};
use crate::view::chrome::TourCard;
use crate::view::{Ink, Seg, Text};

#[cfg(test)]
mod tests;

/// One stop of the tour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Screens,
    Panes,
    Keys,
    /// Only with a part that has the agent dock.
    Dock,
    Done,
}

/// Where the tour is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tour {
    pub stops: Vec<Stop>,
    pub at: usize,
}

impl Tour {
    pub fn stop(&self) -> Stop {
        self.stops[self.at]
    }
}

fn key(words: &str) -> Seg {
    Seg::new(words, Ink::Accent).strong()
}

fn say(words: &str) -> Seg {
    Seg::new(words, Ink::Secondary)
}

/// A line of a key then what it does.
fn keyed(k: &str, words: &str) -> Text {
    Text::new().push(key(k)).push(say(&format!("  {words}")))
}

impl Host {
    /// Start the tour on the first screen.
    pub fn open_tour(&mut self) {
        let mut stops = vec![Stop::Screens, Stop::Panes, Stop::Keys];
        if self.plugins.iter().any(|p| p.dock_tabs().is_some()) {
            stops.push(Stop::Dock);
        }
        stops.push(Stop::Done);
        self.setup = None;
        self.menu = None;
        if !self.screens.is_empty() {
            self.view = View::Screen(0);
        }
        self.tour = Some(Tour { stops, at: 0 });
    }

    /// Go to stop `at`; the last stop shows the work screen when there is
    /// one.
    fn tour_go(&mut self, at: usize) {
        let Some(tour) = self.tour.as_mut() else { return };
        tour.at = at.min(tour.stops.len() - 1);
        if tour.stop() == Stop::Done
            && let Some(i) = self.screens.iter().position(|(_, s)| s.id == "work")
        {
            self.view = View::Screen(i);
        }
    }

    /// The key the stop says to try.
    fn tour_try(&self, stop: Stop) -> Option<KeyCode> {
        match stop {
            Stop::Screens => self.tour_other_tab().map(|(c, _)| KeyCode::Char(c)),
            Stop::Panes => Some(KeyCode::Tab),
            Stop::Keys => Some(KeyCode::Char('x')),
            Stop::Dock => Some(KeyCode::Char(crate::dock::KEY)),
            Stop::Done => None,
        }
    }

    /// A screen tab other than the one showing: its key and its name.
    fn tour_other_tab(&self) -> Option<(char, String)> {
        self.chrome_top_bar()
            .tabs
            .into_iter()
            .find(|t| !t.active && t.mnemonic.is_alphabetic())
            .map(|t| (t.mnemonic, t.label))
    }

    /// A key while the tour shows: `Some` when the tour took it, `None` to
    /// let the app have it (the key a stop says to try goes on as well).
    pub(crate) fn tour_key(&mut self, key: KeyEvent) -> Option<Vec<HostEffect>> {
        let tour = self.tour.as_ref()?;
        if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
            return None;
        }
        let (at, last) = (tour.at, tour.at + 1 == tour.stops.len());
        let stop = tour.stop();
        match key.code {
            KeyCode::Right => {
                if last {
                    self.tour = None;
                } else {
                    self.tour_go(at + 1);
                }
                Some(vec![])
            }
            KeyCode::Enter if last => {
                self.tour = None;
                Some(vec![])
            }
            KeyCode::Left => {
                if at == 0 {
                    self.message = Some("this is the first stop".into());
                } else {
                    self.tour_go(at - 1);
                }
                Some(vec![])
            }
            KeyCode::Esc => {
                self.tour = None;
                Some(vec![])
            }
            code if Some(code) == self.tour_try(stop) => {
                self.tour_go(at + 1);
                None
            }
            _ => None,
        }
    }

    /// The tour's card, while it shows.
    pub fn tour_card(&self) -> Option<TourCard> {
        let tour = self.tour.as_ref()?;
        let stop = tour.stop();
        let (title, lines): (&str, Vec<Text>) = match stop {
            Stop::Screens => {
                let tabs = self.chrome_top_bar().tabs;
                let mut lines = vec![Text::of("underlined in its name; press it from anywhere", Ink::Secondary), Text::new()];
                for row in tabs.chunks(4) {
                    let mut line = Text::new();
                    for t in row {
                        line.add(key(&t.mnemonic.to_string()));
                        line.add(say(&format!(" {:<14}", t.label.to_lowercase())));
                    }
                    lines.push(line);
                }
                lines.extend([Text::new(), Text::of("↑ at the top of a pane reaches the bar, then ← →", Ink::Secondary)]);
                ("Every screen has a letter.", lines)
            }
            Stop::Panes => (
                "Three panes: where, what, and the detail.",
                vec![
                    keyed("1", "views    the lists this screen can show"),
                    keyed("2", "items    what that list holds; the selection lives here"),
                    keyed("3", "detail   the selected item and its actions"),
                    Text::new(),
                    keyed("tab", "next pane"),
                    keyed("1 2 3", "jump to one"),
                    keyed("↑↓ j k", "move"),
                    keyed("+/-", "open and fold a tree; a folded row ends in ⋯ n"),
                ],
            ),
            Stop::Keys => (
                "The bottom line holds the keys for here.",
                vec![
                    Text::of("each one can be clicked", Ink::Secondary),
                    keyed("x", "the menu: every key here, with a filter; a row presses its key"),
                    Text::new().push(say("a key sits inside its word: ")).push(key("b")).push(say("ulk; symbol keys come first")),
                    Text::new()
                        .push(say("same key, same thing, same kind of place: "))
                        .push(key("r"))
                        .push(say(" refresh "))
                        .push(key("o"))
                        .push(say(" open "))
                        .push(key("y"))
                        .push(say(" copy link")),
                ],
            ),
            Stop::Dock => (
                "Agents work here, in the dock.",
                vec![
                    keyed("`", "shows it from any screen; jumps to a session that needs you"),
                    keyed("A", "on a row asks an agent about it; radar describes the item first"),
                    keyed("ctrl-t", "an empty session"),
                    keyed("alt-a", "pick the agent"),
                    keyed("alt-f", "widen"),
                    Text::of("a request to run or change something waits here;", Ink::Secondary),
                    Text::of("a chip on the status line says so elsewhere", Ink::Secondary),
                ],
            ),
            Stop::Done => {
                let mode = self.mode();
                let mut lines = vec![
                    Text::new()
                        .push(say("radar is "))
                        .push(Seg::new(mode.word(), Ink::Info).strong())
                        .push(say(" for now: top right, always")),
                    Text::of("to comment, approve or merge: the mode in Settings / general;", Ink::Secondary),
                    Text::of("it still asks every time", Ink::Secondary),
                    Text::new(),
                    Text::new().push(Seg::new("THREE KEYS TO KEEP", Ink::Muted).strong()),
                    keyed("x", "the menu of every key, wherever you are"),
                    keyed(",", "settings: everything from setup"),
                ];
                if tour.stops.contains(&Stop::Dock) {
                    lines.push(keyed("`", "the agent dock"));
                }
                lines.extend([
                    Text::new(),
                    Text::new().push(key("radar setup")).push(say("  walks setup again")),
                ]);
                ("That's the tour.", lines)
            }
        };
        let try_it = match self.tour_try(stop) {
            Some(KeyCode::Char(c)) => Some(format!("try it: press {c}")),
            Some(KeyCode::Tab) => Some("try it: press tab".to_string()),
            _ => None,
        };
        let mut keys = Text::new();
        if stop == Stop::Done {
            keys.add(key("⏎"));
            keys.add(say(" start working   "));
        } else {
            keys.add(key("→"));
            keys.add(say(" next   "));
        }
        keys.add(key("←"));
        keys.add(say(" back"));
        if stop != Stop::Done {
            keys.add(key("   esc"));
            keys.add(say(" end the tour"));
        }
        Some(TourCard { stop: tour.at + 1, of: tour.stops.len(), title: title.into(), lines, try_it, keys })
    }
}
