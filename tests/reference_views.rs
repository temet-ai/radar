//! The view data the host had when each per-commit recording of `tests/mix.rs`
//! was made: for every `tests/snapshots/mix__*.snap`, one JSON file
//! `ink/test/fixtures/reference/<recording name>.json` holding
//! `{width, height, chrome, screen, section, focus}`, the fields of the
//! snapshot the IPC bridge sends a renderer (`ipc::Bridge::snapshot_now`),
//! so the Ink renderer can be compared with the recording from the same data.
//!
//! The setups are copies of `tests/mix.rs`'s (this file does not change it).
//! Each recording is built by the same fixtures and key presses, its text
//! (the view data's text form, as `tests/mix.rs` records it) is compared
//! with the `.snap` in `tests/snapshots/` (so the view data is the data of
//! that very screen), and its views are read at that moment. Ink draws the
//! views and compares its picture with its own goldens
//! (`ink/test/reference/goldens/<recording name>.txt`), not with the `.snap`.
//!
//! A recording is made by the build of one mix (`radar::mix()`): the first
//! screen of no parts, of each part alone and of all parts; everything else
//! by all parts. The icon packs' first screens are written for Ink and have
//! no `.snap` (view data is the same whatever the pack). Rewrite the files
//! with one build per mix, each a different feature set (a build is
//! `--no-default-features --features <parts>,demo,test-support`):
//!
//! ```bash
//! export RADAR_REFERENCE_VIEWS_DIR=$PWD/ink/test/fixtures/reference CARGO_TARGET_DIR=target/fleet
//! for parts in "" gitlab github jira slack git workspace workbench sessions \
//!     gitlab,github,jira,slack,git,workspace,workbench,sessions; do
//!   cargo nextest run -p radar --no-default-features --features "${parts:+$parts,}demo,test-support" \
//!     --test reference_views
//! done
//! ```
//!
//! Without `RADAR_REFERENCE_VIEWS_DIR` the tests check the build's recordings:
//! each has view data on disk with its own size, the data reads back, and no
//! recording of this mix in `tests/snapshots/` is missing from this file.

use std::path::{Path, PathBuf};

use radar_core::Host;
use radar_core::host::{Event, View};
use radar_core::ipc::Bridge;
use radar_core::view::{Chord, Key};
use serde_json::{Value, json};

/// The recording's text, as `tests/mix.rs` records it: the view data's text
/// form (`radar_core::headless::host_text_of`), not a painted frame.
fn to_text(host: &Host, width: u16, height: u16) -> String {
    radar_core::headless::host_text_of(host, width, height)
}

/// One recording: its `tests/snapshots/` name (without `.snap`), its text and
/// the view data of the host at that moment.
struct Recording {
    name: String,
    width: u16,
    height: u16,
    text: String,
    views: Value,
}

fn mix_name() -> String {
    let mix = radar::mix();
    if mix.is_empty() { "none".into() } else { mix.join("+") }
}

fn every_part() -> bool {
    cfg!(all(
        feature = "gitlab",
        feature = "github",
        feature = "jira",
        feature = "slack",
        feature = "git",
        feature = "workspace",
        feature = "workbench"
    ))
}

/// No parts, one part, or every part: the mixes whose first screen is
/// recorded per commit.
fn recorded_mix() -> bool {
    radar::mix().len() <= 1 || every_part()
}

/// Where the files are written, read before `same_everywhere` wipes the
/// environment.
fn out_dir() -> Option<PathBuf> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| std::env::var_os("RADAR_REFERENCE_VIEWS_DIR").map(PathBuf::from)).clone()
}

fn committed_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ink/test/fixtures/reference")
}

fn snapshots_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

