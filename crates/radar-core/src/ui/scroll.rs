//! Scrolling: the state a panel keeps while it is drawn through `&self`, a
//! body drawn through `tui-scrollview`, and the scrollbar on a panel's edge.

use std::cell::Cell;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Margin, Position, Rect, Size};
use ratatui::text::Line;
use ratatui::widgets::{
    Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget, Widget, Wrap,
};
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

use crate::design::theme::Slot;
use crate::theme::Theme;

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
    pub fn key(&self, key: &KeyEvent) -> bool {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.by(1);
            }
            // At the top `↑` is not the panel's: the host takes it up to
            // the top bar.
            KeyCode::Up | KeyCode::Char('k') if self.at() == 0 => return false,
            KeyCode::Up | KeyCode::Char('k') => {
                self.by(-1);
            }
            KeyCode::PageDown => return self.page(true),
            KeyCode::PageUp => return self.page(false),
            KeyCode::Home | KeyCode::Char('g') => self.top(),
            KeyCode::End | KeyCode::Char('G') => self.at.set(self.max()),
            _ => return false,
        }
        true
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

/// The panel's edge around `inner`, one cell out on every side: what a
/// scrollbar runs down when only the area inside a panel is known (the host
/// draws a settings section's panel and hands the part its inner area).
pub fn around(inner: Rect) -> Rect {
    Rect {
        x: inner.x.saturating_sub(1),
        y: inner.y.saturating_sub(1),
        width: inner.width + 2,
        height: inner.height + 2,
    }
}

/// [`scrollbar`] for a panel known by its `inner` area, not its frame.
pub fn scrollbar_in(theme: &Theme, inner: Rect, buf: &mut Buffer, max: usize, at: usize) {
    scrollbar(theme, around(inner), buf, max, at);
}

/// A scrollbar on the right edge of `frame` (a panel's border), between its
/// corners; nothing when everything fits (`max == 0`).
pub fn scrollbar(theme: &Theme, frame: Rect, buf: &mut Buffer, max: usize, at: usize) {
    if max == 0 {
        return;
    }
    crate::motion::frames::protect(Rect::new(
        frame.right().saturating_sub(1), frame.y.saturating_add(1),
        u16::from(frame.width > 0), frame.height.saturating_sub(2),
    ));
    let mut state = ScrollbarState::new(max + 1).position(at);
    Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_style(theme.s(Slot::BgOverlay))
        .thumb_style(theme.s(Slot::AccentMuted))
        .render(
            frame.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            buf,
            &mut state,
        );
}

/// Draw `height` rows of content into `body` through a scroll view, at the
/// scroll's offset, with the scrollbar on `frame`'s edge. `draw` gets the
/// whole content area (`body.width` wide, `height` tall) and its buffer, so
/// a body can stack any widgets (text, gauges, a table) and scroll as one.
pub fn scrolled_with(
    theme: &Theme,
    scroll: &Scroll,
    frame: Rect,
    body: Rect,
    buf: &mut Buffer,
    height: u16,
    draw: impl FnOnce(Rect, &mut Buffer),
) {
    if body.width == 0 || body.height == 0 {
        return;
    }
    let at = scroll.fit(body.height, height);
    let mut view = ScrollView::new(Size::new(body.width, height.max(1)))
        .scrollbars_visibility(ScrollbarVisibility::Never);
    // The view starts blank; give it the panel's own look so its cells do
    // not wipe the background they are copied onto.
    let base = buf.cell(Position::new(body.x, body.y)).map(|c| c.style());
    let area = view.area();
    if let Some(style) = base {
        view.buf_mut().set_style(area, style);
    }
    draw(area, view.buf_mut());
    let mut state = ScrollViewState::with_offset(Position::new(0, at));
    view.render(body, buf, &mut state);
    scrollbar(theme, frame, buf, scroll.max() as usize, at as usize);
}

/// Lines wrapped to `body`, scrolled, with the scrollbar on `frame`'s edge.
pub fn scrolled(
    theme: &Theme,
    scroll: &Scroll,
    frame: Rect,
    body: Rect,
    buf: &mut Buffer,
    lines: Vec<Line<'static>>,
) {
    let text = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total = text.line_count(body.width).min(u16::MAX as usize) as u16;
    scrolled_with(theme, scroll, frame, body, buf, total, |area, b| {
        text.render(area, b);
    });
}

/// Wrapped text rendered straight into its viewport, without a content-sized
/// backing buffer. Settings explanations can be long, but only their visible
/// rows need cells. Use the same wrap width for measuring and drawing.
pub fn paragraph(
    theme: &Theme,
    scroll: &Scroll,
    frame: Rect,
    body: Rect,
    buf: &mut Buffer,
    lines: Vec<Line<'static>>,
) {
    if body.is_empty() {
        scroll.fit(0, 0);
        return;
    }
    let text = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total = text.line_count(body.width).min(u16::MAX as usize) as u16;
    let at = scroll.fit(body.height, total);
    text.scroll((at, 0)).render(body, buf);
    scrollbar(theme, frame, buf, scroll.max() as usize, at as usize);
}
