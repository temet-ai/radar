//! The Ratatui shell's painted cells, as the styled reference for Ink.
//!
//! Every surface `view_fixtures.rs` records for Ink (each screen, settings
//! section, item page and the host chrome) is drawn here through the
//! Ratatui shell (`radar_core::shell::render`) into a `TestBackend` at 80x24
//! and 160x40, and each cell's character, foreground, background and
//! modifiers are kept in `ink/test/cells/<surface>-<w>x<h>.json`. Ink's
//! tests compare their own cells to these, so a missing highlight or a
//! different colour fails where a text snapshot cannot see it.
//!
//! `highlight_*` scenes are extra states, one per kind of highlight: the
//! selected row, the focused pane, the row the pointer is over, a menu with
//! its selection. Each carries the view it was drawn from (`view`).
//!
//! File format (`format: 1`):
//!
//! ```text
//! { "format": 1, "scene": "<name>", "width": W, "height": H,
//!   "canvas": { "fg": "text.primary", "bg": "bg.base" },
//!   "colours": { "<slot>": { "hex": "#rrggbb", "ink": ["accent"] } },
//!   "view": { "screen": ScreenView|null, "menu": MenuView|null,
//!             "pointer": {"x":..,"y":..}|null },        // highlight scenes only
//!   "rows": [ [ {cell}, ... ], ... ] }                  // H rows of W cells
//! ```
//!
//! A cell is `{"ch": "x", "fg": "slot", "bg": "slot", "mods": ["bold"]}`
//! with every default left out: no `ch` is a space, no `fg` is the canvas
//! foreground, no `bg` the canvas background, no `mods` none. Colours are
//! the theme's slot names (`accent.primary`, `bg.selection`); where slots
//! share one colour the names are joined by `/`, core slots first. `colours`
//! says each name's `#rrggbb` and the Ink names (`Ink` in the views) that
//! paint with that slot. The cell right of a two-column character has
//! `"ch": ""`. The theme is radar's default at the true-colour tier.
//!
//! RADAR_WRITE_CELLS=1 CARGO_TARGET_DIR=target/fleet cargo nextest run -p radar --test ratatui_cells
//!
//! Without RADAR_WRITE_CELLS the files are compared and drift fails the test.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use radar_core::Host;
use radar_core::design::theme::{Slot, Tier};
use radar_core::headless::{Headless, offered};
use radar_core::host::{Hit, Owner, View};
use radar_core::ratatui::backend::TestBackend;
use radar_core::ratatui::buffer::Buffer;
use radar_core::ratatui::layout::Position;
use radar_core::ratatui::style::{Color, Modifier};
use radar_core::ratatui::Terminal;
use radar_core::shell::render;
use radar_core::theme::Theme;
use radar_core::view::{Intent, Node, PaneId, TreeRow};
use serde_json::{Value, json};

const SIZES: [(u16, u16); 2] = [(80, 24), (160, 40)];

fn demo(w: u16, h: u16) -> Headless {
    let mut host = radar_core::shot::demo_host(radar::parts()).unwrap();
    host.view = View::Screen(0);
    Headless::new(host, w, h)
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c.to_ascii_lowercase() {
            c if c.is_ascii_alphanumeric() => out.push(c),
            _ if !out.ends_with('-') => out.push('-'),
            _ => {}
        }
    }
    out.trim_matches('-').to_string()
}

/// The slot each `Ink` paints with (`view_tui::slot`), by Ink's name.
const INK_SLOTS: [(&str, Slot); 15] = [
    ("primary", Slot::TextPrimary),
    ("secondary", Slot::TextSecondary),
    ("muted", Slot::TextMuted),
    ("emphasis", Slot::TextEmphasis),
    ("accent", Slot::AccentPrimary),
    ("accent_muted", Slot::AccentMuted),
    ("accent_secondary", Slot::AccentSecondary),
    ("accent_tertiary", Slot::AccentTertiary),
    ("good", Slot::StatusSuccess),
    ("warn", Slot::StatusWarning),
    ("bad", Slot::StatusError),
    ("info", Slot::StatusInfo),
    ("number", Slot::DataNumber),
    ("added", Slot::DiffAdded),
    ("removed", Slot::DiffRemoved),
];