/// The same start on every machine (as `tests/mix.rs`).
fn same_everywhere() {
    out_dir();
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        const KEEP: [&str; 11] =
            ["PATH", "HOME", "TMPDIR", "TEMP", "TMP", "LANG", "TERM", "USER", "INSTA_UPDATE", "INSTA_FORCE_PASS", "RADAR_SANDBOX"];
        for (name, _) in std::env::vars_os() {
            let keep =
                name.to_str().is_some_and(|n| KEEP.contains(&n) || n.starts_with("CARGO") || n.starts_with("RUST"));
            if !keep {
                // SAFETY: done once, before anything else in this process reads
                // the environment (every path here calls this first).
                unsafe { std::env::remove_var(&name) };
            }
        }
        // Recipe rows show fixed samples, as in `tests/mix.rs`: raising a
        // shipped recipe's version changes no recording.
        // SAFETY: as above.
        unsafe { std::env::set_var("RADAR_SAMPLE_RECIPES", "1") };
        let empty = std::env::temp_dir().join(format!("radar-reference-views-{}", std::process::id()));
        std::fs::create_dir_all(&empty).expect("a scratch folder");
        std::env::set_current_dir(&empty).expect("into the scratch folder");
    });
}

fn started() -> Host {
    same_everywhere();
    let mut host = Host::new(radar::parts(), 1_791_271_000).expect("the parts fit together");
    host.load_config(Some("")).expect("an empty config loads");
    host
}

/// Explicit synthetic fixture, never production restore or credential auth.
#[cfg(feature = "slack")]
fn slack_fixture(host: &mut Host, at: usize, value: serde_json::Value) {
    let cfg: radar_slack::Settings = host.plugins[at].config().unwrap().try_into().unwrap();
    host.plugins[at] = Box::new(radar_slack::test_support::offline_plugin(cfg, value["data"].clone()).unwrap());
}

fn press(host: &mut Host, code: Key) -> usize {
    host.update(Event::Intent(radar_core::view::Intent::Key { chord: Chord::new(code) })).len()
}

/// A key as the recordings press it: after a frame at 160x40.
fn key(host: &mut Host, code: Key) {
    let _ = to_text(host, 160, 40);
    press(host, code);
}

/// Down the tree until the row selected is `text`: the detail pane is
/// titled `panel` (`tests/mix.rs`).
#[cfg(feature = "gitlab")]
fn down_to(host: &mut Host, text: &str, panel: &str) {
    for _ in 0..30 {
        let shown = to_text(host, 160, 40);
        let titled = |l: &str| {
            l.strip_prefix("── detail · ")
                .and_then(|t| t.strip_suffix(" ──"))
                .is_some_and(|t| t.split(' ').any(|w| w == panel))
        };
        if shown.lines().any(titled) {
            return;
        }
        key(host, Key::Down);
    }
    panic!("never reached {text}:\n{}", to_text(host, 160, 40));
}

/// What the IPC bridge sends a renderer for the host as it is, at this size
/// (`Bridge::snapshot_now`, minus the cursor).
/// The bridge itself builds it (it owns the host while it does, and gives it
/// back), so what is written is what a renderer is sent, whatever the bridge
/// adds to a snapshot later.
fn views(host: &mut Host, width: u16, height: u16) -> Value {
    let taken = std::mem::replace(host, Host::new(radar::parts(), 0).expect("the parts fit together"));
    let bridge = Bridge::new(taken, width, height);
    let snapshot = bridge.snapshot_now(0);
    *host = bridge.into_host();
    json!({
        "width": width,
        "height": height,
        "chrome": snapshot.chrome,
        "screen": snapshot.screen,
        "section": snapshot.section,
        "focus": snapshot.focus,
    })
}

/// The recording of `host` as it is: the host draws at this size first
/// (Ink is asked for the views of the host that drew), `text` is what is
/// compared with the `.snap` (the host's text, or a function of it).
fn record(name: String, host: &mut Host, width: u16, height: u16, text: impl FnOnce(String) -> String) -> Recording {
    host.set_view_room(width, height);
    // The views first: the bridge builds this host's theme, which sets the
    // thread's icon pack, and the mode chip's words take their mark from
    // it; read after, the text has this host's pack, not the one of the
    // recording before (the icon packs' screens).
    let views = views(host, width, height);
    let shown = to_text(host, width, height);
    Recording { name, width, height, text: text(shown), views }
}

