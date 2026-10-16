//! The agent dock in the host: on every screen, the host's state, routing to
//! its owner, the Ratatui line; and the fixtures Ink's dock tests draw from.
use super::*;
use crate::dock::{self, DockIntent, DockView};
use crate::view::{Intent, ListRow, Node, PaneId, PaneView, ScreenView};
use std::sync::{Arc, Mutex};

/// A part that owns a dock: it shows `view` and keeps the intents it got.
struct Docked {
    view: Arc<Mutex<DockView>>,
    got: Arc<Mutex<Vec<DockIntent>>>,
    supplies: bool,
    /// What the host told it of the dock being shown, in order.
    told: Arc<Mutex<Vec<bool>>>,
    /// An item page open over its screen, and the intents its screen got.
    page: Arc<Mutex<Page>>,
}

#[derive(Default)]
struct Page {
    open: bool,
    got: Vec<Intent>,
}

impl Plugin for Docked {
    fn id(&self) -> &'static str {
        "docked"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec { id: "main", name: "Docked", key: 'D' }]
    }
    fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<ScreenView> {
        let rows = vec![ListRow::new("one", "one"), ListRow::new("two", "two")];
        let list = |pane, rows: Vec<ListRow>| PaneView::new(pane, "rows", Node::List { rows, selected: Some(0), editing: None });
        if self.page.lock().unwrap().open {
            let page = crate::view::PageView {
                crumbs: vec![],
                title: crate::view::Text::plain("one"),
                details: crate::view::Text::new(),
                under: None,
                tabs: vec![],
                tab: 0,
                notice: None,
                panes: vec![list(PaneId::Middle, rows)],
                layout: None,
            };
            return Some(ScreenView::page(scope, page));
        }
        Some(ScreenView::new(
            scope,
            vec![list(PaneId::Nav, vec![ListRow::new("nav", "nav")]), list(PaneId::Middle, rows)],
        ))
    }
    fn dock(&self) -> Option<DockView> {
        self.supplies.then(|| self.view.lock().unwrap().clone())
    }
    fn dock_intent(&mut self, intent: &DockIntent) -> Outcome {
        self.got.lock().unwrap().push(intent.clone());
        // A part shows the tab it is asked for.
        if let DockIntent::SwitchTab { id } = intent {
            self.view.lock().unwrap().active = Some(id.clone());
        }
        Outcome::done()
    }
    fn dock_visible(&mut self, visible: bool) {
        self.told.lock().unwrap().push(visible);
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &Intent) -> Outcome {
        self.page.lock().unwrap().got.push(intent.clone());
        Outcome::done()
    }
}

type Got = Arc<Mutex<Vec<DockIntent>>>;

fn host_with(view: DockView, supplies: bool) -> (Host, Got) {
    let got = Got::default();
    let part = Docked { view: Arc::new(Mutex::new(view)), got: got.clone(), supplies, told: Default::default(), page: Default::default() };
    (Host::new(vec![Box::new(part)], 0).unwrap(), got)
}

/// A host whose part's dock can be changed under it, and what the part was told.
type Told = Arc<Mutex<Vec<bool>>>;

fn live_host(view: DockView) -> (Host, Arc<Mutex<DockView>>, Got, Told) {
    let (got, told, view) = (Got::default(), Told::default(), Arc::new(Mutex::new(view)));
    let part = Docked { view: view.clone(), got: got.clone(), supplies: true, told: told.clone(), page: Default::default() };
    (Host::new(vec![Box::new(part)], 0).unwrap(), view, got, told)
}

/// The status line Ratatui draws (the last row), at `w` x `h`.
fn status_row(host: &Host, w: u16, h: u16) -> String {
    crate::shell::to_text(host, w, h).lines().last().unwrap_or_default().to_string()
}

#[test]
fn a_session_that_needs_you_out_of_sight_puts_a_chip_on_the_status_line_and_the_key_goes_there() {
    use dock::SessionState as S;
    let content = base(
        vec![tab("s1", "copilot", "review dock", S::Idle), tab("s2", "codex", "codex 2", S::Approval)],
        vec![],
        composer(true, false),
    );
    let (mut host, view, got, _) = live_host(content);
    // The dock hidden: s2 waits for an approval.
    let chip = host.chrome_status().attention.expect("a chip");
    assert_eq!(chip.text, "codex 2 needs you");
    assert_eq!(chip.ink, crate::view::Ink::Warn);
    assert_eq!(chip.keys, dock::KEY.to_string());
    for (w, h) in [(80u16, 24u16), (160, 40)] {
        let row = status_row(&host, w, h);
        assert!(row.contains("` ") && row.contains("codex 2 needs you"), "{w}x{h}: {row}");
        // After the scope chip, before the keys: it never gives way.
        assert!(row.find("codex 2").unwrap() < row.find("menu").unwrap_or(usize::MAX), "{row}");
    }
    // The dock key shows the dock on that tab, with the keys.
    key(&mut host, dock::KEY);
    assert!(host.dock.visible && host.dock.focused);
    assert_eq!(*got.lock().unwrap(), vec![DockIntent::SwitchTab { id: "s2".into() }]);
    assert!(host.chrome_status().attention.is_none(), "in sight now");
    // Another tab showing again: the chip is back, and a click on it goes there.
    view.lock().unwrap().active = Some("s1".into());
    assert!(host.chrome_status().attention.is_some());
    host.update(Event::Intent(Intent::Activate { target: crate::view::chrome::targets::attention() }));
    assert_eq!(view.lock().unwrap().active.as_deref(), Some("s2"));
    // The answer given: nothing wants you, and the key hides the dock as before.
    view.lock().unwrap().tabs[1].state = S::Streaming;
    assert!(host.chrome_status().attention.is_none());
    assert!(!status_row(&host, 160, 40).contains("needs you"));
}

