//! View data as text: what a part's tests record in place of a painted
//! screen. It reads like the screen's content and names no colour, width or
//! cell, so a look change in a renderer moves no part's snapshot.
//!
//! Per pane a rule naming it and its title (`── middle · Merge requests ──`,
//! `*` after the title when the pane has the keys), then one line per row:
//! `>` on the selected row, cells two spaces apart, chips in `[ ]`, icons as
//! `:name:`, and the row's target at the end in `{ }`. An overlay, the open
//! menu and the status line's keys follow the panes.

use crate::view::doc as rich;
use crate::view::chrome::{ChromeView, MenuView, StatusLine};
use crate::view::{Block, CellView, FieldBar, ListRow, Node, PageView, PaneId, PaneView, ScreenView, SectionView, Seg, Target, Text, Tree, TreeRow, PATH_SEP};

/// A screen: its page or its panes, then its overlay.
pub fn screen(view: &ScreenView, focus: Option<PaneId>) -> String {
    let mut out = Out::default();
    match &view.page {
        Some(page) => out.page(page, focus),
        None => {
            for pane in &view.panes {
                out.pane(pane, focus == Some(pane.pane));
            }
        }
    }
    if let Some(overlay) = &view.overlay {
        out.rule(&format!("overlay · {}", overlay.title), false);
        out.pane_body(overlay);
    }
    out.0
}

/// A settings section: its rows, then its detail.
pub fn section(view: &SectionView) -> String {
    let mut out = Out::default();
    out.rule(&format!("section · {}", view.scope), false);
    out.node(&view.body, 0);
    out.rule(&format!("detail · {}", view.detail_title), false);
    if view.detail_blocks.is_empty() {
        for line in &view.detail {
            out.line(&text(line));
        }
    } else {
        out.blocks(&view.detail_blocks, 0);
    }
    out.0
}

/// The open menu or question.
pub fn menu(menu: &MenuView) -> String {
    let mut out = Out::default();
    let kind = if menu.question { "question" } else { "menu" };
    out.rule(&format!("{kind} · {}", menu.title), false);
    if let Some(typed) = &menu.typed {
        out.line(&format!("[{typed}]"));
    }
    for (i, item) in menu.items.iter().enumerate() {
        let mark = if menu.selected == Some(i) { ">" } else { " " };
        let lead = if item.heading { "# " } else { "" };
        let mut line = format!("{mark} {lead}{}", item.label);
        if !item.keys.is_empty() {
            line = format!("{line}  ({})", item.keys);
        }
        if !item.hint.is_empty() {
            line = format!("{line}  {}", item.hint);
        }
        out.line(line.trim_end());
    }
    out.0
}

/// The status line: its scope, keys, message and counts.
pub fn status(status: &StatusLine) -> String {
    let mut out = Out::default();
    out.rule(&format!("status · {}", status.scope), false);
    if let Some(m) = &status.message {
        out.line(&m.text);
    }
    let keys: Vec<String> = status.hints.iter().map(|h| format!("{} {}", h.keys, h.label).trim().to_string()).collect();
    if !keys.is_empty() {
        out.line(&keys.join("  "));
    }
    let counts: Vec<&str> = status.counts.iter().map(|b| b.text.as_str()).collect();
    if !counts.is_empty() {
        out.line(&counts.join(" · "));
    }
    out.0
}

/// The chrome's parts that are not the body: the open menu, then the
/// status line.
pub fn chrome(chrome: &ChromeView) -> String {
    let mut s = String::new();
    if let Some(m) = &chrome.menu {
        s.push_str(&menu(m));
    }
    s.push_str(&status(&chrome.status));
    s
}

/// One line of view text: words, icons as `:name:`, chips in `[ ]`.
pub fn text(t: &Text) -> String {
    t.0.iter().map(seg).collect::<String>()
}

fn seg(s: &Seg) -> String {
    let icon = s.icon.map(|i| format!(":{}:", i.name())).unwrap_or_default();
    let gap = if icon.is_empty() { "" } else { " " };
    let words = if s.chip { format!("[{}]", s.text.trim()) } else { s.text.clone() };
    format!("{icon}{gap}{words}")
}

fn target(t: &Target) -> String {
    format!("{{{}}}", t.0.replace(PATH_SEP, "/"))
}

#[derive(Default)]
struct Out(String);

impl Out {
    fn line(&mut self, s: &str) {
        self.0.push_str(s.trim_end());
        self.0.push('\n');
    }

