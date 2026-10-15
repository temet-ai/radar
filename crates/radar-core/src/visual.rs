//! Seeing what was drawn, not only reading it: a rendered screen as an SVG
//! (every cell at its colour), and as a compact dump of its styles for
//! snapshot tests.
//!
//! A text snapshot proves layout; it cannot show a wrong colour or a lost
//! highlight. These two can:
//!
//! - [`buffer_to_svg`] (and `shell::to_svg` for a host) draws each cell with
//!   its colours and modifiers. Output is deterministic, so two renders diff.
//!   `radar shot` and `scripts/shots` turn it into pictures and a review page.
//! - [`style_dump`] lists each line's runs with the slot names of their
//!   colours (`fg:accent.primary bg:bg.selection bold`), so a test fails when
//!   a colour moves. The kit gallery's colour snapshots use it.

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;

use crate::design::theme::Slot;
use crate::theme::Theme;

pub type Rgb = (u8, u8, u8);

/// The 16 colours a terminal calls black, red, … white, and their bright
/// twins. A theme at the 16-colour tier names these and the terminal picks
/// the shade, so a picture has to pick one too: these are VS Code's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub ansi: [Rgb; 16],
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            ansi: [
                (0x00, 0x00, 0x00),
                (0xcd, 0x31, 0x31),
                (0x0d, 0xbc, 0x79),
                (0xe5, 0xe5, 0x10),
                (0x24, 0x72, 0xc8),
                (0xbc, 0x3f, 0xbc),
                (0x11, 0xa8, 0xcd),
                (0xe5, 0xe5, 0xe5),
                (0x66, 0x66, 0x66),
                (0xf1, 0x4c, 0x4c),
                (0x23, 0xd1, 0x8b),
                (0xf5, 0xf5, 0x43),
                (0x3b, 0x8e, 0xea),
                (0xd6, 0x70, 0xd6),
                (0x29, 0xb8, 0xdb),
                (0xff, 0xff, 0xff),
            ],
        }
    }
}

/// What a cell with no colour of its own looks like, and the palette for
/// named colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    pub palette: Palette,
    pub fg: Rgb,
    pub bg: Rgb,
}

impl Look {
    /// The theme's own text and canvas colours, whatever its tier (a theme
    /// at the no-colour tier still has them; the terminal would show them).
    pub fn of(theme: &Theme) -> Self {
        let rgb = |name: &str| crate::theme::parse_hex(&theme.hex(name)).unwrap_or((0xc0, 0xc0, 0xc0));
        Self {
            palette: Palette::default(),
            fg: rgb(Slot::TextPrimary.token_name()),
            bg: rgb(Slot::BgBase.token_name()),
        }
    }
}

/// A colour as red, green and blue. `Reset` is `default`.
pub fn resolve(color: Color, palette: &Palette, default: Rgb) -> Rgb {
    match color {
        Color::Reset => default,
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(i) => indexed(i, palette),
        Color::Black => palette.ansi[0],
        Color::Red => palette.ansi[1],
        Color::Green => palette.ansi[2],
        Color::Yellow => palette.ansi[3],
        Color::Blue => palette.ansi[4],
        Color::Magenta => palette.ansi[5],
        Color::Cyan => palette.ansi[6],
        Color::Gray => palette.ansi[7],
        Color::DarkGray => palette.ansi[8],
        Color::LightRed => palette.ansi[9],
        Color::LightGreen => palette.ansi[10],
        Color::LightYellow => palette.ansi[11],
        Color::LightBlue => palette.ansi[12],
        Color::LightMagenta => palette.ansi[13],
        Color::LightCyan => palette.ansi[14],
        Color::White => palette.ansi[15],
    }
}

/// An xterm-256 index: the 16 above, the 6x6x6 cube, then 24 greys.
fn indexed(i: u8, palette: &Palette) -> Rgb {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match i {
        0..=15 => palette.ansi[i as usize],
        16..=231 => {
            let n = i - 16;
            (
                LEVELS[(n / 36) as usize],
                LEVELS[(n / 6 % 6) as usize],
                LEVELS[(n % 6) as usize],
            )
        }
        _ => {
            let v = 8 + 10 * (i - 232);
            (v, v, v)
        }
    }
}

fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// `from` moved `part` of the way (out of 100) towards `to`.
fn blend(from: Rgb, to: Rgb, part: u32) -> Rgb {
    let mix = |a: u8, b: u8| ((a as u32 * (100 - part) + b as u32 * part + 50) / 100) as u8;
    (mix(from.0, to.0), mix(from.1, to.1), mix(from.2, to.2))
}

/// A cell's size in SVG units, and where the text sits in it.
const CELL_W: u32 = 10;

/// The SVG's fonts: Nerd Fonts first, so icons draw, then the monospace
/// faces the text was drawn with before.
pub const FONTS: &str =
    "'0xProto Nerd Font Mono', 'Symbols Nerd Font Mono', 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace, 'Noto Color Emoji'";
const CELL_H: u32 = 20;
const BASELINE: u32 = 15;

/// What one cell looks like once colours are resolved.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Ink {
    fg: Rgb,
    bg: Rgb,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    hidden: bool,
}

fn ink(cell: &ratatui::buffer::Cell, look: &Look) -> Ink {
    let m = cell.modifier;
    let mut fg = resolve(cell.fg, &look.palette, look.fg);
    let mut bg = resolve(cell.bg, &look.palette, look.bg);
    if m.contains(Modifier::REVERSED) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if m.contains(Modifier::DIM) {
        fg = blend(fg, bg, 40);
    }
    Ink {
        fg,
        bg,
        bold: m.contains(Modifier::BOLD),
        italic: m.contains(Modifier::ITALIC),
        underline: m.contains(Modifier::UNDERLINED),
        strike: m.contains(Modifier::CROSSED_OUT),
        hidden: m.contains(Modifier::HIDDEN),
    }
}

/// The columns a symbol takes: 1 for most, 2 for wide characters, 0 for
/// nothing (the empty cell after a wide character).
fn columns(symbol: &str) -> usize {
    if symbol.is_empty() {
        0
    } else {
        UnicodeWidthStr::width(symbol).max(1)
    }
}

fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
}

/// One run of text on a row: where it starts (in columns), what it says and
/// how many columns it spans.
struct Run {
    col: usize,
    text: String,
    width: usize,
    ink: Ink,
}

