//! Setup in the Ratatui shell ([`crate::setup`]): `radar  setup / <step>`
//! with `step n of m` and the mode on the top bar, the step rail under it,
//! the card (the step's section, or a host step's rows) beside its sheet
//! (the section's detail), and the key bar on the status line. Painted
//! from the chrome's [`SetupChrome`] and the section the step shows: the
//! same view data Ink is sent.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::text::{Line, Span};

use super::{adopt_hits, ink_style, section_detail_lines};
use crate::compose::Parts;
use crate::design::theme::Slot;
use crate::host::{Hit, Host, Owner};
use crate::plugin::Pane;
use crate::theme::Theme;
use crate::view::chrome::{ChromeView, SetupChrome};
use crate::view_tui::Cx;
use crate::{ui, widgets};

/// The rows the card keeps, below the about sheet, where the frame has them.
const CARD_LEAST: u16 = 22;

/// The boards' size: setup never grows past it, and sits centred in a
/// larger body.
pub const MOST_WIDE: u16 = 140;
pub const MOST_HIGH: u16 = 40;

/// Where setup goes in the body: [`MOST_WIDE`] × [`MOST_HIGH`] at most,
/// centred, a cell in from each side.
pub fn frame(body: Rect) -> Rect {
    let inside = body.inner(Margin::new(1, 0));
    let w = inside.width.min(MOST_WIDE);
    let h = inside.height.min(MOST_HIGH);
    Rect::new(inside.x + (inside.width - w) / 2, inside.y + (inside.height - h) / 2, w, h)
}

/// The about sheet on top (as high as its text, leaving the card at least
/// [`CARD_LEAST`] rows or two thirds), then the card with the choices: read
/// first, then choose. Each sits under its tab row.
fn areas(body: Rect, about_rows: u16) -> (Rect, Rect) {
    let f = frame(body);
    // The choices come first: they keep [`CARD_LEAST`] rows where the frame
    // has them; the about sheet has the rest, and a third at least.
    let about = (about_rows + 1).min((f.height / 3).max(f.height.saturating_sub(CARD_LEAST))).max(2);
    // A blank row between the sheet and the card's tab, so they read apart.
    let [_, sheet, _, _, card] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(about),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(f);
    (card, sheet)
}

/// How many rows the sheet's text takes at the frame's width.
fn about_rows(host: &Host, theme: &Theme, body: Rect) -> u16 {
    let f = frame(body);
    let width = f.width.saturating_sub(4);
    let card = Rect { height: f.height, ..f };
    let view = step_view(host, card);
    let lines = sheet_lines(host, theme, view.as_ref(), width);
    ratatui::widgets::Paragraph::new(lines)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .line_count(width)
        .min(u16::MAX as usize) as u16
}

/// A tab over a panel (` choose `) on `ground`, and the panel filled with
/// `fill`; the inside, two cells in and a row down.
fn tabbed(theme: &Theme, area: Rect, title: &str, ground: Slot, fill: Slot, buf: &mut Buffer) -> Rect {
    let tab = Rect { y: area.y.saturating_sub(1), height: 1, ..area };
    let words = format!(" {title} ");
    let w = (ui::width(&words) as u16).min(tab.width);
    let style = theme
        .bold(Slot::BgBase)
        .patch(ratatui::style::Style::new().bg(theme.s(ground).fg.unwrap_or_default()));
    buf.set_stringn(tab.x, tab.y, &words, w as usize, style);
    buf.set_style(area, theme.bg(fill));
    Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(1),
    }
}

/// ` radar  setup / look & terminal          step 1 of 5   read-only `
pub(super) fn top_bar(host: &Host, theme: &Theme, chrome: &ChromeView, setup: &SetupChrome, area: Rect, buf: &mut Buffer) {
    buf.set_style(area, theme.bg(Slot::BgOverlay));
    let mut left = super::brand_spans(theme, &chrome.top_bar.brand, false);
    left.push(Span::styled("  setup / ", theme.s(Slot::TextMuted)));
    left.push(Span::styled(setup.title.clone(), theme.s(Slot::TextEmphasis)));
    let left = Line::from(left);
    let bar = &chrome.top_bar;
    let mode = Span::styled(format!(" {} ", bar.mode.text), ink_style(theme, bar.mode.ink, bar.mode.strong));
    let mut right = Vec::new();
    if let Some(step) = &setup.step {
        right.push(Span::styled(format!("{step}  "), theme.s(Slot::TextMuted)));
    }
    let mode_w = mode.width() as u16;
    right.push(mode);
    let right = Line::from(right);
    let right_w = (right.width() as u16).min(area.width);
    buf.set_line(area.x, area.y, &left, area.width.saturating_sub(right_w));
    let x = area.right().saturating_sub(right_w);
    buf.set_line(x, area.y, &right, right_w);
    let label = Rect { x: area.right().saturating_sub(mode_w), width: mode_w.min(area.width), ..area };
    if let Some(hit) = host.chrome_hit(&bar.mode.target) {
        host.hits.borrow_mut().push((crate::view_tui::padded(label, area), hit));
    }
}