    fn rule(&mut self, title: &str, focused: bool) {
        let star = if focused { " *" } else { "" };
        self.line(&format!("── {title}{star} ──"));
    }

    fn page(&mut self, page: &PageView, focus: Option<PaneId>) {
        let crumbs: Vec<String> = page.crumbs.iter().map(seg).collect();
        self.rule(&format!("page · {}", crumbs.join(" / ")), false);
        let details = text(&page.details);
        self.line(&join(&[text(&page.title), details]));
        if let Some(under) = &page.under {
            self.line(&text(under));
        }
        if !page.tabs.is_empty() {
            let tabs: Vec<String> = page
                .tabs
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let name = match t.icon {
                        Some(icon) => format!(":{}: {}", icon.name(), t.label),
                        None => t.label.clone(),
                    };
                    let dot = if t.attention { " !" } else { "" };
                    if i == page.tab { format!("<{name}{dot}>") } else { format!("{name}{dot}") }
                })
                .collect();
            self.line(&tabs.join("  "));
        }
        if let Some(notice) = &page.notice {
            self.line(&format!("! {}", text(notice)));
        }
        for pane in &page.panes {
            self.pane(pane, focus == Some(pane.pane));
        }
    }

    fn pane(&mut self, pane: &PaneView, focused: bool) {
        let icon = pane.title_icon.map(|i| format!(":{}: ", i.name())).unwrap_or_default();
        let from = pane.from.map(|w| format!(" · from {w}")).unwrap_or_default();
        self.rule(&format!("{} · {icon}{}{from}", pane_name(pane.pane), pane.title), focused);
        self.pane_body(pane);
    }

    fn pane_body(&mut self, pane: &PaneView) {
        if let Some(head) = &pane.head {
            self.line(&format!("! {}", text(head)));
        }
        for f in &pane.fields {
            self.field(f);
        }
        self.node(&pane.body, 0);
        for note in &pane.notes {
            self.line(&format!("· {}", text(note)));
        }
        if let Some(footer) = &pane.footer {
            self.line(&format!("_ {}", text(footer)));
        }
        if let Some(f) = &pane.bottom_field {
            self.field(f);
        }
        if let Some(foot) = &pane.foot_actions {
            let words: Vec<String> = foot.actions.iter().chain([&foot.menu]).map(|a| format!("[{}]", a.action.label)).collect();
            self.line(&words.join(" "));
        }
    }

    fn field(&mut self, f: &FieldBar) {
        let caret = if f.caret.is_some() { "|" } else { "" };
        self.line(&join(&[format!("{} [{}{caret}]", f.mark, f.text), text(&f.hint)]));
    }

    fn node(&mut self, node: &Node, indent: usize) {
        let pad = " ".repeat(indent);
        match node {
            Node::List { rows, selected, editing } => {
                for (i, row) in rows.iter().enumerate() {
                    let typed = editing.as_ref().filter(|e| e.row == i).map(|e| format!("[{}|]", e.text));
                    self.line(&format!("{pad}{}", list_row(row, *selected == Some(i), typed)));
                }
            }
            Node::Tree(tree) => self.tree(tree, &pad),
            Node::Document { lines, .. } => {
                for l in lines {
                    self.line(&format!("{pad}{}", text(l)));
                }
            }
            Node::Rich { blocks, .. } => self.blocks(blocks, indent),
            Node::Table { columns, rows, selected, empty } => {
                let heads: Vec<&str> = columns.iter().map(|c| c.heading.as_str()).collect();
                self.line(&format!("{pad}  {}", heads.join("  ")));
                if rows.is_empty() && let Some(e) = empty {
                    self.line(&format!("{pad}  {e}"));
                }
                for (i, row) in rows.iter().enumerate() {
                    let mark = if *selected == Some(i) { ">" } else { " " };
                    self.line(&format!("{pad}{mark} {}  {}", cells(&row.cells), target(&row.target)));
                }
            }
            Node::Entries { rows, selected, empty } => {
                if rows.is_empty() {
                    self.line(&format!("{pad}  {empty}"));
                }
                for (i, row) in rows.iter().enumerate() {
                    for b in &row.before {
                        self.line(&format!("{pad}  {}", text(b)));
                    }
                    let mark = if *selected == Some(i) { ">" } else { " " };
                    self.line(&format!("{pad}{mark} {}  {}", text(&row.head), target(&row.target)));
                    for b in &row.body {
                        self.line(&format!("{pad}    {}", text(b)));
                    }
                }
            }
            Node::Log { lines, top, total, empty } => {
                self.line(&format!("{pad}log {}..{} of {total}", top + 1, top + lines.len()));
                if lines.is_empty() {
                    self.line(&format!("{pad}  {empty}"));
                }
                for l in lines {
                    let mark = if l.selected { ">" } else { " " };
                    self.line(&format!("{pad}{mark} {:>4} {}", l.row + 1, text(&l.text)));
                }
            }
            Node::Diff { lines, under, selected, top, split, language, empty, .. } => {
                let lang = language.as_deref().unwrap_or("-");
                let layout = if *split { "split" } else { "unified" };
                self.line(&format!("{pad}diff {lang} {layout} from {top}"));
                if lines.is_empty() {
                    self.line(&format!("{pad}  {empty}"));
                }
                for (i, l) in lines.iter().enumerate() {
                    use crate::view::diff::Kind;
                    let sign = match l.kind {
                        Kind::Hunk => "@",
                        Kind::Context => " ",
                        Kind::Added => "+",
                        Kind::Removed => "-",
                        Kind::Note => "\\",
                    };
                    let num = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
                    let mark = if i == *selected { ">" } else { " " };
                    self.line(&format!("{pad}{mark} {:>4} {:>4} {sign}{}", num(l.old), num(l.new), l.text));
                    for u in under.iter().filter(|u| u.line == i) {
                        for t in &u.lines {
                            self.line(&format!("{pad}      | {}", text(t)));
                        }
                        if let Some(input) = &u.input {
                            self.line(&format!("{pad}      | [{}|]", input.text));
                        }
                    }
                }
            }
            Node::State { state, text: words } => {
                let kind = match state {
                    crate::view::State::Empty => "empty",
                    crate::view::State::Loading => "loading",
                    crate::view::State::Failed => "failed",
                };
                self.line(&format!("{pad}({kind}) {words}"));
            }
            Node::Extension { capability, fallback, .. } => {
                self.line(&format!("{pad}<{capability}>"));
                self.node(fallback, indent);
            }
        }
    }

    fn tree(&mut self, tree: &Tree, pad: &str) {
        let header = text(&tree.header);
        if !header.is_empty() {
            let at = tree.header_target.as_ref().map(target).unwrap_or_default();
            self.line(&format!("{pad}{}", join(&[header, at])));
        }
        if let Some(note) = &tree.note {
            self.line(&format!("{pad}({}) {}", format!("{:?}", note.state).to_lowercase(), note.text));
        }
        let mut path = Vec::new();
        for row in &tree.rows {
            self.tree_row(tree, row, &mut path, 0, pad);
        }
    }

    fn tree_row(&mut self, tree: &Tree, row: &TreeRow, path: &mut Vec<String>, depth: usize, pad: &str) {
        path.push(row.id.clone());
        let selected = !tree.selected.is_empty() && tree.selected == *path;
        let open = tree.open.iter().any(|p| p == path);
        let mark = if selected { ">" } else { " " };
        let lead = if row.heading { "# " } else { "" };
        let mut parts = vec![format!("{lead}{}", text(&row.name))];
        if let Some(chips) = &row.chips {
            // Chip runs bring their own brackets; plain chip words get them here.
            let words = text(&chips.text);
            parts.push(if chips.text.0.iter().any(|s| s.chip) { words } else { format!("[{}]", words.trim()) });
        }
        if let Some(d) = &row.detail {
            parts.push(text(d));
        }
        let c = cells(&row.cells);
        if !c.is_empty() {
            parts.push(c);
        }
        if !row.children.is_empty() && !open {
            parts.push(format!("⋯ {}", row.children.len()));
        }
        if let Some(e) = tree.editing.as_ref().filter(|e| e.path == *path) {
            parts.push(format!("[{}|]", e.text));
        }
        parts.push(target(&TreeRow::target_of(path)));
        let indent = "  ".repeat(depth);
        self.line(&format!("{pad}{mark} {indent}{}", join(&parts)));
        if open {
            for child in &row.children {
                self.tree_row(tree, child, path, depth + 1, pad);
            }
        }
        path.pop();
    }

    fn blocks(&mut self, blocks: &[Block], indent: usize) {
        let pad = " ".repeat(indent);
        for b in blocks {
            match b {
                Block::Line { text: t, target: at, key, .. } => {
                    let key = key.as_ref().map(|k| format!("({k})")).unwrap_or_default();
                    let at = at.as_ref().map(target).unwrap_or_default();
                    self.line(&format!("{pad}{}", join(&[text(t), key, at])));
                }
                Block::Section { mark, title, detail, blocks } => {
                    let mark = mark.map(|i| format!(":{}: ", i.name())).unwrap_or_default();
                    self.line(&format!("{pad}§ {mark}{}", join(&[title.clone(), detail.clone().unwrap_or_default()])));
                    self.blocks(blocks, indent + 2);
                }
                Block::Doc { doc, bar, most, .. } => {
                    let bar = bar.as_ref().map(|b| format!("{} ", seg(b))).unwrap_or_default();
                    let mut lines = Vec::new();
                    doc_lines(&doc.blocks, "", &mut lines);
                    let shown = most.unwrap_or(usize::MAX);
                    for l in lines.iter().take(shown) {
                        self.line(&format!("{pad}{bar}{l}"));
                    }
                    if lines.len() > shown {
                        self.line(&format!("{pad}{bar}…"));
                    }
                }
                Block::Meter { stages } => {
                    let s: Vec<String> = stages.iter().map(|s| format!("{} {:?}", s.name, s.mark).to_lowercase()).collect();
                    self.line(&format!("{pad}{}", s.join(" → ")));
                }
                Block::Wrapped { text: words, prefix, .. } => {
                    self.line(&format!("{pad}{}{words}", seg(prefix)));
                }
            }
        }
    }
}

