//! Walking the parts' views with intents alone: which item pages a screen
//! reaches, which targets a view offers, and whether each page is a
//! bounded, well-formed view. Nothing here draws; a second frontend (Ink)
//! must be able to reach every page this walk reaches.

use serde::{Deserialize, Serialize};

use super::Headless;
use crate::host::Owner;
use crate::host::View;
use crate::view::{Block, Chord, Intent, Node, PaneId, PaneView, ScreenView, SectionView, TabView, Target, TreeRow};

/// The most panes a page may carry, and the most bytes its view may take as
/// JSON: a page over either is a gap, not a view a renderer can be asked for.
pub const PAGE_PANES_MOST: usize = 8;
pub const PAGE_BYTES_MOST: usize = 1 << 20;

/// What kind of page this is, by the screen it opens over, how deep it is
/// and its shape (its tabs' names, else its panes' titles). Item names are
/// not part of it: two items of one kind are one kind.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PageKind {
    pub part: String,
    pub screen: String,
    pub depth: usize,
    pub shape: Vec<String>,
}

/// One page reached, and how: the targets chosen from the screen down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageReach {
    pub kind: PageKind,
    pub title: String,
    pub via: Vec<Target>,
    /// The screen with the page open, as a renderer is sent it.
    pub view: ScreenView,
    /// Why it is not a well-formed bounded view; empty when it is one.
    pub gaps: Vec<String>,
}

/// A target a view offers, in the pane that shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offered {
    pub pane: PaneId,
    pub target: Target,
}

/// Every target the node offers to choose (headings and notes are not).
pub fn node_targets(node: &Node) -> Vec<Target> {
    match node {
        Node::List { rows, .. } => rows.iter().filter(|r| !r.heading).map(|r| r.target.clone()).collect(),
        Node::Tree(tree) => tree
            .visible()
            .into_iter()
            .filter(|(_, r)| !r.heading)
            .map(|(path, _)| TreeRow::target_of(&path))
            .collect(),
        Node::Table { rows, .. } => rows.iter().map(|r| r.target.clone()).collect(),
        Node::Entries { rows, .. } => rows.iter().map(|r| r.target.clone()).collect(),
        Node::Rich { blocks, .. } => block_targets(blocks),
        Node::Extension { fallback, .. } => node_targets(fallback),
        _ => vec![],
    }
}

/// The targets of rich blocks, those in sections too, in order.
fn block_targets(blocks: &[Block]) -> Vec<Target> {
    blocks
        .iter()
        .flat_map(|b| match b {
            Block::Line { target, .. } => target.iter().cloned().collect(),
            Block::Section { blocks, .. } => block_targets(blocks),
            _ => vec![],
        })
        .collect()
}

fn pane_targets(panes: &[PaneView]) -> Vec<Offered> {
    panes
        .iter()
        .flat_map(|p| node_targets(&p.body).into_iter().map(|target| Offered { pane: p.pane, target }))
        .collect()
}

/// The targets the view offers: those of the page when one is open, else of
/// its panes; and the page's tabs.
pub fn offered(view: &ScreenView) -> Vec<Offered> {
    match &view.page {
        Some(page) => {
            let mut out = pane_targets(&page.panes);
            out.extend((0..page.tabs.len()).map(|i| Offered { pane: PaneId::Middle, target: TabView::target(i) }));
            out
        }
        None => pane_targets(&view.panes),
    }
}

fn depth_of(view: &ScreenView) -> usize {
    view.page.as_ref().map_or(0, |p| p.crumbs.len())
}

fn shape_of(view: &ScreenView) -> Vec<String> {
    let Some(page) = &view.page else { return vec![] };
    if page.tabs.is_empty() {
        page.panes.iter().map(|p| p.title.clone()).collect()
    } else {
        page.tabs
            .iter()
            .map(|t| t.label.split(" · ").next().unwrap_or_default().to_string())
            .collect()
    }
}