#[test]
fn a_turn_that_ended_unseen_says_done_or_failed_worst_first_with_how_many_more() {
    use dock::SessionState as S;
    let content = base(
        vec![
            tab("s1", "copilot", "a", S::Streaming),
            unread(tab("s2", "codex", "b", S::Idle)),
            unread(tab("s3", "claude", "c", S::Error)),
            tab("s4", "claude", "d", S::Approval),
        ],
        vec![],
        composer(true, false),
    );
    let (mut host, view, _, _) = live_host(content);
    assert_eq!(host.chrome_status().attention.unwrap().text, "d needs you +2");
    view.lock().unwrap().tabs[3].state = S::Idle;
    let chip = host.chrome_status().attention.unwrap();
    assert_eq!((chip.text.as_str(), chip.ink), ("c failed +1", crate::view::Ink::Bad));
    view.lock().unwrap().tabs[2].unread = false;
    let chip = host.chrome_status().attention.unwrap();
    assert_eq!((chip.text.as_str(), chip.ink), ("b done", crate::view::Ink::Good));
    // The dock shown on that tab: seen, and the dock's own view names nothing.
    view.lock().unwrap().active = Some("s2".into());
    host.update(Event::Intent(Intent::Dock(DockIntent::Show)));
    assert!(host.chrome_status().attention.is_none());
    assert_eq!(shown(&host).unwrap().attention, None);
    // Hidden again: the dock's view names the tab its key goes to.
    host.update(Event::Intent(Intent::Dock(DockIntent::Hide)));
    assert_eq!(shown(&host).unwrap().attention.as_deref(), Some("s2"));
}

#[test]
fn the_part_is_told_each_time_the_dock_shows_or_hides_whatever_did_it() {
    let (mut host, _, _, told) = live_host(DockView::default());
    host.update(Event::Tick(1));
    key(&mut host, dock::KEY);
    key(&mut host, dock::KEY);
    host.route(0, vec![Effect::ShowDock], 0);
    host.update(Event::Tick(2));
    host.update(Event::Intent(Intent::Dock(DockIntent::Blur)));
    assert_eq!(*told.lock().unwrap(), vec![false, true, false, true]);
}

/// Live, the dock key on an item page showed the dock with the keys though
/// the page has no place for it: the keys went nowhere visible, and the
/// lists took them back into the composer. Where the dock is not drawn the
/// key shows or hides it for the lists, and the keys stay with the page.
#[test]
fn on_an_item_page_the_dock_key_never_gives_the_dock_the_keys() {
    let page = Arc::new(Mutex::new(Page { open: true, got: vec![] }));
    let part = Docked { view: Default::default(), got: Got::default(), supplies: true, told: Default::default(), page: page.clone() };
    let mut host = Host::new(vec![Box::new(part)], 0).unwrap();
    assert!(!host.dock_has_place());
    key(&mut host, dock::KEY);
    assert!(host.dock.visible && !host.dock.focused, "shown for the lists, without the keys");
    assert!(!shown(&host).unwrap().focused);
    assert!(host.message.as_deref().is_some_and(|m| m.contains("not on this page")), "{:?}", host.message);
    // The next key is the page's.
    key(&mut host, 'j');
    assert!(!page.lock().unwrap().got.is_empty(), "the page got the key");
    // Nothing else gives it the keys there: a focus, a show from a part.
    host.update(Event::Intent(Intent::Dock(DockIntent::Focus)));
    host.route(0, vec![Effect::ShowDock], 0);
    assert!(host.dock.visible && !host.dock.focused);
    // Pressed again on the page, it hides.
    key(&mut host, dock::KEY);
    assert!(!host.dock.visible);
    // Back on the list the key shows it with the keys, as before.
    page.lock().unwrap().open = false;
    assert!(host.dock_has_place());
    key(&mut host, dock::KEY);
    assert!(host.dock.visible && host.dock.focused && shown(&host).unwrap().focused);
}

#[test]
fn full_height_is_host_state_kept_for_the_run_and_a_drag_gives_the_column_back() {
    let (mut host, got) = host_with(DockView::default(), true);
    host.update(Event::Intent(Intent::Dock(DockIntent::ToggleFull)));
    let d = shown(&host).unwrap();
    assert!(d.full && d.visible);
    // Hidden and shown again, it is still full.
    key(&mut host, dock::KEY);
    key(&mut host, dock::KEY);
    assert!(shown(&host).unwrap().full);
    host.update(Event::Intent(Intent::Dock(DockIntent::ToggleFull)));
    assert!(!shown(&host).unwrap().full);
    host.update(Event::Intent(Intent::Dock(DockIntent::ToggleFull)));
    host.update(Event::Intent(Intent::Dock(DockIntent::Resize { rows: 12 })));
    let d = shown(&host).unwrap();
    assert!(!d.full && d.rows == Some(12));
    assert!(got.lock().unwrap().is_empty(), "the host's own");
}