fn pane_name(p: PaneId) -> &'static str {
    match p {
        PaneId::Nav => "nav",
        PaneId::Middle => "middle",
        PaneId::Detail => "detail",
    }
}

fn list_row(row: &ListRow, selected: bool, typed: Option<String>) -> String {
    let mark = if selected { ">" } else { " " };
    let lead = if row.heading { "# " } else { "" };
    let link = row.link.as_ref().map(|l| format!("<{l}>")).unwrap_or_default();
    let parts = [
        format!("{lead}{}", text(&row.text)),
        typed.unwrap_or_default(),
        row.detail.as_ref().map(text).unwrap_or_default(),
        row.right.as_ref().map(text).unwrap_or_default(),
        link,
        target(&row.target),
    ];
    format!("{mark} {}", join(&parts))
}

fn cells(cells: &[CellView]) -> String {
    let parts: Vec<String> = cells.iter().map(|c| text(&c.text)).collect();
    join(&parts)
}

/// Parts two spaces apart, the empty ones left out.
fn join(parts: &[String]) -> String {
    parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("  ")
}

fn inlines(runs: &[rich::Inline]) -> String {
    runs.iter()
        .map(|r| match &r.kind {
            rich::Kind::Link { url } if *url != r.text => format!("{} <{url}>", r.text),
            rich::Kind::Badge { .. } => format!("[{}]", r.text),
            rich::Kind::Image { src } => format!("▣ {} <{src}>", r.text),
            rich::Kind::Break => "\n".into(),
            _ if r.marks.code => format!("`{}`", r.text),
            _ => r.text.clone(),
        })
        .collect()
}

