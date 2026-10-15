//! Small drawing helpers plugins share, so lists and forms look the same in
//! every plugin.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use crate::design::select;
use crate::design::theme::Slot;

use crate::plugin::Tone;
use crate::theme::Theme;


/// The style for a tone: the one place tones become slots.
pub fn tone(theme: &Theme, tone: Tone) -> Style {
    match tone {
        Tone::Plain => theme.s(Slot::TextPrimary),
        Tone::Muted => theme.s(Slot::TextMuted),
        Tone::Accent => theme.bold(Slot::AccentPrimary),
        Tone::Info => theme.s(Slot::StatusInfo),
        Tone::Good => theme.s(Slot::StatusSuccess),
        Tone::Warn => theme.bold(Slot::StatusWarning),
        Tone::Bad => theme.bold(Slot::StatusError),
    }
}

/// A rounded panel with its title in the border; accent border when focused.
pub fn panel<'a>(theme: &Theme, title: &str, focused: bool) -> ratatui::widgets::Block<'a> {
    use ratatui::widgets::{Block, BorderType, Borders};
    let border = if focused {
        theme.s(Slot::AccentPrimary)
    } else {
        theme.s(Slot::AccentMuted)
    };
    let title_style = if focused {
        theme.bold(Slot::TextEmphasis)
    } else {
        theme.s(Slot::TextSecondary)
    };
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Line::from(Span::styled(format!(" {title} "), title_style)))
        .style(theme.bg(Slot::BgSurface))
}

/// A small heading inside a panel: `PIPELINE`, `CHANGES`.
pub fn heading(theme: &Theme, text: &str) -> Line<'static> {
    Line::styled(
        text.to_uppercase(),
        theme.s(Slot::TextMuted).add_modifier(Modifier::BOLD),
    )
}

/// A group's heading in a settings `look` list (`STATES`,
/// `MESSAGES`): the same muted capitals in every part, not indented.
pub fn look_heading(theme: &Theme, text: &str) -> Line<'static> {
    heading(theme, text)
}

/// The word for a colour the item has set itself: `<name>'s own`.
pub fn own_colour(name: &str) -> String {
    format!("{name}'s own")
}

/// A label set apart from running text: ` needs your review `.
pub fn chip(theme: &Theme, text: &str, t: Tone) -> Span<'static> {
    Span::styled(format!(" {text} "), chip_style(theme, t))
}

/// A chip's look: its tone on [`chip_ground`].
pub fn chip_style(theme: &Theme, t: Tone) -> Style {
    let s = tone(theme, t);
    s.patch(chip_ground(theme, s.fg))
}

/// A chip's ground: its tone's colour `fg` mixed 16% over the overlay
/// shade, so a row of chips reads by their grounds before their words;
/// the flat overlay shade where either is not full colour (256, 16, mono).
pub fn chip_ground(theme: &Theme, fg: Option<Color>) -> Style {
    let overlay = theme.bg(Slot::BgOverlay);
    match (overlay.bg, fg) {
        (Some(Color::Rgb(r, g, b)), Some(Color::Rgb(tr, tg, tb))) => {
            Style::new().bg(crate::theme::mix((r, g, b), (tr, tg, tb), 160))
        }
        _ => overlay,
    }
}

/// A key and what it does, as the status line and action grids show them.
pub fn key_hint(theme: &Theme, key: &str, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(key.to_string(), theme.bold(Slot::AccentSecondary)),
        Span::styled(format!(" {label}"), theme.s(Slot::TextMuted)),
    ]
}

/// One list row across the panel: highlight, left content clipped to the room
/// the right-hand word leaves, the word at the right edge.
pub fn row(
    buf: &mut Buffer,
    inner: Rect,
    y: u16,
    style: Style,
    left: Line<'_>,
    right: Option<Span<'_>>,
) {
    let r = Rect {
        y,
        height: 1,
        ..inner
    };
    select::fill_row(buf, r, style);
    let rw = right.as_ref().map_or(0, |s| s.width() as u16);
    let room = inner.width.saturating_sub(if rw > 0 { rw + 3 } else { 2 });
    buf.set_line(inner.x + 1, y, &left, room);
    if let Some(span) = right {
        buf.set_span(inner.x + inner.width.saturating_sub(rw + 1), y, &span, rw);
    }
}

/// The selection style for a row (full-row treatment, LAYOUT.md).
/// The focused pane's cursor is bold on the selection colour; a pane without
/// focus keeps its cursor on the same colour, not bold, so you can always see
/// where you were.
pub fn row_style(theme: &Theme, focused: bool, selected: bool) -> Style {
    match (selected, focused) {
        (false, _) => select::unselected_row(&theme.d),
        (true, true) => select::selected_row(&theme.d, true),
        (true, false) => {
            let style = theme
                .s(Slot::TextSecondary)
                .patch(theme.bg(Slot::BgSelection));
            if theme.tier() == crate::design::theme::Tier::Mono {
                // No colour to carry it: underline marks where you were.
                style.add_modifier(Modifier::UNDERLINED)
            } else {
                style
            }
        }
    }
}

