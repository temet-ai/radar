//! The legacy Ratatui frontend for [`crate::view`]: it lays out and paints
//! a part's [`ScreenView`] or [`SectionView`] with the kit's widgets, and
//! turns terminal keys and clicks into [`Intent`]s. It is an adapter during
//! the renderer migration, not the plugin contract: drawing geometry and
//! terminal event conversion live here rather than in [`crate::Plugin`].
//! Frontend state is transient (scroll offsets, what a click lands on).

use std::cell::{Cell as StdCell, RefCell};
use std::collections::HashMap;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyEvent;
#[cfg(test)]
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::text::{Line, Span};
use tui_tree_widget::TreeState;

use crate::design::theme::Slot;
use crate::plugin::{Binding, Pane};
use crate::theme::Theme;
use crate::ui::{self, icon};
use crate::view::{
    ActionSpec, Chord, Ink, Intent, Key, ListRow, Node, PaneId, ScreenView, SectionView, Seg,
    State, SubjectRef, Target, Text, Tree, TreeRow,
};
use crate::widgets;

#[cfg(test)]
mod focus_ramp_tests {
    use super::*;
    use crate::design::theme::Tier;
    use crate::theme::ThemeConfig;
    use crate::view::{FieldBar, PageView, PaneView};

    fn theme(tier: Tier) -> Theme {
        let mut config = ThemeConfig { icons: "off".into(), ..Default::default() };
        config.slots.insert("glow.purple".into(), "#102030".into());
        config.slots.insert("accent.tertiary".into(), "#405060".into());
        config.slots.insert("glow.cyan".into(), "#708090".into());
        Theme::build(&config, tier, &[])
    }

    fn view() -> ScreenView {
        let body = || Node::Document { lines: vec![Text::plain("Watch notes")], cut: false };
        let mut nav = PaneView::new(PaneId::Nav, "nav", body());
        nav.width = Some(12);
        ScreenView::new("watch", vec![nav, PaneView::new(PaneId::Middle, "middle", body())])
    }

    fn field() -> FieldBar {
        FieldBar { mark: "/".into(), text: "watch".into(), caret: Some(5), hint: Text::default() }
    }

    fn draw(view: &ScreenView, tier: Tier, focus: Pane, bar: bool, tabs: bool, allow: bool) -> Buffer {
        let theme = theme(tier);
        let cursor = StdCell::new(None);
        let hits = RefCell::new(vec![]);
        let plugins = vec![];
        let cx = Cx {
            theme: &theme, now: 0, focused: false, focus, bar, tabs,
            cursor: &cursor, hits: &hits, parts: crate::compose::Parts::new(&plugins, None),
        };
        let area = Rect::new(0, 0, 160, 8);
        let mut buffer = Buffer::empty(area);
        render_screen_with_ramp(&Frontend::default(), view, area, &mut buffer, &cx, allow);
        buffer
    }

    #[test]
    fn every_pane_body_branch_ramps_only_its_owner() {
        let t = theme(Tier::TrueColor);
        for variant in 0..3 {
            let mut view = view();
            if variant == 1 {
                view.panes[1].body = Node::Document { lines: vec![], cut: true };
            } else if variant == 2 {
                view.panes[1].notes.push(Text::plain("more context"));
            }
            let buf = draw(&view, Tier::TrueColor, Pane::Middle, false, false, true);
            assert_eq!(buf[(0, 0)].fg, t.s(Slot::AccentMuted).fg.unwrap());
            assert_eq!(buf[(12, 0)].fg, t.s(Slot::GlowPurple).fg.unwrap());
        }
    }

    #[test]
    fn caret_field_owns_the_ramp_while_navigation_border_is_quiet() {
        let t = theme(Tier::TrueColor);
        for bottom in [false, true] {
            let mut view = view();
            if bottom {
                view.panes[1].bottom_field = Some(field());
            } else {
                view.panes[1].fields.push(field());
            }
            assert_eq!(field_owner(&view), Some(PaneId::Middle));
            let flat = draw(&view, Tier::TrueColor, Pane::Nav, false, false, false);
            let ramped = draw(&view, Tier::TrueColor, Pane::Nav, false, false, true);
            assert_eq!(ramped[(0, 0)].symbol(), flat[(0, 0)].symbol());
            assert_eq!(ramped[(0, 0)].fg, t.s(Slot::AccentMuted).fg.unwrap());
            assert_eq!(ramped[(12, 0)].fg, t.s(Slot::GlowPurple).fg.unwrap());
            for x in 13..21 {
                assert_eq!(ramped[(x, 0)], flat[(x, 0)], "middle title keeps unfocused style");
            }
        }
    }

    #[test]
    fn chrome_overlay_and_visible_dock_suppress_background_ramps() {
        let base = view();
        for (bar, tabs, allow) in [(true, false, true), (false, true, true), (false, false, false)] {
            assert_eq!(
                draw(&base, Tier::TrueColor, Pane::Nav, bar, tabs, allow),
                draw(&base, Tier::TrueColor, Pane::Nav, bar, tabs, false),
            );
        }
        let mut overlaid = base.clone();
        overlaid.overlay = Some(Box::new(PaneView::new(
            PaneId::Middle, "editor", Node::Document { lines: vec![], cut: false },
        )));
        assert_eq!(
            draw(&overlaid, Tier::TrueColor, Pane::Nav, false, false, true),
            draw(&overlaid, Tier::TrueColor, Pane::Nav, false, false, false),
        );
        let mut docked = base;
        docked.dock = Some(Box::new(crate::dock::DockView { visible: true, focused: true, ..Default::default() }));
        assert_eq!(
            draw(&docked, Tier::TrueColor, Pane::Nav, false, false, true),
            draw(&docked, Tier::TrueColor, Pane::Nav, false, false, false),
        );
        docked.dock.as_mut().unwrap().visible = false;
        assert_ne!(
            draw(&docked, Tier::TrueColor, Pane::Nav, false, false, true),
            draw(&docked, Tier::TrueColor, Pane::Nav, false, false, false),
        );
    }

    #[test]
    fn stale_dock_focus_does_not_take_ramps_from_pages_or_work_areas() {
        let mut page = view();
        page.page = Some(Box::new(PageView {
            crumbs: vec![], title: Text::plain("Watch"), details: Text::default(), under: None,
            tabs: vec![], tab: 0, notice: None, panes: page.panes.clone(), layout: None,
        }));
        let mut work = view();
        work.panes[1].body = Node::Extension {
            capability: "work.watch".into(), version: 1, data: serde_json::json!({}),
            fallback: Box::new(Node::Document { lines: vec![], cut: false }),
        };
        for mut view in [page, work] {
            view.dock = Some(Box::new(crate::dock::DockView { visible: true, focused: true, ..Default::default() }));
            assert!(!view.has_dock_place());
            assert_ne!(
                draw(&view, Tier::TrueColor, Pane::Middle, false, false, true),
                draw(&view, Tier::TrueColor, Pane::Middle, false, false, false),
            );
        }
    }

    #[test]
    fn lower_tier_renderer_cells_do_not_change_with_ramp_ownership() {
        let mut view = view();
        view.panes[1].fields.push(field());
        for tier in [Tier::Ansi256, Tier::Ansi16, Tier::Mono] {
            assert_eq!(
                draw(&view, tier, Pane::Nav, false, false, true),
                draw(&view, tier, Pane::Nav, false, false, false),
            );
        }
    }
}

/// Legacy drawing context, retained here for direct renderer tests while old
/// part-specific drawing helpers are retired.
pub struct Cx<'a> {
    pub theme: &'a Theme,
    pub now: i64,
    /// Whether the panel being drawn has focus (settings sections).
    pub focused: bool,
    /// Which pane has focus (screens, which draw all their panes at once).
    /// While the top bar has the keys ([`Cx::bar`]) it is the pane `↓`
    /// goes back to, and no pane draws as focused.
    pub focus: Pane,
    /// The top bar has the keys: [`Cx::has_focus`] is false for every pane.
    pub bar: bool,
    /// The item page's tab row has the keys ([`Plugin::tabs`]): the page
    /// draws it focused (`PageFrame::focused`) and no pane draws focused.
    pub tabs: bool,
    /// Where the terminal cursor goes. A text input being edited sets it;
    /// the host shows the cursor there, and hides it otherwise.
    pub cursor: &'a std::cell::Cell<Option<ratatui::layout::Position>>,
    /// Clickable areas drawn this frame: the rectangle, the pane it belongs
    /// to, and a target id the adapter turns into an intent.
    pub hits: &'a std::cell::RefCell<Vec<(Rect, Pane, String)>>,
    /// The other parts: what is here, and what they add to what you draw.
    pub parts: crate::compose::Parts<'a>,
}

impl Cx<'_> {
    pub fn has_focus(&self, pane: Pane) -> bool {
        !self.bar && !self.tabs && self.focus == pane
    }

    /// Whether a set-up part owns `kind`.
    pub fn has(&self, kind: &str) -> bool {
        self.parts.has(kind)
    }

    /// Whether showing a subject of `kind` opens a page in radar.
    pub fn can_show(&self, kind: &str) -> bool {
        self.parts.can_show(kind)
    }

    /// Make `rect` clickable. A click focuses `pane` and targets `id`;
    /// a double click then acts like `⏎`.
    /// Later areas win over earlier ones, so mark a pane, then its rows.
    /// The line of it under the mouse pointer is lit (hover) and the
    /// pointer is a hand there, so every line of it must act on a click
    /// ([`crate::check`] clicks each one in a part's tests). Prefer
    /// [`Cx::hit_row`], [`Cx::hit_rows`] and [`Cx::hit_label`], which give
    /// the target its full size.
    pub fn hit(&self, rect: Rect, pane: Pane, id: impl Into<String>) {
        self.hits.borrow_mut().push((rect, pane, id.into()));
    }

    /// A pane's whole area: a click on its empty space or border only
    /// focuses it. Not lit under the pointer, not a target of its own.
    /// Mark it before what is in it.
    pub fn hit_pane(&self, rect: Rect, pane: Pane) {
        self.hit(rect, pane, PANE_HIT);
    }

    /// One row across the whole width of `inner` (the panel's inside), at
    /// line `y`: the row is clicked anywhere along it, not only on its text.
    pub fn hit_row(&self, inner: Rect, y: u16, pane: Pane, id: impl Into<String>) {
        if y >= inner.y && y < inner.bottom() {
            self.hit(
                Rect {
                    y,
                    height: 1,
                    ..inner
                },
                pane,
                id,
            );
        }
    }

    /// The first `rows` lines of `area`, across its width, as one target
    /// whose row the part finds from where it was clicked (a tree's
    /// `TreeState::rendered_at`): only lines that hold a row, so the space
    /// under the last one is the pane's.
    pub fn hit_rows(&self, area: Rect, rows: usize, pane: Pane, id: impl Into<String>) {
        let height = area.height.min(rows.min(u16::MAX as usize) as u16);
        if height > 0 {
            self.hit(Rect { height, ..area }, pane, id);
        }
    }

    /// A short label (a tab, a chip, a mark): one cell more on each side,
    /// and never under [`MIN_TARGET`] cells wide, kept inside `within` (the
    /// line it sits on).
    pub fn hit_label(&self, label: Rect, within: Rect, pane: Pane, id: impl Into<String>) {
        self.hit(padded(label, within), pane, id);
    }
}