fn plain(text: String) -> String {
    text
}

fn start(out: &mut Vec<Recording>) {
    if !recorded_mix() {
        return;
    }
    for (w, h) in [(80, 24), (160, 40)] {
        out.push(record(format!("mix__{}_start_{w}x{h}", mix_name()), &mut started(), w, h, plain));
    }
}

fn icons(out: &mut Vec<Recording>) {
    if !every_part() {
        return;
    }
    for pack in ["unicode", "nerd"] {
        let mut host = started();
        host.colours.cfg.icons = pack.into();
        out.push(record(format!("mix__{}_icons_{pack}_start_160x40", mix_name()), &mut host, 160, 40, plain));
    }
}

fn parts(out: &mut Vec<Recording>) {
    if !every_part() {
        return;
    }
    let mut host = started();
    host.view = View::Settings;
    host.section = host.sections.iter().position(|(_, s)| s.id == "parts").expect("the host's parts section");
    out.push(record(format!("mix__{}_parts_120x30", mix_name()), &mut host, 120, 30, plain));
}

#[cfg(feature = "gitlab")]
mod repos {
    use super::*;

    pub(super) const NOW: i64 = 1_791_271_000;

    pub(super) const GITLAB: &str = r#"
[gitlab]
url = "https://gitlab.example"
token = "env:RADAR_MIX_SURELY_UNSET"
base = "acme"

[[gitlab.source]]
group = "team-a"
code = "a"
color = "green"
"#;

    pub(super) fn gitlab_cache(host: &mut Host) {
        gitlab_cache_as(host, "NOT_APPROVED", false);
    }

    pub(super) fn gitlab_cache_as(host: &mut Host, merge_status: &str, rebase: bool) {
        let at = host.plugins.iter().position(|p| p.name() == "GitLab").expect("GitLab");
        host.plugins[at].restore(json!({
            "me": "me",
            "data": { "acme/team-a": {
                "mrs": [{
                    "iid": 263, "title": "Add a stats endpoint", "draft": false,
                    "web_url": "https://gitlab.example/acme/team-a/api/-/merge_requests/263",
                    "updated_at": "2026-10-06T07:00:00Z", "source_branch": "stats", "target_branch": "main",
                    "notes": 0, "conflicts": false, "merge_status": merge_status, "rebase": rebase,
                    "approved": rebase, "approvals_required": 1, "approvals_left": if rebase { 0 } else { 1 },
                    "author": { "username": "alice", "name": "Alice Example" },
                    "reviewers": [], "approved_by": [], "pipeline": "passed", "pipeline_url": null,
                    "additions": 10, "deletions": 2, "files": 1,
                    "project_path": "acme/team-a/api", "project_name": "api"
                }],
                "projects": [
                    { "name": "api", "full_path": "acme/team-a/api", "default_branch": "main" },
                    { "name": "web", "full_path": "acme/team-a/web", "default_branch": "main" }
                ]
            }}
        }));
    }
}

/// `a_merge_request_that_needs_a_rebase_offers_it_on_its_row_and_asks_first`,
/// up to its two recordings.
#[cfg(feature = "gitlab")]
fn row_actions(out: &mut Vec<Recording>) {
    if !every_part() {
        return;
    }
    let mut host = started();
    host.load_config(Some(repos::GITLAB)).expect("the config loads");
    repos::gitlab_cache_as(&mut host, "NEED_REBASE", true);
    let _ = to_text(&host, 220, 40);
    press(&mut host, Key::Char('R'));
    for _ in 0..2 {
        let _ = to_text(&host, 160, 40);
        press(&mut host, Key::Down);
    }
    out.push(record(format!("mix__{}_row_actions_160x40", mix_name()), &mut host, 160, 40, plain));
    // At 80 wide the detail pane is its own screen.
    let focus = host.focus;
    host.focus = radar_core::plugin::Pane::Detail;
    out.push(record(format!("mix__{}_row_actions_80x24", mix_name()), &mut host, 80, 24, plain));
    host.focus = focus;
}

