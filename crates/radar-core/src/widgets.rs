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
pub fn key_hint(theme: &Theme, key: &str, label: &str, mnemonic_at: Option<usize>, mnemonic_shift: bool) -> Vec<Span<'static>> {
    let valid = crate::mnemonic::valid_at(key, label, mnemonic_at);
    let prefix = crate::mnemonic::prefix(key, label, mnemonic_at, mnemonic_shift);
    let mut spans = Vec::new();
    if !prefix.is_empty() {
        spans.push(Span::styled(format!("{prefix} "), theme.bold(Slot::AccentSecondary)));
    }
    spans.extend(mnemonic_word(label, mnemonic_at.filter(|_| valid), theme.s(Slot::TextMuted)));
    spans
}

/// Apply an authoritative scalar position to the complete containing grapheme,
/// adding underline without replacing the word's other style layers.
pub fn mnemonic_word(words: &str, at: Option<usize>, style: Style) -> Vec<Span<'static>> {
    let Some(range) = at.and_then(|at| crate::mnemonic::grapheme_range(words, at)) else {
        return vec![Span::styled(words.to_string(), style)];
    };
    let chars: Vec<_> = words.chars().collect();
    [(&chars[..range.start], false), (&chars[range.clone()], true), (&chars[range.end..], false)]
        .into_iter()
        .filter(|(text, _)| !text.is_empty())
        .map(|(text, underline)| Span::styled(text.iter().collect::<String>(), if underline { style.add_modifier(Modifier::UNDERLINED) } else { style }))
        .collect()
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
    fill_row(buf, r, style);
    let rw = right.as_ref().map_or(0, |s| s.width() as u16);
    let room = inner.width.saturating_sub(if rw > 0 { rw + 3 } else { 2 });
    buf.set_line(inner.x + 1, y, &left, room);
    if let Some(span) = right {
        buf.set_span(inner.x + inner.width.saturating_sub(rw + 1), y, &span, rw);
    }
}

/// Paint `style` across every cell of `row_area`: the highlight covers the
/// panel's full inner width, not just the text.
pub fn fill_row(buf: &mut Buffer, row_area: Rect, style: Style) {
    if row_area.width == 0 || row_area.height == 0 {
        return;
    }
    buf.set_style(row_area, style);
}

