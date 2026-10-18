//! Native shared detail geometry. One laid-out row list owns painting, scrolling,
//! media offsets and inline targets; controls are measured separately at the foot.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::design::theme::Slot;
use crate::theme::Theme;
use crate::view::detail::{DetailContent, DetailHeader, FootAction, FootActions};
use crate::view::{Block, Ink, Seg, Text};
use crate::view_tui::{coalesced_line, keyed_line, line, rich_lines_with_urls, wrap_line};
use crate::writes::Mode;

/// One inline target in the same row coordinates used for media and paint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub row: usize,
    pub x: u16,
    pub width: u16,
    pub target: String,
}

#[derive(Default)]
pub struct Rows {
    pub lines: Vec<Line<'static>>,
    pub media: Vec<super::rich::Placed>,
    pub hits: Vec<Hit>,
}

/// A footer leaves at least one body row. Zero is a legal available height.
pub fn reserve(area: Rect, present: bool) -> (Rect, Option<Rect>) {
    if !present || area.height < 3 || area.width == 0 { return (area, None); }
    let body = Rect { height: area.height - 2, ..area };
    (body, Some(Rect { y: body.bottom(), height: 2, ..area }))
}

/// Preserve a cell of air on both sides whenever there is still a text cell.
pub fn inset(width: u16) -> u16 { u16::from(width >= 3) }

/// Clip a rich line with an ellipsis, then fill exactly the requested cells.
/// Coalescing before clipping keeps a grapheme crossing style boundaries whole.
fn fit_line(source: Line<'static>, width: usize, style: Style) -> Line<'static> {
    let source = coalesced_line(source);
    let overflow = source.width() > width;
    let room = width.saturating_sub(usize::from(overflow));
    let mut out = Vec::new();
    let mut used = 0;
    for grapheme in source.styled_graphemes(style) {
        let cells = super::width(grapheme.symbol);
        if used + cells > room { break; }
        out.push(Span::styled(grapheme.symbol.to_owned(), grapheme.style));
        used += cells;
    }
    if overflow && width > 0 {
        out.push(Span::styled("…", style));
        used += 1;
    }
    if used < width { out.push(Span::styled(" ".repeat(width - used), style)); }
    Line::from(out).style(style)
}

fn styled(t: &Theme, text: &Text, slot: Slot, bold: bool) -> Line<'static> {
    let style = if bold { t.bold(slot) } else { t.s(slot) };
    let mut spans = Vec::new();
    for source in &text.0 {
        let mut painted = line(t, &Text::from(vec![source.clone()]));
        if source.ink == Ink::Plain && source.look.is_none() && source.icon.is_none() && !source.chip {
            for span in &mut painted.spans { span.style = style.patch(span.style); }
        }
        spans.extend(painted.spans);
    }
    Line::from(spans).style(style)
}

fn header(t: &Theme, head: &DetailHeader, width: u16) -> Vec<Line<'static>> {
    let pad = inset(width);
    let content = usize::from(width - 2 * pad);
    let ground = t.bg(Slot::BgOverlay);
    let fill = |row: Line<'static>| {
        let mut spans = vec![Span::styled(" ".repeat(usize::from(pad)), ground)];
        spans.extend(fit_line(row, content, ground).spans);
        spans.push(Span::styled(" ".repeat(usize::from(pad)), ground));
        Line::from(spans).style(ground)
    };
    let branch = head.branch.as_ref().map(|text| styled(t, text, Slot::TextPrimary, false));
    let author = head.author.as_ref().map(|text| styled(t, text, Slot::TextSecondary, false));
    let age = head.age.as_ref().map(|text| styled(t, text, Slot::TextMuted, false));
    let age_width = age.as_ref().map_or(0, Line::width);
    let author_width = author.as_ref().map_or(0, Line::width);
    let need = age_width + author_width + if age_width > 0 && author_width > 0 { 3 } else { 0 };
    let right_budget = if branch.is_some() { need.min(age_width.max(content / 2)).min(content) } else { need.min(content) };
    let age_room = age_width.min(right_budget);
    let author_room = author_width.min(right_budget.saturating_sub(age_room + if age_room > 0 { 3 } else { 0 }));
    let right_width = age_room + author_room + if age_room > 0 && author_room > 0 { 3 } else { 0 };
    let branch_room = if branch.is_some() { content.saturating_sub(right_width + usize::from(right_width > 0)) } else { 0 };
    let mut metadata = Vec::new();
    if let Some(branch) = branch { metadata.extend(fit_line(branch, branch_room, ground).spans); }
    metadata.push(Span::styled(" ".repeat(content.saturating_sub(branch_room + right_width)), ground));
    if let Some(author) = author.filter(|_| author_room > 0) { metadata.extend(fit_line(author, author_room, ground).spans); }
    if author_room > 0 && age_room > 0 { metadata.push(Span::styled(" · ", t.s(Slot::TextMuted).patch(ground))); }
    if let Some(age) = age.filter(|_| age_room > 0) { metadata.extend(fit_line(age, age_room, ground).spans); }
    vec![
        fill(styled(t, &head.path, Slot::TextSecondary, false)),
        fill(styled(t, &head.title, Slot::TextEmphasis, true)),
        fill(Line::from(metadata)),
    ]
}

