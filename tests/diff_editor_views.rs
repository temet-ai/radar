//! The view data the Ink editor and diff draw from, made by the real host on
//! demo data: !263's Changes tab (a file's `Node::Diff`, unified and
//! side by side), a new comment being written on the merge request (the
//! capturing `Editor` overlay: kept lines, a typed Unicode line, the caret
//! inside it) and an inline comment on a diff line (`DiffUnder.input`).
//!
//! Each case is written at 80x24 and 160x40 as
//! `ink/test/fixtures/diff/<case>_<w>x<h>.json`, the fields of the snapshot
//! the IPC bridge sends a renderer (`ipc::Bridge::snapshot_now`):
//! `{width, height, chrome, screen, section, focus}` plus `overlay`, the
//! screen's own `overlay` pane when it has one. Beside each is
//! `<case>_<w>x<h>.txt`, the picture Ink compares its own with: the
//! text as it was when this test moved to the view data's text form, now
//! Ink's golden; this test no longer writes it. Side by side needs 92 columns, so that case exists at 160x40
//! only.
//!
//! RADAR_DIFF_EDITOR_VIEWS_DIR=$PWD/ink/test/fixtures/diff \
//!   CARGO_TARGET_DIR=target/fleet cargo nextest run -p radar --test diff_editor_views
//!
//! Without the variable the cases are checked: each shows what it is for (the
//! view data's text form), its views read back, and each committed `.json`
//! says what the host now says (its screen's text form is this one's).

#![cfg(feature = "gitlab")]

use std::path::{Path, PathBuf};

use radar_core::Host;
use radar_core::host::{Event, Owner, View};
use radar_core::headless::host_text_of as to_text;
use radar_core::view::{Chord, Key};
use radar_core::view::{PaneId, ScreenView};
use serde_json::{Value, json};

const SIZES: [(u16, u16); 2] = [(80, 24), (160, 40)];

struct Case {
    name: &'static str,
    /// The sizes it exists at.
    sizes: Vec<(u16, u16)>,
    host: Host,
}

/// The demo host with !263's page and diffs held.
fn host(side_by_side: bool) -> Host {
    let mut host = radar_core::demo::demo_host(radar::parts()).unwrap();
    host.view = View::Screen(0);
    let at = host.plugins.iter().position(|p| p.name() == "GitLab").expect("GitLab");
    host.plugins[at] = Box::new(radar_gitlab::sample_reviewed(side_by_side));
    host
}

fn press(host: &mut Host, code: Key) {
    // A frame first: parts fit their words to the room they were drawn in.
    let _ = to_text(host, 160, 40);
    host.update(Event::Intent(radar_core::view::Intent::Key { chord: Chord::new(code) }));
}

fn type_text(host: &mut Host, text: &str) {
    for c in text.chars() {
        press(host, Key::Char(c));
    }
}

/// !263's page (the tree is b, worker, !342, a, api, !263) on `tab`.
fn page(side_by_side: bool, tab: usize) -> Host {
    let mut host = host(side_by_side);
    for _ in 0..5 {
        press(&mut host, Key::Down);
    }
    press(&mut host, Key::Enter);
    for _ in 0..tab {
        press(&mut host, Key::Char(']'));
    }
    host
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    // The Changes tab, the first file open: unified at both sizes.
    out.push(Case { name: "diff_unified", sizes: SIZES.to_vec(), host: page(false, 1) });
    // Side by side, when there is room for it.
    out.push(Case { name: "diff_split", sizes: vec![(160, 40)], host: page(true, 1) });
    // The cursor down in the diff, on a hunk's changed lines.
    let mut moved = page(false, 1);
    for _ in 0..3 {
        press(&mut moved, Key::Char('j'));
    }
    out.push(Case { name: "diff_cursor_down", sizes: SIZES.to_vec(), host: moved });
    // A new comment on the merge request, from the Threads tab: two kept
    // lines, then a line with Unicode in it and the caret two cells back.
    let mut compose = page(false, 2);
    press(&mut compose, Key::Char('c'));
    type_text(&mut compose, "first line");
    press(&mut compose, Key::Enter);
    type_text(&mut compose, "second line");
    press(&mut compose, Key::Enter);
    type_text(&mut compose, "héllo 日本");
    press(&mut compose, Key::Left);
    press(&mut compose, Key::Left);
    out.push(Case { name: "editor_compose", sizes: SIZES.to_vec(), host: compose });
    // An inline comment on the diff's line under the cursor.
    let mut inline = page(false, 1);
    // Off the hunk header, onto an added line.
    for _ in 0..3 {
        press(&mut inline, Key::Char('j'));
    }
    press(&mut inline, Key::Char('c'));
    type_text(&mut inline, "héllo 日本");
    press(&mut inline, Key::Left);
    press(&mut inline, Key::Left);
    out.push(Case { name: "diff_inline_comment", sizes: SIZES.to_vec(), host: inline });
    out
}

