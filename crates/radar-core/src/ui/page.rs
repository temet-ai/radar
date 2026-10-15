//! The page frame: what an item's own page has above its body. A breadcrumb
//! (`Shelves › a › api › #12`), the title with its details at the right, an
//! optional line under it, and the tabs, drawn by ratatui's `Tabs`. The body
//! is the caller's; scroll it with [`super::scroll`] or a
//! [`super::table::Table`].

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Tabs, Widget};

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
    pub tabs: Vec<String>,
    /// The tab showing.
    pub tab: usize,
    /// The tab row has the keys (`Cx::tabs`): the tab showing stands out
    /// as a focused row does, and `← →` move across the row.
    pub focused: bool,
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

/// `[Overview]  Changes · 2  Threads`: the tab showing in brackets and the
/// accent, two spaces between; with the row `focused` (it has the keys)
/// the tab showing stands out as the top bar's does. Returns each tab's
/// area in its original position (an empty rectangle for a hidden tab).
/// When the row is too short, show a window containing the selected tab.
pub fn tab_row(
    theme: &Theme,
    area: Rect,
    buf: &mut Buffer,
    labels: &[String],
    on: usize,
    focused: bool,
) -> Vec<Rect> {
    let mut rects = vec![Rect::default(); labels.len()];
    if area.is_empty() || labels.is_empty() {
        return rects;
    }
    let mut titles: Vec<String> = labels
        .iter()
        .enumerate()
        .map(|(i, l)| if i == on { format!("[{l}]") } else { l.clone() })
        .collect();

    // Keep as many preceding tabs as fit without cutting the selection.
    // When all labels fit this starts at zero, preserving the usual row.
    let width = usize::from(area.width);
    let mut first = if on < labels.len() { on } else { 0 };
    let mut used = super::width(&titles[first]).min(width);
    while first > 0 {
        let before = super::width(&titles[first - 1]) + 2;
        if before > width - used {
            break;
        }
        used += before;
        first -= 1;
    }
    if let Some(title) = titles.get_mut(on)
        && super::width(title) > width
    {
        // A single long label still looks selected, even on a tiny row.
        *title = if width >= 3 {
            format!("[{}]", super::fit(&labels[on], width - 2))
        } else {
            "…".into()
        };
    }
    let end = area.right();
    let mut x = area.x;
    for (i, t) in titles.iter().enumerate().skip(first) {
        if x >= end {
            break;
        }
        let w = super::width(t).min(usize::from(end - x)) as u16;
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
    let showing = if focused {
        theme
            .bold(Slot::BgBase)
            .patch(Style::new().bg(theme.s(Slot::AccentPrimary).fg.unwrap_or_default()))
    } else {
        theme.bold(Slot::AccentPrimary)
    };
    Tabs::new(titles.into_iter().skip(first))
        .select(on.checked_sub(first).filter(|_| on < labels.len()))
        .padding("", "")
        .divider("  ")
        .style(theme.s(Slot::TextSecondary))
        .highlight_style(showing)
        .render(area, buf);
    rects
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
        let labels = labels.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let rects = tab_row(&Theme::new(Tier::Mono), area, &mut buf, &labels, on, false);
        (crate::shell::row_text(&buf, 1), rects)
    }

    #[test]
    fn fitting_tabs_keep_their_labels_spacing_and_hit_indices() {
        let (text, rects) = row(&["Overview", "Changes · 2", "Threads", "Checks"], 1, 60);
        assert_eq!(text.trim(), "Overview  [Changes · 2]  Threads  Checks");
        assert_eq!(
            rects.iter().map(|r| r.x).collect::<Vec<_>>(),
            [1, 11, 26, 35]
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