fn key(host: &mut Host, c: char) {
    host.update(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
}

fn shown(host: &Host) -> Option<Box<DockView>> {
    crate::view::screen_showing(host, 80, 24)?.dock
}

#[test]
fn the_dock_rides_on_every_screen_with_the_hosts_state_over_the_parts_content() {
    let content = states().remove("streaming").unwrap();
    let (host, _) = host_with(content.clone(), true);
    let dock = shown(&host).expect("a dock");
    assert!(!dock.visible && !dock.focused && dock.rows.is_none());
    assert_eq!(dock.tabs, content.tabs);
    assert_eq!(dock.transcript, content.transcript);
    // A part's own `visible: true` is not believed: the host decides.
    let mut loud = content;
    loud.visible = true;
    loud.rows = Some(9);
    let (host2, _) = host_with(loud, true);
    let dock = shown(&host2).unwrap();
    assert!(!dock.visible && dock.rows.is_none());
}

#[test]
fn no_part_with_a_dock_means_no_dock_and_a_dead_key() {
    let (mut host, got) = host_with(DockView::default(), false);
    assert!(shown(&host).is_none());
    key(&mut host, dock::KEY);
    host.update(Event::Intent(Intent::Dock(DockIntent::Send { text: "hi".into() })));
    assert!(!host.dock.visible && got.lock().unwrap().is_empty());
    assert!(!crate::shell::to_text(&host, 80, 24).contains(dock::NOTICE));
}

#[test]
fn settings_carry_the_dock_so_a_sign_in_tab_shows_under_the_section() {
    let (mut host, _) = host_with(states().remove("pty").unwrap(), true);
    key(&mut host, SETTINGS_KEY);
    assert_eq!(host.view, View::Settings);
    key(&mut host, dock::KEY);
    let snap = crate::ipc::Bridge::new(host, 100, 30).snapshot_now(1);
    assert!(snap.screen.is_none() && snap.section.is_some());
    let d = snap.dock.expect("the dock rides with the section");
    assert!(d.visible && d.focused);
    assert!(d.tabs.iter().any(|t| t.terminal.is_some()), "the sign-in tab is there");
}

/// The snapshots a bridge sends now, each acknowledged as Ink does.
fn sent(bridge: &mut crate::ipc::Bridge) -> Vec<crate::ipc::Snapshot> {
    use crate::ipc::{ToHost, ToRenderer};
    let snaps: Vec<_> = bridge
        .outgoing()
        .into_iter()
        .filter_map(|m| match m {
            ToRenderer::Snapshot(s) => Some(*s),
            _ => None,
        })
        .collect();
    if let Some(s) = snaps.last() {
        bridge.receive(ToHost::Ack { cursor: s.cursor });
    }
    snaps
}

/// Live on Settings, Ink kept the dock it was last sent: the key showed and
/// focused it in the host, but Ink still had it hidden and typing reached
/// Settings; esc blurred it in the host, but Ink still had it focused and
/// typing stayed in the composer; a turn that ended still had `stop` on esc.
/// Every change to the dock that rides beside a section is sent.
#[test]
fn on_settings_every_change_to_the_dock_is_sent_to_the_renderer() {
    use crate::ipc::{Bridge, Capabilities, ImageProtocol, ToHost, PROTOCOL};
    let running = states().remove("streaming").unwrap();
    assert!(running.composer.can_stop);
    let content = Arc::new(Mutex::new(running));
    let part = Docked { view: content.clone(), got: Got::default(), supplies: true, told: Default::default(), page: Default::default() };
    let mut bridge = Bridge::new(Host::new(vec![Box::new(part)], 0).unwrap(), 100, 30);
    bridge.receive(ToHost::Hello {
        protocol: PROTOCOL,
        view: crate::view::VERSION,
        capabilities: Capabilities { columns: 100, rows: 30, mouse: true, true_colour: true, image: ImageProtocol::None, plain_text: false },
    });
    sent(&mut bridge);
    bridge.update(Event::Key(KeyEvent::new(KeyCode::Char(SETTINGS_KEY), KeyModifiers::NONE)));
    let on_settings = sent(&mut bridge);
    assert!(on_settings.last().is_some_and(|s| s.section.is_some() && s.screen.is_none()));
    let dock_sent = |snaps: &[crate::ipc::Snapshot]| snaps.last().and_then(|s| s.dock.clone());

    bridge.update(Event::Key(KeyEvent::new(KeyCode::Char(dock::KEY), KeyModifiers::NONE)));
    let d = dock_sent(&sent(&mut bridge)).expect("the key's show is sent");
    assert!(d.visible && d.focused);

    // The turn ends: esc must blur, not stop.
    content.lock().unwrap().composer.can_stop = false;
    let d = dock_sent(&sent(&mut bridge)).expect("the turn's end is sent");
    assert!(!d.composer.can_stop);

    bridge.update(Event::Intent(Intent::Dock(DockIntent::Blur)));
    let d = dock_sent(&sent(&mut bridge)).expect("esc's blur is sent");
    assert!(d.visible && !d.focused, "the keys are Settings' again");

    bridge.update(Event::Key(KeyEvent::new(KeyCode::Char(dock::KEY), KeyModifiers::NONE)));
    let d = dock_sent(&sent(&mut bridge)).expect("the key's focus is sent");
    assert!(d.visible && d.focused);
    bridge.update(Event::Intent(Intent::Dock(DockIntent::Hide)));
    let d = dock_sent(&sent(&mut bridge)).expect("the hide is sent");
    assert!(!d.visible);
}

#[test]
fn a_part_that_opens_a_tab_to_be_answered_shows_the_dock_with_the_keys() {
    let (mut host, _) = host_with(DockView::default(), true);
    key(&mut host, SETTINGS_KEY);
    host.route(0, vec![Effect::ShowDock], 0);
    assert!(host.dock.visible && host.dock.focused);
}

#[test]
fn a_part_that_closed_its_last_tab_hides_the_dock_and_the_page_has_its_keys() {
    let (mut host, _) = host_with(DockView::default(), true);
    key(&mut host, dock::KEY);
    assert!(shown(&host).unwrap().visible);
    host.route(0, vec![Effect::HideDock], 0);
    let d = shown(&host).unwrap();
    assert!(!d.visible && !d.focused);
}

#[test]
fn the_key_shows_the_dock_then_focuses_it_then_hides_it() {
    let (mut host, got) = host_with(DockView::default(), true);
    key(&mut host, dock::KEY);
    let d = shown(&host).unwrap();
    assert!(d.visible && d.focused);
    key(&mut host, dock::KEY);
    let d = shown(&host).unwrap();
    assert!(!d.visible && !d.focused);
    // Blur keeps it shown; the key then takes the keys back.
    host.update(Event::Intent(Intent::Dock(DockIntent::Show)));
    host.update(Event::Intent(Intent::Dock(DockIntent::Blur)));
    let d = shown(&host).unwrap();
    assert!(d.visible && !d.focused);
    key(&mut host, dock::KEY);
    assert!(shown(&host).unwrap().focused);
    // The host's own intents never reach the part.
    assert!(got.lock().unwrap().is_empty());
}

#[test]
fn resize_is_host_state_with_a_floor_and_shows_the_dock() {
    let (mut host, got) = host_with(DockView::default(), true);
    host.update(Event::Intent(Intent::Dock(DockIntent::Resize { rows: 1 })));
    let d = shown(&host).unwrap();
    assert!(d.visible);
    assert_eq!(d.rows, Some(3));
    host.update(Event::Intent(Intent::Dock(DockIntent::Resize { rows: 14 })));
    assert_eq!(shown(&host).unwrap().rows, Some(14));
    assert!(got.lock().unwrap().is_empty());
}

#[test]
fn the_rest_goes_to_the_part_that_owns_the_dock_and_never_to_a_screen() {
    let (mut host, got) = host_with(DockView::default(), true);
    let sent = [
        DockIntent::Send { text: "hello".into() },
        DockIntent::Stop,
        DockIntent::Approve { id: "p1".into() },
        DockIntent::ApproveForSession { id: "p3".into() },
        DockIntent::Deny { id: "p2".into() },
        DockIntent::Fold { item: "t1".into(), open: true },
        DockIntent::PickModel { id: "m".into() },
        DockIntent::SwitchTab { id: "s2".into() },
        DockIntent::NewSession { provider: Some("codex".into()) },
    ];
    for intent in &sent {
        host.update(Event::Intent(Intent::Dock(intent.clone())));
    }
    assert_eq!(*got.lock().unwrap(), sent);
}

#[test]
fn ratatui_says_one_line_where_the_dock_would_be_at_both_sizes() {
    for (w, h) in [(80u16, 24u16), (160, 40)] {
        let (mut host, _) = host_with(states().remove("streaming").unwrap(), true);
        key(&mut host, dock::KEY);
        let text = crate::shell::to_text(&host, w, h);
        let lines: Vec<&str> = text.lines().collect();
        let at: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.contains(dock::NOTICE)).map(|(i, _)| i).collect();
        assert_eq!(at.len(), 1, "{w}x{h}:\n{text}");
        // Under the body, over the status line; no dock controls drawn.
        assert_eq!(at[0], usize::from(h) - 2, "{w}x{h}:\n{text}");
        assert!(!text.contains("approve once"), "{text}");
        // Under the middle pane only: the nav pane keeps its full height, so
        // its own column is not covered at the notice's row.
        if w >= 120 {
            let row = lines[at[0]];
            let col = row.find(dock::NOTICE).unwrap();
            assert!(col >= 20, "notice starts in the middle column: {row}");
            assert!(row.starts_with('│') || row.starts_with(' ') || row.starts_with('┃') || !row[..col].trim().is_empty(), "{row}");
        }
    }
}