/// What the IPC bridge sends a renderer for the host as it is, at this size
/// (`Bridge::snapshot_now`, minus the cursor), and the screen's overlay.
fn views(host: &Host, width: u16, height: u16) -> Value {
    host.set_view_room(width, height);
    let chrome = host.chrome_view(width, height);
    let mut screen = host.screen().and_then(|(p, spec)| host.screen_view_in(p, spec.id, width, height));
    // As the bridge sends it: diffs painted for a renderer with no highlighter.
    if let Some(screen) = screen.as_mut() {
        screen.paint_diffs();
    }
    let section = match host.current_section() {
        Some((Owner::Plugin(p), s)) if host.view == View::Settings => host.section_view_in(p, s.id, width, height),
        _ => None,
    };
    let overlay = screen.as_ref().and_then(|s| s.overlay.clone());
    let mut views = json!({
        "width": width,
        "height": height,
        "chrome": chrome,
        "screen": screen,
        "section": section,
        "focus": PaneId::from(host.focus),
    });
    if let Some(overlay) = overlay {
        views["overlay"] = serde_json::to_value(overlay).unwrap();
    }
    views
}

/// Names paired with contents.
type Named = Vec<(String, String)>;

/// Every file the cases make (name -> contents), and the text form of each
/// case at its size (`<case>_<w>x<h>` -> text).
fn files() -> (Named, Named) {
    let (mut out, mut shown) = (Vec::new(), Vec::new());
    for case in cases() {
        for (w, h) in case.sizes {
            case.host.set_view_room(w, h);
            shown.push((format!("{}_{w}x{h}", case.name), to_text(&case.host, w, h)));
            let json = serde_json::to_string_pretty(&views(&case.host, w, h)).unwrap();
            out.push((format!("{}_{w}x{h}.json", case.name), json + "\n"));
        }
    }
    (out, shown)
}

/// The text form of the screen in a written `.json` (what it says, not how
/// it is laid out or which fields it has).
fn screen_text(json: &Value) -> String {
    let screen: ScreenView = serde_json::from_value(json["screen"].clone()).expect("a screen view");
    let focus: PaneId = serde_json::from_value(json["focus"].clone()).expect("a focus");
    radar_core::headless::text::screen(&screen, Some(focus))
}

fn committed() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ink/test/fixtures/diff")
}

#[test]
fn the_cases_draw_what_they_are_for_and_read_back() {
    let (files, shown) = files();
    let text = |name: &str| files.iter().find(|(n, _)| n == name).map(|(_, t)| t.as_str()).expect(name);
    let shows = |name: &str| shown.iter().find(|(n, _)| n == name).map(|(_, t)| t.as_str()).expect(name);
    let json = |name: &str| -> Value { serde_json::from_str(text(name)).unwrap() };
    // The diff: the first file's lines, in a Node::Diff.
    for size in ["80x24", "160x40"] {
        let shown = shows(&format!("diff_unified_{size}"));
        assert!(shown.contains("Numbers per user and per day"), "{size}:\n{shown}");
        assert!(json(&format!("diff_unified_{size}.json")).to_string().contains("\"node\":\"diff\""));
    }
    assert!(json("diff_split_160x40.json").to_string().contains("\"split\":true"));
    assert!(json("diff_unified_160x40.json").to_string().contains("\"split\":false"));
    // The editor: the overlay holds the kept lines and the line typed.
    for size in ["80x24", "160x40"] {
        let view = json(&format!("editor_compose_{size}.json"));
        let overlay = view["overlay"].to_string();
        assert!(overlay.contains("first line") && overlay.contains("héllo 日本"), "{size}: {overlay}");
        let shown = shows(&format!("editor_compose_{size}"));
        assert!(shown.contains("héllo 日本"), "{size}:\n{shown}");
        let inline = text(&format!("diff_inline_comment_{size}.json"));
        assert!(inline.contains("héllo 日本") && inline.contains("\"input\""), "{size}");
    }
    // The files on disk say what this makes, unless they are being written.
    if std::env::var_os("RADAR_DIFF_EDITOR_VIEWS_DIR").is_none() {
        // Their screens' text form is compared, not the JSON itself: the JSON
        // grows with every field the views gain (it is rewritten with the
        // command above). Each case's Ink golden (`.txt`) stays beside it.
        for (name, want) in files.iter().filter(|(n, _)| n.ends_with(".json")) {
            let path = committed().join(name);
            let have: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let want: Value = serde_json::from_str(want).unwrap();
            assert_eq!(
                screen_text(&have),
                screen_text(&want),
                "{name} is stale: RADAR_DIFF_EDITOR_VIEWS_DIR=$PWD/ink/test/fixtures/diff writes it"
            );
            let golden = committed().join(name.replace(".json", ".txt"));
            assert!(golden.exists(), "{}: Ink's golden for it", golden.display());
        }
    }
}

#[test]
fn the_files_are_written_when_asked() {
    let Ok(dir) = std::env::var("RADAR_DIFF_EDITOR_VIEWS_DIR") else { return };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, contents) in files().0 {
        std::fs::write(dir.join(name), contents).unwrap();
    }
}