/// Why the view is not one a renderer can be asked for.
pub fn view_problems(view: &ScreenView, scope: &str) -> Vec<String> {
    let mut out = Vec::new();
    if view.version != crate::view::VERSION {
        out.push(format!("version {}", view.version));
    }
    if view.scope != scope {
        out.push(format!("scope {:?} for {scope:?}", view.scope));
    }
    if let Some(page) = &view.page {
        if page.panes.is_empty() {
            out.push("a page with no panes".into());
        }
        if page.panes.len() > PAGE_PANES_MOST {
            out.push(format!("{} panes", page.panes.len()));
        }
        if page.tab >= page.tabs.len().max(1) {
            out.push(format!("tab {} of {}", page.tab, page.tabs.len()));
        }
    }
    match serde_json::to_vec(view) {
        Ok(bytes) if bytes.len() > PAGE_BYTES_MOST => out.push(format!("{} bytes", bytes.len())),
        Ok(_) => {}
        Err(e) => out.push(format!("not serialisable: {e}")),
    }
    out
}

fn esc() -> Intent {
    Intent::Key { chord: Chord::parse("esc").expect("esc is a chord") }
}

impl Headless {
    /// Registered screens' indexes in the host, in order.
    pub fn screen_count(&self) -> usize {
        self.host.screens.len()
    }

    /// Show screen `i` (its list, not a page) and return its id.
    pub fn show_screen(&mut self, i: usize) -> &'static str {
        self.host.view = View::Screen(i);
        self.close_overlays();
        self.host.screens[i].1.id
    }

    /// Leave any menu, question or page open over the screen, with keys the
    /// way a user would, until the screen's list shows.
    pub fn close_overlays(&mut self) {
        for _ in 0..8 {
            if self.menu().is_some() || self.screen().is_some_and(|v| v.page.is_some()) {
                self.send(esc());
            } else {
                break;
            }
        }
    }

    /// Choose each target of the view showing (at most `most` of them) and
    /// record every page that opens, then go on from the page up to
    /// `deep` pages down. The screen is left on its list.
    pub fn reach_pages(&mut self, screen: usize, most: usize, deep: usize) -> Vec<PageReach> {
        self.show_screen(screen);
        let mut out = Vec::new();
        self.walk(screen, most, deep, 1, &mut Vec::new(), &mut out);
        self.close_overlays();
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(&mut self, screen: usize, most: usize, deep: usize, depth: usize, via: &mut Vec<Target>, out: &mut Vec<PageReach>) {
        let scope = self.host.screens[screen].1.id;
        let Some(view) = self.screen() else { return };
        let base = depth_of(&view);
        let mut tried: Vec<(PaneId, usize)> = Vec::new();
        let mut done: Vec<Target> = Vec::new();
        // Choosing a row can fill another pane (a nav row fills the middle pane), so
        // what is offered is read again after each choice.
        loop {
            self.unfold();
            let Some(view) = self.screen() else { return };
            let next = offered(&view).into_iter().find(|o| {
                !done.contains(&o.target) && tried.iter().find(|(p, _)| *p == o.pane).is_none_or(|(_, n)| *n < most)
            });
            let Some(Offered { pane, target }) = next else { break };
            done.push(target.clone());
            match tried.iter_mut().find(|(p, _)| *p == pane) {
                Some((_, n)) => *n += 1,
                None => tried.push((pane, 1)),
            }
            self.focus(pane);
            self.send(Intent::Select { target: target.clone() });
            self.send(Intent::Activate { target: target.clone() });
            if self.menu().is_some() {
                self.send(esc());
            }
            let Some(now) = self.screen() else { continue };
            if depth_of(&now) > base {
                via.push(target.clone());
                // A show can land on another part's screen: the page is that part's.
                let part = self.host.screen().map_or(String::new(), |(p, _)| self.host.plugins[p].id().to_string());
                let kind = PageKind { part, screen: now.scope.clone(), depth, shape: shape_of(&now) };
                if !out.iter().any(|r| r.kind == kind) {
                    out.push(PageReach {
                        title: now.page.as_ref().map(|p| p.title.words()).unwrap_or_default(),
                        gaps: view_problems(&now, &now.scope),
                        kind,
                        via: via.clone(),
                        view: now.clone(),
                    });
                }
                if depth < deep {
                    self.walk(screen, most, deep, depth + 1, via, out);
                }
                via.pop();
            }
            // Back to where this page was chosen from.
            for _ in 0..4 {
                if self.menu().is_some() || self.screen().is_some_and(|v| depth_of(&v) > base) {
                    self.send(esc());
                } else {
                    break;
                }
            }
            if self.host.screen().is_none_or(|(_, s)| s.id != scope) {
                self.show_screen(screen);
            }
        }
    }

    /// Open every closed group of the trees showing, so the rows inside are
    /// offered (a feed's threads sit under headings).
    fn unfold(&mut self) {
        for _ in 0..3 {
            let Some(view) = self.screen() else { return };
            let panes = match &view.page {
                Some(page) => &page.panes,
                None => &view.panes,
            };
            let closed: Vec<(PaneId, Target)> = panes
                .iter()
                .filter_map(|p| match &p.body {
                    Node::Tree(t) => Some((p.pane, t)),
                    _ => None,
                })
                .flat_map(|(pane, t)| {
                    t.visible()
                        .into_iter()
                        .filter(|(path, r)| !r.children.is_empty() && !t.open.contains(path))
                        .map(move |(path, _)| (pane, TreeRow::target_of(&path)))
                        .collect::<Vec<_>>()
                })
                .collect();
            if closed.is_empty() {
                return;
            }
            for (pane, target) in closed {
                self.focus(pane);
                self.send(Intent::Fold { target, open: true });
            }
        }
    }
}

