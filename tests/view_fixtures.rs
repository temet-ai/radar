//! General view fixtures are a deterministic contract. Ordinary tests compare
//! the generated top-level inventory and content without writing. Explicit
//! RADAR_VIEW_FIXTURES_DIR exports that same validated content. Handwritten
//! _hand_* files and nested fixture sets belong to other contracts.
//! Timezone and captured database-identity normalization live only here.

use std::collections::BTreeMap;

#[path = "support/mnemonic_acceptance.rs"]
mod mnemonic_acceptance;

use radar_core::headless::Headless;
use radar_core::host::View;
use serde_json::Value;

#[path = "support/view_fixture_contract.rs"]
mod view_fixture_contract;
use view_fixture_contract::{
    FIXTURE_SIZES as SIZES, Surfaces, canonical_fixture_files, compare_fixture_files,
    is_generated_fixture_path,
};

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

fn surfaces() -> Surfaces {
    let mut all: Surfaces = BTreeMap::new();
    let mut put = |name: String, size: (u16, u16), value: Value| {
        all.entry(name).or_default().push((size, value));
    };
    for (w, h) in SIZES {
        let mut head = demo(w, h);
        let gaps = head.view_gaps();
        assert!(
            gaps.is_empty(),
            "missing or invalid registered views at {w}x{h}: {gaps:#?}"
        );
        for (part, view) in head.screen_views() {
            put(
                format!("{part}_{}", slug(&view.scope)),
                (w, h),
                serde_json::to_value(&view).unwrap(),
            );
        }
        for (part, view) in head.section_views() {
            put(
                format!("{part}_settings-{}", slug(&view.scope)),
                (w, h),
                serde_json::to_value(&view).unwrap(),
            );
        }
        for i in 0..head.screen_count() {
            for page in head.reach_pages(i, 12, 2) {
                assert!(page.gaps.is_empty(), "{page:#?}");
                let k = &page.kind;
                let name = format!(
                    "{}_{}-page{}-{}",
                    k.part,
                    slug(&k.screen),
                    k.depth,
                    slug(&k.shape.join(" "))
                );
                put(name, (w, h), serde_json::to_value(&page.view).unwrap());
            }
        }
        put(
            "host_chrome".into(),
            (w, h),
            serde_json::to_value(head.chrome()).unwrap(),
        );
    }
    all
}

const CONTRACT_TEST: &str = "the_general_views_match_committed_fixtures_or_are_written";
const CHILD_MARKER: &str = "RADAR_GENERAL_FIXTURE_CHILD";
const SUCCESS_RECEIPT: &str = "radar general view fixture contract completed v1";

struct FixtureHomeDir(std::path::PathBuf);
impl FixtureHomeDir {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("radar-view-fixture-home-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).expect("create an owned fixture home");
        let home = Self(path);
        std::fs::create_dir_all(home.0.join(".config")).unwrap();
        std::fs::create_dir_all(home.0.join(".local/state")).unwrap();
        std::fs::create_dir_all(home.0.join(".cache")).unwrap();
        std::fs::create_dir_all(home.0.join(".local/share")).unwrap();
        home
    }
}
impl Drop for FixtureHomeDir {
    fn drop(&mut self) {
        // Only this successfully created unique fixture directory is removed.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_fixture_child(test_name: &str) -> std::process::Output {
    let home = FixtureHomeDir::new();
    std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
        .env(CHILD_MARKER, "v1")
        .env("TZ", "UTC")
        .env("RADAR_SANDBOX", "1")
        .env("HOME", &home.0)
        .env("XDG_CONFIG_HOME", home.0.join(".config"))
        .env("XDG_STATE_HOME", home.0.join(".local/state"))
        .env("XDG_CACHE_HOME", home.0.join(".cache"))
        .env("XDG_DATA_HOME", home.0.join(".local/share"))
        .env_remove("RADAR_FAKE_AGENT")
        .env_remove("RADAR_FAKE_AGENT_STATE")
        .env_remove("RADAR_FAKE_AGENT_PACE_MS")
        .output()
        .expect("start the isolated general view fixture contract")
}

fn require_child_receipt(output: &std::process::Output) -> Result<(), String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(format!(
            "fixture child failed: {}\n{stdout}\n{stderr}",
            output.status
        ));
    }
    if stdout
        .lines()
        .filter(|line| *line == SUCCESS_RECEIPT)
        .count()
        != 1
    {
        return Err(format!(
            "fixture child must execute the contract exactly once, not pass zero selected tests\n{stdout}\n{stderr}"
        ));
    }
    Ok(())
}