#[cfg(not(feature = "gitlab"))]
fn row_actions(_: &mut Vec<Recording>) {}

#[cfg(all(feature = "gitlab", feature = "jira"))]
mod joined {
    use super::*;

    const CONFIG: &str = r#"
[gitlab]
url = "https://gitlab.example"
token = "env:RADAR_MIX_SURELY_UNSET"
base = "acme"

[[gitlab.source]]
group = "team-a"
code = "a"
color = "green"

[jira]

[[jira.site]]
name = "acme"
url = "https://acme.atlassian.net"
email = "alice@example.com"
token = "env:RADAR_MIX_SURELY_UNSET"
color = "blue"

[[jira.site.project]]
key = "ABC"

[[jira.view]]
name = "Our project"
jql = "project = ABC ORDER BY updated DESC"
"#;

    fn mr(iid: u64, title: &str, branch: &str, approved: bool) -> serde_json::Value {
        json!({
            "iid": iid, "title": title, "draft": false,
            "web_url": format!("https://gitlab.example/acme/team-a/api/-/merge_requests/{iid}"),
            "updated_at": "2026-10-06T07:00:00Z", "source_branch": branch, "target_branch": "main",
            "notes": 0, "conflicts": false, "merge_status": "NOT_APPROVED",
            "approved": approved, "approvals_required": 1, "approvals_left": if approved { 0 } else { 1 },
            "author": { "username": "alice", "name": "Alice Example" },
            "reviewers": [], "approved_by": if approved { json!(["bob"]) } else { json!([]) },
            "pipeline": "passed", "pipeline_url": null, "additions": 10, "deletions": 2, "files": 1,
            "project_path": "acme/team-a/api", "project_name": "api"
        })
    }

    fn ticket(key: &str, summary: &str, status: &str, category: &str) -> serde_json::Value {
        json!({
            "key": key, "summary": summary, "kind": "Story", "status": status, "category": category,
            "assignee": "Alice Example", "assignee_id": "acc-alice",
            "created": "2026-09-20T09:00:00.000+0000", "updated": "2026-10-06T06:00:00.000+0000",
            "project": "ABC", "project_name": "Acme ABC",
            "description": [{ "t": "para", "text": [{ "text": format!("What {key} is about.") }] }]
        })
    }

    fn both() -> Host {
        let mut host = started();
        host.load_config(Some(CONFIG)).expect("the config loads");
        assert_eq!(host.plugins[0].name(), "GitLab");
        host.plugins[0].restore(json!({
            "me": "me",
            "data": { "acme/team-a": {
                "mrs": [
                    mr(263, "ABC-12: Add a stats endpoint for the dashboard", "feature/abc-12-stats", false),
                    mr(264, "Tidy the build", "abc-20-tidy", true),
                    mr(265, "Unrelated change", "tidy", false)
                ],
                "projects": [{ "name": "api", "full_path": "acme/team-a/api", "default_branch": "main" }]
            }}
        }));
        let jira = host.plugins.iter().position(|p| p.id() == "jira").unwrap();
        host.plugins[jira].restore(json!({
            "data": { "acme": {
                "me": { "account_id": "acc-alice", "name": "Alice Example" },
                "tickets": {
                    "ABC-12": ticket("ABC-12", "Add a stats endpoint for the dashboard", "In Review", "doing"),
                    "ABC-20": ticket("ABC-20", "Stats page shows yesterday twice", "To Do", "todo"),
                    "ABC-21": ticket("ABC-21", "Nothing written for it yet", "To Do", "todo")
                },
                "found": { "project = ABC ORDER BY updated DESC": ["ABC-12", "ABC-20", "ABC-21"] }
            }}
        }));
        host
    }

