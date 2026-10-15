//! What sits behind a menu, a question or help: every cell outside the
//! layer is dimmed (the scrim), and the layer throws a one-cell shadow on
//! the column right of it and the row below it. A shadow dims what is under
//! it and never erases it: every glyph and line keeps its place.
//!
//! In full colour each cell's colours are mixed toward `bg.base` (the
//! shadow's ground toward black); with fewer colours, or none, the cells
//! take SGR dim and the shadow's ground is black. The Ink renderer paints
//! the same over its finished frame (`ink/src/app/scrim.ts`).

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier};

use crate::design::theme::{Slot, Tier};
use crate::theme::{Theme, mix};

/// How far, per mille, the scrim mixes a cell's text and ground toward
/// `bg.base`, and the shadow its ground toward black and its text toward
/// `bg.base` (board 06, Overlays).
const SCRIM_FG: u32 = 720;
const SCRIM_BG: u32 = 430;
const SHADOW_BG: u32 = 600;
const SHADOW_FG: u32 = 450;

type Rgb = (u8, u8, u8);

fn rgb(c: Option<Color>) -> Option<Rgb> {
    match c {
        Some(Color::Rgb(r, g, b)) => Some((r, g, b)),
        _ => None,
    }
}

/// The colours a cell shows, read in full colour: a reset text colour is
/// `text.primary`, a reset ground `bg.base`. None below full colour, or for
/// a colour the terminal picks (an indexed or named one).
struct Ground {
    base: Option<Rgb>,
    primary: Option<Rgb>,
    tier: Tier,
}

impl Ground {
    fn of(theme: &Theme) -> Self {
        let full = theme.tier() == Tier::TrueColor;
        Self {
            base: rgb(theme.s(Slot::BgBase).fg).filter(|_| full),
            primary: rgb(theme.s(Slot::TextPrimary).fg).filter(|_| full),
            tier: theme.tier(),
        }
    }

    /// The cell's text, ground and `bg.base` as colours, when all three are.
    fn colours(&self, fg: Color, bg: Color) -> Option<(Rgb, Rgb, Rgb)> {
        let base = self.base?;
        let fg = if fg == Color::Reset { self.primary } else { rgb(Some(fg)) }?;
        let bg = if bg == Color::Reset { Some(base) } else { rgb(Some(bg)) }?;
        Some((fg, bg, base))
    }
}

/// Dim every cell of `buf` outside `keep` (the layer, drawn after this).
pub fn scrim(buf: &mut Buffer, keep: Rect, theme: &Theme) {
    let g = Ground::of(theme);
    let area = buf.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if keep.contains(Position::new(x, y)) {
                continue;
            }
            let cell = &mut buf[(x, y)];
            cell.modifier.remove(Modifier::BOLD);
            match g.colours(cell.fg, cell.bg) {
                Some((fg, bg, base)) => {
                    cell.fg = mix(fg, base, SCRIM_FG);
                    cell.bg = mix(bg, base, SCRIM_BG);
                }
                None => cell.modifier.insert(Modifier::DIM),
            }
        }
    }
}