#[test]
fn a_hidden_dock_takes_no_row_and_the_key_brings_the_notice() {
    let (mut host, _) = host_with(states().remove("streaming").unwrap(), true);
    assert!(!shown(&host).unwrap().visible);
    let hidden = crate::shell::to_text(&host, 80, 24);
    assert!(!hidden.contains(dock::NOTICE), "{hidden}");
    key(&mut host, dock::KEY);
    let open = crate::shell::to_text(&host, 80, 24);
    assert!(open.contains(dock::NOTICE), "{open}");
}

#[test]
fn every_item_and_intent_round_trips_through_json() {
    for (name, view) in states() {
        let json = serde_json::to_string(&view).unwrap();
        assert_eq!(serde_json::from_str::<DockView>(&json).unwrap(), view, "{name}");
    }
    let intent = Intent::Dock(DockIntent::Resize { rows: 12 });
    let json = serde_json::to_value(&intent).unwrap();
    assert_eq!(json, serde_json::json!({"intent": "dock", "dock": "resize", "rows": 12}));
    assert_eq!(serde_json::from_value::<Intent>(json).unwrap(), intent);
}

// --- the states Ink's dock draws, recorded from the host ----------------------

fn tab(id: &str, provider: &str, title: &str, state: dock::SessionState) -> dock::DockTab {
    let icon = if provider == "pty" { crate::ui::icon::Icon::Terminal } else { crate::ui::icon::Icon::Bot };
    let running = (state == dock::SessionState::Streaming).then_some(14);
    dock::DockTab { id: id.into(), provider: provider.into(), icon: Some(icon), title: title.into(), state, unread: false, running, terminal: None }
}

fn unread(mut t: dock::DockTab) -> dock::DockTab {
    t.unread = true;
    t
}

fn doing(doing: dock::Doing, detail: &str, elapsed: u32, tokens: Option<u64>) -> Option<dock::DockActivity> {
    // The streaming state holds one message sent while the turn runs.
    let queued = u32::from(doing == dock::Doing::Writing);
    Some(dock::DockActivity { doing, detail: detail.into(), elapsed, tokens, queued })
}

fn status() -> dock::DockStatus {
    dock::DockStatus {
        provider: "copilot".into(),
        model: "claude-sonnet-4.5".into(),
        agent: String::new(),
        cwd: "~/projects/radar".into(),
        branch: Some("main".into()),
        usage: Some("12k tokens".into()),
    }
}

fn composer(can_send: bool, can_stop: bool) -> dock::DockComposer {
    dock::DockComposer {
        history: vec!["what does the host own?".into()],
        commands: vec![
            dock::DockCommand { name: "model".into(), detail: "pick a model".into() },
            dock::DockCommand { name: "new".into(), detail: "start a new session".into() },
        ],
        can_send,
        can_stop,
        ..Default::default()
    }
}

fn user(text: &str) -> dock::DockItem {
    dock::DockItem::User { id: "u1".into(), text: text.into(), queued: None }
}

fn tool(open: bool, state: dock::ToolState) -> dock::DockItem {
    dock::DockItem::Tool {
        id: "t1".into(),
        title: "ls crates".into(),
        kind: "shell".into(),
        state,
        open,
        result: vec!["radar-core".into(), "radar-alpha".into(), "radar-beta".into()],
        diff: vec![],
    }
}

