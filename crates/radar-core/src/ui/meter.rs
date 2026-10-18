//! Meters: a stage bar (the steps of a run, one cell each across the width),
//! a ratio on one line (ratatui's `LineGauge`), and a sparkline (ratatui's
//! `Sparkline`), as a widget or as a span inside text.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{LineGauge, Sparkline, Widget};

use super::Mark;
use crate::design::theme::Slot;
use crate::plugin::Tone;
use crate::theme::Theme;
use crate::widgets;

/// One step of a run: its name, its state, and the style for both (a part's
/// look word for the state, or [`Mark::tone`]).
#[derive(Debug, Clone)]
pub struct Stage {
    pub name: String,
    pub mark: Mark,
    pub style: Style,
}

impl Stage {
    pub fn new(name: impl Into<String>, mark: Mark, style: Style) -> Self {
        Self {
            name: name.into(),
            mark,
            style,
        }
    }
}

/// Two lines across `width`: a bar per stage, and its mark and name under
/// it. Empty with no stages.
pub fn stages(stages: &[Stage], width: u16) -> Vec<Line<'static>> {
    if stages.is_empty() {
        return vec![];
    }
    let row = Rect::new(0, 0, width, 1);
    let cells = Layout::horizontal(stages.iter().map(|_| Constraint::Fill(1)))
        .spacing(1)
        .split(row);
    let (mut bars, mut names) = (Vec::new(), Vec::new());
    for (s, c) in stages.iter().zip(cells.iter()) {
        let w = c.width as usize;
        let name = Line::styled(format!("{} {}", s.mark.symbol(), s.name), s.style);
        let mut used = 0;
        for grapheme in name.styled_graphemes(Style::default()) {
            let cells = super::width(grapheme.symbol);
            if used + cells > w { break; }
            names.push(Span::styled(grapheme.symbol.to_owned(), grapheme.style));
            used += cells;
        }
        bars.push(Span::styled("▀".repeat(w), s.style));
        names.push(Span::styled(" ".repeat(w.saturating_sub(used)), s.style));
        bars.push(Span::raw(" "));
        names.push(Span::raw(" "));
    }
    vec![Line::from(bars), Line::from(names)]
}

/// `label ━━━━━━──────` on one line: how much of something is done, in the
/// tone's colour. Readable without colour: the filled part is heavier.
pub fn ratio(theme: &Theme, area: Rect, buf: &mut Buffer, label: &str, ratio: f64, tone: Tone) {
    LineGauge::default()
        .ratio(ratio.clamp(0.0, 1.0))
        .label(Span::styled(format!("{label} "), theme.s(Slot::TextSecondary)))
        .filled_symbol(symbols::line::THICK_HORIZONTAL)
        .unfilled_symbol(symbols::line::HORIZONTAL)
        .filled_style(widgets::tone(theme, tone))
        .unfilled_style(theme.s(Slot::TextMuted))
        .render(area, buf);
}

/// Numbers over time as bars filling `area` (the newest at the right).
pub fn sparkline(theme: &Theme, area: Rect, buf: &mut Buffer, data: &[u64], tone: Tone) {
    Sparkline::default()
        .data(data)
        .style(widgets::tone(theme, tone))
        .render(area, buf);
}

/// A one-row sparkline as a span, to sit inside text (`durations ▂▃▇▅`):
/// the last `width` values.
pub fn spark(theme: &Theme, data: &[u64], width: u16, tone: Tone) -> Span<'static> {
    let shown = &data[data.len().saturating_sub(width as usize)..];
    let w = shown.len() as u16;
    if w == 0 {
        return Span::raw("");
    }
    let area = Rect::new(0, 0, w, 1);
    let mut buf = Buffer::empty(area);
    Sparkline::default().data(shown).render(area, &mut buf);
    let text: String = (0..w)
        .map(|x| buf[(x, 0)].symbol().to_string())
        .collect::<String>()
        .replace(' ', symbols::bar::ONE_EIGHTH);
    Span::styled(text, widgets::tone(theme, tone))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::widgets::{Paragraph, Wrap};

    #[test]
    fn stage_labels_clip_and_pad_display_cells_without_extra_wrapped_rows() {
        for (name, width, expected) in [("界", 3, "✓"), ("e\u{301}", 3, "✓ e\u{301}"), ("界", 4, "✓ 界"), ("👩‍💻", 3, "✓"), ("👩‍💻", 4, "✓ 👩‍💻"), ("lantern", 5, "✓ lan")] {
            let rows = stages(&[Stage::new(name, Mark::Done, Style::default())], width);
            let paragraph = Paragraph::new(rows).wrap(Wrap { trim: false });
            assert_eq!(paragraph.line_count(width), 2, "{name:?}, width {width}");
            let mut buffer = Buffer::empty(Rect::new(0, 0, width, 3));
            paragraph.render(buffer.area, &mut buffer);
            let text = crate::shell::row_text(&buffer, 1);
            assert_eq!(text, expected);
            assert!(crate::shell::row_text(&buffer, 2).trim().is_empty());
        }
    }
}
