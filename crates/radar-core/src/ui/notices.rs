//! Transient notice cells and the inert, session-only recent-notice reader.
//! Paint this layer after retained interaction motion, never into a clean frame.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::icon::{self, Icon};
use crate::design::theme::{Slot, Tier};
use crate::theme::{Theme, mix};
use crate::view::chrome::{NoticeHistoryView, NoticeTone, NoticeView};

const MAX_VISIBLE: usize = 4;
const MAX_WIDTH: u16 = 64;
const TINT_PER_MILLE: u32 = 180;

#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub area: Rect,
    pub notice: NoticeView,
    pub older: bool,
    pub fill: u16,
    pub hidden: u64,
}
#[derive(Debug, Clone)]
pub struct Summary {
    pub area: Rect,
    pub hidden: u64,
}
#[derive(Debug, Clone, Default)]
pub struct Stack {
    pub toasts: Vec<Toast>,
    pub summary: Option<Summary>,
}

/// Newest at the bottom-right inside the content pane. Overflow always has a
/// visible count; in a one-row pane the newest notice carries that count.
pub fn layout(
    notices: &[NoticeView],
    hidden: u64,
    content: Rect,
    now_ms: u64,
    motion: bool,
) -> Stack {
    if content.is_empty() {
        return Stack::default();
    }
    let active: Vec<_> = notices
        .iter()
        .filter(|n| now_ms.saturating_sub(n.created_ms) < n.lifetime_ms)
        .collect();
    if active.is_empty() {
        return Stack::default();
    }
    let mut count = active
        .len()
        .min(MAX_VISIBLE)
        .min(usize::from(content.height));
    let overflow = hidden.saturating_add((active.len() - count) as u64);
    if overflow > 0 && content.height >= 2 {
        count = count.min(usize::from(content.height - 1));
    }
    let hidden = hidden.saturating_add((active.len() - count) as u64);
    let width = content.width.min(MAX_WIDTH);
    let summary = (hidden > 0 && content.height >= 2).then(|| Summary {
        area: Rect::new(
            content.right() - width,
            content.bottom() - count as u16 - 1,
            width,
            1,
        ),
        hidden,
    });
    let toasts = active[active.len() - count..]
        .iter()
        .enumerate()
        .map(|(i, notice)| {
            let remaining = notice
                .lifetime_ms
                .saturating_sub(now_ms.saturating_sub(notice.created_ms));
            let fill = if motion {
                (u128::from(width) * u128::from(remaining)).div_ceil(u128::from(notice.lifetime_ms))
                    as u16
            } else {
                width
            };
            Toast {
                id: notice.id,
                area: Rect::new(
                    content.right() - width,
                    content.bottom() - count as u16 + i as u16,
                    width,
                    1,
                ),
                notice: (*notice).clone(),
                older: i + 1 < count,
                fill,
                hidden: if summary.is_none() && i + 1 == count {
                    hidden
                } else {
                    0
                },
            }
        })
        .collect();
    Stack { toasts, summary }
}

fn tone_slot(tone: NoticeTone) -> Slot {
    match tone {
        NoticeTone::Info => Slot::StatusInfo,
        NoticeTone::Error => Slot::StatusError,
    }
}
fn tone_word(tone: NoticeTone) -> &'static str {
    match tone {
        NoticeTone::Info => "info",
        NoticeTone::Error => "error",
    }
}
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Fit complete graphemes, not scalar values, so emoji sequences and combining
/// marks stay attached. Every result occupies exactly the requested cells.
fn fit(text: &str, cells: usize) -> String {
    if super::width(text) <= cells {
        return super::pad(text, cells);
    }
    if cells == 0 {
        return String::new();
    }
    let span = Span::raw(text);
    let mut out = String::new();
    let mut used = 0;
    for g in span.styled_graphemes(Style::default()) {
        let w = super::width(g.symbol);
        if used + w > cells - 1 {
            break;
        }
        out.push_str(g.symbol);
        used += w;
    }
    out.push('…');
    super::pad(&out, cells)
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = one_line(paragraph);
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() && super::width(&line) + 1 + super::width(word) > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            let span = Span::raw(word);
            for g in span.styled_graphemes(Style::default()) {
                let w = super::width(g.symbol);
                if super::width(&line) + w > width && !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                }
                if w > width {
                    line.push('…');
                } else {
                    line.push_str(g.symbol);
                }
            }
        }
        out.push(line);
    }
    out
}

