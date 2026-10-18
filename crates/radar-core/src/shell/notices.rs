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
    let stack = ui::notices::layout(
        &chrome.notices.active,
        chrome.notices.hidden,
        content(buf.area, !chrome.sources.is_empty()),
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