/// The id [`Cx::hit_pane`] marks a pane with; a part's `click` gets it
/// for a click that only focuses.
pub const PANE_HIT: &str = "pane";

/// The narrowest a click target is, in cells.
pub const MIN_TARGET: u16 = 3;

#[cfg(test)]
mod target_tests {
    use super::*;
    use crate::Plugin;

    #[test]
    fn a_label_gets_a_cell_each_side_and_at_least_three() {
        let line = Rect::new(0, 4, 40, 1);
        assert_eq!(padded(Rect::new(10, 4, 5, 1), line), Rect::new(9, 4, 7, 1));
        assert_eq!(
            padded(Rect::new(10, 4, 1, 1), line),
            Rect::new(9, 4, 3, 1),
            "a one-cell mark"
        );
        assert_eq!(
            padded(Rect::new(0, 4, 1, 1), line),
            Rect::new(0, 4, 3, 1),
            "at the edge it grows inwards"
        );
        assert_eq!(padded(Rect::new(39, 4, 1, 1), line), Rect::new(37, 4, 3, 1));
        assert_eq!(
            padded(Rect::new(0, 4, 2, 1), Rect::new(0, 4, 2, 1)),
            Rect::new(0, 4, 2, 1),
            "no room: as big as the line"
        );
    }

    #[test]
    fn rows_span_the_panel_and_only_the_lines_that_hold_rows() {
        let theme = crate::Theme::new(crate::design::theme::Tier::Mono);
        let (cursor, hits) = (std::cell::Cell::new(None), std::cell::RefCell::new(vec![]));
        let plugins: Vec<Box<dyn Plugin>> = vec![];
        let cx = Cx {
            theme: &theme,
            now: 0,
            focused: true,
            focus: Pane::Middle,
            bar: false,
            tabs: false,
            cursor: &cursor,
            hits: &hits,
            parts: crate::compose::Parts::new(&plugins, None),
        };
        let inner = Rect::new(1, 1, 30, 10);
        cx.hit_row(inner, 3, Pane::Middle, "row:2");
        cx.hit_row(inner, 40, Pane::Middle, "outside");
        cx.hit_rows(inner, 4, Pane::Middle, "tree");
        cx.hit_pane(Rect::new(0, 0, 32, 12), Pane::Middle);
        let got: Vec<(Rect, String)> = hits
            .into_inner()
            .into_iter()
            .map(|(r, _, id)| (r, id))
            .collect();
        assert_eq!(
            got,
            [
                (Rect::new(1, 3, 30, 1), "row:2".to_string()),
                (Rect::new(1, 1, 30, 4), "tree".to_string()),
                (Rect::new(0, 0, 32, 12), PANE_HIT.to_string()),
            ]
        );
    }
}

/// `label` one cell wider on each side and at least [`MIN_TARGET`] wide,
/// inside `within`.
pub fn padded(label: Rect, within: Rect) -> Rect {
    let mut x = label.x.saturating_sub(1).max(within.x);
    let mut right = (label.right() + 1).min(within.right());
    while right - x < MIN_TARGET && (x > within.x || right < within.right()) {
        if right < within.right() {
            right += 1;
        }
        if right - x < MIN_TARGET && x > within.x {
            x -= 1;
        }
    }
    Rect {
        x,
        width: right - x,
        ..label
    }
}

/// Paint a contributed cell using the terminal theme. The shared cell stays
/// independent of this frontend's span and style types.
pub fn cell_span(cell: &crate::compose::Cell, theme: &Theme) -> Span<'static> {
    match cell.tone {
        crate::plugin::Tone::Plain => Span::raw(cell.text.clone()),
        tone => Span::styled(cell.text.clone(), widgets::tone(theme, tone)),
    }
}

pub fn cell_spans(cell: &crate::compose::Cell, theme: &Theme) -> Vec<Span<'static>> {
    let lead = cell.icon.map(|icon| icon::lead(theme, icon)).filter(|span| !span.content.is_empty());
    lead.into_iter().chain([cell_span(cell, theme)]).collect()
}

/// Marks a hit id the adapter made itself (a contributed cell), never a
/// part's target.
const OWN: char = '\u{1e}';

/// What the adapter keeps between frames: scroll offsets, tree layout, the
/// last selection (a new one scrolls the detail back to the top), and what
/// each cell hit of the last frame shows.
#[derive(Default)]
pub struct Frontend {
    scrolls: RefCell<HashMap<(String, PaneId), ui::Scroll>>,
    trees: RefCell<HashMap<String, TreeState<String>>>,
    shown: RefCell<HashMap<String, String>>,
    cells: RefCell<Vec<(Target, SubjectRef)>>,
    room: StdCell<(u16, u16)>,
    mode: StdCell<crate::writes::Mode>,
    lead: StdCell<usize>,
    pictures: RefCell<HashMap<String, ui::image::Fetch>>,
    /// Where a drag can select in the last frame ([`crate::select`]): the
    /// inside of each pane, a menu, a settings pane. Later ones are drawn
    /// over earlier ones.
    selectable: RefCell<Vec<crate::select::Area>>,
}

/// A fetch for pictures nobody can fetch (none are placed then).
fn no_pictures() -> ui::image::Fetch {
    ui::image::fetch(|_src: String| async { anyhow::bail!("no pictures here") })
}

impl Frontend {
    /// Display-only write muting. The host and owner retain dispatch authority.
    pub fn set_mode(&self, mode: crate::writes::Mode) { self.mode.set(mode); }

    /// A new frame: nothing is selectable until it is marked.
    pub fn begin_selectable(&self) {
        self.selectable.borrow_mut().clear();
    }

    /// Mark `inside` (a pane's inside, border and title left out) as a place
    /// a drag can select in; `text` when it holds text to read.
    pub fn mark_selectable(&self, inside: Rect, text: bool) {
        if !inside.is_empty() {
            self.selectable.borrow_mut().push(crate::select::Area { rect: inside, text });
        }
    }

    /// Where a drag can select, as the last frame marked it.
    pub fn selectable(&self) -> Vec<crate::select::Area> {
        self.selectable.borrow().clone()
    }

    /// How the pictures a view names under `source` are fetched, from the
    /// part ([`crate::Plugin::pictures`]), for the frames that follow.
    pub fn set_pictures(&self, source: String, fetch: ui::image::Fetch) {
        self.pictures.borrow_mut().insert(source, fetch);
    }

    /// Resolve the pictures `view` names through `part`, each frame (the
    /// account behind them may have changed).
    pub fn resolve_pictures(&self, view: &ScreenView, part: &dyn crate::Plugin) {
        for s in picture_sources(view) {
            match part.pictures(&s) {
                Some(f) => self.set_pictures(s, f),
                None => {
                    self.pictures.borrow_mut().remove(&s);
                }
            }
        }
    }

    /// How wide an icon's lead is in the pack in use (0 with icons off).
    pub fn lead(&self) -> usize {
        self.lead.get()
    }

    pub fn set_lead(&self, lead: usize) {
        self.lead.set(lead);
    }

    pub fn set_room(&self, width: u16, height: u16) {
        self.room.set((width, height));
    }

    /// The room the last frame had for the body, for views asked between
    /// frames (a key's).
    pub fn room(&self) -> (u16, u16) {
        match self.room.get() {
            (0, 0) => (120, 40),
            r => r,
        }
    }

    fn with_scroll<T>(&self, scope: &str, pane: PaneId, f: impl FnOnce(&ui::Scroll) -> T) -> T {
        let mut all = self.scrolls.borrow_mut();
        f(all.entry((scope.to_string(), pane)).or_default())
    }
}

// --- painting --------------------------------------------------------------------

pub(crate) fn slot(ink: Ink) -> Option<Slot> {
    Some(match ink {
        Ink::Plain => return None,
        Ink::Primary => Slot::TextPrimary,
        Ink::Secondary => Slot::TextSecondary,
        Ink::Muted => Slot::TextMuted,
        Ink::Emphasis => Slot::TextEmphasis,
        Ink::Accent => Slot::AccentPrimary,
        Ink::AccentMuted => Slot::AccentMuted,
        Ink::AccentSecondary => Slot::AccentSecondary,
        Ink::AccentTertiary => Slot::AccentTertiary,
        Ink::Good => Slot::StatusSuccess,
        Ink::Warn => Slot::StatusWarning,
        Ink::Bad => Slot::StatusError,
        Ink::Info => Slot::StatusInfo,
        Ink::Number => Slot::DataNumber,
        Ink::Added => Slot::DiffAdded,
        Ink::Removed => Slot::DiffRemoved,
    })
}

fn span(t: &Theme, seg: &Seg) -> Span<'static> {
    match seg.icon {
        Some(i) if seg.text.is_empty() => return icon::lead(t, i),
        Some(i) if seg.icon_lead => {
            let lead = icon::lead(t, i);
            return if lead.content.is_empty() { words(t, seg) } else { lead };
        }
        Some(i) => return icon::or(t, i, &seg.text, words(t, seg).style),
        None => {}
    }
    words(t, seg)
}

fn words(t: &Theme, seg: &Seg) -> Span<'static> {
    let mut style = match (&seg.look, slot(seg.ink), seg.strong) {
        (Some(look), _, false) => t.spec(look),
        (Some(look), _, true) => t.spec(look).add_modifier(ratatui::style::Modifier::BOLD),
        (None, None, false) => ratatui::style::Style::default(),
        (None, None, true) => t.bold(Slot::TextPrimary),
        (None, Some(s), false) => t.s(s),
        (None, Some(s), true) => t.bold(s),
    };
    if seg.marked {
        style = style.patch(ui::log::found_style(t));
    }
    match &seg.link {
        Some(url) => ui::link::span(seg.text.clone(), style, url),
        None => Span::styled(seg.text.clone(), style),
    }
}

/// A line of view text as one span in its first words' look, icons as
/// their glyphs in it: for a title the kit cuts to fit as one.
fn flat(t: &Theme, text: &Text) -> Span<'static> {
    let mut words = String::new();
    for s in &text.0 {
        match s.icon {
            Some(i) if s.text.is_empty() => words.push_str(&icon::text(t, i)),
            Some(i) if icon::on(t) => words.push_str(&icon::plain(i)),
            _ => words.push_str(&s.text),
        }
    }
    let style = text
        .0
        .iter()
        .find(|s| s.icon.is_none())
        .map(|s| self::words(t, s).style)
        .unwrap_or_default();
    Span::styled(words, style)
}

/// A line of view text as spans, icons drawn from the pack in use.
pub fn spans(t: &Theme, text: &Text) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    for s in &text.0 {
        if s.chip {
            let mut c = crate::compose::Chip::new(s.text.clone(), s.ink.tone());
            c.icon = s.icon;
            c.url = s.link.clone();
            out.extend(ui::chips::chip_spans(t, &c));
        } else {
            out.push(span(t, s));
        }
    }
    out
}

