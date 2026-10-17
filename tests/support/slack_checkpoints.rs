//! Root-owned Slack starting checkpoints. Only this part is composed here.
//! Every constructor owns fresh state and replays real setup; downstream tests
//! can name the starting point without copying keys, requests or domain logic.
use radar_core::host::Event;
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::scenario::Scenario;
use radar_core::view::{Intent, Node, PaneId, TreeRow};
use radar_core::writes::Mode;
use radar_core::{Host, Pane};
use radar_slack::test_support::{
    AcceptedPost, ExpectedRequest, HostedFixture, Request, hosted_plugin,
};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) const NOW: i64 = 1_791_271_000;
pub(super) const CHANNEL: &str = "CWATCH001";
pub(super) const ROOT: &str = "1791270000.000100";
pub(super) const BODY: &str = "Captain Carrot is checking the gate";
pub(super) const BUDGET: usize = 256;
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Scratch(PathBuf);
impl Scratch {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "radar-slack-scenario-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub(super) fn journal(&self) -> PathBuf {
        self.0.join("actions.sqlite3")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read(method: &'static str, params: &[(&str, &str)]) -> Request {
    Request {
        method,
        params: params
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect(),
        write: false,
        body: None,
    }
}
fn search(query: &str) -> Request {
    read(
        "search.messages",
        &[
            ("query", query),
            ("count", "40"),
            ("sort", "timestamp"),
            ("sort_dir", "desc"),
            ("page", "1"),
        ],
    )
}
fn root_message() -> Value {
    json!({"ts": ROOT, "text": "The Watch needs a gate report"})
}
pub(super) fn post() -> Request {
    Request {
        method: "chat.postMessage",
        params: vec![],
        write: true,
        body: Some(
            json!({"channel":CHANNEL,"thread_ts":ROOT,"text":BODY,"unfurl_links":false,"unfurl_media":false}),
        ),
    }
}
pub(super) fn refreshed_thread() -> ExpectedRequest {
    ExpectedRequest::json(
        "refresh-thread",
        read(
            "conversations.replies",
            &[("channel", CHANNEL), ("ts", ROOT), ("limit", "200")],
        ),
        json!({"ok":true,"messages":[root_message(),{"ts":"1791271000.000900","thread_ts":ROOT,"text":BODY}],"has_more":false}),
    )
}

/// Only an observed acceptance may supply this later synthetic server read.
/// Request matching, parsing and ingestion remain the normal production path.
fn accepted_thread(receipt: &AcceptedPost) -> ExpectedRequest {
    assert_eq!(receipt.request(), &post());
    let body = receipt.request().body.as_ref().unwrap();
    ExpectedRequest::json(
        "accepted-thread",
        read(
            "conversations.replies",
            &[
                ("channel", body["channel"].as_str().unwrap()),
                ("ts", body["thread_ts"].as_str().unwrap()),
                ("limit", "200"),
            ],
        ),
        json!({"ok":true,"messages":[root_message(),receipt.message()],"has_more":false}),
    )
}

/// Serialized output of the production snapshot captured by Scenario. This is
/// not the production cache database writer/loader and contains no kept draft.
pub(super) struct CapturedSlackCache(Vec<u8>);

impl CapturedSlackCache {
    pub(super) fn capture(s: &Scenario) -> Self {
        let (_, value) = s
            .cache_writes()
            .iter()
            .rev()
            .find(|(plugin, _)| *plugin == "slack")
            .expect("Slack emitted a cache snapshot");
        Self(serde_json::to_vec(value).unwrap())
    }

