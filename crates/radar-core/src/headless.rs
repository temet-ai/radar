//! A frontend with no terminal: it reads what the parts show as view data
//! ([`crate::view`]) and drives them with [`Intent`]s through the same host,
//! so the same part state, mode and permission checks, confirmations and
//! journal causality apply as with the terminal. It paints nothing and
//! names no Ratatui or Crossterm type (a test reads the source to keep it
//! so); it is the second, independent consumer that proves the contract
//! before another renderer (Ink) speaks it over IPC.

use serde::{Deserialize, Serialize};

mod explore;
pub mod text;
pub use explore::{Capture, CaptureKind, Offered, PageKind, PageReach, capture_of, foot_controls, node_targets, offered, view_problems};

use crate::host::{Event, Host, HostEffect, JournalRecord, Owner};
use crate::view::{Change, Intent, PaneId, ScreenView, SectionView};

/// A registered surface that an independent frontend cannot yet show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewGap {
    pub plugin: String,
    pub scope: String,
    pub section: bool,
    /// Missing data, a contract version mismatch, or a scope mismatch.
    pub reason: String,
}

/// The open menu or question, as data: what it asks and the choices, each
/// by the value [`Intent::Invoke`] answers it with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuView {
    pub id: String,
    pub title: String,
    /// A question (a dialog with buttons), not a list.
    pub question: bool,
    /// The write it is before, when it is one; the host has already added
    /// the mode's note when the mode refuses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<Change>,
    pub items: Vec<Choice>,
    /// The text in its field, when it asks for text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typed: Option<String>,
}

/// One row of a menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub hint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<char>,
    /// A heading or a note: shown, never chosen.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shown_only: bool,
    /// It can be chosen but will not do all it says now (the mode).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
}

/// The host, driven without a terminal.
pub struct Headless {
    host: Host,
    width: u16,
    height: u16,
    records: Vec<JournalRecord>,
}

impl Headless {
    /// Drive `host` as a frontend `width` × `height` cells big would (the
    /// room parts may fit their words to). Action recording is on.
    pub fn new(mut host: Host, width: u16, height: u16) -> Self {
        host.enable_journal();
        host.set_view_room(width, height);
        Self {
            host,
            width,
            height,
            records: Vec::new(),
        }
    }