pub fn line(t: &Theme, text: &Text) -> Line<'static> {
    Line::from(spans(t, text))
}

/// A right-edge word: one span, in its first run's style.
fn word(t: &Theme, text: &Text) -> Span<'static> {
    let style = text
        .0
        .iter()
        .find(|s| s.icon.is_none())
        .map(|s| span(t, s).style)
        .unwrap_or_default();
    Span::styled(text.words(), style)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fit {
    Wide,
    Medium,
    Narrow,
}

fn fit(area: Rect, view: &ScreenView) -> Fit {
    match area.width {
        w if w >= 120 => Fit::Wide,
        w if w >= view.medium_from.unwrap_or(84) => Fit::Medium,
        _ => Fit::Narrow,
    }
}

/// What the selection is now, to scroll the detail back up when it moves
/// (and a page's body, when another page or tab shows).
fn selection(view: &ScreenView) -> String {
    if let Some(page) = &view.page {
        let crumbs: Vec<&str> = page.crumbs.iter().map(|s| s.text.as_str()).collect();
        return format!("page:{}:{}", crumbs.join("/"), page.tab);
    }
    // The selected row by its target, not its place: the same row number
    // on another page is another row.
    let middle = view.pane(PaneId::Middle).map(|p| &p.body);
    let target = |t: Option<&Target>| t.map(|t| t.0.clone()).unwrap_or_default();
    match middle {
        Some(Node::Tree(t)) => t.selected.join("/"),
        Some(Node::List { rows, selected, .. }) => target(selected.and_then(|i| rows.get(i)).map(|r| &r.target)),
        Some(Node::Table { rows, selected, .. }) => target(selected.and_then(|i| rows.get(i)).map(|r| &r.target)),
        Some(Node::Entries { rows, selected, .. }) => target(selected.and_then(|i| rows.get(i)).map(|r| &r.target)),
        _ => String::new(),
    }
}

/// The panes the keys move in: a page's while one is open.
fn panes(view: &ScreenView) -> &[crate::view::PaneView] {
    match &view.page {
        Some(page) => &page.panes,
        None => &view.panes,
    }
}

fn pane_of(view: &ScreenView, pane: PaneId) -> Option<&crate::view::PaneView> {
    panes(view).iter().find(|p| p.pane == pane)
}

/// A field with a caret takes keys even when legacy focus stays elsewhere.
fn field_owner(view: &ScreenView) -> Option<PaneId> {
    panes(view).iter().find(|pane| {
        pane.fields.iter().any(|field| field.caret.is_some())
            || pane.bottom_field.as_ref().is_some_and(|field| field.caret.is_some())
    }).map(|pane| pane.pane)
}

/// A renderer-neutral editor or overlay has precedence over tree navigation.
pub(crate) fn screen_captures_keys(view: &ScreenView) -> bool {
    view.overlay.is_some() || field_owner(view).is_some() || panes(view).iter().any(|pane| match &pane.body {
        Node::List { editing, .. } => editing.is_some(),
        Node::Tree(tree) => tree.editing.is_some(),
        _ => false,
    })
}

/// The sources of the pictures a view's rich text names, for the caller
/// to resolve through the part before drawing ([`Frontend::set_pictures`]).
pub fn picture_sources(view: &ScreenView) -> Vec<String> {
    fn collect(node: &Node, out: &mut Vec<String>) {
        let source = match node {
            Node::Rich { pictures, .. } => pictures.clone(),
            Node::Extension { fallback, .. } => {
                if let Some(content) = crate::view::detail::DetailContent::of(node) {
                    content.pictures
                } else {
                    collect(fallback, out);
                    None
                }
            }
            _ => None,
        };
        if let Some(source) = source && !out.contains(&source) { out.push(source); }
    }
    let mut out = Vec::new();
    for pane in panes(view) { collect(&pane.body, &mut out); }
    if let Some(overlay) = &view.overlay { collect(&overlay.body, &mut out); }
    out
}

/// Lay out and paint a screen in the universal layout: the nav pane, the
/// middle and the detail side by side when there is room, the middle (or
/// the detail while it has the focus) beside the nav pane when there is less,
/// and the focused pane alone when narrow.
pub fn render_screen(fe: &Frontend, view: &ScreenView, area: Rect, buf: &mut Buffer, cx: &Cx<'_>) {
    render_screen_with_ramp(fe, view, area, buf, cx, true);
}

/// Paint a screen while independently gating its focused pane's colour ramp.
/// The shell disables it when chrome owns the keys; legacy focus styles stay.
pub fn render_screen_with_ramp(
    fe: &Frontend,
    view: &ScreenView,
    area: Rect,
    buf: &mut Buffer,
    cx: &Cx<'_>,
    allow_pane_ramps: bool,
) {
    render_screen_with_ramp_and_jumps(fe, view, area, buf, cx, allow_pane_ramps, &[]);
}

/// Paint only the jump labels the host says actually dispatch on this surface.
#[allow(clippy::too_many_arguments)]
pub fn render_screen_with_ramp_and_jumps(
    fe: &Frontend,
    view: &ScreenView,
    area: Rect,
    buf: &mut Buffer,
    cx: &Cx<'_>,
    allow_pane_ramps: bool,
    jumps: &[crate::view::chrome::PaneJump],
) {
    if view.overlay.is_some() || (view.page.is_some() && !crate::motion::frames::page_identified()) {
        crate::motion::frames::context(None, None);
    }
    let owner = (allow_pane_ramps && !cx.bar && !cx.tabs
        && view.overlay.is_none()
        && !(view.has_dock_place()
            && view.dock.as_ref().is_some_and(|d| d.visible && d.focused)))
        .then(|| field_owner(view).unwrap_or_else(|| cx.focus.into()));
    let paint = PanePaint { cx, owner, jumps };
    fe.room.set((area.width, area.height));
    fe.cells.borrow_mut().clear();
    let now = selection(view);
    let moved = fe.shown.borrow().get(&view.scope) != Some(&now);
    if moved {
        fe.with_scroll(&view.scope, PaneId::Detail, ui::Scroll::top);
        if view.page.is_some() {
            fe.with_scroll(&view.scope, PaneId::Middle, ui::Scroll::top);
        }
        fe.shown.borrow_mut().insert(view.scope.clone(), now);
    }
    if let Some(page) = &view.page {
        render_page(fe, &view.scope, page, area, buf, paint);
    } else {
        render_panes(fe, view, area, buf, paint);
    }
    if let Some(overlay) = &view.overlay {
        draw_overlay(fe, &view.scope, overlay, area, buf, cx);
    }
}

/// Drawing-only ownership: never changes the legacy context's focus styles.
#[derive(Clone, Copy)]
struct PanePaint<'a, 'cx> {
    cx: &'a Cx<'cx>,
    owner: Option<PaneId>,
    jumps: &'a [crate::view::chrome::PaneJump],
}

/// An item's page: its frame (crumbs, title and details, the line under
/// it, the tabs), then its panes side by side under it.
fn render_page(fe: &Frontend, scope: &str, page: &crate::view::PageView, area: Rect, buf: &mut Buffer, paint: PanePaint<'_, '_>) {
    let cx = paint.cx;
    let t = cx.theme;
    let frame = ui::page::PageFrame {
        crumbs: page.crumbs.iter().map(|s| span(t, s)).collect(),
        title: flat(t, &page.title),
        details: spans(t, &page.details),
        under: page.under.as_ref().map(|l| line(t, l)),
        tabs: page
            .tabs
            .iter()
            .map(|tab| ui::page::Tab {
                label: match tab.icon {
                    Some(i) => icon::with(t, i, &tab.label),
                    None => tab.label.clone(),
                },
                attention: tab.attention,
            })
            .collect(),
        tab: page.tab,
        focused: cx.tabs,
    };
    let areas = frame.render(t, area, buf);
    areas.hit(cx, Pane::Middle);
    if let (Some(text), Some(row)) = (&page.under, areas.under) {
        let mut x = row.x;
        for seg in &text.0 {
            let width = span(t, seg).width() as u16;
            if let Some(target) = &seg.target && x < row.x + row.width {
                cx.hit(Rect { x, y: row.y, width: width.min(row.x + row.width - x), height: 1 }, Pane::Middle, target.0.clone());
            }
            x = x.saturating_add(width);
        }
    }
    let mut body = areas.body;
    if let Some(notice) = &page.notice && body.height > 0 {
        buf.set_line(body.x + 1, body.y, &line(t, notice), body.width.saturating_sub(2));
        body.y += 1;
        body.height -= 1;
    }
    if let Some(layout) = &page.layout {
        draw_page_layout(fe, scope, page, layout, body, buf, paint);
        return;
    }
    let shown: Vec<&crate::view::PaneView> = page.panes.iter().filter(|p| p.from.is_none_or(|w| body.width >= w)).collect();
    let rects = Layout::horizontal(shown.iter().map(|p| match p.width {
        Some(w) => Constraint::Length(w),
        None => Constraint::Fill(1),
    }))
    .split(body);
    for (pane, at) in shown.into_iter().zip(rects.iter()) {
        draw_pane(fe, scope, pane, *at, buf, paint);
    }
}

fn draw_page_layout(
    fe: &Frontend,
    scope: &str,
    page: &crate::view::PageView,
    layout: &crate::view::PageLayout,
    area: Rect,
    buf: &mut Buffer,
    paint: PanePaint<'_, '_>,
) {
    use crate::view::{Axis, PageLayout, Size};
    match layout {
        PageLayout::Pane { index } => {
            if let Some(pane) = page.panes.get(*index) { draw_pane(fe, scope, pane, area, buf, paint); }
        }
        PageLayout::Responsive { min_width, wide, narrow } => {
            draw_page_layout(fe, scope, page, if area.width >= *min_width { wide } else { narrow }, area, buf, paint);
        }
        PageLayout::Split { axis, parts } => {
            let constraints: Vec<Constraint> = parts.iter().map(|part| match part.size {
                Size::Fill(n) => Constraint::Fill(n),
                Size::Length(n) => Constraint::Length(n),
            }).collect();
            let rects = match axis {
                Axis::Horizontal => Layout::horizontal(constraints).split(area),
                Axis::Vertical => Layout::vertical(constraints).split(area),
            };
            for (part, rect) in parts.iter().zip(rects.iter()) {
                draw_page_layout(fe, scope, page, &part.layout, *rect, buf, paint);
            }
        }
    }
}