fn prefix(theme: &Theme, notice: &NoticeView) -> (String, Style) {
    let default = match notice.tone {
        NoticeTone::Info => Icon::Info,
        NoticeTone::Error => Icon::Error,
    };
    let mark = icon::span(theme, notice.icon.unwrap_or(default));
    if mark.content.is_empty() {
        (
            format!("{} ", tone_word(notice.tone)),
            theme.s(tone_slot(notice.tone)),
        )
    } else {
        (format!("{} ", mark.content), mark.style)
    }
}
fn row(theme: &Theme, notice: &NoticeView, width: u16, style: Style) -> Line<'static> {
    if width <= 2 {
        return Line::styled(" ".repeat(usize::from(width)), style);
    }
    let room = usize::from(width - 2);
    let (prefix, mark) = prefix(theme, notice);
    let prefix_width = super::width(&prefix);
    let mut spans = vec![Span::styled(" ", style)];
    if prefix_width >= room {
        spans.push(Span::styled(fit(&prefix, room), style.patch(mark)));
    } else {
        spans.push(Span::styled(prefix, style.patch(mark)));
        spans.push(Span::styled(
            fit(&one_line(&notice.text), room - prefix_width),
            style,
        ));
    }
    spans.push(Span::styled(" ", style));
    Line::from(spans)
}

fn clear(buf: &mut Buffer, area: Rect, style: Style) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    for y in area.y..area.bottom() {
        // Erase an exposed half outside the rectangle without changing its style.
        if area.x > buf.area.x && super::width(buf[(area.x - 1, y)].symbol()) > 1 {
            buf[(area.x - 1, y)].set_symbol(" ");
        }
        if area.right() < buf.area.right() && super::width(buf[(area.right() - 1, y)].symbol()) > 1
        {
            buf[(area.right(), y)].set_symbol(" ");
        }
        for x in area.x..area.right() {
            buf[(x, y)].reset();
            buf[(x, y)].set_style(style);
        }
    }
}
fn draw_line(buf: &mut Buffer, area: Rect, line: &Line<'_>) {
    if area.is_empty() {
        return;
    }
    buf.set_line(area.x, area.y, line, area.width);
    // Keep wide tails explicit and identically styled for later compositors.
    let mut x = area.x;
    while x < area.right() {
        let w = super::width(buf[(x, area.y)].symbol()).max(1) as u16;
        let style = buf[(x, area.y)].style();
        for tail in x + 1..x.saturating_add(w).min(area.right()) {
            buf[(tail, area.y)].set_symbol("").set_style(style);
        }
        x = x.saturating_add(w);
    }
}
fn tint(theme: &Theme, tone: NoticeTone) -> Option<Color> {
    if theme.tier() != Tier::TrueColor {
        return None;
    }
    match (theme.s(Slot::BgOverlay).fg, theme.s(tone_slot(tone)).fg) {
        (Some(Color::Rgb(r, g, b)), Some(Color::Rgb(tr, tg, tb))) => {
            Some(mix((r, g, b), (tr, tg, tb), TINT_PER_MILLE))
        }
        _ => None,
    }
}

/// Paint after motion; returned rectangles consume a click by occurrence ID.
pub fn paint(stack: &Stack, theme: &Theme, buf: &mut Buffer) -> Vec<(Rect, u64)> {
    if let Some(summary) = &stack.summary {
        let area = summary.area.intersection(buf.area);
        if !area.is_empty() {
            let mut style = theme.s(Slot::TextMuted).patch(theme.bg(Slot::BgOverlay));
            if theme.tier() == Tier::Mono {
                style = style.add_modifier(Modifier::REVERSED);
            }
            clear(buf, area, style);
            let text = format!("+{} earlier notices · ~ history", summary.hidden);
            draw_line(
                buf,
                area,
                &Line::styled(fit(&text, usize::from(area.width)), style),
            );
        }
    }
    let mut hits = Vec::new();
    for toast in &stack.toasts {
        let area = toast.area.intersection(buf.area);
        if area.is_empty() {
            continue;
        }
        let slot = if theme.tier() == Tier::TrueColor {
            Slot::TextPrimary
        } else {
            tone_slot(toast.notice.tone)
        };
        let mut style = theme.s(slot).patch(theme.bg(Slot::BgOverlay));
        if toast.older {
            style = style.add_modifier(Modifier::DIM);
        } else if toast.notice.tone == NoticeTone::Error {
            style = style.add_modifier(Modifier::BOLD);
        }
        if theme.tier() == Tier::Mono {
            style = style.add_modifier(Modifier::REVERSED);
        }
        let mut notice = toast.notice.clone();
        if toast.hidden > 0 {
            notice.text = format!("+{} more · {}", toast.hidden, notice.text);
        }
        clear(buf, area, style);
        draw_line(buf, area, &row(theme, &notice, area.width, style));
        if let Some(ground) = tint(theme, notice.tone) {
            let mut end = area.x.saturating_add(toast.fill.min(area.width));
            if end > area.x
                && end < area.right()
                && super::width(buf[(end - 1, area.y)].symbol()) > 1
            {
                end -= 1;
            }
            for x in area.x..end {
                buf[(x, area.y)].set_bg(ground);
            }
        }
        hits.push((area, toast.id));
    }
    hits
}