/// The buffer as an SVG: each cell is 10x20 units, backgrounds are one
/// rectangle per run, text is one `<text>` per run pinned to its columns
/// with `textLength` (so a fallback font still lines up), wide characters
/// take two columns, and underline and strike-through are drawn as lines.
/// The same buffer always gives the same text.
pub fn buffer_to_svg(buf: &Buffer, look: &Look) -> String {
    let area = buf.area;
    let (w, h) = (area.width as u32, area.height as u32);
    let mut backs = String::new();
    let mut texts = String::new();
    let mut lines = String::new();
    for row in 0..area.height {
        let y = area.y + row;
        let mut runs: Vec<Run> = Vec::new();
        // Background and line runs: (first column, columns, colour).
        let mut backgrounds: Vec<(usize, usize, Rgb)> = Vec::new();
        let mut unders: Vec<(usize, usize, Rgb, bool)> = Vec::new();
        let mut col = 0usize;
        while col < area.width as usize {
            let cell = &buf[(area.x + col as u16, y)];
            let ink = ink(cell, look);
            let span = columns(cell.symbol()).max(1).min(area.width as usize - col);
            // Continuation cells of a wide character share its ink only for
            // the text; their own backgrounds are drawn below.
            for k in 0..span {
                let c = &buf[(area.x + (col + k) as u16, y)];
                let bg = if k == 0 { ink.bg } else { self::ink(c, look).bg };
                match backgrounds.last_mut() {
                    Some((s, n, last)) if *last == bg && *s + *n == col + k => *n += 1,
                    _ => backgrounds.push((col + k, 1, bg)),
                }
            }
            for (on, strike) in [(ink.underline, false), (ink.strike, true)] {
                if on {
                    match unders.last_mut() {
                        Some((s, n, c, st)) if *st == strike && *c == ink.fg && *s + *n == col => *n += span,
                        _ => unders.push((col, span, ink.fg, strike)),
                    }
                }
            }
            if !ink.hidden && !cell.symbol().trim().is_empty() {
                // A wide character, and a symbol a font may not have (an
                // icon, an emoji), stand alone: a font picked to draw one
                // must not draw the words beside it.
                let wide = span > 1 || lone(cell.symbol());
                match runs.last_mut() {
                    Some(r) if !wide && r.width > 0 && r.ink.same_text(&ink) && r.col + r.width == col && !r.wide() => {
                        r.text.push_str(cell.symbol());
                        r.width += span;
                    }
                    _ => runs.push(Run {
                        col,
                        text: cell.symbol().to_string(),
                        width: span,
                        ink,
                    }),
                }
            } else if !ink.hidden && cell.symbol() == " " {
                // A space inside a run keeps it whole; elsewhere it is nothing.
                if let Some(r) = runs.last_mut()
                    && r.col + r.width == col
                    && !r.wide()
                    && r.ink.same_text(&ink)
                {
                    r.text.push(' ');
                    r.width += 1;
                }
            }
            col += span;
        }
        for (start, n, rgb) in backgrounds {
            if rgb != look.bg {
                backs.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>\n",
                    start as u32 * CELL_W,
                    row as u32 * CELL_H,
                    n as u32 * CELL_W,
                    CELL_H,
                    hex(rgb)
                ));
            }
        }
        for run in runs {
            // Spaces at either end draw nothing: leave them out and move on.
            let lead = run.text.chars().take_while(|c| *c == ' ').count();
            let text = run.text.trim();
            if text.is_empty() {
                continue;
            }
            let cols = text.chars().map(|c| columns(&c.to_string())).sum::<usize>();
            let cols = cols.max(1);
            texts.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" textLength=\"{}\" fill=\"{}\"{}{}>",
                (run.col + lead) as u32 * CELL_W,
                row as u32 * CELL_H + BASELINE,
                cols as u32 * CELL_W,
                hex(run.ink.fg),
                if run.ink.bold { " font-weight=\"700\"" } else { "" },
                if run.ink.italic { " font-style=\"italic\"" } else { "" },
            ));
            escape(text, &mut texts);
            texts.push_str("</text>\n");
        }
        for (start, n, rgb, strike) in unders {
            let at = if strike { CELL_H / 2 } else { CELL_H - 3 };
            lines.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"1\" fill=\"{}\"/>\n",
                start as u32 * CELL_W,
                row as u32 * CELL_H + at,
                n as u32 * CELL_W,
                hex(rgb)
            ));
        }
    }
    let (pw, ph) = (w * CELL_W, h * CELL_H);
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{pw}\" height=\"{ph}\" viewBox=\"0 0 {pw} {ph}\" \
         font-family=\"{FONTS}\" font-size=\"16.6\" xml:space=\"preserve\">\n\
         <rect width=\"{pw}\" height=\"{ph}\" fill=\"{}\"/>\n{backs}{texts}{lines}</svg>\n",
        hex(look.bg)
    )
}

impl Ink {
    /// Same look for text (the background is drawn apart).
    fn same_text(&self, other: &Ink) -> bool {
        self.fg == other.fg
            && self.bold == other.bold
            && self.italic == other.italic
    }
}

/// A symbol beyond text and the box-drawing a monospace font has: drawn as
/// its own `<text>`.
fn lone(symbol: &str) -> bool {
    symbol
        .chars()
        .any(|c| c as u32 >= 0x2000 && !(0x2500..=0x259f).contains(&(c as u32)) && c != '…' && c != '·')
}

impl Run {
    /// A wide character is a run of its own.
    fn wide(&self) -> bool {
        self.width > self.text.chars().count() || lone(&self.text)
    }
}

/// The names a colour goes by in this theme: the slots that have it,
/// joined with `/`, or its hex.
struct Names(Vec<(Color, String)>);

impl Names {
    fn of(theme: &Theme) -> Self {
        let mut by: Vec<(Color, String)> = Vec::new();
        for name in theme.slot_names() {
            let Some(c) = theme.style(&name).fg else { continue };
            match by.iter_mut().find(|(k, _)| *k == c) {
                Some((_, names)) => {
                    names.push('/');
                    names.push_str(&name);
                }
                None => by.push((c, name)),
            }
        }
        Names(by)
    }

