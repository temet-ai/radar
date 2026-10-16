//! Local page transitions are separate from the existing thread-read lifecycle.

use super::*;
use crate::screen::Item;
use radar_core::journal::{ActionEvent, Journal, Query};
use radar_core::plugin::{Outcome as KeyOutcome, Pane};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::{Showing, Subject};

const OPEN: &str = "slack.page.open";

fn opened(effects: &[Effect]) -> &ActionDescriptor {
    let found: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Observe { action, outcome } if action.kind.as_str() == OPEN => {
                assert_eq!(*outcome, Outcome::Succeeded);
                Some(action)
            }
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "exactly one independent open");
    found[0]
}

fn no_open(effects: &[Effect]) {
    assert!(!effects.iter().any(|effect| matches!(effect,
        Effect::Observe { action, .. } if action.kind.as_str() == OPEN)));
}

fn handled(outcome: KeyOutcome) -> Vec<Effect> {
    match outcome {
        KeyOutcome::Handled(effects) => effects,
        KeyOutcome::Ignored => vec![],
    }
}

fn select(p: &mut SlackPlugin, item: Item) {
    let path = vec!["fixture row".into()];
    p.shown.borrow_mut().insert(path.clone(), item.clone());
    if let Item::Thread { channel, .. } | Item::Reply { channel, .. } = &item
        && let Some(i) = crate::screen::nav_rows(p).iter().position(|row| {
            matches!(row,
            crate::screen::NavRow::Channel(index) if p.cfg.channels[*index].id == *channel)
        }) {
        p.nav_at = i;
    }
    let cx = radar_core::view::ViewCx {
        now: p.now,
        focus: radar_core::view::PaneId::Middle,
        width: 160,
        height: 40,
        parts: radar_core::compose::Parts::none(),
        icon_lead: 2,
        setup: false,
    };
    let _ = radar_core::Plugin::view(p, crate::SCREEN.id, &cx);
    let actual = p
        .shown
        .borrow()
        .iter()
        .find(|(_, shown)| **shown == item)
        .map(|(path, _)| path.clone());
    if actual.is_none() {
        p.shown.borrow_mut().insert(path.clone(), item);
    }
    p.tree.borrow_mut().select(actual.unwrap_or(path));
}

/// A pointer press on a target of the view, as the host sends it.
fn point(p: &mut SlackPlugin, target: &str) -> KeyOutcome {
    p.intent(
        crate::SCREEN.id,
        Pane::Middle,
        &radar_core::view::Intent::Select {
            target: radar_core::view::Target::new(target),
        },
    )
}

fn key(p: &mut SlackPlugin, code: KeyCode) -> Vec<Effect> {
    handled(p.key(
        crate::SCREEN.id,
        Pane::Middle,
        KeyEvent::new(code, KeyModifiers::NONE),
    ))
}

#[test]
fn open_scope_is_the_requested_thread_with_distinct_workspace_identity() {
    let a = [super::super::page_opened(TEAM, demo::TEAM, ROOT)];
    let b = [super::super::page_opened("T000NEXT1", demo::TEAM, ROOT)];
    scope(opened(&a), TEAM, demo::TEAM, ROOT);
    scope(opened(&b), "T000NEXT1", demo::TEAM, ROOT);
    assert_ne!(opened(&a).subjects, opened(&b).subjects);
    assert!(!opened(&a).metadata.redacted);
    assert_eq!(opened(&a).metadata.omitted_subjects, 0);
}