    /// Jira's search results with Jira's numbers, and the development links
    /// held for the tickets (`code`).
    fn with_ids(host: &mut Host, code: serde_json::Value) {
        let mut tickets = serde_json::Map::new();
        for (n, (k, s, st, c)) in [
            ("ABC-12", "Add a stats endpoint for the dashboard", "In Review", "doing"),
            ("ABC-20", "Stats page shows yesterday twice", "To Do", "todo"),
            ("ABC-21", "Nothing written for it yet", "To Do", "todo"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut t = ticket(k, s, st, c);
            t["id"] = json!((10_001 + n).to_string());
            tickets.insert(k.into(), t);
        }
        let jira = host.plugins.iter().position(|p| p.id() == "jira").unwrap();
        host.plugins[jira].restore(json!({ "data": { "acme": {
            "me": { "account_id": "acc-alice", "name": "Alice Example" },
            "tickets": tickets,
            "found": { "project = ABC ORDER BY updated DESC": ["ABC-12", "ABC-20", "ABC-21"] },
            "code": code
        }}}));
    }

    pub(super) fn all(out: &mut Vec<Recording>) {
        if !every_part() {
            return;
        }
        let name = |what: &str| format!("mix__joined__{}_joined_{what}", mix_name());

        // merge_requests_carry_their_tickets
        let mut host = both();
        key(&mut host, Key::Char('R'));
        out.push(record(name("repositories_220x40"), &mut host, 220, 40, plain));

        // tickets_carry_their_code_at_160x40 and _at_80x24
        let mut host = both();
        key(&mut host, Key::Char('J'));
        out.push(record(name("tickets_160x40"), &mut host, 160, 40, plain));
        let _ = to_text(&host, 160, 40);
        out.push(record(name("tickets_80x24"), &mut host, 80, 24, plain));

        // development_links_read_in_the_background_chip_a_merge_request_...
        let mut again = both();
        let link = json!([{
            "id": "!265", "title": "Unrelated change", "status": "open", "repo": "api", "branch": "tidy",
            "url": "https://gitlab.example/acme/team-a/api/-/merge_requests/265"
        }]);
        let entry = |links: serde_json::Value| {
            json!({ "at": 1_791_271_000, "updated": "2026-10-06T06:00:00.000+0000", "links": links })
        };
        with_ids(
            &mut again,
            json!({ "ABC-12": entry(json!([])), "ABC-20": entry(json!([])), "ABC-21": entry(link) }),
        );
        key(&mut again, Key::Char('R'));
        out.push(record(name("background_chip_220x40"), &mut again, 220, 40, plain));

        // a_tickets_code_tab_shows_the_merge_request_and_esc_comes_back
        let mut host = both();
        key(&mut host, Key::Char('J'));
        down_to(&mut host, "ABC-12", "ABC-12");
        key(&mut host, Key::Enter);
        key(&mut host, Key::Char('['));
        out.push(record(name("ticket_code_tab_160x40"), &mut host, 160, 40, plain));
        key(&mut host, Key::Enter);
        out.push(record(name("mr_page_160x40"), &mut host, 160, 40, plain));

        // jiras_actions_on_a_merge_request_open_and_move_its_ticket
        let mut host = both();
        key(&mut host, Key::Char('R'));
        down_to(&mut host, "!263", "!263");
        key(&mut host, Key::Char('x'));
        out.push(record(name("mr_keys_100x40"), &mut host, 100, 40, plain));
    }
}

#[cfg(not(all(feature = "gitlab", feature = "jira")))]
mod joined {
    use super::*;
    pub(super) fn all(_: &mut Vec<Recording>) {}
}

#[cfg(all(feature = "gitlab", feature = "slack"))]
mod alerts {
    use super::repos::{GITLAB, NOW, gitlab_cache};
    use super::*;