/// An editor over the foot of the body: a titled panel as tall as its
/// lines (up to two thirds of it), the last lines when they do not all
/// fit, the line being typed with the caret. Another node there is drawn
/// as a pane.
fn draw_overlay(fe: &Frontend, scope: &str, overlay: &crate::view::PaneView, area: Rect, buf: &mut Buffer, cx: &Cx<'_>) {
    use ratatui::widgets::Widget;
    let t = cx.theme;
    if area.height == 0 {
        return;
    }
    let Some(ed) = crate::view::Editor::of(&overlay.body) else {
        let height = (area.height / 3).max(5).min(area.height);
        let r = Rect { y: area.y + area.height - height, height, ..area };
        ratatui::widgets::Clear.render(r, buf);
        draw_pane(fe, scope, overlay, r, buf, PanePaint { cx, owner: None, jumps: &[] });
        return;
    };
    let want = ed.lines.len() as u16 + 4;
    let height = want.clamp(5, (area.height * 2 / 3).max(5)).min(area.height);
    let r = Rect {
        y: area.y + area.height - height,
        height,
        ..area
    };
    ratatui::widgets::Clear.render(r, buf);
    let inner = widgets::padded(ui::card::panel(t, r, buf, &ed.title, true));
    fe.mark_selectable(r.inner(ratatui::layout::Margin::new(1, 1)), true);
    if inner.height == 0 {
        return;
    }
    let room = inner.height.saturating_sub(1) as usize;
    let skip = ed.lines.len().saturating_sub(room);
    for (i, l) in ed.lines.iter().skip(skip).enumerate() {
        buf.set_line(
            inner.x,
            inner.y + i as u16,
            &Line::from(Span::styled(l.clone(), t.s(Slot::TextPrimary))),
            inner.width,
        );
    }
    let y = inner.y + (ed.lines.len() - skip) as u16;
    field(t, Rect { y, height: 1, ..inner }, buf, &ed.line, ed.caret, cx.cursor);
}

/// The screen's own panes in the universal layout.
fn render_panes(fe: &Frontend, view: &ScreenView, area: Rect, buf: &mut Buffer, paint: PanePaint<'_, '_>) {
    let cx = paint.cx;
    let get = |p: PaneId| view.pane(p);
    let (nav, mid, det) = (get(PaneId::Nav), get(PaneId::Middle), get(PaneId::Detail));
    let focus = PaneId::from(cx.focus);
    match fit(area, view) {
        Fit::Wide => {
            let nav_w = nav.map_or(0, |p| p.width.unwrap_or(26));
            let det_w = if det.is_some() { ui::detail_width(area.width) } else { 0 };
            let [a, b, c] = Layout::horizontal([
                Constraint::Length(nav_w),
                Constraint::Fill(1),
                Constraint::Length(det_w),
            ])
            .areas(area);
            let b = notice_below(view, b, buf, cx);
            for (pane, at) in [(nav, a), (mid, b), (det, c)] {
                if let Some(pane) = pane {
                    draw_pane(fe, &view.scope, pane, at, buf, paint);
                }
            }
        }
        Fit::Medium => {
            if nav.is_none() && let (Some(mid), Some(det)) = (mid, det) {
                let [a, b] = Layout::horizontal([
                    Constraint::Percentage(58), Constraint::Percentage(42),
                ]).areas(area);
                let a = notice_below(view, a, buf, cx);
                draw_pane(fe, &view.scope, mid, a, buf, paint);
                draw_pane(fe, &view.scope, det, b, buf, paint);
                return;
            }
            let nav_w = nav.map_or(0, |p| p.width.unwrap_or(26).saturating_sub(2));
            let [a, b] =
                Layout::horizontal([Constraint::Length(nav_w), Constraint::Fill(1)]).areas(area);
            let b = notice_below(view, b, buf, cx);
            if let Some(nav) = nav {
                draw_pane(fe, &view.scope, nav, a, buf, paint);
            }
            let right = if focus == PaneId::Detail { det.or(mid) } else { mid };
            if let Some(right) = right {
                draw_pane(fe, &view.scope, right, b, buf, paint);
            }
        }
        Fit::Narrow => {
            let area = notice_below(view, area, buf, cx);
            if let Some(p) = get(focus).or(mid) {
                draw_pane(fe, &view.scope, p, area, buf, paint);
            }
        }
    }
}

/// The agent dock is Ink's. Where it would sit, under the middle pane,
/// Ratatui says so in one line and gives the pane the rest of the column;
/// the nav and detail panes keep their full height. Returns the room left.
/// A hidden dock has no place, so it takes no row (Ink gives it none).
fn notice_below(view: &ScreenView, area: Rect, buf: &mut Buffer, cx: &Cx<'_>) -> Rect {
    if !view.dock.as_ref().is_some_and(|d| d.visible) || area.height <= 3 {
        return area;
    }
    let [rest, notice] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    buf.set_string(
        notice.x + 1,
        notice.y,
        ui::fit(crate::dock::NOTICE, usize::from(notice.width).saturating_sub(2)),
        cx.theme.s(Slot::TextMuted),
    );
    rest
}

fn draw_pane(
    fe: &Frontend,
    scope: &str,
    pane: &crate::view::PaneView,
    area: Rect,
    buf: &mut Buffer,
    paint: PanePaint<'_, '_>,
) {
    crate::motion::frames::with_frame(pane.pane, "body", || draw_pane_contents(fe, scope, pane, area, buf, paint));
}

fn draw_pane_contents(
    fe: &Frontend, scope: &str, pane: &crate::view::PaneView,
    area: Rect, buf: &mut Buffer, paint: PanePaint<'_, '_>,
) {
    let cx = paint.cx;
    let t = cx.theme;
    let id = pane.pane;
    let legacy = Pane::from(id);
    let focused = cx.has_focus(legacy);
    let frame_focus = ui::frame::Focus { focused, owns_keys: paint.owner == Some(id) };
    let title = pane.title_icon.map_or_else(|| pane.title.clone(), |icon| icon::with(t, icon, &pane.title));
    let title = crate::view::chrome::pane_title(paint.jumps, id, &title);
    cx.hit_pane(area, legacy);
    // Inside the border: the title, the border and its scrollbar are not text.
    if pane.foot_actions.is_none() {
        fe.mark_selectable(area.inner(ratatui::layout::Margin::new(1, 1)), pane.body.is_text());
    }
    let plain = pane.foot_actions.is_none() && pane.notes.is_empty() && pane.footer.is_none() && pane.fields.is_empty() && pane.bottom_field.is_none() && pane.head.is_none();
    if let Node::Document { lines, cut: true } = &pane.body
        && plain
    {
        use ratatui::widgets::Widget;
        let inner = widgets::padded(ui::card::panel_owned(t, area, buf, &title, frame_focus));
        let lines: Vec<Line<'static>> = lines.iter().map(|l| line(t, l)).collect();
        ratatui::widgets::Paragraph::new(lines).render(inner, buf);
        return;
    }
    if let Node::Document { lines, .. } = &pane.body
        && plain
    {
        let lines = lines.iter().map(|l| line(t, l)).collect();
        fe.with_scroll(scope, id, |s| {
            ui::card::card_owned(t, area, buf, &title, frame_focus, s, lines)
        });
        return;
    }
    let mut inner = ui::card::panel_owned(t, area, buf, &title, frame_focus);
    for f in &pane.fields {
        draw_field_bar(t, f, Rect { height: 2, ..inner }, buf, cx);
        let used = 3.min(inner.height);
        inner.y += used;
        inner.height -= used;
    }
    let most = pane.notes_most.unwrap_or(2);
    let mut inner = ui::card::shows(t, inner, buf, pane.notes.iter().map(|l| line(t, l)).collect(), most);
    if let Some(footer) = &pane.footer && inner.height > 2 {
        buf.set_line(inner.x + 1, inner.y + inner.height - 1, &line(t, footer), inner.width.saturating_sub(2));
        inner.height -= 2;
    }
    if let Some(field) = &pane.bottom_field && inner.height > 1 {
        draw_field_bar(t, field, Rect { y: inner.y + inner.height - 1, height: 1, ..inner }, buf, cx);
        inner.height -= 1;
    }
    if let Some(head) = &pane.head {
        if inner.height == 0 {
            return;
        }
        buf.set_line(inner.x + 1, inner.y, &line(t, head), inner.width.saturating_sub(2));
        inner.y += 1;
        inner.height -= 1;
    }
    let (body, foot) = ui::detail::reserve(inner, pane.foot_actions.is_some());
    inner = body;
    if pane.foot_actions.is_some() { fe.mark_selectable(inner, pane.body.is_text()); }
    if let (Some(foot), Some(actions)) = (foot, &pane.foot_actions) {
        buf.set_string(foot.x, foot.y, "─".repeat(usize::from(foot.width)), t.s(Slot::TextMuted));
        for control in ui::detail::controls(t, actions, foot.width, fe.mode.get()) {
            let width = control.line.width().min(usize::from(foot.width.saturating_sub(control.x))) as u16;
            let rect = Rect::new(foot.x + control.x, foot.y + 1, width, 1);
            buf.set_line(rect.x, rect.y, &control.line, width);
            if let Some(target) = control.target && width > 0 { cx.hit(rect, legacy, target); }
        }
    }
    if pane.inset { inner = widgets::padded(inner); }
    let frame = if pane.foot_actions.is_some() { ui::card::above(area, body) } else { area };
    draw_node(fe, scope, id, &pane.body, frame, inner, buf, cx, focused);
}

/// A line to type into at the top of a pane: its mark, the field (or what
/// was set, when it does not have the keys), what it does under it.
fn draw_field_bar(t: &Theme, f: &crate::view::FieldBar, area: Rect, buf: &mut Buffer, cx: &Cx<'_>) {
    if area.height == 0 {
        return;
    }
    buf.set_span(area.x, area.y, &Span::styled(f.mark.clone(), t.bold(Slot::AccentSecondary)), 3);
    let at = Rect {
        x: area.x + 3,
        y: area.y,
        width: area.width.saturating_sub(4),
        height: 1,
    };
    match f.caret {
        Some(caret) => field(t, at, buf, &f.text, caret, cx.cursor),
        None => {
            buf.set_span(at.x, at.y, &Span::styled(f.text.clone(), t.s(Slot::TextEmphasis)), at.width);
        }
    }
    if area.height > 1 {
        buf.set_line(area.x + 3, area.y + 1, &line(t, &f.hint), area.width.saturating_sub(4));
    }
}

/// Rich blocks as lines laid out for `width`, and where their pictures go
/// (none when they cannot be fetched: they are named instead).
pub(crate) fn rich_lines(t: &Theme, blocks: &[crate::view::Block], width: u16, pictured: bool) -> (Vec<Line<'static>>, Vec<ui::rich::Placed>) {
    rich_lines_with_urls(t, blocks, width, pictured, !ui::link::on())
}