fn permission() -> dock::DockItem {
    dock::DockItem::Permission(dock::DockPermission {
        id: "perm-1".into(),
        kind: "write".into(),
        target: "crates/radar-core/src/dock.rs".into(),
        diff: vec!["--- a/dock.rs".into(), "+++ b/dock.rs".into(), "+pub const KEY: char = '`';".into()],
        actions: vec![dock::PermissionAction::ApproveOnce, dock::PermissionAction::Deny, dock::PermissionAction::ViewDiff],
        answered: None,
        tool: None,
        lapsed: false,
    })
}

fn base(tabs: Vec<dock::DockTab>, transcript: Vec<dock::DockItem>, composer: dock::DockComposer) -> DockView {
    DockView {
        active: tabs.first().map(|t| t.id.clone()),
        tabs,
        transcript,
        composer,
        status: status(),
        ..Default::default()
    }
}

/// One dock per state the plan lists. Names are the fixture file stems.
fn states() -> std::collections::BTreeMap<&'static str, DockView> {
    use dock::SessionState as S;
    let mut all = std::collections::BTreeMap::new();
    all.insert("empty", {
        let mut v = base(vec![tab("s1", "copilot", "copilot", S::Idle)], vec![], composer(true, false));
        v.status.usage = None;
        v
    });
    let streaming = |activity| DockView {
        activity,
        ..base(
            vec![tab("s1", "copilot", "review dock", S::Streaming)],
            vec![
                user("what does the host own?"),
                dock::DockItem::Reasoning { id: "r1".into(), text: "Looking at the plan first.".into(), open: false },
                dock::DockItem::assistant("a1", "The host owns all state: sessions, cursors and", false),
                // Sent while the turn runs: the provider keeps it for after this turn.
                dock::DockItem::User { id: "queued-0".into(), text: "and the renderer?".into(), queued: Some(dock::Queued::Provider) },
            ],
            composer(true, true),
        )
    };
    all.insert("streaming", streaming(doing(dock::Doing::Writing, "", 14, Some(3400))));
    // The dock over the whole middle column (alt-f): the fixture writer turns it on.
    all.insert("full", streaming(doing(dock::Doing::Writing, "", 14, Some(3400))));
    all.insert("tool-folded", DockView {
        activity: doing(dock::Doing::Running, "cargo test -p radar-core", 72, None),
        ..base(
            vec![tab("s1", "copilot", "review dock", S::Streaming)],
            vec![
                user("list the crates"),
                tool(false, dock::ToolState::Done),
                dock::DockItem::assistant("a1", "There are three crates.", true),
            ],
            composer(true, false),
        )
    });
    all.insert("tool-open", DockView {
        activity: doing(dock::Doing::Done, "", 42, Some(3400)),
        ..base(vec![tab("s1", "copilot", "review dock", S::Idle)], vec![user("list the crates"), tool(true, dock::ToolState::Done)], composer(true, false))
    });
    all.insert("approval", DockView {
        activity: doing(dock::Doing::Approval, "crates/radar-core/src/dock.rs", 65, Some(1200)),
        ..base(
            vec![tab("s1", "copilot", "review dock", S::Approval), tab("s2", "codex", "codex", S::Idle)],
            vec![user("add the key"), permission()],
            composer(true, true),
        )
    });
    // Every tab state at once: the one showing thinks, the others need you, failed or finished out of sight.
    all.insert("attention", {
        let mut v = base(
            vec![
                tab("s1", "copilot", "review dock", S::Approval),
                unread(tab("s2", "codex", "codex 2", S::Error)),
                unread(tab("s3", "claude", "claude 3", S::Idle)),
                tab("s4", "copilot", "copilot 4", S::Streaming),
            ],
            vec![user("what changed since main?")],
            composer(false, true),
        );
        v.active = Some("s4".into());
        v.activity = doing(dock::Doing::Thinking, "", 3, None);
        v
    });
    all.insert("question", base(
        vec![tab("s1", "copilot", "review dock", S::Approval)],
        vec![
            user("rename the key"),
            dock::DockItem::Question(dock::DockQuestion {
                id: "ask-1".into(),
                questions: vec![dock::DockAsk {
                    id: "0".into(),
                    header: Some("Key".into()),
                    text: "Which key should open the dock?".into(),
                    choices: vec![
                        dock::DockAskChoice { label: "backtick".into(), detail: "what it is now".into() },
                        dock::DockAskChoice { label: "ctrl-a".into(), detail: String::new() },
                    ],
                    free_text: true,
                    multiple: false,
                    answered: false,
                    answer: vec![],
                }],
                state: dock::AskState::Waiting,
            }),
        ],
        composer(true, true),
    ));
    all.get_mut("question").unwrap().activity = doing(dock::Doing::Question, "", 40, None);
    // Restored from the last run and read back: what waited when radar closed lapsed, nothing to press; the other tabs
    // are not attached yet, or being read back.
    all.insert("restored", {
        let mut v = base(
            vec![
                tab("s1", "copilot", "review dock", S::Idle),
                tab("s2", "codex", "codex 2", S::Detached),
                tab("s3", "claude", "claude 3", S::Resuming),
            ],
            vec![
                user("push the branch and ask me which remote"),
                dock::DockItem::Permission(dock::DockPermission {
                    id: "perm-1".into(),
                    kind: "shell".into(),
                    target: "git push origin agent-dock".into(),
                    diff: vec![],
                    actions: vec![],
                    answered: None,
                    tool: None,
                    lapsed: true,
                }),
                dock::DockItem::Question(dock::DockQuestion {
                    id: "ask-1".into(),
                    questions: vec![dock::DockAsk {
                        id: "0".into(),
                        header: None,
                        text: "Which remote?".into(),
                        choices: vec![dock::DockAskChoice { label: "origin".into(), detail: String::new() }],
                        free_text: true,
                        multiple: false,
                        answered: true,
                        answer: vec![],
                    }],
                    state: dock::AskState::Lapsed,
                }),
                dock::DockItem::Notice { id: "n1".into(), text: "the turn that was waiting was ended when radar closed; send a message to carry on".into() },
            ],
            composer(true, false),
        );
        v.status.usage = None;
        v
    });
    // A restored tab not attached to its provider: a message, alt-r or /resume reads it back.
    all.insert("detached", {
        let mut c = composer(true, false);
        c.commands.push(dock::DockCommand { name: "resume".into(), detail: "attach to the provider again and read the session back (alt-r)".into() });
        let mut v = base(vec![tab("s1", "copilot", "review dock", S::Detached), tab("s2", "claude", "claude 2", S::Resuming)], vec![], c);
        v.activity = Some(dock::DockActivity { doing: dock::Doing::Detached, detail: "copilot".into(), elapsed: 0, tokens: None, queued: 0 });
        v.status.usage = None;
        v
    });
    all.insert("error", DockView {
        activity: doing(dock::Doing::Failed, "", 12, None),
        ..base(
            vec![tab("s1", "copilot", "review dock", S::Error)],
            vec![user("go on"), dock::DockItem::Error { id: "e1".into(), text: "connection lost: the copilot process exited".into() }],
            dock::DockComposer { disabled: Some("connection lost".into()), ..composer(false, false) },
        )
    });
    all.insert("stopped", DockView {
        activity: doing(dock::Doing::Stopped, "", 31, None),
        ..base(
            vec![tab("s1", "copilot", "review dock", S::Stopped)],
            vec![
                user("run the tests"),
                tool(false, dock::ToolState::Failed),
                dock::DockItem::Notice { id: "n1".into(), text: "stopped".into() },
            ],
            composer(true, false),
        )
    });
    // Three sessions, the middle one showing: every tab carries its close mark (a click, a middle click, alt-w).
    all.insert("tabs", {
        let mut v = base(
            vec![
                tab("s1", "copilot", "review dock", S::Idle),
                tab("s2", "codex", "codex 2", S::Streaming),
                tab("s3", "claude", "claude 3", S::Stopped),
            ],
            vec![user("what changed since main?")],
            composer(true, true),
        );
        v.active = Some("s2".into());
        v.activity = doing(dock::Doing::Thinking, "", 5, None);
        v
    });
    all.insert("pty", {
        let mut pty = tab("t1", "pty", "codex --version", S::Idle);
        pty.terminal = Some(dock::TerminalView {
            id: "t1".into(),
            socket: String::new(),
            columns: 78,
            rows: 12,
            state: dock::TerminalState::Running,
            command: "codex --version".into(),
            cwd: "~/projects/radar".into(),
        });
        let mut v = base(vec![tab("s1", "copilot", "review dock", S::Idle), pty], vec![], composer(false, false));
        v.active = Some("t1".into());
        v
    });
    // An answer in markdown: headings, bold, inline and fenced code (highlighted), nested lists, a quote, a table, a link.
    all.insert("markdown", base(
        vec![tab("s1", "claude", "review", S::Idle)],
        vec![user("review the branch"), dock::DockItem::assistant("a1", REVIEW, true)],
        composer(true, false),
    ));
    // The same answer half received: the open `**` reads as if closed.
    all.insert("markdown-streaming", base(
        vec![tab("s1", "claude", "review", S::Streaming)],
        vec![user("review the branch"), dock::DockItem::assistant("a1", REVIEW[..REVIEW.find("by three").unwrap()].replace("**Verdict**", "**Verdict"), false)],
        composer(false, true),
    ));
    // A run of tool calls as a review makes them: five reads (one folded row), a search, a command, an edit (its
    // `+n −m`), each approved request folded into its call's row, and one still waiting.
    all.insert("tools", {
        let read = |n: usize, path: &str| named(&format!("t{n}"), "Read", &format!("/home/dev/projects/radar/{path}"), dock::ToolState::Done, &["120 lines"], &[]);
        let mut items = vec![user("review the branch")];
        for (n, path) in ["ink/src/dock/rows.ts", "ink/src/dock/Dock.tsx", "ink/src/dock/composer.ts", "crates/radar-core/src/dock.rs", "crates/app-dock/src/dock.rs"].iter().enumerate() {
            items.push(read(n, path));
            items.push(approved(&format!("p{n}"), "read", &format!("/home/dev/projects/radar/{path}"), &format!("t{n}")));
        }
        items.push(named("t5", "Grep", "fn dock_view", dock::ToolState::Done, &["crates/app-dock/src/dock.rs:215"], &[]));
        items.push(named("t6", "Bash", "cargo test -p app-dock dock", dock::ToolState::Failed, &["test result: FAILED. 11 passed; 1 failed"], &[]));
        items.push(named("t7", "Edit", "/home/dev/projects/radar/crates/app-dock/src/dock.rs", dock::ToolState::Done, &["updated"], EDIT));
        items.push(named("t8", "Bash", "cargo test -p app-dock dock", dock::ToolState::Running, &[], &[]));
        items.push(dock::DockItem::Permission(dock::DockPermission {
            id: "p8".into(),
            kind: "shell".into(),
            target: "cargo test -p app-dock dock".into(),
            diff: vec![],
            actions: vec![dock::PermissionAction::ApproveOnce, dock::PermissionAction::ApproveForSession, dock::PermissionAction::Deny],
            answered: None,
            tool: Some("t8".into()),
            lapsed: false,
        }));
        let mut v = base(vec![tab("s1", "claude", "review", S::Approval)], items, composer(false, true));
        v.status.provider = "claude".into();
        v.status.cwd = "/home/dev/projects/radar".into();
        v
    });
    // An edit opened: its diff in the diff slots, under the call's row.
    all.insert("edit-open", {
        let mut edit = named("t7", "Edit", "/home/dev/projects/radar/crates/app-dock/src/dock.rs", dock::ToolState::Done, &["updated"], EDIT);
        if let dock::DockItem::Tool { open, .. } = &mut edit {
            *open = true;
        }
        let mut v = base(vec![tab("s1", "claude", "review", S::Idle)], vec![user("use String::from"), edit, dock::DockItem::assistant("a1", "Done: `lines` uses `String::from`.", true)], composer(true, false));
        v.status.cwd = "/home/dev/projects/radar".into();
        v
    });
    // Reasoning coming in: folded, its latest line shown beside the fold.
    all.insert("reasoning-live", base(
        vec![tab("s1", "claude", "review", S::Streaming)],
        vec![
            user("why does the build fail?"),
            dock::DockItem::Reasoning {
                id: "r1".into(),
                text: "The failing job is the Ink one.\nIts log ends in a snapshot diff, so the look changed and the snapshots did not follow".into(),
                open: false,
            },
        ],
        composer(false, true),
    ));
    all
}