/// Colours by the theme's slot names.
struct Names {
    by: Vec<(Color, String)>,
}

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
        Names { by }
    }

    fn name(&self, c: Color) -> String {
        match self.by.iter().find(|(k, _)| *k == c) {
            Some((_, n)) => n.clone(),
            None => match c {
                Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
                Color::Indexed(i) => format!("idx{i}"),
                other => format!("{other:?}").to_lowercase(),
            },
        }
    }
}

const MODS: [(Modifier, &str); 9] = [
    (Modifier::BOLD, "bold"),
    (Modifier::DIM, "dim"),
    (Modifier::ITALIC, "italic"),
    (Modifier::UNDERLINED, "underline"),
    (Modifier::SLOW_BLINK, "slow_blink"),
    (Modifier::RAPID_BLINK, "rapid_blink"),
    (Modifier::REVERSED, "reversed"),
    (Modifier::HIDDEN, "hidden"),
    (Modifier::CROSSED_OUT, "strike"),
];

/// The host drawn once at this size by the Ratatui shell.
fn paint(host: &Host, theme: &Theme, w: u16, h: u16) -> Buffer {
    let mut term = Terminal::new(TestBackend::new(w, h)).expect("test backend");
    term.draw(|f| render(host, theme, f)).expect("draw");
    term.backend().buffer().clone()
}

/// The scene's file: header lines, then one line a row.
fn file_of(name: &str, buf: &Buffer, theme: &Theme, view: Option<Value>) -> String {
    let names = Names::of(theme);
    let canvas_fg = theme.s(Slot::TextPrimary).fg;
    let canvas_bg = theme.s(Slot::BgBase).fg;
    let a = buf.area;
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut rows: Vec<String> = Vec::new();
    for y in a.y..a.y + a.height {
        let mut cells: Vec<String> = Vec::new();
        let mut covered = false;
        for x in a.x..a.x + a.width {
            let cell = &buf[(x, y)];
            let mut o = serde_json::Map::new();
            let ch = if covered { "" } else { cell.symbol() };
            covered = !covered && radar_core::ui::width(cell.symbol()) > 1;
            if ch != " " {
                o.insert("ch".into(), json!(ch));
            }
            let fg = Some(cell.fg).filter(|c| *c != Color::Reset && Some(*c) != canvas_fg);
            let bg = Some(cell.bg).filter(|c| *c != Color::Reset && Some(*c) != canvas_bg);
            if let Some(c) = fg {
                let n = names.name(c);
                used.insert(n.clone());
                o.insert("fg".into(), json!(n));
            }
            if let Some(c) = bg {
                let n = names.name(c);
                used.insert(n.clone());
                o.insert("bg".into(), json!(n));
            }
            let mods: Vec<&str> = MODS.iter().filter(|(m, _)| cell.modifier.contains(*m)).map(|(_, w)| *w).collect();
            if !mods.is_empty() {
                o.insert("mods".into(), json!(mods));
            }
            cells.push(Value::Object(o).to_string());
        }
        rows.push(format!("[{}]", cells.join(",")));
    }
    let mut colours = BTreeMap::new();
    for n in &used {
        // A name that is a joined run of slots: the hex of its first.
        let first = n.split('/').next().unwrap_or(n);
        let hex = if n.starts_with('#') { n.clone() } else { theme.hex(first) };
        let ink: Vec<&str> = INK_SLOTS
            .iter()
            .filter(|(_, s)| n.split('/').any(|part| part == s.token_name()))
            .map(|(i, _)| *i)
            .collect();
        colours.insert(n.clone(), json!({ "hex": hex, "ink": ink }));
    }
    let canvas = json!({
        "fg": canvas_fg.map(|c| names.name(c)),
        "bg": canvas_bg.map(|c| names.name(c)),
    });
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("\"format\": 1,\n\"scene\": {},\n", json!(name)));
    out.push_str(&format!("\"width\": {},\n\"height\": {},\n", a.width, a.height));
    out.push_str(&format!("\"canvas\": {canvas},\n\"colours\": {},\n", serde_json::to_string(&colours).unwrap()));
    if let Some(view) = view {
        out.push_str(&format!("\"view\": {view},\n"));
    }
    out.push_str("\"rows\": [\n");
    out.push_str(&rows.join(",\n"));
    out.push_str("\n]\n}\n");
    out
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ink/test/cells")
}