fn indent(mut row: Line<'static>, cells: u16) -> Line<'static> {
    if cells > 0 { row.spans.insert(0, Span::raw(" ".repeat(usize::from(cells)))); }
    row
}

/// Explicit line breaks are meaningful in a fact, independently of soft wrapping.
fn value_rows(t: &Theme, text: &Text, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Text::new()];
    for seg in &text.0 {
        for (index, words) in seg.text.split('\n').enumerate() {
            if index > 0 { lines.push(Text::new()); }
            let mut part = seg.clone();
            part.text = words.strip_suffix('\r').unwrap_or(words).to_string();
            if index > 0 { part.icon = None; part.icon_lead = false; }
            lines.last_mut().expect("one value line").add(part);
        }
    }
    lines.into_iter().flat_map(|text| wrap_line(line(t, &text), width)).collect()
}

/// One source of row counts for every detail body consumer.
pub fn layout(t: &Theme, content: &DetailContent, width: u16, pictured: bool) -> Rows {
    if width == 0 { return Rows::default(); }
    let mut out = Rows { lines: header(t, &content.header, width), ..Rows::default() };
    let pad = inset(width);
    let room = width - 2 * pad;
    let facts: Vec<_> = content.facts.iter().filter(|fact| !fact.value.words().trim().is_empty()).collect();
    if !facts.is_empty() || !content.sections.is_empty() { out.lines.push(Line::default()); }
    let label_width = facts.iter().map(|fact| super::width(&fact.label)).max().unwrap_or(0).min(16).min(usize::from(room / 3));
    for fact in &facts {
        let label = Line::styled(fact.label.clone(), t.s(Slot::TextMuted));
        if room < 12 {
            out.lines.push(indent(fit_line(label, usize::from(room), t.s(Slot::TextMuted)), pad));
            out.lines.extend(value_rows(t, &fact.value, usize::from(room)).into_iter().map(|row| indent(row, pad)));
        } else {
            let value_width = usize::from(room) - label_width - 2;
            for (index, value) in value_rows(t, &fact.value, value_width).into_iter().enumerate() {
                let mut row = if index == 0 { fit_line(label.clone(), label_width, t.s(Slot::TextMuted)) } else { Line::raw(" ".repeat(label_width)) };
                // The label owns its muted spans; plain values inherit the
                // surrounding body style, including on their first row.
                row.style = Style::default();
                row.spans.push(Span::raw("  "));
                row.spans.extend(value.spans);
                out.lines.push(indent(row, pad));
            }
        }
    }
    if !facts.is_empty() && !content.sections.is_empty() { out.lines.push(Line::default()); }
    for (index, block) in content.sections.iter().enumerate() {
        if index > 0 { out.lines.push(Line::default()); }
        append_block(t, block, room, pad, pictured, &mut out);
    }
    out
}

fn append_block(t: &Theme, block: &Block, width: u16, left: u16, pictured: bool, out: &mut Rows) {
    if width == 0 { return; }
    if let Block::Section { mark, title, detail, blocks } = block {
        let heading = super::card::section(t, *mark, title, detail.as_deref(), width);
        out.lines.push(indent(fit_line(heading, usize::from(width), t.s(Slot::TextMuted)), left));
        for block in blocks { append_block(t, block, width.saturating_sub(2), left + 2, pictured, out); }
        return;
    }
    let start = out.lines.len();
    let (lines, media) = rich_lines_with_urls(t, std::slice::from_ref(block), width.saturating_sub(1), pictured, !super::link::on());
    let mut starts = Vec::with_capacity(lines.len());
    let mut rows = Vec::new();
    for source in lines {
        starts.push(rows.len());
        rows.extend(wrap_line(source, usize::from(width)));
    }
    if let Block::Line { text, key, mnemonic_at, mnemonic_shift, target } = block {
        if let Some(target) = target {
            for (index, _) in rows.iter().enumerate().filter(|(_, row)| row.width() > 0) {
                out.hits.push(Hit { row: start + index, x: left, width, target: target.0.clone() });
            }
        }
        let source = keyed_line(t, text, key.as_deref(), *mnemonic_at, *mnemonic_shift, width);
        inline_hits(t, text, &source, &rows, start, left, &mut out.hits);
    }
    out.media.extend(media.into_iter().map(|mut placed| {
        placed.line = start + starts.get(placed.line).copied().unwrap_or(rows.len());
        placed.x += left;
        placed
    }));
    out.lines.extend(rows.into_iter().map(|row| indent(row, left)));
}