#[test]
fn missing_invalid_oversized_and_known_secret_scope_is_omitted_only_from_new_observation() {
    let long_team = format!("T{}", "1".repeat(2048));
    let long_channel = format!("C{}", "2".repeat(2048));
    let long_ts = format!("{}.1", "3".repeat(2048));
    for (team, channel, ts) in [
        ("", demo::TEAM, ROOT),
        ("T", demo::TEAM, ROOT),
        ("private workspace", demo::TEAM, ROOT),
        (TEAM, "private channel", ROOT),
        (TEAM, demo::TEAM, "private timestamp"),
        (TEAM, demo::TEAM, "12"),
        (TEAM, demo::TEAM, "12."),
        (TEAM, demo::TEAM, ".1"),
        (TEAM, demo::TEAM, "12.1?token=private"),
        (long_team.as_str(), demo::TEAM, ROOT),
        (TEAM, long_channel.as_str(), ROOT),
        (TEAM, demo::TEAM, long_ts.as_str()),
    ] {
        let effects = [super::super::page_opened(team, channel, ts)];
        let action = opened(&effects);
        assert!(action.subjects.is_empty() && action.metadata.redacted);
        assert_eq!(action.metadata.omitted_subjects, 1);
    }
    // Distinct canaries avoid changing any other test's safe identity registry.
    for (team, channel, ts, secret) in [
        ("TOPENSECRET791", demo::TEAM, ROOT, "TOPENSECRET791"),
        (TEAM, "COPENSEC792", ROOT, "COPENSEC792"),
        (TEAM, demo::TEAM, "9876543210.000793", "9876543210.000793"),
        (
            TEAM,
            "COPENSEC794",
            "9876543210.000794",
            "COPENSEC794/9876543210.000794",
        ),
    ] {
        radar_core::log::secret(secret);
        let effects = [super::super::page_opened(team, channel, ts)];
        let action = opened(&effects);
        assert!(action.subjects.is_empty() && action.metadata.redacted);
        assert_eq!(action.metadata.omitted_subjects, 1);
        assert!(!format!("{action:?}").contains(secret));
        assert_eq!(
            page_thread_action(team, channel, ts).subjects.len(),
            1,
            "the established read scope is unchanged"
        );
    }
}

#[test]
fn whole_cached_push_preserves_exact_marks_focus_scroll_and_zero_request_behavior() {
    let (mut p, rec) = sample(vec![]);
    let held = p.data.channels.get_mut(demo::TEAM).unwrap();
    held.messages[0].reply_count = 1;
    held.messages[0].latest_reply = Some(OTHER.into());
    held.replies
        .insert(ROOT.into(), vec![demo::msg(OTHER, demo::BOB, PRIVATE)]);
    let mut expected = p.data.clone();
    expected.read_to(demo::TEAM, ROOT, OTHER);
    p.page_scroll.set(43);
    for depth in 1..=2 {
        let effects = page::open_thread(&mut p, demo::TEAM, ROOT);
        scope(opened(&effects), TEAM, demo::TEAM, ROOT);
        assert!(matches!(
            &effects[..],
            [
                Effect::Observe { .. },
                Effect::Focus(Pane::Middle),
                Effect::PersistCache
            ]
        ));
        assert_eq!(p.pages.len(), depth);
        assert_eq!(p.page_scroll.at(), 0);
        assert_eq!(p.data, expected);
        assert!(p.threads.is_empty());
        assert!(rec.requests().is_empty());
    }
}