    const SLACK: &str = r#"
[slack]
token = "env:RADAR_MIX_SURELY_UNSET"

[[slack.channel]]
alias = "team-alerts"
id = "C0000ALRT1"
name = "team-alerts"
kind = "alerts"
reader = "alertmanager"
color = "orange"
watched = true

[[slack.alerts.map]]
prefix = "api-"
repo = "acme/team-a/api"
service = "api"
"#;

    fn alert(ts: &str, title: &str, color: &str, env: &str) -> serde_json::Value {
        json!({
            "ts": ts, "bot_name": "alertbot", "subtype": "bot_message",
            "cards": [{ "color": color, "title": title,
                "text": format!("*Environment:* {env}\n*Priority:* P1\n*Console:* <https://console.acme.example/alerts/42|console>") }]
        })
    }

    fn both() -> Host {
        let mut host = started();
        host.load_config(Some(&format!("{GITLAB}{SLACK}"))).expect("the config loads");
        gitlab_cache(&mut host);
        let at = host.plugins.iter().position(|p| p.name() == "Slack").expect("Slack");
        let now = NOW;
        let (fired, other) = (format!("{}.000100", now - 3 * 3600 - 450), format!("{}.000200", now - 7200));
        slack_fixture(
            &mut host,
            at,
            json!({ "data": {
                "me": { "user_id": "U0000ALICE", "user": "alice", "team": "Acme", "team_id": "T000ACME1",
                    "url": "https://acme.slack.com/", "tz_offset": 0 },
                "channels": { "C0000ALRT1": { "messages": [
                    alert(&fired, "[FIRING:1] api-HighErrorRate", "danger", "prod"),
                    alert(&other, "[FIRING:1] web-Slow", "danger", "prod"),
                ] } }
            }}),
        );
        host
    }

    /// The screen with each alert's age as `#h ##m`, so the recording holds
    /// no clock reading (`tests/mix.rs`).
    fn aged(shown: String) -> String {
        let shown = shown.as_str();
        let mut out = String::with_capacity(shown.len());
        let mut rest = shown;
        while let Some(at) = rest.find("firing ") {
            let (head, tail) = rest.split_at(at + "firing ".len());
            out.push_str(head);
            let age = tail
                .find(|c: char| !(c.is_ascii_digit() || matches!(c, 'd' | 'h' | 'm' | 's' | ' ')))
                .unwrap_or(tail.len());
            let age = if tail.starts_with(|c: char| c.is_ascii_digit()) { age } else { 0 };
            out.extend(tail[..age].chars().map(|c| if c.is_ascii_digit() { '#' } else { c }));
            rest = &tail[age..];
        }
        out.push_str(rest);
        out
    }

