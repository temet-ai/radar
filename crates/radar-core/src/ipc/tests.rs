use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ts_rs::TS;

use super::*;
use crate::view::{Choice, Node, Target, Text};

fn intent() -> Intent {
    Intent::Select {
        target: Target::new("row:1"),
    }
}

/// One of every message, as both sides see them.
fn samples() -> (Vec<ToHost>, Vec<ToRenderer>) {
    let to_host = vec![
        ToHost::Hello {
            protocol: PROTOCOL,
            view: crate::view::VERSION,
            capabilities: Capabilities {
                columns: 80,
                rows: 24,
                mouse: true,
                true_colour: true,
                image: ImageProtocol::Kitty,
                plain_text: false,
            },
        },
        ToHost::Resize {
            columns: 160,
            rows: 40,
        },
        ToHost::Pointer { column: 60, row: 5 },
        ToHost::Intent {
            seq: 1,
            cursor: 2,
            intent: intent(),
        },
        ToHost::Ack { cursor: 2 },
        ToHost::Log {
            level: LogLevel::Warn,
            text: "slow frame".into(),
        },
        ToHost::Rows {
            scope: "watch".into(),
            panes: vec![crate::view::PaneRows {
                pane: PaneId::Detail,
                rows: 30,
                content: 84,
                at: 12,
            }],
        },
        ToHost::Bye,
    ];
    let menu = MenuView {
        palette: None,
        gear: false,
        customising: false,
        id: "confirm".into(),
        title: "delete the branch spike?".into(),
        question: true,
        change: None,
        items: vec![Choice {
            mnemonic_at: Some(0),
            mnemonic_shift: false,            value: "yes".into(),
            label: "yes".into(),
            hint: String::new(),
            key: Some('y'),
            keys: "y".into(),
            icon: None,
            heading: false,
            note: false,
            sample: None,
            shown_only: false,
            muted: false,
            target: Some(Target::new("chrome:menu:0")),
        }],
        icon: None,
        selected: Some(0),
        typed: None,
        caret: None,
        at: None,
        scroll: 0,
        lines: 0,
        shown: 0,
        usable: true,
        refused: None,
    };
    let to_renderer = vec![
        ToRenderer::Welcome {
            protocol: PROTOCOL,
            view: crate::view::VERSION,
            cursor: 1,
        },
        ToRenderer::Refuse {
            wanted: PROTOCOL,
            got: 9,
            message: "radar speaks protocol 1, this renderer 9".into(),
        },
        ToRenderer::Look(Box::new(Look {
            tier: ColourTier::TrueColor,
            slots: [("bg.selection", "#44475a"), ("text.emphasis", "#ffffff")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            modifiers: [("text.emphasis".to_string(), "bold".to_string())].into(),
            icons: [(
                "run_failed".to_string(),
                LookIcon {
                    glyph: "✗".into(),
                    style: "status.error".into(),
                },
            )]
            .into(),
            icon_width: 1,
            pack: "unicode".into(),
            hyperlinks: true,
            pointer: false,
        })),
        ToRenderer::Snapshot(Box::new(Snapshot {
            motion: Default::default(),
            cursor: 2,
            chrome: ChromeView::new(80, 24),
            screen: Some(ScreenView::new("branch-screen", vec![])),
            section: Some(SectionView {
                version: crate::view::VERSION,
                scope: "look".into(),
                body: Node::Document {
                    lines: vec![Text::from("theme")],
                    cut: false,
                },
                detail_title: String::new(),
                detail: vec![],
                detail_blocks: vec![],
                actions: vec![],
            }),
            focus: PaneId::Middle,
            splash: None,
            dock: None,
            trace: None,
        })),
        ToRenderer::Question(Question {
            id: 1,
            cursor: 3,
            menu,
        }),
        ToRenderer::InputRequest(InputRequest {
            id: 2,
            cursor: 4,
            title: "filter".into(),
            value: "ab".into(),
        }),
        ToRenderer::Notice(Notice {
            cursor: 5,
            text: "copied 4 characters".into(),
            occurrence: None,
            revision: 0,
            now_ms: 0,
        }),
        ToRenderer::Clipboard {
            text: "spike".into(),
        },
        ToRenderer::Shutdown {
            reason: "quit".into(),
        },
    ];
    (to_host, to_renderer)
}

#[test]
fn every_message_has_one_schema_sample_and_reads_back() {
    let (to_host, to_renderer) = samples();
    let mut lines = Vec::new();
    for m in &to_host {
        let line = encode(m);
        let back: ToHost = serde_json::from_slice(&line).unwrap();
        assert_eq!(&back, m);
        lines.push(String::from_utf8(line).unwrap());
    }
    for m in &to_renderer {
        let line = encode(m);
        let back: ToRenderer = serde_json::from_slice(&line).unwrap();
        assert_eq!(&back, m);
        lines.push(String::from_utf8(line).unwrap());
    }
    insta::assert_snapshot!("schema", lines.concat());
}

#[test]
fn lines_reassemble_across_pieces_and_refuse_a_line_too_long() {
    let mut lines = Lines::default();
    assert_eq!(lines.push(b"{\"type\":").unwrap(), Vec::<Vec<u8>>::new());
    let got = lines.push(b"\"bye\"}\n{\"type\":\"bye\"}\n{").unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(
        serde_json::from_slice::<ToHost>(&got[0]).unwrap(),
        ToHost::Bye
    );
    let mut long = Lines::default();
    assert_eq!(long.push(&vec![b'x'; MAX_LINE + 1]), Err(TooLong));
}

#[test]
fn a_log_line_is_cut_to_its_bound() {
    let long = ToHost::Log {
        level: LogLevel::Info,
        text: "é".repeat(MAX_LOG + 5),
    };
    let ToHost::Log { text, .. } = long.bounded() else {
        panic!()
    };
    assert_eq!(text.chars().count(), MAX_LOG);
}

// --- the TypeScript side ---------------------------------------------------------

fn generated_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ink/src/protocol/generated")
}

