//! The tour's card, painted from the chrome's [`TourCard`] at the body's
//! lower right, over the app, with a shadow.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::design::theme::Slot;
use crate::theme::Theme;
use crate::view::chrome::TourCard;
use crate::{ui, view_tui};

/// Where the card goes in `body`: its lines' width and height, at the lower
/// right, a cell in from the edge.
pub fn place(card: &TourCard, body: Rect) -> Rect {
    let lines = lines_of(card);
    let widest = lines.iter().map(|l| l.0).max().unwrap_or(0) as u16;
    let w = (widest + 4).max(40).min(body.width.saturating_sub(2));
    let h = (lines.len() as u16 + 2).min(body.height.saturating_sub(1));
    Rect::new(body.right().saturating_sub(w + 1), body.bottom().saturating_sub(h + 1), w, h)
}

/// Each line's width and the line, in the card's order.
fn lines_of(card: &TourCard) -> Vec<(usize, Option<&crate::view::Text>, Option<String>)> {
    let mut out = vec![(ui::width(&card.title), None, Some(card.title.clone())), (0, None, None)];
    out.extend(card.lines.iter().map(|t| (ui::width(&t.words()), Some(t), None)));
    out.push((0, None, None));
    if let Some(t) = &card.try_it {
        out.push((ui::width(t), None, Some(format!("\u{0}{t}"))));
    }
    out.push((ui::width(&card.keys.words()), Some(&card.keys), None));
    out
}

pub fn card(theme: &Theme, card: &TourCard, body: Rect, buf: &mut Buffer) {
    let frame = place(card, body);
    if frame.width < 10 || frame.height < 4 {
        return;
    }
    ratatui::widgets::Clear.render(frame, buf);
    buf.set_style(frame, theme.bg(Slot::BgOverlay));
    let inner = ui::card::panel(theme, frame, buf, &format!("tour · {} of {}", card.stop, card.of), true);
    let lines: Vec<Line> = lines_of(card)
        .into_iter()
        .map(|(_, text, words)| match (text, words) {
            (Some(t), _) => view_tui::line(theme, t),
            (None, Some(w)) if w.starts_with('\u{0}') => {
                Line::from(Span::styled(w.trim_start_matches('\u{0}').to_string(), theme.s(Slot::TextMuted)))
            }
            (None, Some(w)) => Line::from(Span::styled(w, theme.bold(Slot::TextEmphasis))),
            (None, None) => Line::default(),
        })
        .collect();
    Paragraph::new(lines).render(inner.inner(ratatui::layout::Margin::new(1, 0)), buf);
    ui::scrim::shadow(buf, frame, theme);
}