#[test]
fn page_only_cache_cold_inflight_and_missing_client_keep_existing_read_decisions() {
    let (mut p, rec) = sample(vec![]);
    p.data
        .channels
        .get_mut(demo::TEAM)
        .unwrap()
        .messages
        .clear();
    p.threads
        .entry(thread_key(demo::TEAM, ROOT))
        .or_default()
        .value = Some(vec![demo::msg(ROOT, demo::BOB, PRIVATE)]);
    // Only Data.thread establishes whole content, never the page-only cache.
    let first = page::open_thread(&mut p, demo::TEAM, ROOT);
    scope(opened(&first), TEAM, demo::TEAM, ROOT);
    assert!(matches!(
        &first[..],
        [
            Effect::Observe { .. },
            Effect::Focus(Pane::Middle),
            Effect::PersistCache,
            Effect::SpawnAction { .. }
        ]
    ));
    let (_, task) = take_task(first);
    drop(task);
    assert_eq!(p.data.opened[&format!("{}/{ROOT}", demo::TEAM)], ROOT);
    let second = page::open_thread(&mut p, demo::TEAM, ROOT);
    opened(&second);
    count(&second, "suppressed", 1, Outcome::Blocked, false);
    assert_eq!(second.len(), 5);
    assert!(matches!(&second[4], Effect::Notify(s) if s == "already reading this thread"));
    p.via = Via(None);
    p.cfg.token.clear();
    let before = format!("{:?}", p.threads);
    let third = page::open_thread(&mut p, demo::TEAM, ROOT);
    opened(&third);
    count(&third, "unavailable", 1, Outcome::Blocked, false);
    assert_eq!(third.len(), 4);
    assert_eq!(format!("{:?}", p.threads), before);
    assert_eq!(p.pages.len(), 3);
    assert!(rec.requests().is_empty());
}

#[test]
fn real_enter_paths_capture_the_thread_root_and_preserve_selected_message_marks() {
    let routes = [
        Item::Thread {
            channel: demo::TEAM.into(),
            ts: ROOT.into(),
        },
        Item::Reply {
            channel: demo::TEAM.into(),
            thread: ROOT.into(),
            ts: OTHER.into(),
        },
        Item::Found(0),
        Item::Alert {
            channel: demo::TEAM.into(),
            ts: ROOT.into(),
        },
    ];
    for item in routes {
        let (mut p, rec) = sample(vec![]);
        p.data.for_you[0].channel_id = demo::TEAM.into();
        p.data.for_you[0].message.ts = OTHER.into();
        p.data.for_you[0].message.thread_ts = Some(ROOT.into());
        let marked = if matches!(item, Item::Reply { .. } | Item::Found(_)) {
            OTHER
        } else {
            ROOT
        };
        let mut expected = p.data.clone();
        expected.read_to(demo::TEAM, ROOT, ROOT);
        expected.read_to(demo::TEAM, ROOT, marked);
        select(&mut p, item);
        let effects = key(&mut p, KeyCode::Enter);
        scope(opened(&effects), TEAM, demo::TEAM, ROOT);
        assert_eq!(p.data, expected);
        assert_eq!(
            p.pages,
            [page::Page::Thread {
                channel: demo::TEAM.into(),
                ts: ROOT.into(),
                tab: 0,
                row: 0
            }]
        );
        assert!(matches!(
            &effects[..],
            [
                Effect::Observe { .. },
                Effect::Focus(Pane::Middle),
                Effect::PersistCache
            ]
        ));
        assert!(rec.requests().is_empty());
    }
}

#[test]
fn accepted_legacy_show_uses_consistent_fields_and_draft_is_not_a_thread_open() {
    let (mut p, rec) = sample(vec![]);
    let subject = Subject::new("thread", format!("{}/{ROOT}", demo::TEAM), PRIVATE)
        .with("channel", demo::TEAM)
        .with("ts", ROOT);
    let Showing::Page { effects, .. } = p.show(&subject) else {
        panic!("accepted show")
    };
    scope(opened(&effects), TEAM, demo::TEAM, ROOT);
    assert!(
        matches!(p.pages.last(), Some(page::Page::Thread { channel, ts, .. }) if channel == demo::TEAM && ts == ROOT)
    );
    for rejected in [
        Subject::new("thread", PRIVATE, PRIVATE),
        Subject::new("thread", PRIVATE, PRIVATE).with("channel", demo::TEAM),
        Subject::new("other", PRIVATE, PRIVATE),
    ] {
        assert!(matches!(p.show(&rejected), Showing::Cannot));
        assert_eq!(p.pages.len(), 1);
    }
    p.via = Via(None);
    p.cfg.token.clear();
    p.data.me = None;
    let unsafe_subject = Subject::new("thread", "untrusted id", PRIVATE)
        .with("channel", "private channel")
        .with("ts", "private timestamp");
    assert!(matches!(p.show(&unsafe_subject), Showing::Cannot));
    assert_eq!(p.pages.len(), 1);
    p.draft = Some(crate::ask::Draft::default());
    let Showing::Page { effects, .. } = p.show(&Subject::new("review-ask", "draft", PRIVATE))
    else {
        panic!("draft show")
    };
    no_open(&effects);
    assert_eq!(p.pages.last(), Some(&page::Page::Draft));
    assert!(rec.requests().is_empty());
}