/// Where text is being typed, as view data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    /// A line at the top of a pane (a filter, go to).
    Field,
    /// A line kept at the bottom of a pane.
    BottomField,
    /// A row of a list or tree being renamed.
    Row,
    /// A multi-line editor (a comment, a reply).
    Editor,
}

/// Text being typed: what is in it and where its caret is, read from the
/// view alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capture {
    pub kind: CaptureKind,
    pub text: String,
    pub caret: usize,
}

fn node_capture(node: &Node) -> Option<Capture> {
    match node {
        Node::List { editing: Some(e), .. } => Some(Capture { kind: CaptureKind::Row, text: e.text.clone(), caret: e.caret }),
        Node::Tree(t) => t
            .editing
            .as_ref()
            .map(|e| Capture { kind: CaptureKind::Row, text: e.text.clone(), caret: e.caret }),
        Node::Extension { .. } => crate::view::Editor::of(node).map(|e| {
            let mut text = e.lines.join("\n");
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&e.line);
            Capture { kind: CaptureKind::Editor, text, caret: e.caret }
        }),
        _ => None,
    }
}

fn pane_capture(p: &PaneView) -> Option<Capture> {
    if let Some(f) = p.fields.iter().find(|f| f.caret.is_some()) {
        return Some(Capture { kind: CaptureKind::Field, text: f.text.clone(), caret: f.caret.unwrap_or_default() });
    }
    if let Some(f) = p.bottom_field.as_ref().filter(|f| f.caret.is_some()) {
        return Some(Capture { kind: CaptureKind::BottomField, text: f.text.clone(), caret: f.caret.unwrap_or_default() });
    }
    node_capture(&p.body)
}

/// The text being typed in the view, if any: the overlay's editor first,
/// then the page's panes, then the screen's.
pub fn capture_of(view: &ScreenView) -> Option<Capture> {
    view.overlay
        .as_deref()
        .and_then(pane_capture)
        .or_else(|| view.page.as_ref().and_then(|p| p.panes.iter().find_map(pane_capture)))
        .or_else(|| view.panes.iter().find_map(pane_capture))
}

impl Headless {
    /// Whether the host takes typed keys as text now.
    pub fn capturing(&self) -> bool {
        self.host.capturing()
    }

    /// The text being typed, as the screen's view shows it.
    pub fn capture(&self) -> Option<Capture> {
        capture_of(&self.screen()?)
    }
}

impl Headless {
    /// The host chrome (top bar, status line, menu, settings navigation) as
    /// view data, in the room the frontend has.
    pub fn chrome(&self) -> crate::view::ChromeView {
        self.host.chrome_view(self.width, self.height)
    }

    /// Every plugin-owned settings section as a view, with its part's id.
    pub fn section_views(&self) -> Vec<(String, SectionView)> {
        self.host
            .sections
            .iter()
            .filter_map(|(owner, s)| {
                let Owner::Plugin(p) = owner else { return None };
                let view = self.host.section_view_in(*p, s.id, self.width, self.height)?;
                Some((self.host.plugins[*p].id().to_string(), view))
            })
            .collect()
    }

    /// Every screen's list as a view, with its part's id.
    pub fn screen_views(&mut self) -> Vec<(String, ScreenView)> {
        (0..self.screen_count())
            .filter_map(|i| {
                self.show_screen(i);
                let view = self.screen()?;
                Some((self.host.plugins[self.host.screens[i].0].id().to_string(), view))
            })
            .collect()
    }
}