/// A text cursor at the end of an input.
pub fn cursor(theme: &Theme) -> Span<'static> {
    Span::styled(
        " ",
        theme.s(Slot::TextPrimary).add_modifier(Modifier::REVERSED),
    )
}

/// `label     value`, label muted and padded.
pub fn kv<'a>(theme: &Theme, label: &str, width: usize, value: Vec<Span<'a>>) -> Line<'a> {
    let mut spans = vec![Span::styled(
        crate::ui::pad(label, width),
        theme.s(Slot::TextMuted),
    )];
    spans.extend(value);
    Line::from(spans)
}

/// Muted text wrapped into the panel — the empty state of a panel.
pub fn empty(buf: &mut Buffer, inner: Rect, theme: &Theme, text: &str) {
    let icon = crate::ui::icon::lead(theme, crate::ui::icon::Icon::Empty);
    Paragraph::new(Line::from(vec![icon, Span::styled(text.to_string(), theme.s(Slot::TextMuted))]))
        .wrap(Wrap { trim: true })
        .render(padded(inner), buf);
}

/// Wrapped lines inside the panel's one-column padding.
pub fn paragraph(buf: &mut Buffer, inner: Rect, lines: Vec<Line<'_>>) {
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .render(padded(inner), buf);
}

pub fn padded(inner: Rect) -> Rect {
    Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    }
}

/// First item to draw so `selected` stays visible in `visible` slots.
pub fn scroll_offset(selected: usize, visible: usize) -> usize {
    selected.saturating_sub(visible.max(1) - 1)
}

/// Move a selection one step, clamped to `0..n`.
pub fn step(i: usize, n: usize, down: bool) -> usize {
    if n == 0 {
        0
    } else if down {
        (i + 1).min(n - 1)
    } else {
        i.saturating_sub(1)
    }
}

/// The vim/arrow direction of a key, if it is one: `Some(true)` down.
pub fn vertical(key: &ratatui::crossterm::event::KeyEvent) -> Option<bool> {
    use ratatui::crossterm::event::KeyCode;
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(true),
        KeyCode::Char('k') | KeyCode::Up => Some(false),
        _ => None,
    }
}

/// The direction of a key along a row, if it is one: `Some(true)` right
/// (`l`, `→`, `tab`), `Some(false)` left (`h`, `←`, `shift-tab`).
pub fn horizontal(key: &ratatui::crossterm::event::KeyEvent) -> Option<bool> {
    use ratatui::crossterm::event::KeyCode;
    match key.code {
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Tab => Some(true),
        KeyCode::Char('h') | KeyCode::Left | KeyCode::BackTab => Some(false),
        _ => None,
    }
}

/// `a` blended `amount` of the way over `b`: `None` unless both are RGB
/// (with fewer colours there is nothing in between).
pub fn blend(a: Color, b: Color, amount: f32) -> Option<Color> {
    let (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) = (a, b) else {
        return None;
    };
    let mix = |x: u8, y: u8| (f32::from(y) + (f32::from(x) - f32::from(y)) * amount).round() as u8;
    Some(Color::Rgb(mix(ar, br), mix(ag, bg), mix(ab, bb)))
}

/// A choice drawn as a raised button (a question's `y post it`).
#[derive(Debug, Clone, Default)]
pub struct Button {
    /// The key that chooses it, as the key column shows it (`y`, `⏎`).
    pub keys: String,
    /// An icon before the words, in its own colour.
    pub icon: Option<Span<'static>>,
    /// What choosing it does, in a few words.
    pub words: String,
    /// The highlighted one, which `⏎` chooses: filled with the accent.
    pub focused: bool,
    /// It will not do all it says now (a write the mode refuses).
    pub muted: bool,
}

impl Button {
    /// Its width in cells: a cell either side of the key and the words.
    pub fn width(&self) -> usize {
        3 + crate::ui::width(&self.keys) + self.icon.as_ref().map_or(0, Span::width) + crate::ui::width(&self.words)
    }
}