/// A rich document's lines, unwrapped: one per paragraph or item.
fn doc_lines(blocks: &[rich::Block], pad: &str, out: &mut Vec<String>) {
    let mut push = |s: String| out.extend(s.split('\n').map(|l| format!("{pad}{l}")));
    for b in blocks {
        match b {
            rich::Block::Para { text } => push(inlines(text)),
            rich::Block::Heading { level, text } => push(format!("{} {}", "#".repeat(usize::from(*level)), inlines(text))),
            rich::Block::Code { lang, text } => {
                push(format!("```{lang}"));
                push(text.trim_end().to_string());
                push("```".into());
            }
            rich::Block::Rule => push("---".into()),
            rich::Block::Media(m) => {
                let label = if m.image { "▣ image" } else { "▤ file" };
                push([label, &m.name, &m.url].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" "));
            }
            rich::Block::Table { head, rows, .. } => {
                let row = |cells: &Vec<Vec<rich::Inline>>| cells.iter().map(|c| inlines(c)).collect::<Vec<_>>().join(" | ");
                if !head.is_empty() {
                    push(row(head));
                }
                for r in rows {
                    push(row(r));
                }
            }
            rich::Block::List { items } => {
                for item in items {
                    let marker = match item.marker {
                        rich::Marker::Bullet => "•".to_string(),
                        rich::Marker::Number(n) => format!("{n}."),
                        rich::Marker::Todo => "☐".into(),
                        rich::Marker::Done => "☑".into(),
                        rich::Marker::Decision => "◇".into(),
                    };
                    let mut inner = Vec::new();
                    doc_lines(&item.blocks, "", &mut inner);
                    for (i, l) in inner.into_iter().enumerate() {
                        if i == 0 { push(format!("{marker} {l}")) } else { push(format!("  {l}")) }
                    }
                }
            }
            rich::Block::Quote { blocks } => {
                let mut inner = Vec::new();
                doc_lines(blocks, "", &mut inner);
                for l in inner {
                    push(format!("> {l}"));
                }
            }
            rich::Block::Callout { symbol, blocks, .. } => {
                let mut inner = Vec::new();
                doc_lines(blocks, "", &mut inner);
                for (i, l) in inner.into_iter().enumerate() {
                    if i == 0 && !symbol.is_empty() { push(format!("┃ {symbol} {l}")) } else { push(format!("┃ {l}")) }
                }
            }
            rich::Block::Details { summary, blocks } => {
                push(inlines(summary));
                let mut inner = Vec::new();
                doc_lines(blocks, "", &mut inner);
                for l in inner {
                    push(format!("  {l}"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::icon::Icon;
    use crate::view::{Ink, Note, State};

    #[test]
    fn a_screen_reads_as_its_panes_rows_and_targets() {
        let tree = Tree {
            header: Text::plain("2 watches"),
            rows: vec![
                TreeRow::new("watch", Text::plain("Night Watch"))
                    .cell("Lead", Text::plain("Sam Vimes"))
                    .chips(0, Text::default().push(Seg { text: "on duty".into(), ink: Ink::Good, chip: true, ..Default::default() }))
                    .child(TreeRow::new("nobby", Text::plain("Nobby Nobbs")))
                    .child(TreeRow::new("colon", Text::plain("Fred Colon"))),
                TreeRow::new("day", Text::plain("Day Watch")).child(TreeRow::new("x", Text::plain("hidden"))),
            ],
            selected: vec!["watch".into(), "colon".into()],
            open: vec![vec!["watch".into()]],
            note: Some(Note { state: State::Loading, text: "asking the Watch".into() }),
            ..Default::default()
        };
        let nav = ListRow::heading("h", Text::plain("WATCH"));
        let mut row = ListRow::new("watch:12", Text::default().push(Seg::icon(Icon::Settings)).push(Seg::new("WATCH-12", Ink::Accent)));
        row.right = Some(Text::plain("3"));
        let mut middle = PaneView::new(PaneId::Middle, "Rota", Node::Tree(tree));
        middle.notes = vec![Text::plain("shows 2 of 2")];
        let view = ScreenView::new(
            "watch",
            vec![
                PaneView::new(PaneId::Nav, "Views", Node::List { rows: vec![nav, row], selected: Some(1), editing: None }),
                middle,
            ],
        );
        insta::assert_snapshot!(screen(&view, Some(PaneId::Middle)), @r"
        ── nav · Views ──
          # WATCH  {h}
        > :settings: WATCH-12  3  {watch:12}
        ── middle · Rota * ──
        2 watches
        (loading) asking the Watch
          Night Watch  [on duty]  Sam Vimes  {watch}
            Nobby Nobbs  {watch/nobby}
        >   Fred Colon  {watch/colon}
          Day Watch  ⋯ 1  {day}
        · shows 2 of 2
        ");
    }

    #[test]
    fn a_rich_pane_reads_as_its_blocks() {
        let doc = rich::Doc::new(vec![
            rich::Block::Heading { level: 1, text: vec![rich::Inline::text("Rota")] },
            rich::Block::Para {
                text: vec![
                    rich::Inline::text("see "),
                    rich::Inline { text: "the board".into(), kind: rich::Kind::Link { url: "https://watch.example/board".into() }, marks: Default::default() },
                ],
            },
            rich::Block::List { items: vec![rich::Item { marker: rich::Marker::Todo, blocks: vec![rich::Block::para(vec![rich::Inline::text("patrol")])] }] },
        ]);
        let blocks = vec![
            Block::keyed("act:approve", Text::plain("approve"), "a"),
            Block::section(None, "description", Some("by vimes".into()), vec![Block::doc(doc)]),
        ];
        let view = ScreenView::new("watch", vec![PaneView::new(PaneId::Detail, "WATCH-12", Node::Rich { blocks, pictures: None })]);
        insta::assert_snapshot!(screen(&view, None), @r"
        ── detail · WATCH-12 ──
        approve  (a)  {act:approve}
        § DESCRIPTION  by vimes
          # Rota
          see the board <https://watch.example/board>
          ☐ patrol
        ");
    }
}