    fn name(&self, c: Color) -> String {
        match self.0.iter().find(|(k, _)| *k == c) {
            Some((_, n)) => n.clone(),
            None => match c {
                Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
                Color::Indexed(i) => format!("idx{i}"),
                other => format!("{other:?}").to_lowercase(),
            },
        }
    }
}

/// Each line as its runs of equal style, for a snapshot: the text, then the
/// style when it is not the default text on the canvas. Colours are slot
/// names (`fg:accent.primary`, `bg:bg.selection`), so a changed colour reads
/// as a changed name. A run of spaces is `_N`; the text is cut after 40
/// characters. The theme should be at the full-colour tier.
///
/// ```text
/// 01 " radar " fg:accent.primary bold | "Books" fg:text.emphasis bg:bg.selection bold
/// ```
pub fn style_dump(buf: &Buffer, theme: &Theme) -> String {
    let names = Names::of(theme);
    let default_fg = theme.s(Slot::TextPrimary).fg;
    let default_bg = theme.s(Slot::BgBase).fg;
    let mut out = Vec::new();
    let area = buf.area;
    for row in 0..area.height {
        let y = area.y + row;
        // (text, fg, bg, modifier)
        let mut runs: Vec<(String, Option<Color>, Option<Color>, Modifier)> = Vec::new();
        let mut col = 0usize;
        while col < area.width as usize {
            let cell = &buf[(area.x + col as u16, y)];
            let span = columns(cell.symbol()).max(1);
            let fg = Some(cell.fg).filter(|c| *c != Color::Reset);
            let bg = Some(cell.bg).filter(|c| *c != Color::Reset);
            let m = cell.modifier;
            match runs.last_mut() {
                Some(r) if r.1 == fg && r.2 == bg && r.3 == m => r.0.push_str(cell.symbol()),
                _ => runs.push((cell.symbol().to_string(), fg, bg, m)),
            }
            col += span;
        }
        // A trailing run of default blanks is the rest of the line.
        let plain = |r: &(String, Option<Color>, Option<Color>, Modifier)| {
            r.0.trim().is_empty()
                && r.3.is_empty()
                && (r.2.is_none() || r.2 == default_bg)
        };
        while runs.last().is_some_and(plain) {
            runs.pop();
        }
        let pieces: Vec<String> = runs
            .iter()
            .map(|(text, fg, bg, m)| {
                let mut style: Vec<String> = Vec::new();
                if let Some(c) = fg.filter(|c| Some(*c) != default_fg) {
                    style.push(format!("fg:{}", names.name(c)));
                }
                if let Some(c) = bg.filter(|c| Some(*c) != default_bg) {
                    style.push(format!("bg:{}", names.name(c)));
                }
                for (flag, word) in [
                    (Modifier::BOLD, "bold"),
                    (Modifier::DIM, "dim"),
                    (Modifier::ITALIC, "italic"),
                    (Modifier::UNDERLINED, "underline"),
                    (Modifier::REVERSED, "reversed"),
                    (Modifier::CROSSED_OUT, "strike"),
                    (Modifier::HIDDEN, "hidden"),
                ] {
                    if m.contains(flag) {
                        style.push(word.into());
                    }
                }
                let said = if text.trim().is_empty() {
                    format!("_{}", text.chars().count())
                } else {
                    let cut: String = text.chars().take(40).collect();
                    let more = if text.chars().count() > 40 { "…" } else { "" };
                    format!("{:?}", format!("{cut}{more}"))
                };
                if style.is_empty() {
                    said
                } else {
                    format!("{said} {}", style.join(" "))
                }
            })
            .collect();
        out.push(format!("{row:02} {}", pieces.join(" | ")).trim_end().to_string());
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::theme::Tier;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    fn look() -> Look {
        Look {
            palette: Palette::default(),
            fg: (200, 200, 200),
            bg: (10, 10, 10),
        }
    }

    #[test]
    fn named_indexed_and_rgb_colours_resolve() {
        let p = Palette::default();
        assert_eq!(resolve(Color::Rgb(1, 2, 3), &p, (9, 9, 9)), (1, 2, 3));
        assert_eq!(resolve(Color::Reset, &p, (9, 9, 9)), (9, 9, 9));
        assert_eq!(resolve(Color::Red, &p, (0, 0, 0)), p.ansi[1]);
        assert_eq!(resolve(Color::Indexed(9), &p, (0, 0, 0)), p.ansi[9]);
        assert_eq!(resolve(Color::Indexed(16), &p, (0, 0, 0)), (0, 0, 0));
        assert_eq!(resolve(Color::Indexed(196), &p, (0, 0, 0)), (255, 0, 0));
        assert_eq!(resolve(Color::Indexed(231), &p, (0, 0, 0)), (255, 255, 255));
        assert_eq!(resolve(Color::Indexed(232), &p, (0, 0, 0)), (8, 8, 8));
        assert_eq!(resolve(Color::Indexed(255), &p, (0, 0, 0)), (238, 238, 238));
    }

    #[test]
    fn the_svg_draws_colours_modifiers_and_wide_characters() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 2));
        buf.set_string(0, 0, "plain ", Style::new());
        buf.set_string(
            0,
            0,
            "bold",
            Style::new().fg(Color::Rgb(255, 0, 0)).add_modifier(Modifier::BOLD),
        );
        buf.set_string(
            5,
            0,
            "on",
            Style::new().bg(Color::Indexed(21)).add_modifier(Modifier::UNDERLINED),
        );
        buf.set_string(0, 1, "日本a", Style::new().fg(Color::Green).add_modifier(Modifier::ITALIC));
        buf.set_string(8, 1, "r", Style::new().fg(Color::Blue).add_modifier(Modifier::REVERSED));
        let svg = buffer_to_svg(&buf, &look());
        assert_eq!(svg, buffer_to_svg(&buf, &look()), "the same buffer, the same svg");
        assert!(svg.contains("width=\"120\" height=\"40\""));
        assert!(svg.contains("fill=\"#ff0000\" font-weight=\"700\">bold</text>"));
        // Indexed 21 is #0000ff, behind columns 5 and 6.
        assert!(svg.contains("<rect x=\"50\" y=\"0\" width=\"20\" height=\"20\" fill=\"#0000ff\"/>"));
        // Underline under both cells.
        assert!(svg.contains("<rect x=\"50\" y=\"17\" width=\"20\" height=\"1\""));
        // Wide characters take two columns each, as their own text.
        assert!(svg.contains("x=\"0\" y=\"35\" textLength=\"20\""));
        assert!(svg.contains("x=\"20\" y=\"35\" textLength=\"20\""));
        assert!(svg.contains("font-style=\"italic\""));
        // Reversed: the blue becomes the background.
        let blue = Palette::default().ansi[4];
        assert!(svg.contains(&format!("width=\"10\" height=\"20\" fill=\"{}\"/>", hex(blue))));
    }

    #[test]
    fn a_hyperlink_carried_in_a_cells_style_changes_neither_the_svg_nor_the_dump() {
        use crate::ui::link;
        let theme = Theme::new(Tier::TrueColor);
        let draw = |url: Option<&str>| {
            let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
            let word = match url {
                Some(u) => link::span("a link", Style::new(), u),
                None => ratatui::text::Span::raw("a link"),
            };
            buf.set_span(0, 0, &word, 20);
            (buffer_to_svg(&buf, &Look::of(&theme)), style_dump(&buf, &theme))
        };
        link::set(true);
        let linked = draw(Some("https://example.com/x"));
        link::set(false);
        assert_eq!(linked, draw(None));
    }

    #[test]
    fn the_style_dump_names_slots() {
        let theme = Theme::new(Tier::TrueColor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 2));
        buf.set_style(buf.area, theme.bg(Slot::BgBase));
        buf.set_string(1, 0, "Books", theme.bold(Slot::AccentPrimary).patch(theme.bg(Slot::BgSelection)));
        let dump = style_dump(&buf, &theme);
        let first = dump.lines().next().unwrap();
        assert!(first.starts_with("00 _1 | \"Books\" fg:accent.primary"), "{first}");
        assert!(first.contains("bg:bg.selection bold"), "{first}");
        assert_eq!(dump.lines().nth(1).unwrap(), "01");
    }
}