pub(crate) fn rich_lines_with_urls(t: &Theme, blocks: &[crate::view::Block], width: u16, pictured: bool, urls: bool) -> (Vec<Line<'static>>, Vec<ui::rich::Placed>) {
    use crate::view::Block;
    fn unpictured(blocks: &mut [ui::rich::Block]) {
        for b in blocks {
            match b {
                ui::rich::Block::Media(m) => m.src.clear(),
                ui::rich::Block::Quote { blocks } | ui::rich::Block::Callout { blocks, .. } | ui::rich::Block::Details { blocks, .. } => {
                    unpictured(blocks)
                }
                ui::rich::Block::List { items } => items.iter_mut().for_each(|i| unpictured(&mut i.blocks)),
                _ => {}
            }
        }
    }
    let mut lines = Vec::new();
    let mut media = Vec::new();
    for b in blocks {
        match b {
            Block::Line { text, key, mnemonic_at, mnemonic_shift, .. } => lines.push(keyed_line(t, text, key.as_deref(), *mnemonic_at, *mnemonic_shift, width.saturating_add(1))),
            Block::Section { mark, title, detail, blocks } => {
                lines.push(ui::card::section(t, *mark, title, detail.as_deref(), width.saturating_add(1)));
                let (rows, placed) = section_rows(t, blocks, width, pictured, urls);
                media.extend(placed.into_iter().map(|m| m.moved(lines.len(), 0)));
                lines.extend(rows);
            }
            Block::Doc { doc, bar, most, urls: show_urls } => {
                let bar = bar.as_ref().map(|s| span(t, s));
                let right = bar.as_ref().map_or(0, |b| b.width() as u16);
                let mut doc = doc.clone();
                if !pictured || most.is_some() {
                    unpictured(&mut doc.blocks);
                }
                let mut doc_look = ui::rich::Look::new(t);
                doc_look.urls = show_urls.unwrap_or(urls);
                let laid = ui::rich::layout(&doc, &doc_look, width.saturating_sub(right));
                if most.is_none() {
                    media.extend(laid.media.into_iter().map(|m| m.moved(lines.len(), right)));
                }
                let mut shown = laid.lines;
                if let Some(n) = most
                    && shown.len() > *n
                {
                    shown.truncate(*n);
                    shown.push(Line::styled("…", t.s(Slot::TextMuted)));
                }
                for mut l in shown {
                    if let Some(b) = &bar {
                        l.spans.insert(0, b.clone());
                    }
                    lines.push(l);
                }
            }
            Block::Meter { stages } => {
                let stages: Vec<_> = stages.iter().map(|stage| ui::meter::Stage::new(stage.name.clone(), stage.mark, t.spec(&stage.look))).collect();
                lines.extend(ui::meter::stages(&stages, width.saturating_add(1)));
            }
            Block::Wrapped { text, prefix, ink } => {
                let style = slot(*ink).map_or_else(Default::default, |slot| t.s(slot));
                lines.extend(ui::wrapped(text, width.saturating_add(1), span(t, prefix), style));
            }
        }
    }
    (lines, media)
}

/// A word mnemonic with its optional Shift prefix, or a named/glyph shortcut
/// before the words. Missing/invalid letter metadata keeps a discoverable
/// legacy ` [k]` suffix, right-aligned when it fits and wrapped otherwise.
pub(crate) fn keyed_line(t: &Theme, text: &Text, key: Option<&str>, mnemonic_at: Option<usize>, mnemonic_shift: bool, room: u16) -> Line<'static> {
    let mut out = coalesced_line(line(t, text));
    let Some(key) = key else { return out };
    if key.is_empty() { return out; }
    if crate::mnemonic::letter(key).is_none() {
        out.spans.insert(0, Span::styled(format!("{key} "), t.bold(Slot::AccentSecondary)));
        return out;
    }
    if let Some(at) = mnemonic_at
        && crate::mnemonic::eligible(text, at)
        && crate::mnemonic::valid_at(key, &text.words(), Some(at))
    {
        out = mnemonic_line(t, text, at);
        if mnemonic_shift { out.spans.insert(0, Span::styled("shift ", t.bold(Slot::AccentSecondary))); }
        return out;
    }
    let shortcut = format!(" [{key}]");
    let (used, need) = (out.width(), ui::width(&shortcut));
    out.spans.push(Span::raw(" ".repeat(usize::from(room).saturating_sub(used + need))));
    out.spans.push(Span::styled(shortcut, t.s(Slot::TextMuted)));
    out
}

/// Preserve source-scalar offsets across hidden icon fallbacks. Splitting spans
/// only changes underline; custom looks, link metadata and chip grounds survive.
fn mnemonic_line(t: &Theme, text: &Text, at: usize) -> Line<'static> {
    let Some(range) = crate::mnemonic::grapheme_range(&text.words(), at) else { return line(t, text) };
    let mut offset = 0;
    let mut out = Vec::new();
    for seg in &text.0 {
        let end = offset + seg.text.chars().count();
        let painted = spans(t, &Text::from(vec![seg.clone()]));
        let intersects = range.start < end && range.end > offset && (seg.icon.is_none() || seg.chip);
        if !intersects {
            out.extend(painted);
        } else {
            let prefix = if seg.chip {
                painted.iter().map(|s| s.content.chars().count()).sum::<usize>()
                    .saturating_sub(seg.text.chars().count() + 1)
            } else { 0 };
            let start = prefix + range.start.saturating_sub(offset);
            let stop = prefix + (range.end.min(end) - offset);
            let mut painted_at = 0;
            for span in painted {
                let chars: Vec<_> = span.content.chars().collect();
                let next = painted_at + chars.len();
                let a = start.saturating_sub(painted_at).min(chars.len());
                let b = stop.saturating_sub(painted_at).min(chars.len());
                for (part, underlined) in [(&chars[..a], false), (&chars[a..b.max(a)], true), (&chars[b.max(a)..], false)] {
                    if !part.is_empty() {
                        out.push(Span::styled(part.iter().collect::<String>(), if underlined { span.style.add_modifier(ratatui::style::Modifier::UNDERLINED) } else { span.style }));
                    }
                }
                painted_at = next;
            }
        }
        offset = end;
    }
    coalesced_line(Line::from(out))
}

/// Keep a grapheme that crosses a source style boundary in one painted cell.
/// The base scalar's colour wins; modifiers compose, and a continuation link
/// is kept when the base has none. No text or cell width is changed.
pub(crate) fn coalesced_line(line: Line<'static>) -> Line<'static> {
    use ratatui::style::Style;
    let mut text = String::new();
    let mut styles = Vec::new();
    for span in &line.spans {
        let start = text.len();
        text.push_str(&span.content);
        styles.push((start..text.len(), line.style.patch(span.style)));
    }
    let raw = Span::raw(&text);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cursor = 0;
    for g in raw.styled_graphemes(Style::default()) {
        let start = g.symbol.as_ptr() as usize - text.as_ptr() as usize;
        let end = start + g.symbol.len();
        while cursor < styles.len() && styles[cursor].0.end <= start { cursor += 1; }
        let mut style = styles.get(cursor).map_or(line.style, |(_, style)| *style);
        let mut next = cursor + 1;
        while let Some((range, other)) = styles.get(next) {
            if range.start >= end { break; }
            style = style.add_modifier(other.add_modifier);
            if style.underline_color.is_none() { style.underline_color = other.underline_color; }
            next += 1;
        }
        match spans.last_mut() {
            Some(last) if last.style == style => last.content.to_mut().push_str(g.symbol),
            _ => spans.push(Span::styled(g.symbol.to_owned(), style)),
        }
    }
    let mut out = Line::from(spans);
    out.alignment = line.alignment;
    out
}

/// A section's blocks laid out for the room inside it (`width` less the
/// two cells it sets them in), each line broken between words to rows of
/// that room ([`wrap_line`]) and set in two cells; and where their
/// pictures go, counted from the first row.
fn section_rows(t: &Theme, blocks: &[crate::view::Block], width: u16, pictured: bool, urls: bool) -> (Vec<Line<'static>>, Vec<ui::rich::Placed>) {
    let inside = width.saturating_sub(2);
    let (lines, media) = rich_lines_with_urls(t, blocks, inside, pictured, urls);
    let mut starts = Vec::with_capacity(lines.len());
    let mut rows = Vec::new();
    for l in lines {
        starts.push(rows.len());
        rows.extend(wrap_line(l, usize::from(inside) + 1).into_iter().map(ui::card::section_body));
    }
    let media = media
        .into_iter()
        .map(|mut m| {
            m.line = starts.get(m.line).copied().unwrap_or(rows.len());
            m.x += 2;
            m
        })
        .collect();
    (rows, media)
}

/// `line` broken between words to rows at most `width` cells wide, as
/// Ink's `wrapCells` breaks it (and as `Paragraph`'s wrap without
/// trimming): a word wider than a row is cut at its edge, the spaces where
/// a row breaks are dropped. A line that fits is kept as it is.
pub(crate) fn wrap_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    let line = coalesced_line(line);
    if width == 0 || line.width() <= width {
        return vec![line];
    }
    type Cell = (String, usize, ratatui::style::Style);
    let cells: Vec<Cell> = line.styled_graphemes(ratatui::style::Style::default())
        .map(|g| (g.symbol.to_owned(), ui::width(g.symbol), g.style)).collect();
    let to_line = |cs: Vec<Cell>| {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (ch, _, style) in cs {
            match spans.last_mut() {
                Some(last) if last.style == style => last.content.to_mut().push_str(&ch),
                _ => spans.push(Span::styled(ch, style)),
            }
        }
        Line::from(spans)
    };
    // Match Paragraph's WordWrapper with trim:false. Keep leading spaces,
    // discard only the gap consumed at a wrap, and retain trailing spaces.
    // The same rows drive section painting and whole-action continuation hits.
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut row = Vec::new();
    let mut word = Vec::new();
    let mut gap = std::collections::VecDeque::<Cell>::new();
    let (mut row_width, mut word_width, mut gap_width) = (0, 0, 0);
    let mut previous_word = false;
    for cell in cells {
        let whitespace = cell.0 == "\u{200b}" || (cell.0 != "\u{a0}" && cell.0.chars().all(char::is_whitespace));
        let cell_width = cell.1;
        if cell_width > width { continue; }
        if (previous_word && whitespace) || (row.is_empty() && word_width + gap_width + cell_width > width) {
            row.extend(gap.drain(..));
            row.append(&mut word);
            row_width += gap_width + word_width;
            gap_width = 0;
            word_width = 0;
        }
        if row_width >= width || (cell_width > 0 && row_width + gap_width + word_width >= width) {
            let mut remaining = width.saturating_sub(row_width);
            rows.push(std::mem::take(&mut row));
            row_width = 0;
            while let Some(space) = gap.front() {
                if space.1 > remaining { break; }
                remaining -= space.1;
                gap_width -= space.1;
                gap.pop_front();
            }
            if whitespace && gap.is_empty() { continue; }
        }
        if whitespace {
            gap_width += cell_width;
            gap.push_back(cell);
        } else {
            word_width += cell_width;
            word.push(cell);
        }
        previous_word = !whitespace;
    }
    row.extend(gap);
    row.append(&mut word);
    if !row.is_empty() || rows.is_empty() { rows.push(row); }
    rows.into_iter().map(to_line).collect()
}