    pub(super) fn all(out: &mut Vec<Recording>) {
        if !every_part() {
            return;
        }
        let name = |what: &str| format!("mix__alerts__{}_alerts_{what}", mix_name());

        let mut host = both();
        let _ = to_text(&host, 220, 40);
        press(&mut host, Key::Char('R'));
        out.push(record(name("repositories_220x40"), &mut host, 220, 40, aged));

        let mut host = both();
        key(&mut host, Key::Char('R'));
        key(&mut host, Key::Down);
        key(&mut host, Key::Enter);
        key(&mut host, Key::Char('['));
        out.push(record(name("repo_page_160x40"), &mut host, 160, 40, aged));
        out.push(record(name("repo_page_80x24"), &mut host, 80, 24, aged));
    }
}

#[cfg(not(all(feature = "gitlab", feature = "slack")))]
mod alerts {
    use super::*;
    pub(super) fn all(_: &mut Vec<Recording>) {}
}

/// Every recording this build's mix makes.
fn recordings() -> Vec<Recording> {
    let mut out = Vec::new();
    start(&mut out);
    icons(&mut out);
    parts(&mut out);
    row_actions(&mut out);
    joined::all(&mut out);
    alerts::all(&mut out);
    out
}

/// The text of a `tests/snapshots/` file: everything after insta's header.
fn snapshot_text(name: &str) -> String {
    let path = snapshots_dir().join(format!("{name}.snap"));
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut parts = raw.splitn(3, "---\n");
    assert_eq!(parts.next(), Some(""), "{name}: the header starts the file");
    parts.next().expect("the header");
    parts.next().expect("the text").trim_end().to_string()
}

/// The recordings in `tests/snapshots/` that this build's mix makes.
fn snapshots_of_this_mix() -> Vec<String> {
    let mix = mix_name();
    let mut names = Vec::new();
    for entry in std::fs::read_dir(snapshots_dir()).unwrap() {
        let file = entry.unwrap().file_name().into_string().unwrap();
        let Some(name) = file.strip_prefix("mix__").and_then(|n| n.strip_suffix(".snap")) else { continue };
        // `mix__<mix>_<what>`, `mix__joined__<mix>_<what>`, `mix__alerts__<mix>_<what>`.
        let rest = name.strip_prefix("joined__").or_else(|| name.strip_prefix("alerts__")).unwrap_or(name);
        if rest.strip_prefix(&mix).is_some_and(|r| r.starts_with('_')) {
            names.push(format!("mix__{name}"));
        }
    }
    names.sort();
    names
}

#[test]
fn each_recording_has_the_views_of_the_screen_it_recorded() {
    let all = recordings();
    assert!(!all.is_empty());
    for r in &all {
        // Only the recordings kept per commit are in tests/snapshots/; the
        // others (the icon packs of the close) are not compared.
        let kept = snapshots_dir().join(format!("{}.snap", r.name)).exists();
        if kept {
            assert_eq!(r.text.trim_end(), snapshot_text(&r.name), "{}: not the screen that was recorded", r.name);
        }
        assert_eq!(r.views["width"], r.width);
        assert_eq!(r.views["height"], r.height);
        // A host-owned settings section (parts, mode, colours, icons) has no
        // body view: the bridge sends the settings navigation in the chrome
        // only (T9's to give; see the finding note of T1).
        // Every recording has a body: a screen, or the settings section.
        assert!(
            r.views["screen"].is_object() || r.views["section"].is_object(),
            "{}: no body view data (screen or section)",
            r.name
        );
        let text = serde_json::to_string(&r.views).unwrap();
        let back: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(back, r.views, "{}: does not read back", r.name);
    }
}

#[test]
fn every_recording_of_this_mix_is_made_here() {
    let made: Vec<String> = {
        let mut names: Vec<String> = recordings().into_iter().map(|r| r.name).collect();
        names.sort();
        names
    };
    let kept: Vec<String> = snapshots_of_this_mix();
    for name in &kept {
        assert!(made.contains(name), "{name} is in tests/snapshots/ and has no view data here");
    }
}

#[test]
fn the_view_data_is_written_when_asked_and_is_on_disk_otherwise() {
    let all = recordings();
    match out_dir() {
        Some(dir) => {
            std::fs::create_dir_all(&dir).unwrap();
            for r in &all {
                let text = serde_json::to_string_pretty(&r.views).unwrap() + "\n";
                std::fs::write(dir.join(format!("{}.json", r.name)), text).unwrap();
            }
        }
        None => {
            for r in &all {
                let path = committed_dir().join(format!("{}.json", r.name));
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("{}: no view data ({e}); see the top of tests/reference_views.rs", path.display()));
                let on_disk: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(
                    (on_disk["width"].clone(), on_disk["height"].clone()),
                    (json!(r.width), json!(r.height)),
                    "{}",
                    path.display()
                );
                assert!(on_disk["chrome"].is_object(), "{}", path.display());
            }
        }
    }
}