/// Open every closed group of the trees showing, as `reach_pages` does.
fn unfold(head: &mut Headless) {
    for _ in 0..3 {
        let Some(view) = head.screen() else { return };
        let panes = match &view.page {
            Some(page) => &page.panes,
            None => &view.panes,
        };
        let closed: Vec<(PaneId, _)> = panes
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
            head.focus(pane);
            head.send(Intent::Fold { target, open: true });
        }
    }
}

/// The offer of `t` on the screen showing. A choice can fill another pane
/// (a nav pane row fills its list), so when `t` is not there yet the other
/// offers are chosen, in order, until it is.
fn find_offered(head: &mut Headless, t: &radar_core::view::Target) -> Option<radar_core::headless::Offered> {
    let look = |head: &mut Headless| {
        unfold(head);
        offered(&head.screen()?).into_iter().find(|o| &o.target == t)
    };
    if let Some(o) = look(head) {
        return Some(o);
    }
    let others = offered(&head.screen()?);
    for o in others {
        head.focus(o.pane);
        head.send(Intent::Select { target: o.target });
        if let Some(found) = look(head) {
            return Some(found);
        }
    }
    None
}

/// One scene to write: its name and the file's text.
type Scene = (String, String);

fn theme_of(head: &Headless) -> Theme {
    head.host().build_theme(Tier::TrueColor)
}

/// Draw what `head` shows now.
fn scene(name: String, head: &Headless, w: u16, h: u16, view: Option<Value>) -> Scene {
    let theme = theme_of(head);
    let buf = paint(head.host(), &theme, w, h);
    let text = file_of(&name, &buf, &theme, view);
    (name, text)
}

/// The surfaces `view_fixtures.rs` records, drawn.
fn surfaces(w: u16, h: u16) -> Vec<Scene> {
    let mut out = Vec::new();
    let mut head = demo(w, h);
    // Screens' lists.
    for i in 0..head.screen_count() {
        head.show_screen(i);
        let Some(view) = head.screen() else { continue };
        let part = head.host().plugins[head.host().screens[i].0].id().to_string();
        out.push(scene(format!("{part}_{}", slug(&view.scope)), &head, w, h, None));
    }
    // Settings sections.
    for s in 0..head.host().sections.len() {
        let Owner::Plugin(p) = head.host().sections[s].0 else { continue };
        let spec = head.host().sections[s].1;
        let Some(view) = head.host().section_view_in(p, spec.id, w, h) else { continue };
        let part = head.host().plugins[p].id().to_string();
        head.host_mut().view = View::Settings;
        head.host_mut().section = s;
        out.push(scene(format!("{part}_settings-{}", slug(&view.scope)), &head, w, h, None));
    }
    // Item pages: reached, then reached again and drawn.
    let mut reached = Vec::new();
    for i in 0..head.screen_count() {
        for page in head.reach_pages(i, 12, 2) {
            reached.push((i, page));
        }
    }
    for (i, page) in reached {
        // Each page is replayed from a fresh host: the walk leaves the parts' own
        // selections (a nav pane row chosen, a tree folded) behind.
        let mut head = demo(w, h);
        head.show_screen(i);
        for t in &page.via {
            unfold(&mut head);
            let at = find_offered(&mut head, t).unwrap_or_else(|| panic!("{t:?} is offered on the way to {:?}; via {:?}; offered {:?}", page.kind, page.via, head.screen().map(|v| offered(&v))));
            head.focus(at.pane);
            head.send(Intent::Select { target: t.clone() });
            head.send(Intent::Activate { target: t.clone() });
            if head.menu().is_some() {
                head.send(Intent::Key { chord: radar_core::view::Chord::parse("esc").unwrap() });
            }
        }
        let now = head.screen().expect("the page's screen");
        assert_eq!(serde_json::to_value(&now).unwrap(), serde_json::to_value(&page.view).unwrap(), "{:?} is the page reached", page.kind);
        let k = &page.kind;
        let name = format!("{}_{}-page{}-{}", k.part, slug(&k.screen), k.depth, slug(&k.shape.join(" ")));
        out.push(scene(name, &head, w, h, None));
    }
    // The chrome, over the first screen.
    head.show_screen(0);
    out.push(scene("host_chrome".into(), &head, w, h, None));
    out
}