/// Where the targeted lines of `blocks` and their targeted runs are drawn,
/// as `(row, x, width, target)`: rows counted from `top` (wrapped in `room`
/// cells at the top level, as the pane wraps them; a section's rows as
/// [`section_rows`] lays them), `x` from `left`.
#[allow(clippy::too_many_arguments)]
fn rich_hits(t: &Theme, blocks: &[crate::view::Block], width: u16, room: u16, inside: bool, top: usize, left: u16, hits: &mut Vec<(usize, u16, u16, String)>) {
    use crate::view::Block;
    let rows_before = |i: usize| {
        if inside {
            // `width` is the room inside the section: its parent's is two more.
            section_rows(t, &blocks[..i], width.saturating_add(2), false, false).0.len()
        } else {
            radar_core_rich_height(&rich_lines(t, &blocks[..i], width, false).0, room)
        }
    };
    for (i, block) in blocks.iter().enumerate() {
        match block {
            Block::Line { text, target, key, mnemonic_at, mnemonic_shift } => {
                let y = top + rows_before(i);
                if let Some(target) = target {
                    let line = keyed_line(t, text, key.as_deref(), *mnemonic_at, *mnemonic_shift, width.saturating_add(1));
                    for (row, line) in wrap_line(line, usize::from(room)).iter().enumerate() {
                        if line.width() > 0 { hits.push((y + row, left, room, target.0.clone())); }
                    }
                }
                let mut x = left;
                for seg in &text.0 {
                    let w = span(t, seg).width() as u16;
                    if let Some(target) = &seg.target {
                        hits.push((y, x, w, target.0.clone()));
                    }
                    x = x.saturating_add(w);
                }
            }
            Block::Section { blocks: inner, .. } => {
                let y = top + rows_before(i) + 1;
                rich_hits(t, inner, width.saturating_sub(2), room.saturating_sub(2), true, y, left + 2, hits);
            }
            _ => {}
        }
    }
}