#[test]
fn selection_click_fold_tab_scroll_refresh_and_idle_tick_are_not_page_pushes() {
    let (mut p, rec) = sample(vec![]);
    select(&mut p, Item::Heading("group".into()));
    no_open(&key(&mut p, KeyCode::Enter));
    assert!(p.pages.is_empty());
    no_open(&handled(point(&mut p, "rail:0")));
    no_open(&key(&mut p, KeyCode::Down));
    no_open(&key(&mut p, KeyCode::Char(' ')));
    assert!(p.pages.is_empty());
    page::open_thread(&mut p, demo::TEAM, ROOT);
    for code in [
        KeyCode::Enter,
        KeyCode::Char(']'),
        KeyCode::Char('['),
        KeyCode::Down,
        KeyCode::PageDown,
        KeyCode::Char('r'),
    ] {
        no_open(&key(&mut p, code));
        assert_eq!(p.pages.len(), 1);
    }
    no_open(&handled(point(&mut p, "tab:1")));
    no_open(&p.tick(demo::NOW));
    no_open(&key(&mut p, KeyCode::Esc));
    assert!(p.pages.is_empty());
    assert!(rec.requests().is_empty());
}

fn real_host(p: SlackPlugin, sender: bool) -> Host {
    let mut parts: Vec<Box<dyn Plugin>> = vec![Box::new(demo::sample())];
    if sender {
        parts.push(Box::new(ShowSender));
    }
    let mut host = Host::new(parts, demo::NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    host.focus = Pane::Middle;
    host.enable_journal();
    host
}

fn slack(host: &mut Host) -> &mut SlackPlugin {
    assert_eq!(host.plugins[0].id(), "slack");
    // SAFETY: real_host installs this exact plugin type at index zero.
    unsafe { &mut *(host.plugins[0].as_mut() as *mut dyn Plugin as *mut SlackPlugin) }
}

fn press(host: &mut Host, code: KeyCode) -> Vec<HostEffect> {
    host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn begin(records: &[JournalRecord], kind: &str) -> (ActionId, ActionDescriptor, Option<ActionId>) {
    let found: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } if descriptor.kind.as_str() == kind => {
                Some((id.clone(), descriptor.clone(), parent.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "one {kind}");
    found.into_iter().next().unwrap()
}

fn no_host_open(records: &[JournalRecord]) {
    assert!(!records.iter().any(|record| matches!(record,
        JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == OPEN)));
}

fn export(all: &[JournalRecord], forbidden: &[&str]) -> Vec<ActionEvent> {
    let dir = std::env::temp_dir().join(format!("radar-slack-page-open-{}", ActionId::new().as_str()));
    let mut db = Journal::open(&dir.join("actions.sqlite3")).unwrap();
    let session = db.start_session().unwrap();
    for record in all {
        match record {
            JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } => db
                .begin_with_id(&session, id, descriptor.clone(), parent.as_ref())
                .unwrap(),
            JournalRecord::Outcome {
                id,
                outcome,
                metadata,
            } => {
                db.record(id, *outcome, metadata.clone()).unwrap();
            }
            JournalRecord::Gap { .. } => panic!("unexpected gap"),
        }
    }
    db.finish_session(&session).unwrap();
    let events = db
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    let json = serde_json::to_string(&events).unwrap();
    for private in
        forbidden
            .iter()
            .copied()
            .chain([PRIVATE, "private.example", "xoxp-", "PRIVATE_ERROR"])
    {
        assert!(
            !json.contains(private),
            "private text reached exported history: {private}"
        );
        for entry in std::fs::read_dir(&dir).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            assert!(
                !bytes
                    .windows(private.len())
                    .any(|window| window == private.as_bytes()),
                "private text reached SQLite or sidecars: {private}"
            );
        }
    }
    drop(db);
    std::fs::remove_dir_all(dir).unwrap();
    events
}

#[test]
fn host_cached_enter_records_distinct_successful_opens_under_input_and_persists_no_payload() {
    let (mut p, rec) = sample(vec![]);
    p.data.channels.get_mut(demo::TEAM).unwrap().messages[0].text = PRIVATE.into();
    p.cfg.channels[0].alias = PRIVATE.into();
    p.data.users.insert(demo::BOB.into(), PRIVATE.into());
    select(
        &mut p,
        Item::Thread {
            channel: demo::TEAM.into(),
            ts: ROOT.into(),
        },
    );
    let mut host = real_host(p, false);
    let mut all = vec![];
    let mut ids = vec![];
    for _ in 0..2 {
        let effects = press(&mut host, KeyCode::Enter);
        assert!(!effects.iter().any(|effect| match effect {
            HostEffect::Spawn { .. } => true,
            HostEffect::Tracked { effect, .. } => matches!(&**effect, HostEffect::Spawn { .. }),
            _ => false,
        }));
        let got = records(&effects);
        let (id, action, parent) = begin(&got, OPEN);
        let (input, _, _) = begin(&got, "plugin.input");
        assert_eq!(parent, Some(input));
        scope(&action, TEAM, demo::TEAM, ROOT);
        assert_eq!(terminals(&got, &id), [Outcome::Succeeded]);
        assert_eq!(host.focus, Pane::Middle);
        ids.push(id);
        all.extend(got);
        all.extend(records(&press(&mut host, KeyCode::Esc)));
    }
    assert_ne!(ids[0], ids[1]);
    assert!(slack(&mut host).pages.is_empty());
    let events = export(&all, &[]);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind.as_str() == OPEN && event.outcome == Outcome::Succeeded)
            .count(),
        2
    );
    assert!(!events.iter().any(|event| event.kind.as_str() == PREFIX));
    assert!(rec.requests().is_empty());
}