/// One scene for each kind of highlight, with the view it is drawn from.
fn highlights(w: u16, h: u16) -> Vec<Scene> {
    let mut out = Vec::new();
    let view_of = |head: &Headless, pointer: Option<Position>| {
        Some(json!({
            "screen": head.screen(),
            "menu": head.menu(),
            "pointer": pointer.map(|p| json!({"x": p.x, "y": p.y})),
        }))
    };

    // Selected row: the first screen's list, as it opens.
    let mut head = demo(w, h);
    head.show_screen(0);
    unfold(&mut head);
    let v = view_of(&head, None);
    out.push(scene("highlight_selected-row".into(), &head, w, h, v));

    // Focused pane: the same screen with the focus moved to each other pane.
    let panes: Vec<PaneId> = head.screen().map(|v| v.panes.iter().map(|p| p.pane).collect()).unwrap_or_default();
    for pane in panes {
        head.focus(pane);
        let v = view_of(&head, None);
        let n = format!("highlight_focused-pane-{}", format!("{pane:?}").to_lowercase());
        out.push(scene(n, &head, w, h, v));
    }

    // Hovered row: draw once so the hits are known, then put the pointer on
    // the first row of a pane's list and draw again.
    let mut head = demo(w, h);
    head.show_screen(0);
    let theme = theme_of(&head);
    paint(head.host(), &theme, w, h);
    let spots: Vec<Position> = head
        .host()
        .hits
        .borrow()
        .iter()
        .filter(|(r, hit)| matches!(hit, Hit::Plugin { id, .. } if id != radar_core::view_tui::PANE_HIT) && r.y > 2 && r.y < h - 2)
        .map(|(r, _)| Position::new(r.x + r.width.min(4) / 2, r.y))
        .collect();
    // The cursor row keeps its own look under the pointer, so take the first
    // spot where the pointer changes what is drawn.
    let plain = paint(head.host(), &theme, w, h);
    let mut pointer = None;
    for p in spots {
        head.host_mut().hover(Some(p));
        if paint(head.host(), &theme, w, h) != plain {
            pointer = Some(p);
            break;
        }
    }
    if let Some(p) = pointer {
        let v = view_of(&head, Some(p));
        out.push(scene("highlight_hovered-row".into(), &head, w, h, v));
    }

    // A menu with its selection: a context click on the first row offered.
    // (an activation that asks something opens one too).
    let mut head = demo(w, h);
    'found: for i in 0..head.screen_count() {
        head.show_screen(i);
        unfold(&mut head);
        let Some(view) = head.screen() else { continue };
        for o in offered(&view) {
            head.focus(o.pane);
            head.send(Intent::Select { target: o.target.clone() });
            head.send(Intent::Context { target: o.target.clone() });
            if head.menu().is_some() {
                break 'found;
            }
            head.send(Intent::Activate { target: o.target });
            if head.menu().is_some() {
                break 'found;
            }
            head.close_overlays();
        }
    }
    if head.menu().is_some() {
        // The selection moves down one, so it is not the first item's by default.
        head.send(Intent::Key { chord: radar_core::view::Chord::parse("↓").unwrap() });
        let v = view_of(&head, None);
        out.push(scene("highlight_menu".into(), &head, w, h, v));
    }

    // A detail pane's sections: the first merge request of the first screen,
    // its STATE, ACTIONS (keys at the edge), PIPELINE and CHANGES.
    let mut head = demo(w, h);
    head.show_screen(0);
    unfold(&mut head);
    let has_actions = |head: &Headless| {
        head.screen().and_then(|v| v.pane(PaneId::Detail).cloned()).is_some_and(|p| match &p.body {
            Node::Rich { blocks, .. } => blocks
                .iter()
                .any(|b| matches!(b, radar_core::view::Block::Section { title, .. } if title == "ACTIONS")),
            _ => false,
        })
    };
    let rows = head.screen().map(|v| offered(&v)).unwrap_or_default();
    for o in rows.into_iter().filter(|o| o.pane == PaneId::Middle) {
        head.focus(o.pane);
        head.send(Intent::Select { target: o.target });
        if has_actions(&head) {
            let v = view_of(&head, None);
            out.push(scene("highlight_detail-sections".into(), &head, w, h, v));
            break;
        }
    }

    // The filter box: `/` on the first screen, then `g` `g` typed.
    let mut head = demo(w, h);
    head.show_screen(0);
    unfold(&mut head);
    for k in ["/", "g", "g"] {
        head.send(Intent::Key { chord: radar_core::view::Chord::parse(k).unwrap() });
    }
    let v = view_of(&head, None);
    out.push(scene("highlight_typing-filter".into(), &head, w, h, v));
    out
}

