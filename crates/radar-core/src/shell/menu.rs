//! Measured modal content. Only visible paragraphs are painted; a long
//! confirmation never needs a document-sized cell buffer.

use std::cell::Cell;
use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use tui_popup::{KnownSize, Popup};

use crate::design::theme::Slot;
use crate::host::{Hit, Host, MenuViewport};
use crate::motion::{self, Region};
use crate::view::{Choice as MenuItem, MenuView, Target};
use crate::theme::Theme;
use crate::ui::icon::{self, Icon};
use crate::{ui, widgets};

struct Part {
    text: Paragraph<'static>,
    prefix: Line<'static>,
    inset: u16,
    rows: Range<usize>,
    item: Option<usize>,
    selected: bool,
}

struct Choice {
    item: usize,
    lines: [Line<'static>; 3],
    area: Rect,
    raised: bool,
}

/// All coordinates are measured once, before the status line and the
/// popup are drawn, so hints, pixels, and hits agree on the first frame.
pub(super) struct Layout {
    frame: Rect,
    body: Rect,
    field: Option<Rect>,
    title: Line<'static>,
    parts: Vec<Part>,
    choices: Vec<Choice>,
    usable: bool,
    /// Each row's target, as the view gives it: what a pointer chooses.
    targets: Vec<Option<Target>>,
    /// Lines of content, the body's width, and whether it is a question.
    total: usize,
    width: u16,
    question: bool,
}

fn lead(theme: &Theme, item: &MenuItem) -> Option<Span<'static>> {
    item.icon
        .map(|i| icon::lead(theme, i))
        .filter(|s| !s.content.is_empty())
}

fn row_style(theme: &Theme, item: &MenuItem, selected: bool) -> Style {
    if selected {
        theme
            .bold(Slot::TextEmphasis)
            .patch(theme.bg(Slot::BgSelection))
    } else {
        theme.s(if item.muted {
            Slot::TextMuted
        } else {
            Slot::TextPrimary
        })
    }
}

fn compact_line(
    theme: &Theme,
    item: &MenuItem,
    selected: bool,
    key_w: usize,
    label_w: usize,
) -> Line<'static> {
    let row = row_style(theme, item, selected);
    let bg = row.bg.map_or(Style::new(), |c| Style::new().bg(c));
    let mut spans = vec![Span::styled(
        if key_w == 0 {
            " ".to_string()
        } else {
            format!(" {} ", ui::pad(&item.keys, key_w))
        },
        theme.bold(Slot::AccentSecondary).patch(bg),
    )];
    if let Some(spec) = &item.sample {
        spans.push(Span::styled(" ██", theme.spec(spec).patch(bg)));
    }
    match lead(theme, item) {
        Some(mut icon) => {
            let w = label_w.saturating_sub(icon.width());
            icon.style = icon.style.patch(bg);
            spans.push(Span::styled(" ", row));
            spans.push(icon);
            spans.push(Span::styled(ui::pad(&item.label, w), row));
        }
        None => spans.push(Span::styled(
            format!(" {}", ui::pad(&item.label, label_w)),
            row,
        )),
    }
    spans.push(Span::styled(
        format!("   {} ", item.hint),
        theme.s(Slot::TextMuted).patch(bg),
    ));
    Line::from(spans)
}

/// Split explicit newlines without normalizing the text's spaces or case.
fn text_lines(text: &str, style: Style) -> Vec<Line<'static>> {
    text.split('\n')
        .map(|s| Line::styled(s.trim_end_matches('\r').to_string(), style))
        .collect()
}