#[test]
fn failed_partial_and_interrupted_read_siblings_cannot_rewrite_successful_opens() {
    for outcome in [Outcome::Failed, Outcome::Partial, Outcome::Unknown] {
        let body = if outcome == Outcome::Failed {
            json!({"ok": false, "error": PRIVATE})
        } else {
            json!({"ok": true, "messages": null})
        };
        let (mut p, rec) = sample(vec![ok(body)]);
        p.data.channels.get_mut(demo::TEAM).unwrap().messages[0].reply_count = 1;
        select(
            &mut p,
            Item::Thread {
                channel: demo::TEAM.into(),
                ts: ROOT.into(),
            },
        );
        let mut host = real_host(p, false);
        let start = press(&mut host, KeyCode::Enter);
        let mut all = records(&start);
        let (open, _, open_parent) = begin(&all, OPEN);
        let (read, _, read_parent) = begin(&all, PREFIX);
        assert_eq!(open_parent, read_parent, "read and open are siblings");
        assert_ne!(open, read);
        assert_eq!(terminals(&all, &open), [Outcome::Succeeded]);
        let (runtime, semantic, task) = tracked(start);
        assert_eq!(semantic, read);
        all.extend(records(&press(&mut host, KeyCode::Esc)));
        let again = press(&mut host, KeyCode::Enter);
        let got = records(&again);
        let (reopen, _, _) = begin(&got, OPEN);
        assert_ne!(reopen, open);
        let (suppressed, _, _) = begin(&got, "slack.page.thread.suppressed");
        assert_eq!(terminals(&got, &suppressed), [Outcome::Blocked]);
        all.extend(got);
        if outcome == Outcome::Unknown {
            drop(task);
            assert!(rec.requests().is_empty());
        } else {
            let completed = deliver(&mut host, runtime, semantic, run(task));
            let got = records(&completed);
            assert_eq!(terminals(&got, &read), [outcome]);
            assert!(terminals(&got, &open).is_empty());
            assert!(terminals(&got, &reopen).is_empty());
            all.extend(got);
            assert_eq!(rec.requests().len(), 1);
        }
        assert_eq!(terminals(&all, &open), [Outcome::Succeeded]);
        assert_eq!(terminals(&all, &reopen), [Outcome::Succeeded]);
        let events = export(&all, &[]);
        for id in [&open, &reopen] {
            assert_eq!(
                events
                    .iter()
                    .filter(|event| &event.action_id == id && event.outcome.is_terminal())
                    .map(|event| event.outcome)
                    .collect::<Vec<_>>(),
                [Outcome::Succeeded]
            );
        }
        assert!(
            events
                .iter()
                .any(|event| event.action_id == read && event.outcome == outcome)
        );
    }
}

