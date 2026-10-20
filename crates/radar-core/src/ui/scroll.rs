//! Scrolling, drawn: a body drawn through `tui-scrollview` and the scrollbar
//! on a panel's edge. The state ([`Scroll`]) is `view::scroll`'s.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::{Margin, Position, Rect, Size};
use ratatui::text::Line;
use ratatui::widgets::{
    Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget, Widget, Wrap,
};
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

pub use crate::view::scroll::{AsChord, Scroll};

impl AsChord for KeyEvent {
    fn as_chord(&self) -> Option<crate::view::Chord> {
        crate::view_tui::chord_of(self)
    }
}

use crate::design::theme::Slot;
use crate::theme::Theme;

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