fn words(row: &Line<'_>) -> String { row.spans.iter().map(|span| span.content.as_ref()).collect() }

/// Match each laid-out substring back to the rendered source. Wrapping only
/// removes consumed gaps; byte ranges keep combining and wide glyphs together.
fn inline_hits(t: &Theme, text: &Text, source: &Line<'_>, rows: &[Line<'_>], top: usize, left: u16, hits: &mut Vec<Hit>) {
    if !text.0.iter().any(|seg| seg.target.is_some()) { return; }
    let source = words(source);
    let base = words(&line(t, text));
    let mut offset = source.find(&base).unwrap_or(0);
    let mut targets = Vec::new();
    for seg in &text.0 {
        let rendered = words(&line(t, &Text::from(vec![seg.clone()])));
        if let Some(target) = &seg.target { targets.push((offset..offset + rendered.len(), &target.0)); }
        offset += rendered.len();
    }
    let mut cursor = 0;
    for (index, row) in rows.iter().enumerate() {
        let rendered = words(row);
        let Some(found) = source[cursor..].find(&rendered) else { continue; };
        let row_start = cursor + found;
        let mut at = row_start;
        let mut x = left;
        for grapheme in row.styled_graphemes(Style::default()) {
            let end = at + grapheme.symbol.len();
            let width = super::width(grapheme.symbol) as u16;
            for (range, target) in &targets {
                if range.start < end && range.end > at && width > 0 {
                    if let Some(last) = hits.last_mut()
                        && last.row == top + index && last.target == **target && last.x + last.width == x {
                        last.width += width;
                    } else {
                        hits.push(Hit { row: top + index, x, width, target: (*target).clone() });
                    }
                }
            }
            at = end;
            x += width;
        }
        cursor = row_start + rendered.len();
    }
}

pub struct Control {
    pub x: u16,
    pub line: Line<'static>,
    pub target: Option<String>,
}

fn action_line(t: &Theme, item: &FootAction, mode: Mode) -> Line<'static> {
    let mut text = Text::of(item.action.label.clone(), Ink::Secondary);
    if let Some(icon) = item.action.icon { text.0.insert(0, Seg::icon(icon)); }
    let key = item.action.key.map(|key| key.name());
    let mut row = keyed_line(t, &text, key.as_deref(), item.action.mnemonic_at, item.action.mnemonic_shift, 0);
    let blocked = item.action.change.is_some_and(|change| !mode.allows(match change {
        crate::view::Change::Add => crate::writes::Change::Add,
        crate::view::Change::Edit => crate::writes::Change::Edit,
    }));
    if item.disabled.is_some() || blocked {
        for span in &mut row.spans {
            span.style.fg = t.s(Slot::TextMuted).fg;
            span.style = span.style.remove_modifier(Modifier::BOLD);
        }
    }
    row
}

/// Menu first, then the whole prefix that fits. Gaps and disabled controls are
/// never hit regions; mode muting is explanatory and does not replace admission.
pub fn controls(t: &Theme, foot: &FootActions, width: u16, mode: Mode) -> Vec<Control> {
    if width == 0 { return vec![]; }
    let mut menu = action_line(t, &foot.menu, mode);
    if menu.width() > usize::from(width) {
        let key = foot.menu.action.key.map(|key| key.name()).unwrap_or_default();
        menu = Line::styled(key, t.bold(Slot::AccentSecondary));
    }
    let menu_width = menu.width().min(usize::from(width)) as u16;
    let menu_x = width - menu_width;
    let limit = menu_x.saturating_sub(2);
    let mut out = Vec::new();
    let mut x: u16 = 0;
    for item in &foot.actions {
        if item.action.heading || item.action.id.trim().is_empty() { continue; }
        let row = action_line(t, item, mode);
        let need = row.width();
        if need == 0 || need > usize::from(limit.saturating_sub(x)) { break; }
        out.push(Control { x, line: row, target: item.target.as_ref().filter(|target| item.disabled.is_none() && target.0.starts_with(crate::view::detail::TARGET_PREFIX)).map(|target| target.0.clone()) });
        x = x.saturating_add(need as u16).saturating_add(2);
    }
    out.push(Control { x: menu_x, line: menu, target: foot.menu.target.as_ref().filter(|target| foot.menu.disabled.is_none() && target.0.starts_with(crate::view::detail::TARGET_PREFIX)).map(|target| target.0.clone()) });
    out
}

#[cfg(test)]
mod tests;
