//! Host geometry and hit ownership for the kit's late notice layer.
use crate::{
    host::{Hit, Host},
    theme::Theme,
    ui,
    view::chrome::ChromeView,
};
use ratatui::{
    buffer::Buffer,
    layout::{Margin, Rect},
};

pub(crate) fn content(area: Rect, has_sources: bool) -> Rect {
    let body = Rect::new(
        area.x,
        area.y.saturating_add(1).min(area.bottom()),
        area.width,
        area.height.saturating_sub(2),
    );
    let source = u16::from(has_sources && body.height > 1);
    Rect::new(
        body.x,
        body.y.saturating_add(source),
        body.width,
        body.height.saturating_sub(source),
    )
    .inner(Margin::new(1, 1))
}
/// The inside of the panel standing on the status line at the right edge, read
/// from the frame just drawn: notices belong inside its border, so they never
/// cross a panel's edge. `fallback` when no such panel is drawn there.
pub(crate) fn anchor(buf: &Buffer, fallback: Rect) -> Rect {
    let a = buf.area;
    if a.width < 3 || a.height < 4 {
        return fallback;
    }
    let (right, bottom) = (a.right() - 1, a.bottom() - 2);
    let symbol = |x: u16, y: u16| buf[(x, y)].symbol();
    if !matches!(symbol(right, bottom), "╯" | "┘" | "┛") {
        return fallback;
    }
    let Some(left) =
        (a.x..right).rev().find(|&x| matches!(symbol(x, bottom), "╰" | "└" | "┗" | "╯" | "┘" | "┛"))
    else {
        return fallback;
    };
    if !matches!(symbol(left, bottom), "╰" | "└" | "┗") {
        return fallback;
    }
    let mut top = bottom;
    loop {
        if top <= a.y {
            return fallback;
        }
        top -= 1;
        match symbol(left, top) {
            "╭" | "┌" | "┏" => break,
            "│" | "┃" | "├" | "┣" => {}
            _ => return fallback,
        }
    }
    let inner = Rect::new(left + 1, top + 1, right - left - 1, bottom - top - 1);
    if inner.is_empty() { fallback } else { inner }
}
pub(super) fn history(
    host: &Host,
    theme: &Theme,
    chrome: &ChromeView,
    body: Rect,
    buf: &mut Buffer,
) {
    let Some(view) = &chrome.notice_history else {
        return;
    };
    let layout = ui::notices::history_layout(body.intersection(buf.area));
    host.frontend.begin_selectable();
    host.frontend.mark_selectable(layout.list, false);
    host.frontend.mark_selectable(layout.detail, true);
    host.hits.borrow_mut().extend(
        ui::notices::history(theme, view, body, buf)
            .into_iter()
            .map(|(area, id)| (area, Hit::NoticeHistory(id))),
    );
}
pub(super) fn transient(host: &Host, theme: &Theme, buf: &mut Buffer) {
    let chrome = host.chrome_view_with(buf.area.width, buf.area.height, theme);
    if !chrome.notices_visible {
        return;
    }
    // In the bottom-right panel when every notice reads whole there; across the content otherwise.
    let wide = content(buf.area, !chrome.sources.is_empty());
    let panel = anchor(buf, wide);
    let area = if ui::notices::fits(theme, &chrome.notices.active, panel) { panel } else { wide };
    let stack = ui::notices::layout(
        theme,
        &chrome.notices.active,
        chrome.notices.hidden,
        area,
        chrome.notices.now_ms,
        host.notice_motion_enabled(),
    );
    if let Some(summary) = &stack.summary {
        host.hits
            .borrow_mut()
            .push((summary.area, Hit::NoticeHistoryOpen));
    }
    host.hits.borrow_mut().extend(
        ui::notices::paint(&stack, theme, buf)
            .into_iter()
            .map(|(area, id)| (area, Hit::Notice(id))),
    );
}

#[cfg(test)]
mod tests;