#[derive(Debug, Clone, Copy)]
pub struct HistoryLayout {
    pub heading: Rect,
    pub subtitle: Rect,
    pub list: Rect,
    pub detail_heading: Rect,
    pub detail: Rect,
}
pub fn history_layout(body: Rect) -> HistoryLayout {
    let x = body.x.saturating_add(1).min(body.right());
    let width = body.width.saturating_sub(2);
    let line = |offset: u16| {
        Rect::new(
            x,
            body.y.saturating_add(offset).min(body.bottom()),
            width,
            u16::from(offset < body.height),
        )
    };
    let start = body.height.min(3);
    let list_height = ((u32::from(body.height.saturating_sub(5)) * 2 / 3) as u16)
        .max(1)
        .min(body.height - start);
    let detail_at = start + list_height;
    let detail_start = detail_at.saturating_add(1).min(body.height);
    HistoryLayout {
        heading: line(0),
        subtitle: line(1),
        list: Rect::new(x, body.y + start, width, list_height),
        detail_heading: line(detail_at),
        detail: Rect::new(x, body.y + detail_start, width, body.height - detail_start),
    }
}

/// An explicit overlay, not a provider menu. Reading a row can never replay it.
pub fn history(
    theme: &Theme,
    view: &NoticeHistoryView,
    body: Rect,
    buf: &mut Buffer,
) -> Vec<(Rect, u64)> {
    let body = body.intersection(buf.area);
    let layout = history_layout(body);
    let plain = theme.s(Slot::TextPrimary).patch(theme.bg(Slot::BgBase));
    clear(buf, body, plain);
    draw_line(
        buf,
        layout.heading,
        &Line::styled("Recent notices", theme.bold(Slot::TextEmphasis)),
    );
    let omitted = if view.omitted == 0 {
        String::new()
    } else {
        format!(" · {} older omitted", view.omitted)
    };
    draw_line(
        buf,
        layout.subtitle,
        &Line::styled(
            format!(
                "This session only · {} retained{omitted}",
                view.entries.len()
            ),
            theme.s(Slot::TextMuted),
        ),
    );
    if view.entries.is_empty() {
        draw_line(
            buf,
            Rect {
                height: layout.list.height.min(1),
                ..layout.list
            },
            &Line::styled("No notices this session.", theme.s(Slot::TextMuted)),
        );
        return Vec::new();
    }
    let selected = view.selected.min(view.entries.len() - 1);
    let height = usize::from(layout.list.height);
    let mut scroll = view.scroll.min(view.entries.len().saturating_sub(height));
    if selected < scroll {
        scroll = selected;
    } else if height > 0 && selected >= scroll + height {
        scroll = selected + 1 - height;
    }
    let mut hits = Vec::new();
    for (i, notice) in view
        .entries
        .iter()
        .rev()
        .enumerate()
        .skip(scroll)
        .take(height)
    {
        let area = Rect::new(
            layout.list.x,
            layout.list.y + (i - scroll) as u16,
            layout.list.width,
            1,
        );
        let mut style = if i == selected {
            theme
                .bold(Slot::TextEmphasis)
                .patch(theme.bg(Slot::BgSelection))
        } else {
            plain
        };
        if i == selected && theme.tier() == Tier::Mono {
            style = style.add_modifier(Modifier::REVERSED);
        }
        clear(buf, area, style);
        draw_line(buf, area, &row(theme, notice, area.width, style));
        hits.push((area, notice.id));
    }
    if let Some(detail) = &view.detail {
        let lines = wrap(&detail.text, usize::from(layout.detail.width));
        let height = usize::from(layout.detail.height);
        let top = view.detail_scroll.min(lines.len().saturating_sub(height));
        let mut heading = format!("Notice details · {}", tone_word(detail.tone));
        if lines.len() > height && height > 0 {
            heading.push_str(&format!(
                " · lines {}–{} of {}",
                top + 1,
                (top + height).min(lines.len()),
                lines.len()
            ));
        }
        draw_line(
            buf,
            layout.detail_heading,
            &Line::styled(heading, theme.s(Slot::TextSecondary)),
        );
        for (i, line) in lines.iter().skip(top).take(height).enumerate() {
            draw_line(
                buf,
                Rect::new(
                    layout.detail.x,
                    layout.detail.y + i as u16,
                    layout.detail.width,
                    1,
                ),
                &Line::styled(line.clone(), plain),
            );
        }
    }
    hits
}

#[cfg(test)]
mod tests;