fn all_scenes()-> Vec<(u16, u16, Scene)> {
    let mut out = Vec::new();
    for (w, h) in SIZES {
        for s in surfaces(w, h).into_iter().chain(highlights(w, h)) {
            out.push((w, h, s));
        }
    }
    out
}

#[test]
fn the_cells_match_the_files_or_are_written() {
    let scenes = all_scenes();
    let dir = dir();
    let writing = std::env::var_os("RADAR_WRITE_CELLS").is_some();
    if writing {
        std::fs::create_dir_all(&dir).unwrap();
        // A scene that is gone leaves no stale file.
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            if e.path().extension().is_some_and(|x| x == "json") {
                std::fs::remove_file(e.path()).unwrap();
            }
        }
    }
    let mut drift = Vec::new();
    let mut seen = BTreeSet::new();
    for (w, h, (name, text)) in &scenes {
        let file = format!("{name}-{w}x{h}.json");
        assert!(seen.insert(file.clone()), "{file} is drawn twice");
        let path = dir.join(&file);
        if writing {
            std::fs::write(&path, text).unwrap();
        } else {
            match std::fs::read_to_string(&path) {
                Ok(on_disk) if on_disk == *text => {}
                Ok(_) => drift.push(format!("{file} differs")),
                Err(_) => drift.push(format!("{file} is missing")),
            }
        }
    }
    if !writing {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let f = e.file_name().to_string_lossy().to_string();
            if f.ends_with(".json") && !seen.contains(&f) {
                drift.push(format!("{f} has no scene"));
            }
        }
        assert!(
            drift.is_empty(),
            "Ratatui's cells drifted from ink/test/cells; rerun with RADAR_WRITE_CELLS=1 and read the diff:\n{}",
            drift.join("\n")
        );
    }
}

/// The reference is only worth having if it holds what it is for.
#[test]
fn each_highlight_shows_in_the_cells() {
    let (w, h) = (160, 40);
    let scenes = highlights(w, h);
    let find = |n: &str| scenes.iter().find(|(name, _)| name == n).map(|(_, t)| serde_json::from_str::<Value>(t).unwrap());
    let has_bg = |v: &Value, needle: &str| {
        v["rows"].as_array().unwrap().iter().flat_map(|r| r.as_array().unwrap()).any(|c| c["bg"].as_str().is_some_and(|b| b.contains(needle)))
    };
    let selected = find("highlight_selected-row").expect("selected row scene");
    assert!(has_bg(&selected, "selection"), "a selected row has the selection background: {}", selected["colours"]);
    let hovered = find("highlight_hovered-row").expect("the pointer finds a row to light on the first screen");
    assert!(has_bg(&hovered, "hover"), "a hovered row has the hover background: {}", hovered["colours"]);
    let menu = find("highlight_menu").expect("a context click opens a menu on some screen");
    assert!(menu["view"]["menu"].is_object(), "the menu scene carries its menu");
    // Moving the focus to another pane changes how the screen is painted.
    let focused: Vec<Value> = scenes.iter().filter(|(n, _)| n.starts_with("highlight_focused-pane-")).map(|(_, t)| serde_json::from_str::<Value>(t).unwrap()).collect();
    assert!(focused.len() >= 2, "the first screen has panes to focus");
    assert!(focused.windows(2).any(|p| p[0]["rows"] != p[1]["rows"]), "the focused pane is painted differently from the others");
}