/// A row of raised buttons, three lines tall: a light edge, the face with
/// the key and the words, a dark edge below (a bevel). The focused button
/// is filled with the accent, the others with the selection's grey; `pad`
/// cells from the left, two apart. With where each starts and its width,
/// for clicks. Without colour the edges alone draw the button.
pub fn buttons(theme: &Theme, row: &[Button], pad: usize) -> ([Line<'static>; 3], Vec<(usize, usize)>) {
    let mut top = vec![Span::raw(" ".repeat(pad))];
    let mut mid = top.clone();
    let mut bottom = top.clone();
    let mut at = pad;
    let mut places = Vec::new();
    for (n, b) in row.iter().enumerate() {
        if n > 0 {
            for line in [&mut top, &mut mid, &mut bottom] {
                line.push(Span::raw("  "));
            }
            at += 2;
        }
        let w = b.width();
        places.push((at, w));
        at += w;
        let face = theme.bg(if b.focused { Slot::AccentPrimary } else { Slot::BgSelection });
        // The edges: the face towards white above, towards black below;
        // with no colour to blend, the muted text and the canvas.
        let edge = |to: Color, part: f32, or: Slot| {
            face.bg
                .and_then(|c| blend(to, c, part))
                .map_or_else(|| theme.s(or), |c| Style::new().fg(c))
                .patch(face)
        };
        top.push(Span::styled("▔".repeat(w), edge(Color::Rgb(255, 255, 255), 0.45, Slot::TextMuted)));
        bottom.push(Span::styled("▁".repeat(w), edge(Color::Rgb(0, 0, 0), 0.55, Slot::BgBase)));
        let (key, words) = if b.focused {
            let on = theme.s(Slot::BgBase).patch(face).add_modifier(Modifier::BOLD);
            (on, on)
        } else {
            let words = if b.muted { Slot::TextMuted } else { Slot::TextPrimary };
            (theme.bold(Slot::AccentSecondary).patch(face), theme.s(words).patch(face))
        };
        mid.push(Span::styled(format!(" {} ", b.keys), key));
        if let Some(icon) = &b.icon {
            let mut icon = icon.clone();
            icon.style = icon.style.patch(face);
            mid.push(icon);
        }
        mid.push(Span::styled(format!("{} ", b.words), words));
    }
    ([Line::from(top), Line::from(mid), Line::from(bottom)], places)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::theme::Tier;

    #[test]
    fn a_chip_stands_on_its_tones_tint_in_full_colour_and_on_the_overlay_otherwise() {
        let t = Theme::new(Tier::TrueColor);
        // status.success #50fa7b 16% over bg.overlay #1a162a (board 01, Tone tints)
        assert_eq!(chip(&t, "passed", Tone::Good).style.bg, Some(Color::Rgb(0x23, 0x3a, 0x37)));
        assert_eq!(chip(&t, "passed", Tone::Good).style.fg, t.s(Slot::StatusSuccess).fg);
        assert_ne!(chip(&t, "x", Tone::Bad).style.bg, chip(&t, "x", Tone::Good).style.bg, "read by the ground");
        for tier in [Tier::Ansi256, Tier::Ansi16, Tier::Mono] {
            let t = Theme::new(tier);
            assert_eq!(chip(&t, "passed", Tone::Good).style.bg, t.bg(Slot::BgOverlay).bg, "{tier:?} as today");
        }
    }

    fn button(keys: &str, words: &str, focused: bool) -> Button {
        Button {
            keys: keys.into(),
            words: words.into(),
            focused,
            ..Default::default()
        }
    }

    #[test]
    fn a_row_of_buttons_is_three_lines_of_one_width_with_each_buttons_place() {
        let theme = Theme::new(Tier::TrueColor);
        let row = [button("y", "post it", true), button("n", "drop it", false)];
        let ([top, mid, bottom], places) = buttons(&theme, &row, 3);
        assert_eq!(mid.to_string(), "    y post it    n drop it ");
        assert_eq!(top.to_string(), format!("   {}  {}", "▔".repeat(11), "▔".repeat(11)));
        assert_eq!(bottom.to_string(), format!("   {}  {}", "▁".repeat(11), "▁".repeat(11)));
        assert_eq!(places, vec![(3, 11), (16, 11)]);
        // The focused face is the accent, the other the selection's grey;
        // the edges are tints of the face, not the face.
        let accent = theme.bg(Slot::AccentPrimary).bg;
        assert_eq!(mid.spans[1].style.bg, accent);
        assert_eq!(mid.spans[4].style.bg, theme.bg(Slot::BgSelection).bg);
        assert_eq!(top.spans[1].style.bg, accent);
        assert_ne!(top.spans[1].style.fg, accent);
        assert_ne!(top.spans[1].style.fg, bottom.spans[1].style.fg);
        // Without colour the edges still draw the button.
        let mono = Theme::new(Tier::Mono);
        let ([top, mid, _], _) = buttons(&mono, &row, 0);
        assert_eq!(mid.spans[1].style.bg, None);
        assert!(top.to_string().starts_with("▔▔▔"));
    }
}