fn export(dir: &Path) {
    let cfg = ts_rs::Config::new()
        .with_out_dir(dir)
        .with_large_int("number")
        .with_import_extension(Some("js"));
    ToHost::export_all(&cfg).unwrap();
    ToRenderer::export_all(&cfg).unwrap();
    // Extension payloads are carried as JSON, so dependency discovery cannot
    // see their typed contract. Keep the detail reader generated from Rust.
    crate::view::detail::DetailContent::export_all(&cfg).unwrap();
}

/// Every `.ts` file under `dir`, by its path relative to `dir`.
fn files(dir: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else if p.extension().is_some_and(|x| x == "ts") {
                let name = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                out.insert(name, std::fs::read_to_string(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// The committed TypeScript is what the Rust types export. `RADAR_TS_UPDATE=1`
/// writes it (and removes files no type exports any more).
#[test]
fn the_committed_typescript_matches_the_rust_types() {
    let fresh = std::env::temp_dir().join(format!("radar-ipc-ts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&fresh);
    export(&fresh);
    let want = files(&fresh);
    let _ = std::fs::remove_dir_all(&fresh);
    let dir = generated_dir();
    if std::env::var_os("RADAR_TS_UPDATE").is_some() {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in &want {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }
    let have = files(&dir);
    assert_eq!(
        have.keys().collect::<Vec<_>>(),
        want.keys().collect::<Vec<_>>(),
        "ink/src/protocol/generated differs from the Rust types: run with RADAR_TS_UPDATE=1"
    );
    for (name, text) in &want {
        assert_eq!(
            &have[name], text,
            "ink/src/protocol/generated/{name} differs from the Rust types: run with RADAR_TS_UPDATE=1"
        );
    }
}