    /// Construct a frontend that requires every registered plugin surface to
    /// satisfy the neutral view contract. Ink must use this gate at startup.
    pub fn requiring_views(host: Host, width: u16, height: u16) -> Result<Self, Vec<ViewGap>> {
        let frontend = Self::new(host, width, height);
        let gaps = frontend.view_gaps();
        if gaps.is_empty() { Ok(frontend) } else { Err(gaps) }
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut Host {
        &mut self.host
    }

    /// Audit every registered plugin surface before enabling a renderer that
    /// accepts only views. A missing view must never silently become a blank
    /// screen or fall back to a plugin-specific terminal renderer.
    pub fn view_gaps(&self) -> Vec<ViewGap> {
        let mut gaps = Vec::new();
        for (p, screen) in &self.host.screens {
            let reason = match self.host.screen_view_in(*p, screen.id, self.width, self.height) {
                None => Some("missing".to_string()),
                Some(view) if view.version != crate::view::VERSION => Some("version".to_string()),
                Some(view) if view.scope != screen.id => Some("scope".to_string()),
                Some(_) => None,
            };
            if let Some(reason) = reason {
                gaps.push(ViewGap { plugin: self.host.plugins[*p].id().into(), scope: screen.id.into(), section: false, reason });
            }
        }
        for (owner, section) in &self.host.sections {
            let Owner::Plugin(p) = owner else { continue };
            let reason = match self.host.section_view_in(*p, section.id, self.width, self.height) {
                None => Some("missing".to_string()),
                Some(view) if view.version != crate::view::VERSION => Some("version".to_string()),
                Some(view) if view.scope != section.id => Some("scope".to_string()),
                Some(_) => None,
            };
            if let Some(reason) = reason {
                gaps.push(ViewGap { plugin: self.host.plugins[*p].id().into(), scope: section.id.into(), section: true, reason });
            }
        }
        gaps
    }

    /// The screen showing, as its part describes it; `None` on a screen
    /// still drawn the legacy way, or off the screens.
    pub fn screen(&self) -> Option<ScreenView> {
        crate::view::screen_showing(&self.host, self.width, self.height)
    }

    /// The displayed screen with admitted single-press foot controls. Raw `screen`
    /// remains the deterministic source for ordinary fixture and page enumeration.
    pub fn displayed_screen(&self) -> Option<ScreenView> {
        self.host.displayed_screen(self.width, self.height)
    }

    /// The settings section showing, as its part describes it.
    pub fn section(&self) -> Option<SectionView> {
        section_of(&self.host, self.width, self.height)
    }

    /// What shows, as text ([`text`]): the screen, or the settings section,
    /// with the pane that has the keys marked; then the open menu and the
    /// status line. What a part's tests snapshot in place of a frame.
    pub fn text(&self) -> String {
        text_of(&self.host, self.width, self.height)
    }

    /// The screen showing as text, panes only ([`Headless::text`] without
    /// the chrome).
    pub fn screen_text(&self) -> String {
        screen_text_of(&self.host, self.width, self.height)
    }

    /// The menu or question open, if one is.
    pub fn menu(&self) -> Option<MenuView> {
        let open = self.host.menu.as_ref()?;
        let m = &open.menu;
        let mut view = MenuView {
            id: m.id.clone(),
            title: m.title.clone(),
            question: m.question,
            change: m.change.map(Change::from),
            items: m
                .items
                .iter()
                .map(|i| Choice {
                    value: i.value.clone(),
                    label: i.label.clone(),
                    hint: i.hint.clone(),
                    key: i.key,
                    shown_only: i.heading || i.note,
                    muted: i.muted,
                })
                .collect(),
            typed: open.input.as_ref().map(|i| i.value()),
        };
        crate::view::clean::multiline(&mut view);
        Some(view)
    }

    /// Move the focus to a pane, as a frontend does on its own; not the
    /// panes under the History overlay while it is open.
    pub fn focus(&mut self, pane: PaneId) {
        if self.host.history.is_none() {
            self.host.focus = pane.into();
        }
    }

    /// Send what the user did; the host's effects come back, its journal
    /// records are kept ([`Headless::records`]).
    pub fn send(&mut self, intent: Intent) -> Vec<HostEffect> {
        let out = self.host.update(Event::Intent(intent));
        self.records.extend(out.iter().filter_map(|e| match e {
            HostEffect::Journal(r) => Some(r.clone()),
            _ => None,
        }));
        out
    }

    /// Every journal record the intents produced, in order.
    pub fn records(&self) -> &[JournalRecord] {
        &self.records
    }
}

/// [`Headless::text`] for a host a test drives itself (keys, effects): says
/// the room once, as [`Headless::new`] does, then reads the views.
pub fn text_of(host: &Host, width: u16, height: u16) -> String {
    host.set_view_room(width, height);
    let body = match crate::view::screen_showing(host, width, height) {
        Some(screen) => text::screen(&screen, Some(host.focus.into())),
        None => section_of(host, width, height).map(|s| text::section(&s)).unwrap_or_default(),
    };
    body + &text::chrome(&host.chrome_view(width, height))
}

/// The whole host screen as text: the top bar, settings navigation, setup's
/// rail and the tour's card ([`text::frame`]), then [`text_of`]. A core
/// test of the host's own screens reads this; a part's test reads
/// [`text_of`], which holds what the part shows.
pub fn host_text_of(host: &Host, width: u16, height: u16) -> String {
    host.set_view_room(width, height);
    text::frame(&host.chrome_view(width, height)) + &text_of(host, width, height)
}

/// [`Headless::screen_text`] for a host a test drives itself.
pub fn screen_text_of(host: &Host, width: u16, height: u16) -> String {
    host.set_view_room(width, height);
    crate::view::screen_showing(host, width, height).map(|s| text::screen(&s, Some(host.focus.into()))).unwrap_or_default()
}

/// [`Headless::section`] for a host a test drives itself.
pub fn section_of(host: &Host, width: u16, height: u16) -> Option<SectionView> {
    // Setup's own steps (a step that is a section is that section).
    if host.setup_host_step() {
        return host.setup_section_view();
    }
    match host.current_section()? {
        (Owner::Plugin(p), s) => host.section_view_in(p, s.id, width, height),
        _ => crate::settings_view::section_view(host, crate::design::theme::Tier::TrueColor),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_headless_frontend_names_no_terminal_library() {
        let source = include_str!("headless.rs");
        for word in ["ratatui", "crossterm", "Buffer", "Rect", "KeyEvent"] {
            let used = source
                .lines()
                .take_while(|l| !l.starts_with("#[cfg(test)]"))
                .filter(|l| !l.trim_start().starts_with("//"))
                .any(|l| l.contains(word));
            assert!(!used, "headless.rs uses {word}");
        }
    }
}
