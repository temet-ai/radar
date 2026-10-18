//! The page frame: what an item's own page has above its body. A breadcrumb
//! (`Shelves › a › api › #12`), the title with its details at the right, an
//! optional line under it, and the names-only tabs. The body
//! is the caller's; scroll it with [`super::scroll`] or a
//! [`super::table::Table`].

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::design::theme::Slot;
use crate::theme::Theme;

/// The parts of a page's frame.
#[derive(Debug, Clone, Default)]
pub struct PageFrame {
    /// From the screen down to this page; the last stands out.
    pub crumbs: Vec<Span<'static>>,
    /// The page's title, cut to fit.
    pub title: Span<'static>,
    /// At the right of the title; the first ones give way first.
    pub details: Vec<Span<'static>>,
    /// A line under the title (where it is on this machine, its parent).
    pub under: Option<Line<'static>>,
    pub tabs: Vec<Tab>,
    /// The tab showing.
    pub tab: usize,
    /// The tab row has the keys (`Cx::tabs`): the tab showing stands out
    /// as a focused row does, and `← →` move across the row.
    pub focused: bool,
}

/// A page tab's name, including its optional leading icon, and attention state.
#[derive(Debug, Clone, Default)]
pub struct Tab {
    pub label: String,
    pub attention: bool,
}

impl From<String> for Tab {
    fn from(label: String) -> Self {
        Self { label, attention: false }
    }
}

impl From<&str> for Tab {
    fn from(label: &str) -> Self {
        label.to_string().into()
    }
}

/// Where the frame put things.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageAreas {
    pub body: Rect,
    /// Each tab's label with a cell either side, for clicks. Hidden tabs
    /// keep an empty rectangle so the indices stay the tabs' own.
    pub tabs: Vec<Rect>,
    /// The tab showing.
    pub tab: usize,
    /// Where the line under the title was drawn (its first cell is the
    /// line's start), when there is one.
    pub under: Option<Rect>,
}

impl PageFrame {
    pub fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer) -> PageAreas {
        let [head, title, under, tabs, body] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(u16::from(self.under.is_some())),
            Constraint::Length(u16::from(!self.tabs.is_empty())),
            Constraint::Fill(1),
        ])
        .areas(area);
        let head = inset(head);
        buf.set_line(
            head.x,
            head.y,
            &crumbs(theme, self.crumbs.clone()),
            head.width,
        );
        title_line(buf, inset(title), self.title.clone(), self.details.clone());
        let under = self.under.as_ref().map(|line| {
            let r = inset(under);
            buf.set_line(r.x, r.y, line, r.width);
            r
        });
        let tabs = if self.tabs.is_empty() {
            vec![]
        } else {
            tab_row(theme, inset(tabs), buf, &self.tabs, self.tab, self.focused)
        };
        PageAreas {
            body,
            tabs,
            tab: self.tab,
            under,
        }
    }
}

impl PageAreas {
    /// Mark the tabs clickable, as `tab:N`, and the body as `pane`'s
    /// background: every tab but the one showing (a click there would do
    /// nothing, so it is not lit under the pointer either).
    pub fn hit(&self, cx: &crate::view_tui::Cx<'_>, pane: crate::plugin::Pane) {
        cx.hit_pane(self.body, pane);
        for (i, r) in self.tabs.iter().enumerate() {
            if i != self.tab && !r.is_empty() {
                cx.hit(*r, pane, format!("tab:{i}"));
            }
        }
    }
}

/// One cell in from each side.
fn inset(r: Rect) -> Rect {
    Rect {
        x: r.x + 1,
        width: r.width.saturating_sub(2),
        ..r
    }
}

/// `Shelves › a › api › #12`: separated by muted `›`, the last in bold.
pub fn crumbs(theme: &Theme, parts: Vec<Span<'static>>) -> Line<'static> {
    let n = parts.len();
    let mut spans = Vec::new();
    for (i, mut s) in parts.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" › ", theme.s(Slot::TextMuted)));
        }
        if i + 1 == n {
            s.style = s.style.add_modifier(Modifier::BOLD);
        }
        spans.push(s);
    }
    Line::from(spans)
}