/// The status line while the dock is hidden and sessions want the user, as
/// the host sends it, with the row Ratatui draws from it beside it: Ink's
/// status line is compared with that row at both sizes. Written with the
/// dock fixtures (`RADAR_DOCK_FIXTURES_DIR`) as `status/attention_<w>x<h>.json`.
#[test]
fn the_attention_chip_is_recorded_with_the_row_ratatui_draws() {
    let dir = std::env::var("RADAR_DOCK_FIXTURES_DIR").ok().map(std::path::PathBuf::from);
    for (w, h) in [(80u16, 24u16), (160, 40)] {
        let (mut host, _) = host_with(states().remove("attention").unwrap(), true);
        host.set_view_room(w, h);
        let chrome = host.chrome_view(w, h);
        let chip = chrome.status.attention.as_ref().expect("the dock is hidden: s1 needs you");
        assert_eq!(chip.text, "review dock needs you +2");
        let row = status_row(&host, w, h);
        assert!(row.contains("` ") && row.contains(&chip.text), "{w}x{h}: {row}");
        let json = serde_json::json!({ "chrome": chrome, "ratatui": row });
        if let Some(dir) = &dir {
            let dir = dir.join("status");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("attention_{w}x{h}.json")), serde_json::to_string_pretty(&json).unwrap() + "\n").unwrap();
        }
        // A press of the dock key there shows the dock on that tab.
        key(&mut host, dock::KEY);
        assert!(host.dock.visible);
    }
}