#[test]
fn an_unmatched_fixture_child_cannot_report_success() {
    let output = run_fixture_child("no_test_has_this_fixture_receipt_negative_control_name");
    assert!(
        output.status.success(),
        "libtest accepts a filter selecting zero tests"
    );
    assert!(
        require_child_receipt(&output)
            .unwrap_err()
            .contains("exactly once")
    );
}

#[test]
fn the_general_views_match_committed_fixtures_or_are_written() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        // Set timezone before process startup, rather than changing process-global
        // C runtime timezone state while other test threads may be running.
        let output = run_fixture_child(CONTRACT_TEST);
        require_child_receipt(&output).unwrap_or_else(|error| panic!("{error}"));
        return;
    }
    assert_eq!(std::env::var(CHILD_MARKER).as_deref(), Ok("v1"));
    assert_eq!(std::env::var("TZ").as_deref(), Ok("UTC"));
    assert_eq!(std::env::var("RADAR_SANDBOX").as_deref(), Ok("1"));
    for epoch in [1_791_184_300, 1_791_253_000] {
        assert_eq!(
            radar_core::time::local_offset(epoch),
            0,
            "fixture timestamp must use UTC"
        );
    }
    for name in [
        "RADAR_FAKE_AGENT",
        "RADAR_FAKE_AGENT_STATE",
        "RADAR_FAKE_AGENT_PACE_MS",
    ] {
        assert!(
            std::env::var_os(name).is_none(),
            "fixture child must not inherit {name}"
        );
    }
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("owned fixture home"));
    for (variable, relative) in [
        ("XDG_CONFIG_HOME", ".config"),
        ("XDG_STATE_HOME", ".local/state"),
        ("XDG_CACHE_HOME", ".cache"),
        ("XDG_DATA_HOME", ".local/share"),
    ] {
        let path =
            std::path::PathBuf::from(std::env::var_os(variable).expect("owned fixture directory"));
        assert_eq!(path, home.join(relative));
        assert_eq!(
            std::fs::read_dir(path)
                .expect("read empty fixture directory")
                .count(),
            0
        );
    }
    // These are runtime contract requirements: a reduced build must still compile
    // its pure helper tests, while the full fixture journey must fail closed.
    for (enabled, requirement) in [
        (
            cfg!(all(
                feature = "sessions-copilot",
                feature = "sessions-codex",
                feature = "sessions-claude"
            )),
            "recorded provider availability requires the three native adapters",
        ),
        (
            cfg!(feature = "demo"),
            "general view fixtures require demo data",
        ),
    ] {
        assert!(enabled, "{requirement}");
    }
    assert_eq!(
        radar::mix(),
        [
            "gitlab",
            "github",
            "jira",
            "slack",
            "git",
            "workspace",
            "workbench",
            "sessions"
        ],
        "general view fixtures require the complete owner mix"
    );
    let all = surfaces();
    assert!(all.len() > 20, "{:?}", all.keys().collect::<Vec<_>>());
    for (name, sizes) in &all {
        assert_eq!(sizes.len(), SIZES.len(), "{name} is not at every size");
        for (_, value) in sizes {
            let text = serde_json::to_string(value).unwrap();
            assert!(text.len() < 1 << 20, "{name}: {} bytes", text.len());
            let back: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(&back, value, "{name}");
        }
    }
    let has = |part: &str| {
        all.keys()
            .any(|n| n.starts_with(&format!("{part}_")) && n.contains("-page"))
    };
    assert!(
        has("gitlab") && has("github") && has("jira"),
        "{:?}",
        all.keys().collect::<Vec<_>>()
    );
    for ((w, h), view) in &all["slack_feed"] {
        assert_eq!(
            view.pointer("/panes/2/body/data/sections/0/blocks/0/text/1/text")
                .and_then(Value::as_str),
            Some("07:11"),
            "feed {w}x{h}"
        );
    }
    for ((w, h), view) in &all["slack_feed-page1-thread-links"] {
        assert_eq!(
            view.pointer("/page/panes/0/body/blocks/1/text/1/text")
                .and_then(Value::as_str),
            Some("02:16"),
            "thread {w}x{h}"
        );
    }
    let mnemonic_gaps: Vec<_> = all
        .iter()
        .flat_map(|(name, sizes)| {
            sizes.iter().flat_map(move |(size, value)| {
                mnemonic_acceptance::fixture_gaps(value)
                    .into_iter()
                    .map(move |gap| format!("{name} {size:?}: {gap}"))
            })
        })
        .collect();
    assert!(
        mnemonic_gaps.is_empty(),
        "first-party mnemonic gaps:\n{}",
        mnemonic_gaps.join("\n")
    );
    let actual =
        canonical_fixture_files(&all).expect("validate generated fixture content and identities");
    if let Some(dir) = std::env::var_os("RADAR_VIEW_FIXTURES_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, value) in actual {
            std::fs::write(
                dir.join(name),
                format!("{}\n", serde_json::to_string_pretty(&value).unwrap()),
            )
            .unwrap();
        }
    } else {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ink/test/fixtures");
        let mut expected = BTreeMap::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_file() {
                continue;
            }
            let name = entry.file_name().into_string().expect("UTF-8 fixture name");
            if is_generated_fixture_path(&name) {
                let text = std::fs::read_to_string(entry.path()).unwrap_or_else(|error| {
                    panic!("{name}: cannot read committed fixture: {error}")
                });
                let value = serde_json::from_str(&text).unwrap_or_else(|error| {
                    panic!("{name}: invalid committed fixture JSON: {error}")
                });
                expected.insert(name, value);
            }
        }
        compare_fixture_files(&actual, &expected).expect(
            "general fixture drift; export to scratch and review before updating committed files",
        );
    }
    println!("\n{SUCCESS_RECEIPT}");
}

#[test]
fn the_producer_guard_rejects_missing_wrong_and_hidden_mnemonics() {
    use mnemonic_acceptance::{fixture_gaps, key_gap};
    assert!(key_gap("b", "bulk", Some(0), false).is_none());
    assert!(key_gap("Y", "copy", Some(3), true).is_none());
    assert!(key_gap("i", "İstanbul", Some(0), false).is_none());
    assert!(key_gap("İ", "I\u{307}stanbul", Some(0), true).is_none());
    assert!(key_gap("esc", "back", None, false).is_none());
    for (key, word, at, shift) in [
        ("b", "bulk", None, false),
        ("b", "bulk", Some(1), false),
        ("b", "bulk", Some(9), false),
        ("Y", "copy", Some(3), false),
        ("j/k", "move", None, false),
        ("esc", "back", Some(0), false),
    ] {
        assert!(key_gap(key, word, at, shift).is_some());
    }
    let hidden = serde_json::json!({"block":"line", "text":[{"text":"copy", "icon":"copy"}], "key":"y", "mnemonic_at":3});
    assert!(
        fixture_gaps(&hidden)
            .iter()
            .any(|gap| gap.contains("replaceable icon fallback"))
    );
}