fn radar_core_rich_height(lines: &[Line<'_>], width: u16) -> usize {
    crate::ratatui::widgets::Paragraph::new(lines.to_vec())
        .wrap(crate::ratatui::widgets::Wrap { trim: false })
        .line_count(width)
}

#[allow(clippy::too_many_arguments)]
fn draw_detail(
    fe: &Frontend, scope: &str, id: PaneId, content: &crate::view::detail::DetailContent,
    frame: Rect, inner: Rect, buf: &mut Buffer, cx: &Cx<'_>,
) {
    let t = cx.theme;
    let fetch = content.pictures.as_ref().and_then(|source| fe.pictures.borrow().get(source).cloned());
    let laid = ui::detail::layout(t, content, inner.width, fetch.is_some());
    fe.with_scroll(scope, id, |scroll| {
        if inner.is_empty() { scroll.fit(0, 0); return; }
        let height = laid.lines.len().min(usize::from(u16::MAX)) as u16;
        ui::scroll::scrolled_with(t, scroll, frame, inner, buf, height, |area, buffer| {
            for (row, line) in laid.lines.iter().take(usize::from(area.height)).enumerate() {
                buffer.set_line(area.x, area.y + row as u16, line, area.width);
            }
        });
        let at = usize::from(scroll.at());
        if let Some(fetch) = &fetch {
            for media in &laid.media {
                let top = media.line as i32 - at as i32;
                if top + i32::from(media.rows) <= 0 || top >= i32::from(inner.height) { continue; }
                ui::image::want(&media.src, fetch);
                ui::image::draw(&media.src, media.cols, media.rows, inner, buf, media.x, top);
            }
        }
        for hit in &laid.hits {
            if hit.row >= at && hit.row < at + usize::from(inner.height) && hit.x < inner.width {
                let width = hit.width.min(inner.width - hit.x);
                if width > 0 {
                    cx.hit(Rect::new(inner.x + hit.x, inner.y + (hit.row - at) as u16, width, 1), Pane::from(id), hit.target.clone());
                }
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_node(
    fe: &Frontend,
    scope: &str,
    id: PaneId,
    node: &Node,
    frame: Rect,
    inner: Rect,
    buf: &mut Buffer,
    cx: &Cx<'_>,
    focused: bool,
) {
    let t = cx.theme;
    let legacy = Pane::from(id);
    if let Some(content) = crate::view::detail::DetailContent::of(node) {
        draw_detail(fe, scope, id, &content, frame, inner, buf, cx);
        return;
    }
    match node {
        // No room for a row: none is drawn or marked.
        Node::List { .. } if inner.height == 0 || inner.width == 0 => {}
        Node::List { rows, selected, .. } => {
            let shown = list_rows(t, rows);
            let drawn = fe.with_scroll(scope, id, |s| {
                ui::rows::RowList::new(shown)
                    .cursor(selected.unwrap_or(0))
                    .focused(focused)
                    .render(t, frame, inner, buf, s)
            });
            for (r, i) in drawn {
                cx.hit(r, legacy, rows[i].target.0.clone());
            }
        }
        Node::Tree(tree) => draw_tree(fe, scope, tree, inner, buf, cx, focused, legacy),
        Node::Document { lines, .. } => {
            let lines: Vec<Line<'static>> = lines.iter().map(|l| line(t, l)).collect();
            widgets::paragraph(buf, inner, lines);
        }
        Node::Rich { blocks, pictures } => {
            let inner = widgets::padded(inner);
            let fetch = pictures.as_ref().and_then(|s| fe.pictures.borrow().get(s).cloned());
            let (lines, media) = rich_lines(t, blocks, inner.width.saturating_sub(1), fetch.is_some());
            let mut hits = Vec::new();
            rich_hits(t, blocks, inner.width.saturating_sub(1), inner.width, false, 0, 0, &mut hits);
            let fetch = fetch.unwrap_or_else(no_pictures);
            fe.with_scroll(scope, id, |s| {
                ui::image::scrolled(t, s, frame, inner, buf, lines, &media, &fetch);
                for (y, x, width, target) in &hits {
                    let visible = *y as i32 - i32::from(s.at());
                    if visible >= 0 && visible < i32::from(inner.height) && *x < inner.width {
                        cx.hit(Rect { x: inner.x + *x, y: inner.y + visible as u16, width: (*width).min(inner.width - *x), height: 1 }, legacy, target.clone());
                    }
                }
            });
        }
        // A table needs its heading and a row.
        Node::Table { .. } if inner.height < 2 => {}
        Node::Table { columns, rows, selected, empty } => {
            let cols = ui::Columns::new(columns.iter().map(|c| {
                let col = if c.flex {
                    ui::Column::flex(c.heading.clone(), c.width as usize)
                } else {
                    ui::Column::new(c.heading.clone(), c.width as usize).keep(c.keep.min(255) as u8)
                };
                match c.icon {
                    Some(i) => col.icon(i),
                    None => col,
                }
            }));
            let cells = rows
                .iter()
                .map(|r| r.cells.iter().map(|c| (c.heading.clone(), line(t, &c.text))).collect())
                .collect();
            let drawn = fe.with_scroll(scope, id, |s| {
                ui::table::Table::lines(&cols, cells)
                    .empty(empty.as_deref().unwrap_or("Nothing here."))
                    .cursor(selected.unwrap_or(0))
                    .focused(focused)
                    .render(t, frame, inner, buf, s)
            });
            for (r, i) in drawn {
                cx.hit(r, legacy, rows[i].target.0.clone());
            }
        }
        Node::Entries { rows, selected, empty } => {
            let entries = rows.iter().map(|row| ui::entries::Entry {
                head: line(t, &row.head),
                before: row.before.iter().map(|text| line(t, text)).collect(),
                body: row.body.iter().map(|text| line(t, text)).collect(),
            }).collect();
            let mut list = ui::entries::Entries::new(entries).empty(empty).focused(focused);
            if let Some(index) = selected { list = list.cursor(*index); }
            let drawn = fe.with_scroll(scope, id, |scroll| list.render(t, frame, inner, buf, scroll));
            for (rect, index) in drawn {
                cx.hit(rect, legacy, rows[index].target.0.clone());
            }
        }
        Node::Log { lines, top, total, empty } => {
            if lines.is_empty() {
                widgets::empty(buf, inner, t, empty);
            }
            for (i, row) in lines.iter().take(inner.height as usize).enumerate() {
                let rect = Rect { y: inner.y + i as u16, height: 1, ..inner };
                buf.set_line(inner.x, rect.y, &line(t, &row.text), inner.width);
                if row.selected {
                    buf.set_style(rect, widgets::row_style(t, focused, true));
                }
                cx.hit(rect, legacy, format!("log:{}", row.row));
            }
            let shown = lines.iter().map(|row| row.row).collect::<std::collections::BTreeSet<_>>().len();
            ui::scroll::scrollbar(t, frame, buf, total.saturating_sub(shown), (*top).min(total.saturating_sub(shown)));
        }
        Node::Diff { lines, under, selected, top, split, language, empty, .. } => {
            let state = ui::diff::DiffState::new();
            state.set(*selected);
            state.set_top(*top);
            let mut view = ui::diff::DiffView::new(lines)
                .split(*split)
                .language(language.clone())
                .focused(focused)
                .empty(empty);
            for block in under {
                view = view.under(block.line, block.lines.iter().map(|text| line(t, text)).collect());
            }
            let drawn = view.render(t, frame, inner, buf, &state);
            for block in under {
                if let Some(input) = &block.input
                    && let Some((_, rect)) = drawn.under.iter().find(|(line, _)| *line == block.line)
                    && rect.height > 0 {
                    field(t, Rect { y: rect.y + rect.height - 1, height: 1, ..*rect }, buf,
                        &input.text, input.caret, cx.cursor);
                }
            }
            for (rect, index) in drawn.lines { cx.hit(rect, legacy, format!("line:{index}")); }
            for (rect, index) in drawn.numbers { cx.hit(rect, legacy, format!("comment:{index}")); }
        }
        Node::State { state, text } => {
            if *state == State::Loading {
                crate::motion::loading(inner);
            }
            ui::states::empty(t, inner, buf, text);
        }
        // An unsupported detail payload already carries its complete identity,
        // facts and actions. Give that fallback the same body room, without a
        // warning row that would change its final scroll position.
        Node::Extension { capability, fallback, .. } if capability == crate::view::detail::CAPABILITY => {
            draw_node(fe, scope, id, fallback, frame, inner, buf, cx, focused);
        }
        Node::Extension {
            capability,
            fallback,
            ..
        } => {
            // A frontend without the capability says so, and draws what the
            // part gave for frontends like it.
            let note = Line::styled(
                format!("shown simply: this needs {capability}"),
                t.s(Slot::TextMuted),
            );
            let inner = ui::card::shows(t, inner, buf, vec![note], 1);
            draw_node(fe, scope, id, fallback, frame, inner, buf, cx, focused);
        }
    }
}

fn list_rows(t: &Theme, rows: &[ListRow]) -> Vec<ui::rows::ListRow> {
    rows.iter()
        .map(|r| {
            let text = match &r.link {
                Some(url) => ui::link::line(line(t, &r.text), url),
                None => line(t, &r.text),
            };
            let mut row = if r.heading {
                ui::rows::ListRow::heading(text)
            } else {
                ui::rows::ListRow::new(text)
            };
            if let Some(detail) = &r.detail {
                row = row.detail(flat(t, detail));
            }
            match &r.right {
                Some(right) => row.right(word(t, right)),
                None => row,
            }
        })
        .collect()
}

/// A tree row as the kit's node.
fn node_of(t: &Theme, row: &TreeRow) -> ui::Node {
    let mut node = ui::Node::new(row.id.clone(), spans(t, &row.name));
    if let Some(chips) = &row.chips {
        node = node.chips(chips.gives, spans(t, &chips.text));
    }
    if let Some(look) = &row.stripe {
        node = node.stripe(t.spec(look));
    }
    if let Some(detail) = &row.detail {
        node = node.detail(flat(t, detail));
    }
    for c in &row.cells {
        node = node.cell(c.heading.clone(), line(t, &c.text));
    }
    for child in &row.children {
        node = node.child(node_of(t, child));
    }
    node
}

#[allow(clippy::too_many_arguments)]
fn draw_tree(
    fe: &Frontend,
    scope: &str,
    tree: &Tree,
    inner: Rect,
    buf: &mut Buffer,
    cx: &Cx<'_>,
    focused: bool,
    pane: Pane,
) {
    let t = cx.theme;
    let mut inner = inner;
    // Why there are no rows yet, over where they go; any rows under it.
    if let Some(note) = &tree.note {
        if note.state == State::Loading {
            crate::motion::loading(inner);
        }
        widgets::empty(buf, inner, t, &note.text);
        inner = Rect {
            y: inner.y + 3,
            height: inner.height.saturating_sub(3),
            ..inner
        };
    }
    let width = inner.width.saturating_sub(1) as usize;
    let column = |c: &crate::view::ColumnSpec| {
        let col = ui::Column::new(c.heading.clone(), c.width as usize).keep(c.keep.min(255) as u8);
        match c.icon {
            Some(i) => col.icon(i),
            None => col,
        }
    };
    let own = ui::Columns::new(tree.columns.iter().filter(|c| !c.extra).map(column));
    let extra: Vec<ui::Column> = tree.columns.iter().filter(|c| c.extra).map(column).collect();
    let name = tree.name_width as usize;
    let cols = if tree.ordered_columns {
        ui::Columns::new(tree.columns.iter().map(column)).fit(width, name)
    } else {
        own.fit(width, name).with_extra(&extra, width, name)
    };
    let mut nodes: Vec<ui::Node> = tree.rows.iter().map(|r| node_of(t, r)).collect();
    let mut trees = fe.trees.borrow_mut();
    let state = trees.entry(scope.to_string()).or_default();
    state.close_all();
    for path in &tree.open {
        state.open(path.clone());
    }
    state.select(tree.selected.clone());
    let look = match &tree.look {
        Some(l) => ui::TreeLook::from_specs(t, &l.lines, &l.band, &l.heading).band_level(l.band_level),
        None => ui::TreeLook::new(t),
    };
    // A row being typed into: the field on it, where its words were.
    let typing = tree.editing.as_ref().map(|e| {
        fn mark(nodes: &mut [ui::Node], path: &[String], at: u16) {
            let Some((first, rest)) = path.split_first() else { return };
            if let Some(n) = nodes.iter_mut().find(|n| n.id == *first) {
                if rest.is_empty() {
                    n.input_at = Some(at);
                } else {
                    mark(&mut n.children, rest, at);
                }
            }
        }
        mark(&mut nodes, &e.path, e.at);
        let mut input = crate::input::TextInput::new(&e.text);
        input.set_cursor(e.caret);
        input
    });
    let mut banded = ui::BandedTree::new(&nodes, &cols, look).focused(focused).cut_names(tree.cut_names);
    if let Some(input) = &typing {
        banded = banded.input(input, cx.cursor);
    }
    let drawn = if tree.note.is_some() || (tree.header.is_empty() && tree.header_target.is_none()) {
        banded.render(t, inner, buf, state)
    } else {
        let mut header = vec![Span::raw(" ")];
        header.extend(spans(t, &tree.header));
        if let Some(target) = &tree.header_target {
            // The header's words, as far as they show, choose its target.
            let room = width.saturating_sub(cols.width());
            let w = header.iter().map(Span::width).sum::<usize>().min(room.saturating_sub(1));
            let at = Rect { height: 1, ..inner };
            cx.hit_label(Rect { width: w as u16, ..at }, at, pane, target.0.clone());
        }
        banded.header(header).render(t, inner, buf, state)
    };
    // Each drawn line is its row's target; a contributed cell that shows
    // a subject is a target of its own over its column.
    let rows: HashMap<Vec<String>, &TreeRow> = tree.visible().into_iter().collect();
    let end = drawn.x + drawn.width.saturating_sub(1);
    let mut cells = fe.cells.borrow_mut();
    let mut motion_rows = Vec::new();
    for y in drawn.y..drawn.y + drawn.height {
        let Some(path) = state
            .rendered_at(Position { x: drawn.x, y })
            .map(<[String]>::to_vec)
        else {
            continue;
        };
        let target = TreeRow::target_of(&path);
        let Some(row) = rows.get(&path) else { continue };
        if crate::motion::frames::enabled() { motion_rows.push((path.clone(), y)); }
        if !row.heading {
            let id = if tree.pointer_action { format!("node:{}", path.last().unwrap_or(&row.id)) } else { target.0.clone() };
            cx.hit(Rect { y, height: 1, ..drawn }, pane, id);
        }
        if let Some(key) = &row.glow {
            crate::motion::row(key.clone(), Rect { y, height: 1, ..drawn });
        }
        if !ui::tree::row_columns_visible(
            path.len().saturating_sub(1), width, cols.width(), !row.cells.is_empty(),
            !row.children.is_empty() && !tree.open.contains(&path),
        ) { continue; }
        for cell in &row.cells {
            if let (Some(target), Some((x, width))) = (&cell.target, cols.at(&cell.heading, end)) {
                cx.hit(Rect { x, y, width, height: 1 }, pane, target.0.clone());
            }
            let (Some(show), Some((x, width))) = (&cell.show, cols.at(&cell.heading, end)) else {
                continue;
            };
            let hit = format!("{OWN}cell:{}", cells.len());
            cx.hit(Rect { x, y, width, height: 1 }, pane, hit);
            cells.push((target.clone(), show.clone()));
        }
    }
    if crate::motion::frames::enabled() {
        let open = tree.open.iter().filter(|path| motion_rows.iter().any(|(visible, _)| visible == *path)).cloned().collect();
        let viewport = Rect { y: drawn.y, height: inner.bottom().saturating_sub(drawn.y), width: inner.width.saturating_sub(1), ..inner };
        crate::motion::frames::tree(pane.into(), "body", viewport, motion_rows, open);
    }
}

/// Paint a settings section's rows into `inner` (inside the host's panel),
/// with the field being typed into drawn on its row.
pub fn render_section(fe: &Frontend, view: &SectionView, inner: Rect, buf: &mut Buffer, cx: &Cx<'_>) {
    let t = cx.theme;
    let Node::List {
        rows,
        selected,
        editing,
    } = &view.body
    else {
        draw_node(fe, &view.scope, PaneId::Middle, &view.body, inner, inner, buf, cx, cx.focused);
        return;
    };
    // Keep the row being typed into in view, even a new one at the end.
    let cursor = editing.as_ref().map_or(selected.unwrap_or(0), |e| e.row);
    let shown = fe.with_scroll(&view.scope, PaneId::Middle, |s| {
        ui::rows::RowList::new(list_rows(t, rows))
            .cursor(cursor)
            .focused(cx.focused)
            .render_in(t, inner, buf, s)
    });
    for (rect, i) in shown {
        if !rows[i].heading {
            cx.hit(rect, Pane::Middle, rows[i].target.0.clone());
        }
        if let Some(e) = editing
            && e.row == i
        {
            let r = Rect {
                x: rect.x + e.at,
                width: rect.width.saturating_sub(e.at + 1),
                ..rect
            };
            field(t, r, buf, &e.text, e.caret, cx.cursor);
        }
    }
}

/// The section's detail panel: its lines, wrapped and scrolled.
pub fn render_section_detail(view: &SectionView, area: Rect, buf: &mut Buffer, cx: &Cx<'_>) {
    let lines: Vec<Line<'static>> = if view.detail_blocks.is_empty() {
        view.detail.iter().map(|l| line(cx.theme, l)).collect()
    } else {
        rich_lines_with_urls(cx.theme, &view.detail_blocks, area.width, false, false).0
    };
    widgets::paragraph(buf, area, lines);
}

/// A field being typed into, on one row, scrolled to keep the caret in
/// view; the terminal cursor goes to the caret.
fn field(
    t: &Theme,
    area: Rect,
    buf: &mut Buffer,
    text: &str,
    caret: usize,
    cursor: &StdCell<Option<Position>>,
) {
    let mut input = crate::input::TextInput::new(text);
    input.set_cursor(caret);
    input.render(t, area, buf, cursor);
}

// --- input -----------------------------------------------------------------------

pub use crate::key_compat::{chord_of, key_of};

/// The actions menu's rows from a view's actions.
pub fn bindings(actions: &[ActionSpec]) -> Vec<Binding> {
    actions
        .iter()
        .map(|a| {
            let mut b = if a.heading {
                Binding::heading(a.label.clone())
            } else {
                let keys = a.key.map_or_else(|| a.id.clone(), |c| c.name());
                Binding::new(keys, a.label.clone())
            };
            b.icon = a.icon;
            b.change = a.change.map(|c| match c {
                crate::view::Change::Add => crate::writes::Change::Add,
                crate::view::Change::Edit => crate::writes::Change::Edit,
            });
            b
        })
        .collect()
}

/// What a key or click comes to on a view.
#[derive(Debug, PartialEq)]
pub enum Routed {
    /// For the part.
    Intent(Intent),
    /// The frontend used it (it scrolled).
    Done,
    /// Nobody's here: the host's own keys may take it.
    Ignored,
}

/// A key on a screen's view: movement the frontend does itself (scrolling
/// the detail, moving along a list or tree, which selects through the
/// part), `⏎` activating the selection, any other key passed on.
pub fn key(fe: &Frontend, view: &ScreenView, focus: PaneId, chord: Chord) -> Routed {
    // While something is written in, every key is the part's.
    if screen_captures_keys(view) {
        return Routed::Intent(Intent::Key { chord });
    }
    let page = matches!(chord.key, Key::PageUp | Key::PageDown) && !chord.ctrl;
    let down = chord.key == Key::PageDown;
    if page
        && view.page.is_none()
        && let Some(det) = view.pane(PaneId::Detail)
        && matches!(det.body, Node::Document { .. })
    {
        return if fe.with_scroll(&view.scope, PaneId::Detail, |s| s.page(down)) {
            Routed::Done
        } else {
            Routed::Ignored
        };
    }
    let Some(pane) = pane_of(view, focus) else {
        return Routed::Intent(Intent::Key { chord });
    };
    // Rows to move along: a list's or a table's.
    let (rows, headings) = match &pane.body {
        Node::List { rows, selected, .. } => (
            Some((rows.iter().map(|r| &r.target).collect::<Vec<_>>(), *selected)),
            rows.iter().map(|r| r.heading).collect(),
        ),
        Node::Table { rows, selected, .. } => (Some((rows.iter().map(|r| &r.target).collect::<Vec<_>>(), *selected)), vec![]),
        Node::Entries { rows, selected, .. } => (Some((rows.iter().map(|r| &r.target).collect::<Vec<_>>(), *selected)), vec![]),
        _ => (None, vec![]),
    };
    if let Some((rows, selected)) = rows {
        if selected.is_none() && !rows.is_empty() {
            return Routed::Intent(Intent::Key { chord });
        }
        let at = selected.unwrap_or(0);
        // On a page a list moves by a page; when it cannot move the key
        // is nobody's.
        if page && view.page.is_some() && !rows.is_empty() {
            let step = fe.with_scroll(&view.scope, focus, |s| s.height()).saturating_sub(1).max(1) as usize;
            let to = if down {
                (at + step).min(rows.len() - 1)
            } else {
                at.saturating_sub(step)
            };
            return if to == at {
                Routed::Ignored
            } else {
                Routed::Intent(Intent::Select {
                    target: rows[to].clone(),
                })
            };
        }
        let heading = |i: usize| headings.get(i).copied().unwrap_or(false);
        if let Some(i) = crate::view::rows_step_by(rows.len(), at, chord, heading) {
            return Routed::Intent(Intent::Select {
                target: rows[i].clone(),
            });
        }
        if chord == Chord::new(Key::Enter)
            && let Some(r) = rows.get(at)
        {
            return Routed::Intent(Intent::Activate { target: (*r).clone() });
        }
        return Routed::Intent(Intent::Key { chord });
    }
    fn scrolling(node: &Node) -> &Node {
        match node {
            Node::Extension { capability, fallback, .. } if capability == crate::view::detail::CAPABILITY => scrolling(fallback),
            _ => node,
        }
    }
    match scrolling(&pane.body) {
        Node::List { .. } | Node::Table { .. } | Node::Entries { .. } | Node::Log { .. } | Node::Diff { .. } => {}
        Node::Rich { .. } => {
            // Scrolled to the end: the part may bring more.
            let (moved, end) = fe.with_scroll(&view.scope, focus, |s| (s.key(&key_of(chord)), s.at() >= s.max()));
            match (moved, end) {
                (true, true) => return Routed::Intent(Intent::End { pane: focus }),
                (true, false) => return Routed::Done,
                _ => {}
            }
        }
        Node::Tree(tree) => {
            if let Some(i) = crate::view::tree_step(tree, chord) {
                return Routed::Intent(i);
            }
            if crate::view::tree_key(chord) && matches!(chord.key, Key::Left | Key::Right) {
                return Routed::Done;
            }
            if chord == Chord::new(Key::Enter) && !tree.selected.is_empty() {
                return Routed::Intent(Intent::Activate {
                    target: TreeRow::target_of(&tree.selected),
                });
            }
        }
        Node::Document { .. } => {
            if fe.with_scroll(&view.scope, focus, |s| s.key(&key_of(chord))) {
                return Routed::Done;
            }
        }
        Node::State { .. } | Node::Extension { .. } => {}
    }
    Routed::Intent(Intent::Key { chord })
}

/// A key in a settings section's view: movement along its rows selects
/// through the part (`↑` on the first row stays there: a section's rows
/// keep the keys, as they always have); anything else is passed on.
pub fn section_key(view: &SectionView, chord: Chord) -> Intent {
    if let Node::List {
        rows,
        selected,
        editing: None,
    } = &view.body
        && !rows.is_empty()
    {
        let at = selected.unwrap_or(0);
        let up = matches!(chord.key, Key::Up | Key::Char('k')) && !chord.ctrl && !chord.alt;
        let to = crate::view::rows_step(rows, at, chord).or_else(|| up.then_some(at));
        if let Some(i) = to {
            return Intent::Select {
                target: rows[i].target.clone(),
            };
        }
    }
    // A section's tree moves and folds as a screen's does; `↑` on its
    // first row stays there. While a row is typed into, keys are the part's.
    if let Node::Tree(tree) = &view.body
        && tree.editing.is_none()
    {
        if let Some(i) = crate::view::tree_step(tree, chord) {
            return i;
        }
        if matches!(chord.key, Key::Up | Key::Char('k')) && !chord.ctrl && !chord.alt
            && !chord.shift && tree.selectable(&tree.selected)
        {
            return Intent::Select {
                target: TreeRow::target_of(&tree.selected),
            };
        }
    }
    Intent::Key { chord }
}

/// A click (or a right click, `context`) on a hit the adapter drew.
pub fn click(fe: &Frontend, id: &str, context: bool) -> Routed {
    // Only host admission may dispatch pinned controls. Never turn a reserved
    // target into a provider row Select/Context through a fallback adapter.
    if id.starts_with(crate::view::detail::TARGET_PREFIX) { return Routed::Done; }
    // A pane's background only focuses it; it has no menu of its own.
    if id == PANE_HIT {
        return if context { Routed::Ignored } else { Routed::Done };
    }
    if let Some(n) = id
        .strip_prefix(OWN)
        .and_then(|r| r.strip_prefix("cell:"))
        .and_then(|n| n.parse::<usize>().ok())
    {
        return match fe.cells.borrow().get(n) {
            Some((row, show)) if !context => Routed::Intent(Intent::Show {
                target: row.clone(),
                subject: show.clone(),
            }),
            Some((row, _)) => Routed::Intent(Intent::Context { target: row.clone() }),
            None => Routed::Ignored,
        };
    }
    let target = Target::new(id);
    Routed::Intent(if context {
        Intent::Context { target }
    } else {
        Intent::Select { target }
    })
}

// --- one part on its own -----------------------------------------------------------

/// A part's screen or settings section, for driving one part without a
/// host (its tests, [`crate::check`]) through its view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope<'s> {
    Screen(&'s str),
    Section(&'s str),
}

fn view_cx<'a>(cx: &Cx<'a>, width: u16, height: u16) -> crate::view::ViewCx<'a> {
    crate::view::ViewCx {
        now: cx.now,
        focus: cx.focus.into(),
        width,
        height,
        parts: cx.parts,
        icon_lead: icon::lead_width(cx.theme),
        setup: false,
    }
}

/// Draw `scope` of `p` into `area`.
pub fn draw_part(fe: &Frontend, p: &dyn crate::Plugin, scope: Scope<'_>, area: Rect, buf: &mut Buffer, cx: &Cx<'_>) {
    fe.set_lead(icon::lead_width(cx.theme));
    let vcx = view_cx(cx, area.width, area.height);
    match scope {
        Scope::Screen(id) => match p.view(id, &vcx) {
            Some(v) => {
                fe.resolve_pictures(&v, p);
                render_screen(fe, &v, area, buf, cx)
            }
            None => widgets::empty(buf, area, cx.theme, "This part has no view for this screen."),
        },
        Scope::Section(id) => {
            if let Some(view) = p.section_view(id, &vcx) {
                render_section(fe, &view, area, buf, cx);
            }
        }
    }
}

fn part_view_cx<'a>(fe: &Frontend, pane: Pane, parts: crate::compose::Parts<'a>, now: i64) -> crate::view::ViewCx<'a> {
    let (width, height) = fe.room();
    crate::view::ViewCx {
        now,
        focus: pane.into(),
        width,
        height,
        parts,
        icon_lead: fe.lead(),
        setup: false,
    }
}

/// A key on `scope` of `p` with `pane` focused.
pub fn press_part(
    fe: &Frontend,
    p: &mut dyn crate::Plugin,
    scope: Scope<'_>,
    pane: Pane,
    key: KeyEvent,
    now: i64,
) -> crate::Outcome {
    let capturing = p.capturing();
    let routed = {
        let cx = part_view_cx(fe, pane, crate::compose::Parts::none(), now);
        match scope {
            Scope::Screen(id) => match (p.view(id, &cx), chord_of(&key)) {
                (Some(v), Some(c)) => Some(if capturing { Routed::Intent(Intent::Key { chord: c }) } else { key_routed(fe, &v, pane, c) }),
                (Some(_), None) => return crate::Outcome::Ignored,
                (None, _) => None,
            },
            Scope::Section(id) => match (p.section_view(id, &cx), chord_of(&key)) {
                (Some(v), Some(c)) => Some(Routed::Intent(if capturing { Intent::Key { chord: c } } else { section_key(&v, c) })),
                (Some(_), None) => return crate::Outcome::Ignored,
                (None, _) => None,
            },
        }
    };
    match (routed, scope) {
        (Some(r), Scope::Screen(id) | Scope::Section(id)) => part_routed(p, id, pane, r),
        (None, _) => crate::Outcome::Ignored,
    }
}

fn key_routed(fe: &Frontend, v: &ScreenView, pane: Pane, c: Chord) -> Routed {
    key(fe, v, pane.into(), c)
}

/// A click (`context`: a right click) on the hit `id` of `scope`.
#[allow(clippy::too_many_arguments)]
pub fn click_part(
    fe: &Frontend,
    p: &mut dyn crate::Plugin,
    scope: Scope<'_>,
    pane: Pane,
    id: &str,
    _at: Position,
    context: bool,
    now: i64,
) -> crate::Outcome {
    let has = {
        let cx = part_view_cx(fe, pane, crate::compose::Parts::none(), now);
        match scope {
            Scope::Screen(s) => p.view(s, &cx).is_some(),
            Scope::Section(s) => p.section_view(s, &cx).is_some(),
        }
    };
    let (Scope::Screen(s) | Scope::Section(s)) = scope;
    if !has {
        return crate::Outcome::Ignored;
    }
    part_routed(p, s, pane, click(fe, id, context))
}

fn part_routed(p: &mut dyn crate::Plugin, scope: &str, pane: Pane, routed: Routed) -> crate::Outcome {
    match routed {
        Routed::Intent(i) => to_part(p, scope, pane, &i),
        Routed::Done => crate::Outcome::done(),
        Routed::Ignored => crate::Outcome::Ignored,
    }
}

/// An intent to its part, as the host gives it: a contributed cell chosen
/// ([`Intent::Show`]) selects its row through the owner, then shows the
/// subject as the owner's effect.
pub fn to_part(p: &mut dyn crate::Plugin, scope: &str, pane: Pane, intent: &Intent) -> crate::Outcome {
    let Intent::Show { target, subject } = intent else {
        return p.intent(scope, pane, intent);
    };
    let select = Intent::Select {
        target: target.clone(),
    };
    let mut fx = match p.intent(scope, Pane::Middle, &select) {
        crate::Outcome::Handled(fx) => fx,
        crate::Outcome::Ignored => vec![],
    };
    fx.push(crate::Effect::Show(subject.clone().into()));
    crate::Outcome::Handled(fx)
}

/// The actions menu's rows for `scope` with `pane` focused.
pub fn keys_part(fe: &Frontend, p: &dyn crate::Plugin, scope: Scope<'_>, pane: Pane, now: i64) -> Vec<Binding> {
    let cx = part_view_cx(fe, pane, crate::compose::Parts::none(), now);
    match scope {
        Scope::Screen(id) => match p.view(id, &cx) {
            Some(v) => bindings(&v.actions),
            None => p.keys(id, pane),
        },
        Scope::Section(id) => match p.section_view(id, &cx) {
            Some(v) => bindings(&v.actions),
            None => p.section_keys(id),
        },
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod interaction_tests;

#[cfg(test)]
mod detail_tests;