/// The title on the left, the details on the right; the title gives way,
/// and the details drop their first parts (`Alice · `) before their last.
pub fn title_line(
    buf: &mut Buffer,
    area: Rect,
    left: Span<'static>,
    mut right: Vec<Span<'static>>,
) {
    let width = |spans: &[Span<'_>]| spans.iter().map(Span::width).sum::<usize>() as u16;
    while width(&right) > area.width / 2 && right.len() > 1 {
        right.remove(0);
        if right.first().is_some_and(|s| s.content.trim() == "·") {
            right.remove(0);
        }
    }
    let right = Line::from(right);
    let rw = (right.width() as u16).min(area.width / 2);
    let lw = area.width.saturating_sub(rw + 2);
    let text = super::fit(&left.content, lw as usize);
    buf.set_span(area.x, area.y, &Span::styled(text, left.style), lw);
    if rw > 0 {
        buf.set_line(area.x + area.width - rw, area.y, &right, rw);
    }
}

/// `[Overview]  Changes  Threads ●`: the tab showing in brackets and the
/// accent, two spaces between; with the row `focused` (it has the keys)
/// the tab showing stands out as the top bar's does. Returns each tab's
/// area in its original position (an empty rectangle for a hidden tab).
/// When the row is too short, show a window containing the selected tab.
pub fn tab_row(
    theme: &Theme,
    area: Rect,
    buf: &mut Buffer,
    labels: &[Tab],
    on: usize,
    focused: bool,
) -> Vec<Rect> {
    let mut rects = vec![Rect::default(); labels.len()];
    if area.is_empty() || labels.is_empty() {
        return rects;
    }
    buf.set_style(area, theme.s(Slot::TextSecondary));
    let showing = if focused {
        theme
            .bold(Slot::BgBase)
            .patch(Style::new().bg(theme.s(Slot::AccentPrimary).fg.unwrap_or_default()))
    } else {
        theme.bold(Slot::AccentPrimary)
    };
    let base = |i| if i == on { showing } else { theme.s(Slot::TextSecondary) };
    let mut titles = labels
        .iter()
        .enumerate()
        .map(|(i, tab)| tab_title(theme, tab, i == on, base(i), None))
        .collect::<Vec<_>>();

    // Keep as many preceding tabs as fit without cutting the selection.
    // When all labels fit this starts at zero, preserving the usual row.
    let width = usize::from(area.width);
    let mut first = if on < labels.len() { on } else { 0 };
    let mut used = titles[first].width().min(width);
    while first > 0 {
        let before = titles[first - 1].width() + 2;
        if before > width - used {
            break;
        }
        used += before;
        first -= 1;
    }
    if let Some(title) = titles.get_mut(on)
        && title.width() > width
    {
        *title = tab_title(theme, &labels[on], true, showing, Some(width));
    }
    let end = area.right();
    let mut x = area.x;
    for (i, t) in titles.iter().enumerate().skip(first) {
        if x >= end {
            break;
        }
        let remaining = usize::from(end - x);
        let title = if t.width() > remaining {
            tab_title(theme, &labels[i], i == on, base(i), Some(remaining))
        } else {
            t.clone()
        };
        let w = title.width().min(remaining) as u16;
        buf.set_line(x, area.y, &title, w);
        // A cell either side: the two between tabs are shared, so there
        // is no dead cell from the first tab to the last.
        if w > 0 {
            let from = x.saturating_sub(1);
            let to = x.saturating_add(w).saturating_add(1).min(end);
            rects[i] = Rect {
                x: from,
                y: area.y,
                width: to - from,
                height: 1,
            };
        }
        x = x.saturating_add(w).saturating_add(2);
    }
    rects
}

/// Fit selected labels inside their brackets, preserving attention when it fits.
fn tab_title(theme: &Theme, tab: &Tab, selected: bool, base: Style, width: Option<usize>) -> Line<'static> {
    let suffix = if tab.attention { format!(" {}", super::icon::ATTENTION_DOT) } else { String::new() };
    let text = format!("{}{suffix}", tab.label);
    if let Some(width) = width {
        if selected && width < 3 {
            return Line::from(Span::styled("…", base));
        }
        if !selected || (tab.attention && width < 5) {
            let text = super::fit(&text, width.saturating_sub(usize::from(selected) * 2));
            return Line::from(Span::styled(if selected { format!("[{text}]") } else { text }, base));
        }
    }
    let label = match width {
        Some(width) => super::fit(&tab.label, width.saturating_sub(2 + super::width(&suffix))),
        None => tab.label.clone(),
    };
    let mut spans = Vec::new();
    if selected {
        spans.push(Span::styled("[", base));
    }
    spans.push(Span::styled(label, base));
    if tab.attention {
        spans.push(Span::styled(" ", base));
        let mut dot = super::icon::attention(theme);
        dot.style = base.patch(dot.style);
        spans.push(dot);
    }
    if selected {
        spans.push(Span::styled("]", base));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;
    use crate::design::theme::Tier;
    use crate::plugin::{Pane, Plugin};
    use crate::view_tui::Cx;

    fn row(labels: &[&str], on: usize, width: u16) -> (String, Vec<Rect>) {
        let area = Rect::new(2, 1, width, 1);
        let mut buf = Buffer::empty(Rect::new(0, 0, width + 4, 3));
        let labels = labels.iter().map(|s| Tab::from(*s)).collect::<Vec<_>>();
        let rects = tab_row(&Theme::new(Tier::Mono), area, &mut buf, &labels, on, false);
        (crate::shell::row_text(&buf, 1), rects)
    }

    #[test]
    fn fitting_tabs_keep_their_labels_spacing_and_hit_indices() {
        let (text, rects) = row(&["Overview", "Changes", "Threads", "Checks"], 1, 60);
        assert_eq!(text.trim(), "Overview  [Changes]  Threads  Checks");
        assert_eq!(
            rects.iter().map(|r| r.x).collect::<Vec<_>>(),
            [1, 11, 22, 31]
        );
        assert!(rects.iter().all(|r| !r.is_empty()));
    }

    #[test]
    fn a_late_selected_tab_stays_visible_when_the_row_shrinks() {
        let labels = ["Overview", "Changes", "Threads", "Checks"];
        for width in [60, 24, 18, 10] {
            let (text, rects) = row(&labels, 3, width);
            assert!(text.contains("[Checks]"), "{width}: {text}");
            assert!(!rects[3].is_empty());
        }
        let (text, rects) = row(&labels, 3, 18);
        assert_eq!(text.trim(), "Threads  [Checks]");
        assert!(rects[0].is_empty() && rects[1].is_empty());
        assert_eq!(rects[2].x, 1);
    }

    #[test]
    fn windowed_tab_hits_keep_original_indices_and_skip_hidden_tabs() {
        let (_, tabs) = row(&["Overview", "Changes", "Threads", "Checks"], 2, 24);
        let theme = Theme::new(Tier::Mono);
        let cursor = Cell::new(None);
        let hits = RefCell::new(Vec::new());
        let plugins: Vec<Box<dyn Plugin>> = vec![];
        let cx = Cx {
            theme: &theme,
            now: 0,
            focused: false,
            focus: Pane::Middle,
            bar: false,
            tabs: true,
            cursor: &cursor,
            hits: &hits,
            parts: crate::compose::Parts::new(&plugins, None),
        };
        PageAreas {
            tabs,
            tab: 2,
            ..PageAreas::default()
        }
        .hit(&cx, Pane::Middle);
        let hits = hits.borrow();
        let ids = hits
            .iter()
            .skip(1)
            .map(|(_, _, id)| id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["tab:1", "tab:3"]);
        assert!(hits.iter().skip(1).all(|(r, _, _)| !r.is_empty()));
    }

    #[test]
    fn oversized_selected_labels_fit_by_display_cells() {
        let (text, rects) = row(&["Overview", "日本語の検査 🧪"], 1, 10);
        assert_eq!(text.trim(), "[日本語… ]");
        assert!(rects[0].is_empty());
        assert_eq!(rects[1], Rect::new(1, 1, 11, 1));
        assert_eq!(super::super::width(text.trim()), 10);
        let (text, _) = row(&["Overview", "🧪 done"], 1, 9);
        assert_eq!(text.trim(), "[🧪 done]");
    }

    #[test]
    fn attention_dot_is_warning_coloured_and_part_of_hit_width() {
        let theme = Theme::new(Tier::TrueColor);
        let area = Rect::new(2, 1, 40, 1);
        let mut buf = Buffer::empty(Rect::new(0, 0, 44, 3));
        let tabs = [Tab::from("Overview"), Tab { label: "日本Notes".into(), attention: true }];
        let rects = tab_row(&theme, area, &mut buf, &tabs, 0, false);
        let text = crate::shell::row_text(&buf, 1);
        assert_eq!(text.trim(), format!("[Overview]  日本Notes {}", super::super::icon::ATTENTION_DOT));
        let dot = text.find(super::super::icon::ATTENTION_DOT).unwrap();
        let x = super::super::width(&text[..dot]) as u16;
        assert_eq!(Some(buf[(x, 1)].fg), theme.s(Slot::StatusWarning).fg);
        assert!(rects[1].contains(ratatui::layout::Position::new(x, 1)));
        assert_eq!(rects[1].right(), x + 2, "the dot and one padding cell are clickable");
        assert_eq!(text.matches(super::super::icon::ATTENTION_DOT).count(), 1);
        for x in [12, 13, area.right() - 1] {
            assert_eq!(buf[(x, 1)].symbol(), " ");
            assert_eq!(Some(buf[(x, 1)].fg), theme.s(Slot::TextSecondary).fg,
                "gaps and trailing cells retain the row's base style");
        }
    }

    #[test]
    fn selected_attention_survives_narrow_unicode_windows() {
        let theme = Theme::new(Tier::TrueColor);
        let tabs = [Tab::from("Overview"), Tab { label: "日本語の検査".into(), attention: true }];
        for width in [0, 1, 2, 3, 4, 5, 8, 10] {
            let area = Rect::new(2, 1, width, 1);
            let mut buf = Buffer::empty(Rect::new(0, 0, width + 4, 3));
            let rects = tab_row(&theme, area, &mut buf, &tabs, 1, true);
            assert!(rects[0].is_empty());
            assert_eq!(rects[1].is_empty(), width == 0);
            let text = crate::shell::row_text(&buf, 1);
            assert!(super::super::width(text.trim()) <= usize::from(width));
            if width >= 5 {
                let dot = text.find(super::super::icon::ATTENTION_DOT).unwrap();
                let x = super::super::width(&text[..dot]) as u16;
                assert_eq!(Some(buf[(x, 1)].fg), theme.s(Slot::StatusWarning).fg);
                assert_eq!(Some(buf[(x, 1)].bg), theme.s(Slot::AccentPrimary).fg);
                assert!(text.trim().ends_with(&format!("{}]", super::super::icon::ATTENTION_DOT)));
                assert!(rects[1].contains(ratatui::layout::Position::new(x, 1)));
            }
        }
    }

    #[test]
    fn empty_and_tiny_tab_rows_are_safe() {
        for width in 0..=3 {
            let (text, rects) = row(&["Overview", "Changes"], 1, width);
            assert!(rects[0].is_empty());
            assert_eq!(rects[1].is_empty(), width == 0);
            assert!(super::super::width(text.trim()) <= usize::from(width));
        }
        assert_eq!(row(&[], 0, 10).1, []);
        let (_, rects) = row(&["Overview"], usize::MAX, 10);
        assert_eq!(rects.len(), 1);
        let (_, rects) = row(&["", "Changes"], 1, 20);
        assert!(rects[0].is_empty() && !rects[1].is_empty());
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        let rects = tab_row(
            &Theme::new(Tier::Mono),
            Rect::new(0, 0, 10, 0),
            &mut buf,
            &["a".into()],
            0,
            false,
        );
        assert!(rects[0].is_empty());
    }
}