/// The one-cell shadow of `layer`: the column right of it from its second
/// row down, and the row below it from its second column across, corner
/// included. Drawn after the layer, over the scrim.
pub fn shadow(buf: &mut Buffer, layer: Rect, theme: &Theme) {
    if layer.is_empty() {
        return;
    }
    let g = Ground::of(theme);
    let (right, bottom) = (layer.right(), layer.bottom());
    let column = (layer.y + 1..=bottom).map(|y| (right, y));
    let row = (layer.x + 1..right).map(|x| (x, bottom));
    for (x, y) in column.chain(row) {
        if !buf.area.contains(Position::new(x, y)) {
            continue;
        }
        let cell = &mut buf[(x, y)];
        match g.colours(cell.fg, cell.bg) {
            Some((fg, bg, base)) => {
                cell.bg = mix(bg, (0, 0, 0), SHADOW_BG);
                cell.fg = mix(fg, base, SHADOW_FG);
            }
            None => {
                cell.modifier.insert(Modifier::DIM);
                if g.tier != Tier::Mono {
                    cell.bg = Color::Black;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeConfig;

    fn theme(tier: Tier) -> Theme {
        Theme::build(&ThemeConfig::default(), tier, &[])
    }

    #[test]
    fn mixing_rounds_per_channel() {
        assert_eq!(mix((0, 100, 255), (255, 0, 255), 720), Color::Rgb(184, 28, 255));
        assert_eq!(mix((10, 20, 30), (0, 0, 0), 0), Color::Rgb(10, 20, 30));
        assert_eq!(mix((10, 20, 30), (0, 0, 0), 1000), Color::Rgb(0, 0, 0));
    }

    #[test]
    fn in_full_colour_the_scrim_mixes_toward_the_base_and_keeps_the_layer() {
        let t = theme(Tier::TrueColor);
        let base = rgb(t.s(Slot::BgBase).fg).unwrap();
        let text = rgb(t.s(Slot::TextPrimary).fg).unwrap();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 3));
        buf.set_string(0, 0, "└─ api", t.bold(Slot::TextPrimary));
        buf.set_string(4, 1, "menu", t.s(Slot::TextPrimary));
        scrim(&mut buf, Rect::new(3, 1, 5, 1), &t);
        let c = &buf[(0, 0)];
        assert_eq!(c.symbol(), "└", "the glyph stays");
        assert_eq!(c.fg, mix(text, base, SCRIM_FG));
        assert_eq!(c.bg, mix(base, base, SCRIM_BG), "a reset ground reads as bg.base");
        assert!(!c.modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(4, 1)].fg, t.s(Slot::TextPrimary).fg.unwrap(), "inside the layer as drawn");
    }

    #[test]
    fn the_shadow_is_one_column_right_and_one_row_below_and_keeps_its_lines() {
        let t = theme(Tier::TrueColor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 5));
        for y in 0..5 {
            buf.set_string(0, y, "│││││││││", t.s(Slot::TextMuted).patch(t.bg(Slot::BgSurface)));
        }
        let before = buf.clone();
        shadow(&mut buf, Rect::new(1, 1, 3, 2), &t);
        let shaded: Vec<(u16, u16)> = (0..5)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .filter(|&(x, y)| buf[(x, y)] != before[(x, y)])
            .collect();
        assert_eq!(shaded, vec![(4, 2), (2, 3), (3, 3), (4, 3)]);
        assert!(shaded.iter().all(|&(x, y)| buf[(x, y)].symbol() == "│"), "never erased");
        let surface = rgb(t.bg(Slot::BgSurface).bg).unwrap();
        assert_eq!(buf[(4, 3)].bg, mix(surface, (0, 0, 0), SHADOW_BG), "the corner once");
    }

    #[test]
    fn with_fewer_colours_the_scrim_is_dim_and_the_shadow_black() {
        for tier in [Tier::Ansi256, Tier::Ansi16, Tier::Mono] {
            let t = theme(tier);
            let mut buf = Buffer::empty(Rect::new(0, 0, 6, 3));
            buf.set_string(0, 0, "repo", t.bold(Slot::TextPrimary));
            scrim(&mut buf, Rect::new(0, 1, 2, 1), &t);
            let c = &buf[(1, 0)];
            assert!(c.modifier.contains(Modifier::DIM) && !c.modifier.contains(Modifier::BOLD), "{tier:?}");
            assert_eq!(c.fg, t.s(Slot::TextPrimary).fg.unwrap_or(Color::Reset), "{tier:?} keeps its colour");
            assert!(!buf[(0, 1)].modifier.contains(Modifier::DIM), "{tier:?} the layer");
            shadow(&mut buf, Rect::new(0, 1, 2, 1), &t);
            let s = &buf[(2, 2)];
            assert!(s.modifier.contains(Modifier::DIM));
            let want = if tier == Tier::Mono { Color::Reset } else { Color::Black };
            assert_eq!(s.bg, want, "{tier:?}");
        }
    }
}