/// The selection style for a row (full-row treatment, LAYOUT.md).
/// The focused pane's cursor is bold on the selection colour; a pane without
/// focus keeps its cursor on the same colour, not bold, so you can always see
/// where you were.
pub fn row_style(theme: &Theme, focused: bool, selected: bool) -> Style {
    match (selected, focused) {
        (false, _) => select::unselected_row(&theme.d).into(),
        (true, true) => select::selected_row(&theme.d, true).into(),
        // Only the pane with the keys carries the selection ground; another pane's current row sits quietly on the
        // overlay shade, its words still emphasised (the Ink style guide, sheet 08).
        (true, false) => {
            let style = theme
                .bold(Slot::TextEmphasis)
                .patch(theme.bg(Slot::BgOverlay));
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
    /// Host-owned scalar position in words and explicit uppercase modifier.
    pub mnemonic_at: Option<usize>,
    pub mnemonic_shift: bool,
    /// The highlighted one, which `⏎` chooses: filled with the accent.
    pub focused: bool,
    /// It will not do all it says now (a write the mode refuses).
    pub muted: bool,
}

impl Button {
    /// Its width in cells: a cell either side of the key and the words.
    pub fn width(&self) -> usize {
        let prefix = crate::mnemonic::prefix(&self.keys, &self.words, self.mnemonic_at, self.mnemonic_shift);
        2 + if prefix.is_empty() { 0 } else { crate::ui::width(prefix) + 1 }
            + self.icon.as_ref().map_or(0, Span::width) + crate::ui::width(&self.words)
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
        let prefix = crate::mnemonic::prefix(&b.keys, &b.words, b.mnemonic_at, b.mnemonic_shift);
        let inline_shift = crate::mnemonic::valid_at(&b.keys, &b.words, b.mnemonic_at) && b.mnemonic_shift;
        mid.push(Span::styled(if prefix.is_empty() || inline_shift { " ".into() } else { format!(" {prefix} ") }, key));
        if let Some(icon) = &b.icon {
            let mut icon = icon.clone();
            icon.style = icon.style.patch(face);
            mid.push(icon);
        }
        if inline_shift { mid.push(Span::styled("shift ", key)); }
        let at = b.mnemonic_at.filter(|_| crate::mnemonic::valid_at(&b.keys, &b.words, b.mnemonic_at));
        mid.extend(mnemonic_word(&b.words, at, words));
        mid.push(Span::styled(" ", words));
    }
    ([Line::from(top), Line::from(mid), Line::from(bottom)], places)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::theme::Tier;

    mod mnemonic_tests {
    use super::*;

    #[test]
    fn key_words_use_authoritative_scalar_graphemes_and_preserve_styles() {
        let theme = Theme::new(Tier::TrueColor);
        let spans = key_hint(&theme, "e", "😀 e\u{301}dit", Some(2), false);
        assert_eq!(Line::from(spans.clone()).to_string(), "😀 e\u{301}dit");
        let lit: String = spans.iter().filter(|s| s.style.add_modifier.contains(Modifier::UNDERLINED)).map(|s| s.content.as_ref()).collect();
        assert_eq!(lit, "e\u{301}");
        assert!(spans.iter().all(|s| s.style.fg == theme.s(Slot::TextMuted).fg));
        assert_eq!(Line::from(key_hint(&theme, "A", "ask", Some(0), true)).to_string(), "shift ask");
        for invalid in [None, Some(1), Some(100)] {
            assert_eq!(Line::from(key_hint(&theme, "r", "refresh", invalid, true)).to_string(), "r refresh");
        }
        assert_eq!(Line::from(key_hint(&theme, "ctrl-r", "refresh", Some(0), false)).to_string(), "ctrl-r refresh");
    }

    #[test]
    fn keyed_button_measurement_matches_painted_caption_and_underlined_zero() {
        let mut button = Button { keys: "y".into(), words: "yes, post it".into(), mnemonic_at: Some(0), ..Default::default() };
        for focused in [false, true] {
            button.focused = focused;
            for tier in [Tier::Mono, Tier::TrueColor] {
                let theme = Theme::new(tier);
                let (lines, places) = buttons(&theme, std::slice::from_ref(&button), 0);
                assert_eq!(lines[1].to_string(), " yes, post it ");
                assert_eq!(button.width(), 14);
                assert_eq!(places, [(0, 14)]);
                assert!(lines.iter().all(|line| line.width() == 14));
                assert_eq!(lines[1].spans.iter().filter(|s| s.style.add_modifier.contains(Modifier::UNDERLINED)).map(|s| s.content.as_ref()).collect::<String>(), "y");
            }
        }
    }

    }

    mod chip_tone_acceptance {
        use super::*;
        use crate::theme::ThemeConfig;

        // These are explicit widget tone examples, not application-state mapping
        // tests. A producer may assign a different tone or render a state as
        // ordinary text; these cases only establish the chip primitive contract.
        const NAMED: [(&str, Tone, Color, Color, bool); 5] = [
            ("passed", Tone::Good, Color::Rgb(0x50, 0xfa, 0x7b), Color::Rgb(0x23, 0x3a, 0x37), false),
            ("needs review", Tone::Warn, Color::Rgb(0xf1, 0xfa, 0x8c), Color::Rgb(0x3c, 0x3a, 0x3a), true),
            ("failed", Tone::Bad, Color::Rgb(0xff, 0x63, 0x63), Color::Rgb(0x3f, 0x22, 0x33), true),
            ("running", Tone::Info, Color::Rgb(0x82, 0xaa, 0xff), Color::Rgb(0x2b, 0x2e, 0x4c), false),
            ("merged", Tone::Muted, Color::Rgb(0x62, 0x72, 0xa4), Color::Rgb(0x26, 0x25, 0x3e), false),
        ];

        fn theme(tier: Tier) -> Theme {
            // Explicit off avoids auto-pack/thread-local discovery in pure cases.
            Theme::build(&ThemeConfig { icons: "off".into(), ..Default::default() }, tier, &[])
        }

        #[test]
        fn named_tones_keep_square_padding_and_exact_rgb_tints() {
            let t = theme(Tier::TrueColor);
            assert_eq!(t.bg(Slot::BgOverlay).bg, Some(Color::Rgb(0x1a, 0x16, 0x2a)));
            for (text, tone, fg, bg, bold) in NAMED {
                let span = chip(&t, text, tone);
                assert_eq!(span.content, format!(" {text} "), "{text}: square, one space either side");
                assert_eq!(span.width(), text.len() + 2, "{text}");
                assert_eq!(span.style.fg, Some(fg), "{text}: word colour");
                // Literal expectations: rounded 16% foreground over #1a162a;
                // do not calculate the oracle with chip_ground or theme::mix.
                assert_eq!(span.style.bg, Some(bg), "{text}: ground");
                assert_eq!(span.style.add_modifier, if bold { Modifier::BOLD } else { Modifier::empty() }, "{text}");
            }
            assert_eq!(chip(&t, "passed", Tone::Good).style.fg, t.s(Slot::StatusSuccess).fg);
            assert_ne!(chip(&t, "x", Tone::Bad).style.bg, chip(&t, "x", Tone::Good).style.bg, "read by the ground");
            for (tone, fg, bg, bold) in [
                (Tone::Plain, Color::Rgb(0xf8, 0xf8, 0xf2), Color::Rgb(0x3e, 0x3a, 0x4a), false),
                (Tone::Accent, Color::Rgb(0xe1, 0x35, 0xff), Color::Rgb(0x3a, 0x1b, 0x4c), true),
            ] {
                let style = chip(&t, "label", tone).style;
                assert_eq!((style.fg, style.bg), (Some(fg), Some(bg)));
                assert_eq!(style.add_modifier.contains(Modifier::BOLD), bold);
            }
        }

        #[test]
        fn lower_tiers_keep_flat_overlay_and_tone_modifiers() {
            for (tier, overlay, foregrounds) in [
                (Tier::Ansi256, Some(Color::Indexed(235)), [Some(Color::Indexed(84)), Some(Color::Indexed(228)), Some(Color::Indexed(203)), Some(Color::Indexed(111)), Some(Color::Indexed(61))]),
                (Tier::Ansi16, Some(Color::DarkGray), [Some(Color::LightGreen), Some(Color::LightYellow), Some(Color::LightRed), Some(Color::LightBlue), Some(Color::DarkGray)]),
                (Tier::Mono, None, [None; 5]),
            ] {
                let t = theme(tier);
                assert_eq!(t.bg(Slot::BgOverlay).bg, overlay);
                for ((text, tone, _, _, bold), fg) in NAMED.into_iter().zip(foregrounds) {
                    let span = chip(&t, text, tone);
                    assert_eq!(span.content, format!(" {text} "));
                    assert_eq!((span.style.fg, span.style.bg), (fg, overlay), "{tier:?}: {text}");
                    assert_eq!(span.style.add_modifier, if bold { Modifier::BOLD } else { Modifier::empty() }, "{tier:?}: {text}");
                }
            }
        }

        #[test]
        fn custom_rgb_slots_and_non_rgb_inputs_obey_the_same_ground_rule() {
            let cfg = ThemeConfig {
                icons: "off".into(),
                slots: [("bg.overlay".into(), "#102030".into()), ("status.success".into(), "#f010e0".into())].into(),
                ..Default::default()
            };
            let t = Theme::build(&cfg, Tier::TrueColor, &[]);
            let span = chip(&t, "passed", Tone::Good);
            assert_eq!(span.style.fg, Some(Color::Rgb(0xf0, 0x10, 0xe0)));
            // (16,32,48)*.84 + (240,16,224)*.16 rounds to (52,29,76).
            assert_eq!(span.style.bg, Some(Color::Rgb(0x34, 0x1d, 0x4c)));
            for fg in [None, Some(Color::Reset), Some(Color::Indexed(84)), Some(Color::LightGreen)] {
                assert_eq!(chip_ground(&t, fg).bg, Some(Color::Rgb(0x10, 0x20, 0x30)), "{fg:?}");
            }
            for tier in [Tier::Ansi256, Tier::Ansi16, Tier::Mono] {
                let t = Theme::build(&cfg, tier, &[]);
                // An RGB foreground alone must not tint a non-RGB overlay.
                assert_eq!(chip_ground(&t, Some(Color::Rgb(240, 16, 224))), t.bg(Slot::BgOverlay));
            }
        }

        #[test]
        fn icon_spans_keep_the_words_ground_and_the_icons_own_foreground() {
            use crate::compose::Chip;
            use crate::ui::chips::{chip as contributed_chip, chip_spans};
            use crate::ui::icon::Icon;
            // Contract: explicit built-in icon resolution reads the pack registry.
            let t = Theme::build(&ThemeConfig { icons: "unicode".into(), ..Default::default() }, Tier::TrueColor, &[]);
            let c = Chip::new("failed", Tone::Bad).icon(Icon::Thread);
            let spans = chip_spans(&t, &c);
            assert_eq!(spans.iter().map(|s| s.content.as_ref()).collect::<Vec<_>>(), [" ", "❝ ", "failed "]);
            assert_eq!(spans.iter().map(|s| s.content.as_ref()).collect::<String>(), contributed_chip(&t, &c).content);
            assert!(spans.iter().all(|s| s.style.bg == Some(Color::Rgb(0x3f, 0x22, 0x33))));
            assert_eq!(spans[1].style.fg, Some(Color::Rgb(0x80, 0xff, 0xea)), "icon keeps accent.secondary");
            for i in [0, 2] {
                assert_eq!(spans[i].style.fg, Some(Color::Rgb(0xff, 0x63, 0x63)));
                assert!(spans[i].style.add_modifier.contains(Modifier::BOLD));
            }
            assert_eq!(chip_spans(&theme(Tier::TrueColor), &c)[0].content, " failed ", "icons off preserve both padding cells");
        }

        #[test]
        fn native_chip_cells_match_the_reviewed_capture() {
            use crate::compose::Chip;
            use crate::ipc::look::{Look, colour_word};
            use crate::ui::icon::Icon;
            use crate::view::{Ink, Seg, Text};
            use serde_json::json;
            let mut cases = Vec::new();
            for (name, tier, icons, custom) in [
                ("truecolor-off", Tier::TrueColor, "off", false),
                ("truecolor-icons", Tier::TrueColor, "unicode", false),
                ("256-icons", Tier::Ansi256, "unicode", false),
                ("16-icons", Tier::Ansi16, "unicode", false),
                ("mono-icons", Tier::Mono, "unicode", false),
                ("custom-rgb-icons", Tier::TrueColor, "unicode", true),
            ] {
                let mut cfg = ThemeConfig { icons: icons.into(), ..Default::default() };
                if custom {
                    cfg.slots.insert("bg.overlay".into(), "#102030".into());
                    cfg.slots.insert("status.success".into(), "#f010e0".into());
                }
                let t = Theme::build(&cfg, tier, &[]);
                let mut rows = Vec::new();
                for (text, tone, _, _, _) in NAMED {
                    let chip = Chip::new(text, tone).icon(Icon::Thread);
                    let input = Text::from(vec![Seg::new("|", Ink::Plain), Seg::chip(&chip), Seg::new("|", Ink::Plain)]);
                    let line = crate::view_tui::line(&t, &input);
                    let width = line.width() as u16;
                    let mut buf = Buffer::empty(Rect::new(0, 0, width, 1));
                    buf.set_line(0, 0, &line, width);
                    // Compact runs come from the actual native cells, including the
                    // unstyled delimiters immediately outside each padded chip.
                    let mut runs: Vec<(String, String, String, String)> = Vec::new();
                    for x in 0..width {
                        let cell = &buf[(x, 0)];
                        let fg = colour_word(Some(cell.fg));
                        let bg = colour_word(Some(cell.bg));
                        let modifiers = [
                            (Modifier::BOLD, "bold"), (Modifier::DIM, "dim"),
                            (Modifier::ITALIC, "italic"), (Modifier::UNDERLINED, "underline"),
                            (Modifier::REVERSED, "inverse"), (Modifier::CROSSED_OUT, "strike"),
                        ].into_iter().filter_map(|(flag, name)| cell.modifier.contains(flag).then_some(name))
                            .collect::<Vec<_>>().join(" ");
                        if let Some(run) = runs.last_mut().filter(|r| r.1 == fg && r.2 == bg && r.3 == modifiers) {
                            run.0.push_str(cell.symbol());
                        } else {
                            runs.push((cell.symbol().into(), fg, bg, modifiers));
                        }
                    }
                    rows.push(json!({ "text": input, "width": width, "runs": runs }));
                }
                cases.push(json!({ "name": name, "look": Look::of(&t), "rows": rows }));
            }
            let actual = json!({ "format": 1, "cases": cases });
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../ink/test/fixtures/chips/tone-acceptance.ratatui.json");
            // Explicit export only. Ordinary tests compare without rewriting.
            if std::env::var_os("RADAR_CHIP_CELLS_DIR").is_some() {
                let dir = std::path::PathBuf::from(std::env::var_os("RADAR_CHIP_CELLS_DIR").unwrap());
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join("tone-acceptance.ratatui.json"), format!("{}\n", serde_json::to_string_pretty(&actual).unwrap())).unwrap();
            } else {
                let expected: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("reviewed native chip capture")).unwrap();
                assert_eq!(actual, expected, "native chip capture is stale; review changes before explicit export");
            }
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
        assert_eq!(mid.spans.iter().find(|span| span.content == " n ").unwrap().style.bg, theme.bg(Slot::BgSelection).bg);
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