    pub(super) fn value(&self) -> Value {
        serde_json::from_slice(&self.0).unwrap()
    }
}

fn refresh() -> Vec<ExpectedRequest> {
    vec![
        ExpectedRequest::json(
            "identity",
            read("auth.test", &[]),
            json!({"ok":true,"team_id":"TWATCH001","team":"Ankh-Morpork Watch","user_id":"UVIMES001","user":"vimes","url":"https://watch.slack.com/"}),
        ),
        ExpectedRequest::json(
            "timezone",
            read("users.info", &[("user", "UVIMES001")]),
            json!({"ok":true,"user":{"id":"UVIMES001","name":"vimes","tz_offset":0}}),
        ),
        ExpectedRequest::json(
            "groups",
            read("usergroups.list", &[("include_users", "true")]),
            json!({"ok":true,"usergroups":[]}),
        ),
        ExpectedRequest::json(
            "mentions",
            search("<@UVIMES001> after:2026-09-29"),
            json!({"ok":true,"messages":{"matches":[{"ts":ROOT,"text":"The Watch needs a gate report","channel":{"id":CHANNEL,"name":"watch"}}],"paging":{"pages":1}}}),
        ),
        ExpectedRequest::json(
            "direct",
            search("to:me after:2026-09-29"),
            json!({"ok":true,"messages":{"matches":[],"paging":{"pages":1}}}),
        ),
    ]
}
pub(super) fn key(s: &mut Scenario, code: KeyCode) {
    s.dispatch(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .unwrap();
}
pub(super) fn view(s: &Scenario) -> Value {
    s.inspect(|host| {
        serde_json::to_value(host.screen_view_in(0, "feed", 120, 40).expect("Slack view")).unwrap()
    })
}

pub(super) fn writes(f: &HostedFixture) -> usize {
    f.ledger().iter().filter(|r| r.request.write).count()
}

fn start(
    mode: Mode,
    expected: Vec<ExpectedRequest>,
    scratch: &Scratch,
    cache: Option<&CapturedSlackCache>,
) -> (Scenario, HostedFixture) {
    let (plugin, fixture) = hosted_plugin(expected).unwrap();
    let mut host = Host::new(vec![Box::new(plugin)], NOW).unwrap();
    host.load_config(Some("[slack]\ntoken = 'xoxp-offline-synthetic-fixture'\n"))
        .unwrap();
    host.set_env_mode(Some((mode, "RADAR_MODE".into())));
    host.focus = Pane::Middle;
    if let Some(cache) = cache {
        let value = cache.value();
        host.plugins[0].restore(value.clone());
        assert_eq!(host.plugins[0].snapshot(), Some(value));
    }
    (
        Scenario::with_journal(host, &scratch.journal()).unwrap(),
        fixture,
    )
}

/// A fresh Host using the real restore consumer. Hold identity so cached
/// presentation and the absence of authenticated authority can be inspected.
pub(super) fn at_restored_cached_feed(
    cache: &CapturedSlackCache,
    receipt: &AcceptedPost,
    scratch: &Scratch,
) -> (Scenario, HostedFixture) {
    let mut expected: Vec<_> = refresh()
        .into_iter()
        .enumerate()
        .map(|(i, request)| if i == 0 { request.held() } else { request })
        .collect();
    expected.push(accepted_thread(receipt));
    start(Mode::Write, expected, scratch, Some(cache))
}

pub(super) async fn at_open_thread(
    mode: Mode,
    extra: Vec<ExpectedRequest>,
    scratch: &Scratch,
) -> (Scenario, HostedFixture) {
    let mut expected = refresh();
    expected.push(ExpectedRequest::json(
        "open-thread",
        read(
            "conversations.replies",
            &[("channel", CHANNEL), ("ts", ROOT), ("limit", "200")],
        ),
        json!({"ok":true,"messages":[root_message()],"has_more":false}),
    ));
    expected.extend(extra);
    let (mut s, fixture) = start(mode, expected, scratch, None);
    s.advance_time(NOW).unwrap();
    s.settle(BUDGET).await.unwrap();
    assert_eq!(
        fixture.ledger().len(),
        5,
        "real bootstrap and For you reads"
    );
    assert!(
        !s.cache_writes().is_empty(),
        "refresh results reached the real reducer"
    );
    activate_gate_thread(&mut s);
    s.settle(BUDGET).await.unwrap();
    assert!(
        view(&s)
            .to_string()
            .contains("The Watch needs a gate report")
    );
    (s, fixture)
}

pub(super) fn activate_gate_thread(s: &mut Scenario) {
    let screen = s.inspect(|host| host.screen_view_in(0, "feed", 120, 40).unwrap());
    let Node::Tree(tree) = &screen
        .pane(PaneId::Middle)
        .unwrap_or_else(|| panic!("missing feed middle pane: {screen:?}"))
        .body
    else {
        panic!("For you tree: {screen:?}")
    };
    let path = tree
        .visible()
        .into_iter()
        .find(|(path, _)| path.last().is_some_and(|id| id.contains(ROOT)))
        .expect("refreshed message row")
        .0;
    s.dispatch(Event::Intent(Intent::Select {
        target: TreeRow::target_of(&path),
    }))
    .unwrap();
    s.dispatch(Event::Intent(Intent::Activate {
        target: TreeRow::target_of(&path),
    }))
    .unwrap();
}

pub(super) fn compose_reply(s: &mut Scenario, fixture: &HostedFixture) {
    key(s, KeyCode::Char('c'));
    assert!(s.host().plugins[0].capturing());
    s.dispatch(Event::Paste(BODY.into())).unwrap();
    assert!(
        view(s).to_string().contains(BODY),
        "composer view holds the draft"
    );
    s.dispatch(Event::Key(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    )))
    .unwrap();
    let menu = &s
        .host()
        .menu
        .as_ref()
        .expect("real write confirmation")
        .menu;
    assert!(menu.title.starts_with("post this in "));
    assert!(menu.items.iter().any(|row| row.label.contains(BODY)));
    assert_eq!(writes(fixture), 0, "preview must not send");
}

/// A replayed checkpoint: this executes refresh, selection and composition
/// through the real host. It does not deserialize or invent a confirmation state.
/// `extra` changes only the provider responses needed after this checkpoint.
pub(super) async fn at_reply_confirmation(
    mode: Mode,
    extra: Vec<ExpectedRequest>,
    scratch: &Scratch,
) -> (Scenario, HostedFixture) {
    let (mut scenario, fixture) = at_open_thread(mode, extra, scratch).await;
    compose_reply(&mut scenario, &fixture);
    (scenario, fixture)
}
