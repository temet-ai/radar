//! Chips: short labels set apart from running text (` needs you `,
//! ` mergeable `), each in its tone's colour on its tone's tint
//! ([`widgets::chip_ground`]).

use ratatui::text::Span;

use crate::plugin::Tone;
use crate::theme::Theme;
use crate::widgets;

/// A contributed chip as one span, opening its url when it has one; its
/// icon, if any, takes the chip's colour. [`chip_spans`] keeps the icon's.
pub fn chip(theme: &Theme, c: &crate::compose::Chip) -> Span<'static> {
    let text = match c.icon {
        Some(i) => super::icon::with(theme, i, &c.text),
        None => c.text.clone(),
    };
    let mut s = widgets::chip(theme, &text, c.tone);
    if let Some(url) = &c.url {
        s.style = super::link::style(s.style, url);
    }
    s
}

/// A contributed chip, its icon in the icon's own colour on the chip's
/// shade, then its words in its tone; the same cells as [`chip`]. Every
/// span opens its url when it has one.
pub fn chip_spans(theme: &Theme, c: &crate::compose::Chip) -> Vec<Span<'static>> {
    let lead = c.icon.map(|i| super::icon::lead(theme, i)).filter(|s| !s.content.is_empty());
    let Some(mut lead) = lead else {
        return vec![chip(theme, c)];
    };
    let words = widgets::chip(theme, &c.text, c.tone);
    lead.style = lead.style.patch(widgets::chip_ground(theme, words.style.fg));
    let mut spans = vec![
        Span::styled(" ", words.style),
        lead,
        Span::styled(words.content[1..].to_string(), words.style),
    ];
    if let Some(url) = &c.url {
        for s in &mut spans {
            s.style = super::link::style(s.style, url);
        }
    }
    spans
}

/// Contributed words in a row, as running text, not chips: each its icon
/// in the icon's colour, then its words in its tone, opening its url when
/// it has one (a contributed line under a title, `Contribution::Line`).
pub fn words(theme: &Theme, items: &[crate::compose::Chip]) -> ratatui::text::Line<'static> {
    words_at(theme, items).0
}

/// [`words`], and where each chip is on the line: `(from, width, index)` in
/// cells from the line's start, for marking the ones that act on a click.
pub fn words_at(
    theme: &Theme,
    items: &[crate::compose::Chip],
) -> (ratatui::text::Line<'static>, Vec<(u16, u16, usize)>) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut at = Vec::new();
    for (i, c) in items.iter().enumerate() {
        let from: usize = spans.iter().map(Span::width).sum();
        if let Some(icon) = c.icon {
            let lead = super::icon::lead(theme, icon);
            if !lead.content.is_empty() {
                spans.push(lead);
            }
        }
        let mut s = Span::styled(c.text.clone(), widgets::tone(theme, c.tone));
        if let Some(url) = &c.url {
            s.style = super::link::style(s.style, url);
        }
        spans.push(s);
        let to: usize = spans.iter().map(Span::width).sum();
        at.push((from as u16, (to - from) as u16, i));
    }
    (ratatui::text::Line::from(spans), at)
}

/// Chips in a row, a space between.
pub fn chips(theme: &Theme, items: &[(&str, Tone)]) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    for (i, (text, tone)) in items.iter().enumerate() {
        if i > 0 {
            out.push(Span::raw(" "));
        }
        out.push(widgets::chip(theme, text, *tone));
    }
    out
}

/// `name` followed by `chips` in `name_w` cells: the span at `gives` (a
/// title) gives way to the chips while it keeps 12 cells; with less room
/// than that the chips are left out.
pub fn after_name(
    mut name: Vec<Span<'static>>,
    gives: usize,
    chips: Vec<Span<'static>>,
    name_w: usize,
) -> Vec<Span<'static>> {
    if chips.is_empty() || gives >= name.len() {
        return name;
    }
    let width = |spans: &[Span<'_>]| spans.iter().map(Span::width).sum::<usize>();
    let fixed = width(&name) - name[gives].width();
    let left = name_w.saturating_sub(fixed + width(&chips));
    if left < 12 {
        return name;
    }
    let text = name[gives].content.trim_end().to_string();
    name[gives].content = super::fit(&text, left.min(super::width(&text))).into();
    name.extend(chips);
    name
}