/// Markdown at its edges: a level-1 heading, a word wider than the line,
/// marks, nested and numbered lists, task items, nested quotes, a rule, an
/// aligned table, code with a tab and trailing blanks, links, a hard break.
const EDGES: &str = "# Title one\n\nA paragraph with a verylongwordthatmustbreakacrossthelineedgeforsure and **bold, then** _italic_ ~~gone~~.\n\n1. first\n   - nested *one*\n   - nested two\n2. second with `code span` here\n\n- [ ] todo\n- [x] done\n\n> quoted **text** that goes on long enough to wrap at narrow widths\n>\n> > nested quote\n\n---\n\n| Name | Value | Notes |\n|:--|--:|:-:|\n| a | 1 | short |\n| longer name here | 12345 | a note that wraps |\n\n```\nplain\tcode with tab\n\n\n```\n\nLink <https://example.test/x> and [words](https://example.test/y).  \nhard break";

/// ui::rich's layout of the dock's markdown as Ratatui lays it out (link
/// addresses after their words, as the dock's cells show them), at several
/// widths: the reference `ink/src/text/richRows.ts` is held to. With
/// `RADAR_DOCK_FIXTURES_DIR` set it is written there as `rich-layout.json`.
#[test]
fn rich_layout_reference() {
    use crate::ui::rich;
    let theme = crate::theme::Theme::new(crate::design::theme::Tier::Mono);
    let mut look = rich::Look::new(&theme);
    look.urls = true;
    let samples: Vec<serde_json::Value> = [("review", REVIEW), ("edges", EDGES)]
        .iter()
        .map(|(name, md)| {
            let doc = crate::ui::markdown::parse(md);
            let widths: serde_json::Map<String, serde_json::Value> = [12u16, 24, 38, 76, 156]
                .iter()
                .map(|w| {
                    let text = rich::text_of(&rich::layout(&doc, &look, *w).lines);
                    (w.to_string(), serde_json::json!(text.split('\n').collect::<Vec<_>>()))
                })
                .collect();
            serde_json::json!({ "name": name, "doc": doc, "widths": widths })
        })
        .collect();
    assert!(samples.iter().all(|s| s["widths"]["76"].as_array().is_some_and(|l| l.len() > 5)));
    if let Ok(dir) = std::env::var("RADAR_DOCK_FIXTURES_DIR") {
        let json = serde_json::to_string_pretty(&samples).unwrap() + "\n";
        std::fs::write(std::path::Path::new(&dir).join("rich-layout.json"), json).unwrap();
    }
}

#[test]
fn a_streamed_answer_reads_as_if_its_open_marks_were_closed() {
    use dock::streamed;
    // bold and code still open on the last line close against its last word
    assert_eq!(streamed("**Verdict: Request changes "), "**Verdict: Request changes** ");
    assert_eq!(streamed("see `origin/ma"), "see `origin/ma`");
    assert_eq!(streamed("**a** `b` ~~c"), "**a** `b` ~~c~~");
    // a mark with no words yet is dropped, a link still coming shows its words
    assert_eq!(streamed("a **b** and **"), "a **b** and ");
    assert_eq!(streamed("read [the plan](https://exa"), "read the plan");
    // a `**` inside code is code
    assert_eq!(streamed("`a ** b` and"), "`a ** b` and");
    // ended lines, and an open fence, are left as they are
    assert_eq!(streamed("**done**\n"), "**done**\n");
    assert_eq!(streamed("```rust\nfn x() { **"), "```rust\nfn x() { **");
    assert_eq!(streamed("1. **one"), "1. **one**");
}