/// Paragraph scroll offsets are u16. Split exceptionally long logical
/// lines into source-sized chunks, on grapheme boundaries, before counting
/// rows. Every local paragraph then fits that coordinate type even at one
/// column. This allocates text, never width × document-height cells.
fn bounded_lines(line: Line<'static>) -> Vec<Line<'static>> {
    const MAX_CELLS: usize = 32_768;
    if line.width() <= MAX_CELLS {
        return vec![line];
    }
    let mut out = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cells = 0;
    for g in line.styled_graphemes(Style::default()) {
        let w = ui::width(g.symbol);
        if cells + w > MAX_CELLS {
            out.push(Line::from(std::mem::take(&mut spans)));
            cells = 0;
        }
        if let Some(last) = spans.last_mut().filter(|last| last.style == g.style) {
            last.content.to_mut().push_str(g.symbol);
        } else {
            spans.push(Span::styled(g.symbol.to_string(), g.style));
        }
        cells += w;
    }
    if !spans.is_empty() {
        out.push(Line::from(spans));
    }
    out
}

struct Content {
    width: u16,
    parts: Vec<Part>,
    total: usize,
}

impl Content {
    fn push(
        &mut self,
        lines: Vec<Line<'static>>,
        inset: u16,
        prefix: Line<'static>,
        item: Option<usize>,
        selected: bool,
    ) {
        let inset = inset.min(self.width.saturating_sub(2));
        let width = self.width.saturating_sub(inset);
        let mut prefix = Some(prefix);
        for line in lines.into_iter().flat_map(bounded_lines) {
            let text = Paragraph::new(line).wrap(Wrap { trim: false });
            let height = text.line_count(width).max(1);
            let end = self.total.saturating_add(height);
            self.parts.push(Part {
                text,
                prefix: prefix.take().unwrap_or_default(),
                inset,
                rows: self.total..end,
                item,
                selected,
            });
            self.total = end;
        }
    }
}

impl Layout {
    /// The menu's box, border included: what the scrim leaves and the
    /// shadow falls from.
    pub(super) fn frame(&self) -> Rect {
        self.frame
    }

    /// Lay out the menu `view` (the host's view data: rows, field, where it
    /// opened, how far it is scrolled) in `area`. Pure: the host measures
    /// with it ([`measure`]) and the shell paints with it.
    pub(super) fn new(view: &MenuView, theme: &Theme, area: Rect) -> Self {
        let max_w = area.width.saturating_sub(2);
        let max_h = area.height.saturating_sub(2);
        let items = &view.items;
        let sel = view.selected.unwrap_or(0);
        let question = view.question;
        let title_icon = match view.id.as_str() {
            _ if question => Some(Icon::Warning),
            crate::host::BULK_MENU => Some(Icon::Bulk),
            "keys" => Some(Icon::Menu),
            "quit" => Some(Icon::Quit),
            _ => None,
        };
        let mut title = vec![Span::styled(" ", theme.bold(Slot::TextEmphasis))];
        for icon in title_icon.into_iter().chain(view.icon) {
            title.push(icon::lead(theme, icon));
        }
        title.push(Span::styled(
            format!("{} ", view.title),
            theme.bold(Slot::TextEmphasis),
        ));
        let title = Line::from(title);
        let key_w = items.iter().map(|i| ui::width(&i.keys)).max().unwrap_or(0);
        let label_w = items
            .iter()
            .filter(|i| !i.heading)
            .map(|i| ui::width(&i.label) + lead(theme, i).map_or(0, |s| s.width()))
            .max()
            .unwrap_or(0);
        // Measure before padding. One exceptionally long label must not
        // cause every other row to allocate that label's natural width.
        let compact_widths: Vec<usize> = items
            .iter()
            .map(|item| {
                (if key_w == 0 {
                    1
                } else {
                    key_w.saturating_add(2)
                })
                .saturating_add(if item.sample.is_some() { 3 } else { 0 })
                .saturating_add(1)
                .saturating_add(label_w)
                .saturating_add(4)
                .saturating_add(ui::width(&item.hint))
            })
            .collect();
        let buttons: Vec<(usize, widgets::Button)> = if question {
            items
                .iter()
                .enumerate()
                .filter(|(_, i)| !i.heading && !i.note)
                .map(|(i, item)| {
                    (
                        i,
                        widgets::Button {
                            keys: item.keys.clone(),
                            icon: lead(theme, item),
                            words: if item.hint.is_empty() {
                                item.label.clone()
                            } else {
                                item.hint.clone()
                            },
                            focused: i == sel,
                            muted: item.muted,
                        },
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        let natural = compact_widths
            .iter()
            .enumerate()
            .filter(|(i, _)| !items[*i].heading && !question)
            .map(|(_, w)| *w)
            .chain(
                items
                    .iter()
                    .filter(|i| i.heading)
                    .map(|i| 1 + ui::width(&i.label) + lead(theme, i).map_or(0, |s| s.width())),
            )
            .chain([title.width(), if view.typed.is_some() { 58 } else { 0 }])
            .max()
            .unwrap_or(1);
        let button_width = button_row_width(&buttons, max_w);
        let width = natural.max(button_width).min(usize::from(max_w)) as u16;
        let columns_fit = compact_widths.iter().enumerate().all(|(i, w)| {
            items[i].heading
                || (*w <= usize::from(width)
                    && !items[i].label.contains('\n')
                    && !items[i].hint.contains('\n'))
        });
        let long_title = title.width() > usize::from(max_w) || view.title.contains('\n');
        let mut content = Content {
            width,
            parts: Vec::new(),
            total: 0,
        };
        if long_title {
            let mut lines = text_lines(&view.title, theme.bold(Slot::TextEmphasis));
            if let Some(first) = lines.first_mut() {
                let mut icons: Vec<Span<'static>> = title_icon
                    .into_iter()
                    .chain(view.icon)
                    .map(|i| icon::lead(theme, i))
                    .collect();
                icons.append(&mut first.spans);
                first.spans = icons;
            }
            content.push(lines, 1, Line::default(), None, false);
        }
        for (i, item) in items.iter().enumerate() {
            if item.heading || item.note {
                let mut lines = if item.note {
                    text_lines(&item.label, theme.s(Slot::TextPrimary))
                } else {
                    item.label
                        .split('\n')
                        .map(|s| widgets::heading(theme, s))
                        .collect()
                };
                if let Some(first) = lines.first_mut()
                    && let Some(icon) = lead(theme, item)
                {
                    first.spans.insert(0, icon);
                }
                content.push(lines, 1, Line::default(), None, false);
            } else if !question {
                if columns_fit {
                    content.push(
                        vec![compact_line(theme, item, i == sel, key_w, label_w)],
                        0,
                        Line::default(),
                        Some(i),
                        i == sel,
                    );
                } else {
                    let row = row_style(theme, item, i == sel);
                    let bg = row.bg.map_or(Style::new(), |c| Style::new().bg(c));
                    // Leave room for at least one two-cell grapheme. At
                    // tiny widths the key becomes its own wrapped block.
                    let prefix_w = if key_w == 0 {
                        2
                    } else {
                        key_w.saturating_add(3)
                    };
                    let inset = if prefix_w + 2 <= usize::from(width) {
                        prefix_w as u16
                    } else {
                        0
                    };
                    let key_style = theme.bold(Slot::AccentSecondary).patch(bg);
                    let prefix = if inset == 0 {
                        Line::default()
                    } else {
                        Line::styled(
                            if key_w == 0 {
                                "  ".to_string()
                            } else {
                                format!(" {}  ", ui::pad(&item.keys, key_w))
                            },
                            key_style,
                        )
                    };
                    if inset == 0 && !item.keys.is_empty() {
                        content.push(
                            vec![Line::styled(item.keys.clone(), key_style)],
                            0,
                            Line::default(),
                            Some(i),
                            i == sel,
                        );
                    }
                    let mut lines = text_lines(&item.label, row);
                    if let Some(first) = lines.first_mut() {
                        if let Some(mut icon) = lead(theme, item) {
                            icon.style = icon.style.patch(bg);
                            first.spans.insert(0, icon);
                        }
                        if let Some(spec) = &item.sample {
                            first
                                .spans
                                .insert(0, Span::styled("██ ", theme.spec(spec).patch(bg)));
                        }
                    }
                    content.push(
                        lines,
                        inset,
                        if inset > 0 { prefix } else { Line::default() },
                        Some(i),
                        i == sel,
                    );
                    if !item.hint.is_empty() {
                        content.push(
                            text_lines(&item.hint, theme.s(Slot::TextMuted).patch(bg)),
                            inset,
                            Line::default(),
                            Some(i),
                            i == sel,
                        );
                    }
                }
            }
        }
        let field_h = usize::from(view.typed.is_some());
        let min_body = usize::from(content.total > 0);
        let (mut choices, mut footer_h) = choices(theme, &buttons, width, true);
        if footer_h + field_h + min_body > usize::from(max_h) {
            (choices, footer_h) = self::choices(theme, &buttons, width, false);
        }
        let min_width = if view.typed.is_some() { 4 } else { 2 };
        let usable =
            width >= min_width && usize::from(max_h) >= footer_h + field_h + min_body && max_h > 0;
        let gap = usize::from(
            content.total > 0 && footer_h > 0 && usize::from(max_h) > footer_h + field_h + min_body,
        );
        let body_h = content
            .total
            .min(usize::from(max_h).saturating_sub(footer_h + field_h + gap));
        let height = if usable {
            (body_h + field_h + gap + footer_h) as u16
        } else {
            max_h
        };
        let frame_w = width.saturating_add(2).min(area.width);
        let frame_h = height.saturating_add(2).min(area.height);
        let frame = Rect {
            x: view.at.map_or_else(
                || area.x + area.width.saturating_sub(frame_w) / 2,
                |at| at.column.clamp(area.x, area.right().saturating_sub(frame_w)),
            ),
            y: view.at.map_or_else(
                || area.y + area.height.saturating_sub(frame_h) / 2,
                |at| at.row.clamp(area.y, area.bottom().saturating_sub(frame_h)),
            ),
            width: frame_w,
            height: frame_h,
        };
        let inner = Rect::new(
            frame.x.saturating_add(1),
            frame.y.saturating_add(1),
            width,
            height,
        );
        let body = Rect::new(
            inner.x,
            inner.y.saturating_add(field_h as u16),
            width,
            body_h as u16,
        );
        let field = view.typed.as_ref().map(|_| {
            Rect::new(
                inner.x.saturating_add(1),
                inner.y,
                width.saturating_sub(2),
                1,
            )
        });
        for choice in &mut choices {
            choice.area.x = choice.area.x.saturating_add(inner.x);
            choice.area.y = choice
                .area
                .y
                .saturating_add(inner.y + (field_h + body_h + gap) as u16);
        }
        Self {
            frame,
            body,
            field,
            title: if long_title { Line::default() } else { title },
            parts: content.parts,
            choices,
            usable,
            targets: items.iter().map(|i| i.target.clone()).collect(),
            total: content.total,
            width,
            question,
        }
    }

    /// Where the menu reads from after this layout: `at` (the reading
    /// position so far) kept so the selected row `sel` is in sight when the
    /// selection or the size changed. Questions begin at their first words:
    /// their choice stays visible in the fixed footer.
    fn scroll_for(&self, at: usize, sel: usize, selection_changed: bool, resized: bool) -> usize {
        let body_h = usize::from(self.body.height);
        let max = self.total.saturating_sub(body_h);
        let mut at = at.min(max);
        if !self.question && (selection_changed || resized) {
            let range = self
                .parts
                .iter()
                .filter(|p| p.item == Some(sel))
                .fold(None, |range: Option<Range<usize>>, p| {
                    Some(range.map_or_else(|| p.rows.clone(), |r| r.start..p.rows.end))
                });
            if let Some(range) = range {
                if range.len() > body_h {
                    if selection_changed || range.end <= at || range.start >= at + body_h {
                        at = range.start;
                    }
                } else if range.start < at {
                    at = range.start;
                } else if range.end > at + body_h {
                    at = range.end.saturating_sub(body_h);
                }
            }
        }
        at.min(max)
    }
}

/// Measure the menu `view` laid out in `area` and keep the host's reading
/// state for it: how many lines it has, how many show, whether its controls
/// are usable, and where it reads from (the selected row kept in sight).
/// The host calls this while it builds the chrome, so every frontend gets
/// the same [`MenuView`] scroll and paging.
pub(crate) fn measure(view: &MenuView, viewport: &MenuViewport, theme: &Theme, area: Rect) {
    let layout = Layout::new(view, theme, area);
    let body_h = usize::from(layout.body.height);
    let sel = view.selected.unwrap_or(0);
    let resized = viewport.width.replace(layout.width) != layout.width || viewport.height.get() != body_h;
    let selection_changed = viewport.selected.replace(Some(sel)) != Some(sel);
    viewport.height.set(body_h);
    viewport.total.set(layout.total);
    viewport.usable.set(layout.usable);
    viewport.at.set(layout.scroll_for(viewport.at.get(), sel, selection_changed, resized));
}

fn button_row_width(buttons: &[(usize, widgets::Button)], width: u16) -> usize {
    let mut current = 0;
    let mut widest = 0;
    for (_, button) in buttons {
        let w = button.width().min(usize::from(width));
        if current > 0 && current + 2 + w > usize::from(width) {
            widest = widest.max(current);
            current = 0;
        }
        current += usize::from(current > 0) * 2 + w;
    }
    widest.max(current)
}

fn choices(
    theme: &Theme,
    buttons: &[(usize, widgets::Button)],
    width: u16,
    raised: bool,
) -> (Vec<Choice>, usize) {
    let mut rows: Vec<Vec<Choice>> = Vec::new();
    let mut used = 0u16;
    for (item, button) in buttons {
        let w = button.width().min(usize::from(width)) as u16;
        let (lines, _) = widgets::buttons(theme, std::slice::from_ref(button), 0);
        let caption_h = Paragraph::new(lines[1].clone())
            .wrap(Wrap { trim: false })
            .line_count(w)
            .max(1);
        let height = caption_h
            .saturating_add(if raised { 2 } else { 0 })
            .min(u16::MAX as usize) as u16;
        if rows.is_empty() || usize::from(used) + 2 + usize::from(w) > usize::from(width) {
            rows.push(Vec::new());
            used = 0;
        }
        let gap = if used == 0 { 0 } else { 2 };
        rows.last_mut().expect("a button row").push(Choice {
            item: *item,
            lines,
            area: Rect::new(used.saturating_add(gap), 0, w, height),
            raised,
        });
        used = used.saturating_add(gap).saturating_add(w);
    }
    let mut out = Vec::new();
    let mut y = 0usize;
    for row in rows {
        let end = row.last().map_or(0, |c| c.area.right());
        let pad = width.saturating_sub(end);
        let h = row.iter().map(|c| c.area.height).max().unwrap_or(0);
        for mut choice in row {
            choice.area.x = choice.area.x.saturating_add(pad);
            choice.area.y = y.min(u16::MAX as usize) as u16;
            out.push(choice);
        }
        y = y.saturating_add(usize::from(h));
    }
    (out, y)
}

/// Popup supplies its normal border, clearing, and overlay style. Its
/// body is the already measured visible canvas, not offscreen content.
struct Canvas {
    width: u16,
    height: u16,
}
impl KnownSize for Canvas {
    fn width(&self) -> usize {
        self.width.into()
    }
    fn height(&self) -> usize {
        self.height.into()
    }
}
impl Widget for Canvas {
    fn render(self, _: Rect, _: &mut Buffer) {}
}

/// What a pointer on row `i` chooses: the row's target from the view.
fn hit_of(host: &Host, targets: &[Option<Target>], i: usize) -> Option<Hit> {
    host.chrome_hit(targets.get(i)?.as_ref()?)
}

pub(super) fn render(
    host: &Host,
    view: &MenuView,
    theme: &Theme,
    layout: Layout,
    buf: &mut Buffer,
    cursor: &Cell<Option<Position>>,
) {
    let r = layout.frame;
    if r.is_empty() {
        return;
    }
    Popup::new(Canvas {
        width: r.width.saturating_sub(2),
        height: r.height.saturating_sub(2),
    })
    .title(layout.title)
    .style(theme.bg(Slot::BgOverlay))
    .border_set(ratatui::symbols::border::ROUNDED)
    .border_style(theme.s(Slot::AccentPrimary))
    .render(r, buf);
    motion::region(Region::Popup, r);
    host.hits.borrow_mut().push((r, Hit::MenuBox));
    if !layout.usable {
        Paragraph::new("Make the terminal larger to read this menu.")
            .style(theme.s(Slot::TextMuted))
            .wrap(Wrap { trim: false })
            .render(r.inner(ratatui::layout::Margin::new(1, 1)), buf);
        return;
    }
    // Read from where the view says, as far as it says shows.
    let visible = view.scroll..view.scroll.saturating_add(usize::from(layout.body.height));
    for part in layout.parts {
        let start = part.rows.start.max(visible.start);
        let end = part.rows.end.min(visible.end);
        if start >= end {
            continue;
        }
        let row = Rect::new(
            layout.body.x,
            layout.body.y + (start - visible.start) as u16,
            layout.body.width,
            (end - start) as u16,
        );
        if part.selected {
            buf.set_style(row, theme.bg(Slot::BgSelection));
        }
        let text_area = Rect {
            x: row.x + part.inset,
            width: row.width.saturating_sub(part.inset),
            ..row
        };
        let skip = u16::try_from(start - part.rows.start).expect("source paragraphs fit u16 rows");
        part.text.scroll((skip, 0)).render(text_area, buf);
        if skip == 0 {
            buf.set_line(row.x, row.y, &part.prefix, part.inset);
        }
        if let Some(hit) = part.item.and_then(|i| hit_of(host, &layout.targets, i)) {
            host.hits.borrow_mut().push((row, hit));
        }
    }
    for choice in layout.choices {
        let area = choice.area.intersection(r);
        let edge = u16::from(choice.raised);
        if choice.raised {
            buf.set_line(area.x, area.y, &choice.lines[0], area.width);
            buf.set_line(
                area.x,
                area.bottom().saturating_sub(1),
                &choice.lines[2],
                area.width,
            );
        }
        Paragraph::new(choice.lines[1].clone())
            .style(
                choice.lines[1]
                    .spans
                    .iter()
                    .find_map(|s| s.style.bg)
                    .map_or(Style::default(), |bg| Style::default().bg(bg)),
            )
            .wrap(Wrap { trim: false })
            .render(
                Rect::new(
                    area.x,
                    area.y + edge,
                    area.width,
                    area.height.saturating_sub(edge * 2),
                ),
                buf,
            );
        if let Some(hit) = hit_of(host, &layout.targets, choice.item) {
            host.hits.borrow_mut().push((area, hit));
        }
    }
    if let (Some(text), Some(at)) = (&view.typed, layout.field) {
        let caret = view.caret.unwrap_or_else(|| text.chars().count());
        crate::input::render_field(text, caret, theme, at, buf, cursor);
    }
    let max = view.lines.saturating_sub(view.shown);
    if view.usable && max > 0 && layout.body.height > 0 {
        ui::scroll::scrollbar(theme, ui::scroll::around(layout.body), buf, max, view.scroll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrapped_button_fills_its_whole_face_including_continuation_padding() {
        let host = Host::new(vec![], 0).unwrap();
        let open = crate::host::OpenMenu::open(
            crate::host::Owner::Host,
            crate::Menu::confirm(
                "ask",
                "keep the notes?",
                "Keep these notes until review-end",
                "Leave these notes until leave-end",
            ),
        );
        let theme = Theme::build(
            &Default::default(),
            crate::design::theme::Tier::TrueColor,
            &[],
        );
        let area = Rect::new(0, 1, 34, 8);
        measure(&crate::host::menu_view_of(&open), &open.viewport, &theme, area);
        let view = crate::host::menu_view_of(&open);
        let layout = Layout::new(&view, &theme, area);
        let choice = &layout.choices[0];
        let edge = u16::from(choice.raised);
        let face = Rect::new(
            choice.area.x,
            choice.area.y + edge,
            choice.area.width,
            choice.area.height - edge * 2,
        );
        assert!(face.height > 1, "the caption wraps");
        let mut buf = Buffer::empty(Rect::new(0, 0, 34, 10));
        render(&host, &view, &theme, layout, &mut buf, &Cell::new(None));
        let expected = theme.bg(Slot::AccentPrimary).bg.unwrap();
        for y in face.top()..face.bottom() {
            for x in face.left()..face.right() {
                assert_eq!(buf[(x, y)].bg, expected, "button face at {x},{y}");
            }
        }
    }

    #[test]
    fn exceptionally_long_lines_keep_every_grapheme_without_large_cell_buffers() {
        let words = format!(
            "{}👩‍💻{}e\u{301}",
            "界".repeat(20_000),
            "word ".repeat(10_000)
        );
        let lines = bounded_lines(Line::styled(words.clone(), Style::new()));
        assert!(lines.len() > 1);
        let joined: String = lines
            .iter()
            .flat_map(|line| &line.spans)
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(joined, words);
        for line in lines {
            assert!(line.width() <= 32_768);
            assert!(
                Paragraph::new(line)
                    .wrap(Wrap { trim: false })
                    .line_count(2)
                    < usize::from(u16::MAX)
            );
        }
    }

    #[test]
    fn button_rows_fit_the_available_width_and_keep_original_indices() {
        let theme = Theme::build(&Default::default(), crate::design::theme::Tier::Mono, &[]);
        let buttons = vec![
            (
                12,
                widgets::Button {
                    keys: "y".into(),
                    words: "keep these notes until review-end".into(),
                    ..Default::default()
                },
            ),
            (
                15,
                widgets::Button {
                    keys: "n".into(),
                    words: "leave them".into(),
                    ..Default::default()
                },
            ),
        ];
        let (choices, height) = choices(&theme, &buttons, 22, true);
        assert!(height > 6);
        assert_eq!(choices.iter().map(|c| c.item).collect::<Vec<_>>(), [12, 15]);
        assert!(choices.iter().all(|c| c.area.right() <= 22));
        assert!(
            choices
                .iter()
                .all(|c| usize::from(c.area.bottom()) <= height)
        );
    }
}