struct ShowSender;
impl Plugin for ShowSender {
    fn id(&self) -> &'static str {
        "sender"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        vec![Effect::Show(*msg.downcast::<Subject>().unwrap())]
    }
}

#[test]
fn host_show_retains_accepted_target_distinct_teams_and_redacted_private_scope() {
    let (p, rec) = sample(vec![]);
    let mut host = real_host(p, true);
    let mut all = vec![];
    let mut scopes = vec![];
    for team in [TEAM, "T000NEXT1", "TOPENPRIVATE795"] {
        let p = slack(&mut host);
        let channels = p.data.channels.clone();
        let mut me = p.data.me.clone().unwrap();
        me.team_id = team.into();
        assert!(p.install_me(me));
        // Each account independently has the same synthetic cached thread.
        p.data.channels = channels;
        if team == "TOPENPRIVATE795" {
            radar_core::log::secret(team);
        }
        let subject = Subject::new("thread", format!("{}/{ROOT}", demo::TEAM), PRIVATE)
            .with("channel", demo::TEAM)
            .with("ts", ROOT);
        let shown = host.update(Event::Msg {
            plugin: 1,
            msg: Box::new(subject),
        });
        let got = records(&shown);
        let (open, action, parent) = begin(&got, OPEN);
        let (show, _, _) = begin(&got, "ui.show");
        assert_eq!(parent, Some(show));
        assert_eq!(terminals(&got, &open), [Outcome::Succeeded]);
        if team == "TOPENPRIVATE795" {
            assert!(action.subjects.is_empty() && action.metadata.redacted);
            assert_eq!(action.metadata.omitted_subjects, 1);
        } else {
            scope(&action, team, demo::TEAM, ROOT);
            scopes.push(action.subjects[0].key.clone());
        }
        assert!(
            matches!(slack(&mut host).pages.last(), Some(page::Page::Thread { channel, ts, .. }) if channel == demo::TEAM && ts == ROOT)
        );
        all.extend(got);
    }
    assert_eq!(scopes[0].id, scopes[1].id);
    assert_ne!(scopes[0].instance, scopes[1].instance);
    let rejected = host.update(Event::Msg {
        plugin: 1,
        msg: Box::new(Subject::new("thread", PRIVATE, PRIVATE)),
    });
    no_host_open(&records(&rejected));
    assert_eq!(slack(&mut host).pages.len(), 1);
    all.extend(records(&rejected));
    export(&all, &["TOPENPRIVATE795"]);
    assert!(rec.requests().is_empty());
}