#[test]
fn an_answer_is_read_as_markdown_with_its_code_highlighted() {
    let dock::DockItem::Assistant { doc, painted, .. } = dock::DockItem::assistant("a", "## Hi\n\n```rust\nfn x() {}\n```\n\n- in a list:\n\n  ```nosuchlang\n  plain\n  ```", true) else {
        panic!("not an answer")
    };
    assert!(matches!(doc.blocks[0], crate::ui::rich::Block::Heading { level: 2, .. }));
    // one entry per code block, in document order; the unknown language has none
    assert_eq!(painted.len(), 2);
    assert!(painted[0][0].iter().any(|r| r.token == "keyword" && r.text == "fn"));
    assert!(painted[1].is_empty());
    // a TypeScript fence reads with the JavaScript grammar (none of its own is bundled)
    let dock::DockItem::Assistant { painted, .. } = dock::DockItem::assistant("a", "```typescript\nconst x = 1;\n```", true) else { panic!() };
    assert!(painted[0][0].iter().any(|r| r.token == "keyword" || r.token == "type"), "{painted:?}");
    // the half-received answer is read with its marks closed
    let dock::DockItem::Assistant { doc, .. } = dock::DockItem::assistant("a", "**Verd", false) else { panic!() };
    let crate::ui::rich::Block::Para { text } = &doc.blocks[0] else { panic!("{doc:?}") };
    assert!(text[0].marks.bold && text[0].text == "Verd", "{text:?}");
}

/// The answer the markdown states show.
const REVIEW: &str = "## Review\n\n**Verdict**: Request changes. The branch is behind `origin/main` by three commits.\n\n- **Findings**:\n  - The tool rows take *two lines* each; one is enough.\n  - Reads in a row should fold together.\n- **Fixed**: `lines` uses `String::from`.\n\n1. Rebase on `origin/main`.\n2. Run the checks again.\n\n> The host owns all state; Ink only draws it.\n\n```rust\nfn lines(text: &str) -> Vec<String> {\n    text.lines().map(String::from).collect() // one per line\n}\n```\n\n| Check | Result |\n|---|---|\n| typecheck | ok |\n| tests | 12 passed |\n\nSee [the plan](https://example.test/agent-dock) for the dock's rules.";

/// The edit the tool states make.
const EDIT: &[&str] = &[" fn lines(text: &str) -> Vec<String> {", "-    text.lines().map(str::to_string).collect()", "+    text.lines().map(String::from).collect()", " }"];

fn named(id: &str, name: &str, title: &str, state: dock::ToolState, result: &[&str], diff: &[&str]) -> dock::DockItem {
    dock::DockItem::Tool {
        id: id.into(),
        title: title.into(),
        kind: name.into(),
        state,
        open: false,
        result: result.iter().map(|s| s.to_string()).collect(),
        diff: diff.iter().map(|s| s.to_string()).collect(),
    }
}

fn approved(id: &str, kind: &str, target: &str, tool: &str) -> dock::DockItem {
    dock::DockItem::Permission(dock::DockPermission {
        id: id.into(),
        kind: kind.into(),
        target: target.into(),
        diff: vec![],
        actions: vec![dock::PermissionAction::ApproveOnce, dock::PermissionAction::Deny],
        answered: Some(dock::PermissionAction::ApproveOnce),
        tool: Some(tool.into()),
        lapsed: false,
    })
}

/// The dock states as the host sends them: a whole `ScreenView` with the dock
/// laid over the part's content, the dock shown and focused, at both sizes.
/// With `RADAR_DOCK_FIXTURES_DIR` set the JSON is written there:
///
/// RADAR_DOCK_FIXTURES_DIR=$PWD/ink/test/fixtures/dock \
///   CARGO_TARGET_DIR=target/fleet cargo nextest run -p radar-core -E 'test(dock_fixtures)'
#[test]
fn dock_fixtures_are_what_the_host_sends() {
    let dir = std::env::var("RADAR_DOCK_FIXTURES_DIR").ok().map(std::path::PathBuf::from);
    for (name, content) in states() {
        for (w, h) in [(80u16, 24u16), (160, 40)] {
            let (mut host, _) = host_with(content.clone(), true);
            host.set_view_room(w, h);
            host.update(Event::Intent(Intent::Dock(DockIntent::Show)));
            if name == "full" {
                host.update(Event::Intent(Intent::Dock(DockIntent::ToggleFull)));
            }
            let view = crate::view::screen_showing(&host, w, h).unwrap();
            let dock = view.dock.as_deref().unwrap();
            assert!(dock.visible && dock.focused, "{name}");
            let json = serde_json::to_string_pretty(&view).unwrap() + "\n";
            let back: ScreenView = serde_json::from_str(&json).unwrap();
            assert_eq!(back, view, "{name}");
            if let Some(dir) = &dir {
                std::fs::create_dir_all(dir).unwrap();
                std::fs::write(dir.join(format!("{name}_{w}x{h}.json")), json).unwrap();
            }
        }
    }
}

/// A part that keeps its waker and notes the times it was asked what woke it.
struct Waking {
    id: &'static str,
    waker: Arc<Mutex<Option<crate::wake::Waker>>>,
    woken: Arc<Mutex<Vec<i64>>>,
}

impl Plugin for Waking {
    fn id(&self) -> &'static str {
        self.id
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn set_waker(&mut self, waker: crate::wake::Waker) {
        *self.waker.lock().unwrap() = Some(waker);
    }
    fn woken(&mut self, now: i64) -> Vec<Effect> {
        self.woken.lock().unwrap().push(now);
        vec![]
    }
}

#[test]
fn a_part_that_wakes_the_host_is_asked_at_the_next_wake_and_the_others_are_not() {
    let part = |id| {
        let (waker, woken) = (Arc::new(Mutex::new(None)), Arc::new(Mutex::new(Vec::new())));
        (Waking { id, waker: waker.clone(), woken: woken.clone() }, waker, woken)
    };
    let (a, a_waker, a_woken) = part("a");
    let (b, _, b_woken) = part("b");
    let mut host = Host::new(vec![Box::new(a), Box::new(b)], 0).unwrap();
    let waker = a_waker.lock().unwrap().clone().expect("the host gave the part its waker");
    // From the part's own thread, three times before the host looks: one call.
    std::thread::spawn(move || (0..3).for_each(|_| waker.wake())).join().unwrap();
    host.update(Event::Wake(7));
    host.update(Event::Wake(8));
    assert_eq!(*a_woken.lock().unwrap(), vec![7]);
    assert!(b_woken.lock().unwrap().is_empty());
}
