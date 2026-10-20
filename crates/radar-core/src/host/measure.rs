//! What a frame measures before the chrome is built, so every frontend is
//! sent the same numbers: the body the screen gets, how far settings' detail
//! (and setup's sheet) scrolls, and the open menu's reading position. The
//! arithmetic is [`crate::view::measure`]'s; this reads the host's state for it.

use super::{Host, Owner, View};
use crate::theme::Theme;
use crate::view::SectionView;
use crate::view::measure::{self, Size};

impl Host {
    /// The body a frame `width` × `height` cells big leaves the screen or
    /// settings: under the top bar (and a source row) and over the status line.
    pub(crate) fn body_size(&self, width: u16, height: u16) -> Size {
        let sources = matches!(self.view, View::Screen(i) if self.group_screens(i).len() > 1);
        measure::body_of(Size::new(width, height), sources)
    }

    /// Measure a frame `width` × `height` cells big: how far settings' detail
    /// (or setup's sheet) can scroll, and the open menu's lines, rows shown,
    /// usable controls and reading position. [`Host::chrome_view_with`] runs it.
    pub(crate) fn measure_frame(&self, theme: &Theme, width: u16, height: u16) {
        if self.history.is_some() {
            return;
        }
        let body = self.body_size(width, height);
        if self.setup.is_some() {
            self.measure_setup(theme, body);
        } else if self.screen().is_none() {
            self.measure_settings_detail(theme, body);
        }
        if let Some(open) = &self.menu {
            measure::apply_menu(&self.menu_view().expect("menu is open"), &open.viewport, theme, body);
        }
    }

    /// The view of the section showing, host-owned or a part's, laid out for a
    /// section pane `room` big.
    fn measured_section(&self, theme: &Theme, room: Size) -> Option<SectionView> {
        match self.current_section()? {
            (Owner::Plugin(p), s) => self.section_view_in(p, s.id, room.width, room.height),
            _ => crate::settings_view::section_view(self, theme.tier()),
        }
    }

    /// How far settings' detail can scroll at this size ([`Host::settings_detail`]).
    fn measure_settings_detail(&self, theme: &Theme, body: Size) {
        if self.current_section().is_none() {
            return;
        }
        if self.capturing() {
            self.settings_detail.top();
        }
        let room = measure::settings_detail_text(body);
        let view = self.measured_section(theme, measure::settings_middle_inside(body));
        match view {
            Some(view) if room.width > 0 && room.height > 0 => {
                let rows = measure::detail_rows(&view, room.width, measure::SETTINGS_INSET);
                self.settings_detail.fit(room.height, rows.min(usize::from(u16::MAX)) as u16);
            }
            _ => {
                self.settings_detail.fit(0, 0);
            }
        }
    }

    /// The view of the step showing: a host step's own, else the part's section.
    fn setup_step_view(&self, theme: &Theme, card: Size) -> Option<SectionView> {
        if self.setup_host_step() {
            return self.setup_section_view();
        }
        match self.current_section()? {
            (Owner::Plugin(p), s) => {
                self.section_view_in(p, s.id, card.width.saturating_sub(2), card.height.saturating_sub(2))
            }
            _ => crate::settings_view::section_view(self, theme.tier()),
        }
    }

    /// How far setup's sheet can scroll at this size ([`Host::settings_detail`]).
    fn measure_setup(&self, theme: &Theme, body: Size) {
        let frame = measure::setup_frame(body);
        let about = self
            .setup_step_view(theme, frame)
            .map_or(0, |v| measure::detail_rows(&v, measure::setup_about_width(body), measure::SETUP_INSET));
        let (card, sheet) = measure::setup_panes(body, about.min(usize::from(u16::MAX)) as u16);
        if self.capturing() {
            self.settings_detail.top();
        }
        match self.setup_step_view(theme, card) {
            Some(view) if sheet.width > 0 && sheet.height > 0 => {
                let rows = measure::detail_rows(&view, sheet.width, measure::SETUP_INSET);
                self.settings_detail.fit(sheet.height, rows.min(usize::from(u16::MAX)) as u16);
            }
            _ => {
                self.settings_detail.fit(0, 0);
            }
        }
    }
}