/// The card and the sheet's view for the step showing: a host step's own,
/// else the part's section.
fn step_view(host: &Host, card: Rect) -> Option<crate::view::SectionView> {
    if host.setup_host_step() {
        return host.setup_section_view();
    }
    match host.current_section()? {
        (Owner::Plugin(p), s) => host.section_view_in(p, s.id, card.width.saturating_sub(2), card.height.saturating_sub(2)),
        _ => None,
    }
}

/// The sheet's lines at `width` (inside its panel).
fn sheet_lines(host: &Host, theme: &Theme, view: Option<&crate::view::SectionView>, width: u16) -> Vec<Line<'static>> {
    match view {
        Some(view) => section_detail_lines(theme, view, width),
        None if host.on_icons() => host.colours.icons_detail_lines(theme),
        None => host.colours.detail_lines(theme),
    }
}

/// How far the sheet scrolls at this size (`Host::settings_detail`).
pub(super) fn measure(host: &Host, theme: &Theme, body: Rect) {
    let (card, sheet) = areas(body, about_rows(host, theme, body));
    let det = Rect { x: sheet.x + 2, y: sheet.y + 1, width: sheet.width.saturating_sub(4), height: sheet.height.saturating_sub(1) };
    if host.capturing() {
        host.settings_detail.top();
    }
    let view = step_view(host, card);
    let lines = sheet_lines(host, theme, view.as_ref(), det.width);
    let text = det;
    if text.is_empty() {
        host.settings_detail.fit(0, 0);
        return;
    }
    let total = ratatui::widgets::Paragraph::new(lines)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .line_count(text.width)
        .min(u16::MAX as usize) as u16;
    host.settings_detail.fit(text.height, total);
}

/// The body under the top bar: the rail, the card, the sheet.
pub(super) fn body(
    host: &Host,
    theme: &Theme,
    setup: &SetupChrome,
    body: Rect,
    buf: &mut Buffer,
    cursor: &std::cell::Cell<Option<ratatui::layout::Position>>,
) {
    let (card, sheet) = areas(body, about_rows(host, theme, body));
    buf.set_style(body, theme.bg(Slot::BgBase));
    host.hits.borrow_mut().push((card, Hit::Pane(Pane::Middle)));
    let view = step_view(host, card);
    let about = view.as_ref().map_or_else(|| host.colours.detail_title(), |v| v.detail_title.clone());
    let inner = tabbed(theme, card, &setup.title, Slot::AccentPrimary, Slot::BgSurface, buf);
    // The card's heading and the line under it, then a blank row.
    let mut lead_rows = 0;
    for (words, slot) in [(&setup.heading, Slot::TextEmphasis), (&setup.lead, Slot::TextMuted)] {
        if !words.is_empty() && lead_rows < inner.height {
            let style = if slot == Slot::TextEmphasis { theme.bold(slot) } else { theme.s(slot) };
            buf.set_stringn(inner.x, inner.y + lead_rows, words, inner.width as usize, style);
            lead_rows += 1;
        }
    }
    let skip = if lead_rows > 0 { lead_rows + 1 } else { 0 };
    let mid = Rect { y: inner.y + skip.min(inner.height), height: inner.height.saturating_sub(skip), ..inner };
    let det = tabbed(theme, sheet, &about, Slot::GlowCyan, Slot::BgOverlay, buf);
    host.frontend.mark_selectable(mid, false);
    host.frontend.mark_selectable(det, true);
    if host.capturing() {
        host.settings_detail.top();
    }
    let lines = sheet_lines(host, theme, view.as_ref(), det.width);
    crate::ui::scroll::paragraph(theme, &host.settings_detail, sheet, det, buf, lines);
    host.frontend.set_lead(ui::icon::lead_width(theme));
    let plugin_hits = std::cell::RefCell::new(Vec::new());
    let me = match host.current_section() {
        Some((Owner::Plugin(p), _)) if !host.setup_host_step() => Some(p),
        _ => None,
    };
    let cx = Cx {
        theme,
        now: host.now,
        focused: true,
        focus: Pane::Middle,
        bar: false,
        tabs: false,
        cursor,
        hits: &plugin_hits,
        parts: Parts::new(&host.plugins, me),
    };
    match (&view, host.current_section()) {
        (Some(view), _) if host.setup_host_step() => {
            crate::view_tui::render_section(&host.frontend, view, mid, buf, &cx);
            let mut hits = host.hits.borrow_mut();
            for (rect, _, id) in plugin_hits.into_inner() {
                if id != crate::view_tui::PANE_HIT {
                    hits.push((rect, Hit::Setup(id)));
                }
            }
        }
        (Some(view), Some((Owner::Plugin(p), s))) => {
            crate::view_tui::render_section(&host.frontend, view, mid, buf, &cx);
            adopt_hits(host, p, s.id, plugin_hits);
        }
        (None, Some((Owner::Host, _))) => {
            let rows = if host.on_icons() {
                host.colours.icons_render(theme, mid, buf, true, cursor)
            } else {
                host.colours.render(theme, mid, buf, true, cursor)
            };
            host.hits.borrow_mut().extend(rows.into_iter().map(|(r, i)| (r, Hit::ColourRow(i))));
        }
        _ => widgets::empty(buf, mid, theme, "This step has nothing to show."),
    }
}
