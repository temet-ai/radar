//! Real Host popups across injected refreshes; no task or service is run.

use super::*;
use crate::Via;
use crate::client::{ListEvidence, MessagesRead, Recorded};
use crate::demo::{self, BOB, NOW, TEAM};
use radar_core::host::{Event, Hit, HostEffect, MouseKind};
use radar_core::ratatui::crossterm::event::KeyModifiers;
use radar_core::shell::to_text;
use radar_core::{Host, Plugin};
use std::sync::{Arc, Mutex};

const ROOT: &str = "1791200000.000100";
const OTHER: &str = "1791200001.000100";
const A: &str = "https://example.com/private-first";
const B: &str = "https://example.com/private-second";

fn slack(host: &Host) -> &SlackPlugin {
    assert_eq!(host.plugins[0].id(), "slack");
    // SAFETY: the fixture installs SlackPlugin at index zero.
    unsafe { &*(host.plugins[0].as_ref() as *const dyn Plugin as *const SlackPlugin) }
}

fn slack_mut(host: &mut Host) -> &mut SlackPlugin {
    assert_eq!(host.plugins[0].id(), "slack");
    // SAFETY: the fixture installs SlackPlugin at index zero, exclusively borrowed.
    unsafe { &mut *(host.plugins[0].as_mut() as *mut dyn Plugin as *mut SlackPlugin) }
}

fn fixture(words: &str, extra: Option<Box<dyn Plugin>>) -> (Host, Arc<Recorded>) {
    let recorded = Recorded::new(vec![]);
    let mut p = demo::sample();
    p.via = Via(Some(recorded.clone()));
    on_thread(&mut p, TEAM, ROOT, Ok(vec![demo::msg(ROOT, BOB, words)]));
    p.pages.push(Page::Thread {
        channel: TEAM.into(),
        ts: ROOT.into(),
        tab: 1,
        row: 0,
    });
    let mut plugins: Vec<Box<dyn Plugin>> = vec![Box::new(SlackPlugin::new())];
    plugins.extend(extra);
    let mut host = Host::new(plugins, NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    to_text(&host, 160, 40);
    (host, recorded)
}

fn press(host: &mut Host, code: KeyCode) -> Vec<HostEffect> {
    host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn popup(host: &mut Host) {
    press(host, KeyCode::Char('x'));
    assert!(host.menu.is_some());
    to_text(host, 160, 40);
}

fn refresh(host: &mut Host, words: &str) {
    let p = slack(host);
    let stamp = p.read_stamp();
    let action =
        crate::sync_history::page_thread_action(&p.data.me.as_ref().unwrap().team_id, TEAM, ROOT);
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(SlackMsg::Thread {
            stamp,
            channel: TEAM.into(),
            ts: ROOT.into(),
            action,
            result: Ok(MessagesRead {
                messages: vec![demo::msg(ROOT, BOB, words)],
                evidence: ListEvidence::default(),
            }),
        }),
    });
}

fn choose(host: &mut Host, how: &str) -> Vec<HostEffect> {
    match how {
        "copy" => press(host, KeyCode::Char('y')),
        "open" => press(host, KeyCode::Char('o')),
        "enter" => press(host, KeyCode::Enter),
        "click" => {
            let i = host
                .menu
                .as_ref()
                .unwrap()
                .menu
                .items
                .iter()
                .position(|item| item.key == Some('y'))
                .unwrap();
            let area = host
                .hits
                .borrow()
                .iter()
                .find_map(|(area, hit)| (*hit == Hit::MenuItem(i)).then_some(*area))
                .unwrap();
            host.update(Event::Mouse {
                kind: MouseKind::Press,
                column: area.x,
                row: area.y,
                at_ms: 1000,
            })
        }
        _ => unreachable!(),
    }
}

fn targets(effects: &[HostEffect]) -> Vec<&str> {
    effects
        .iter()
        .flat_map(|e| match e {
            HostEffect::Copy(url) | HostEffect::Open(url) => vec![url.as_str()],
            HostEffect::Tracked { effect, .. } => targets(std::slice::from_ref(effect.as_ref())),
            _ => vec![],
        })
        .collect()
}

fn refused(host: &Host, effects: &[HostEffect]) {
    assert!(effects.is_empty(), "{effects:?}");
    assert!(host.menu.is_none());
    let message = host.message.as_deref().unwrap();
    assert!(message.contains("press x"));
    assert!(!message.contains("private-"));
    for (width, height) in [(80, 24), (160, 40)] {
        let frame = to_text(host, width, height);
        assert!(
            frame.lines().last().unwrap().contains(message),
            "the full recovery guidance must fit beside the counts: {frame}"
        );
    }
}

#[test]
fn popup_link_refresh_cannot_retarget_before_or_after_redraw_by_key_enter_or_click() {
    for redraw in [false, true] {
        for how in ["copy", "enter", "click"] {
            let (mut host, recorded) = fixture(&format!("<{A}|first>"), None);
            popup(&mut host);
            refresh(&mut host, &format!("<{B}|second>"));
            if redraw {
                to_text(&host, 160, 40);
            }
            let effects = choose(&mut host, how);
            refused(&host, &effects);
            to_text(&host, 160, 40);
            popup(&mut host);
            assert_eq!(targets(&choose(&mut host, how)), [B]);
            assert!(recorded.requests().is_empty());
        }
    }
}

#[test]
fn unchanged_link_survives_refresh_and_labels_or_counts_changing() {
    for how in ["copy", "enter", "click"] {
        let (mut host, recorded) = fixture(&format!("<{A}|first>"), None);
        popup(&mut host);
        refresh(
            &mut host,
            &format!("<{A}|a renamed link> <{B}|another link>"),
        );
        to_text(&host, 160, 40);
        assert_eq!(targets(&choose(&mut host, how)), [A]);
        assert!(recorded.requests().is_empty());
    }
}

#[test]
fn popup_captures_the_selected_nonzero_link_and_not_the_first_row() {
    for same in [false, true] {
        let (mut host, recorded) = fixture(&format!("<{A}|first> <{B}|second>"), None);
        let area = host
            .hits
            .borrow()
            .iter()
            .find_map(|(area, hit)| {
                matches!(hit, Hit::Plugin { id, .. } if id == "link:1").then_some(*area)
            })
            .unwrap();
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: area.x,
            row: area.y,
            at_ms: 100,
        });
        assert!(matches!(
            slack(&host).pages.last(),
            Some(Page::Thread { row: 1, .. })
        ));
        popup(&mut host);
        let next = if same {
            B
        } else {
            "https://example.com/replacement"
        };
        refresh(&mut host, &format!("<{A}|first> <{next}|second>"));
        to_text(&host, 160, 40);
        let effects = choose(&mut host, "copy");
        if same {
            assert_eq!(targets(&effects), [B]);
        } else {
            refused(&host, &effects);
        }
        assert!(recorded.requests().is_empty());
    }
}

#[test]
fn missing_links_account_revisions_and_changed_thread_cannot_borrow_a_popup() {
    for change in [
        "removed",
        "token",
        "token-aba",
        "team",
        "user",
        "thread",
        "tab",
    ] {
        for redraw in [false, true] {
            let (mut host, recorded) = fixture(&format!("<{A}|first>"), None);
            popup(&mut host);
            match change {
                "removed" => refresh(&mut host, "Nothing linked now."),
                "token" | "token-aba" => {
                    let p = slack_mut(&mut host);
                    let original = p.cfg.token.clone();
                    p.replace_token("env:RADAR_MENU_TEST_UNSET_TOKEN".into());
                    if change == "token-aba" {
                        p.replace_token(original);
                    }
                }
                "team" | "user" => {
                    let p = slack_mut(&mut host);
                    let mut me = p.data.me.clone().unwrap();
                    if change == "team" {
                        me.team_id = "T000OTHER".into();
                    } else {
                        me.user_id = "U000OTHER".into();
                    }
                    p.install_me(me);
                }
                "thread" => {
                    let p = slack_mut(&mut host);
                    on_thread(
                        p,
                        TEAM,
                        OTHER,
                        Ok(vec![demo::msg(OTHER, BOB, &format!("<{A}|same URL>"))]),
                    );
                    p.pages.push(Page::Thread {
                        channel: TEAM.into(),
                        ts: OTHER.into(),
                        tab: 1,
                        row: 0,
                    });
                }
                "tab" => {
                    show_tab(slack_mut(&mut host), 0);
                }
                _ => unreachable!(),
            }
            if redraw {
                to_text(&host, 160, 40);
            }
            let effects = choose(&mut host, "copy");
            refused(&host, &effects);
            assert!(recorded.requests().is_empty());
        }
    }
}

struct Recogniser(Arc<Mutex<Subject>>);

impl Plugin for Recogniser {
    fn id(&self) -> &'static str {
        "target-probe"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn owns(&self) -> Vec<radar_core::Owned> {
        vec![radar_core::Owned::link("test-object", "test objects")]
    }
    fn recognise(&self, text: &str) -> Vec<(std::ops::Range<usize>, Subject)> {
        text.find("TARGET")
            .map(|i| (i..i + 6, self.0.lock().unwrap().clone()))
            .into_iter()
            .collect()
    }
}

struct ExactRepository;

const REPOSITORY_URL: &str = "https://code.example/acme/api";

impl Plugin for ExactRepository {
    fn id(&self) -> &'static str {
        "repository-probe"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn owns(&self) -> Vec<radar_core::Owned> {
        vec![radar_core::Owned::link("repository", "repositories")]
    }
    fn recognise(&self, text: &str) -> Vec<(std::ops::Range<usize>, Subject)> {
        text.eq_ignore_ascii_case(REPOSITORY_URL)
            .then(|| (0..text.len(), Subject::new("repository", REPOSITORY_URL, "api")
                .with_url(REPOSITORY_URL)))
            .into_iter()
            .collect()
    }
}

#[test]
fn repository_link_keeps_unsupported_subroutes_queries_and_same_prefix_siblings() {
    let urls = [
        REPOSITORY_URL,
        "https://code.example/acme/api/pull/7/files",
        "https://code.example/acme/api/pull/7?context=1",
        "https://code.example/acme/api?context=1",
        "https://code.example/acme/api-sibling",
    ];
    let words = urls.iter().map(|url| format!("<{url}|link>")).collect::<Vec<_>>().join(" ");
    let (mut host, recorded) = fixture(&words, Some(Box::new(ExactRepository)));
    let links = page_links(slack(&host));
    assert_eq!(links.len(), urls.len());
    assert_eq!(links[0].kind, "repository");
    assert_eq!(links.iter().filter_map(|s| s.url.as_deref()).collect::<Vec<_>>(), urls);
    assert!(links[1..].iter().all(|s| s.is(radar_core::compose::LINK)));
    drop(links);
    for url in &urls[1..] {
        press(&mut host, KeyCode::Down);
        assert_eq!(targets(&press(&mut host, KeyCode::Char('o'))), [*url]);
    }
    assert!(recorded.requests().is_empty());
}

#[test]
fn full_span_recognition_deduplicates_a_canonicalized_repository_link() {
    let (host, recorded) = fixture(
        "<https://code.example/ACME/API|api>",
        Some(Box::new(ExactRepository)),
    );
    let links = page_links(slack(&host));
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].kind, "repository");
    assert_eq!(links[0].url.as_deref(), Some(REPOSITORY_URL));
    assert!(recorded.requests().is_empty());
}

#[test]
fn recognised_subject_url_and_routing_fields_are_guarded_even_when_subject_equality_agrees() {
    for changed in ["url", "field", "related"] {
        for how in ["copy", "open", "enter", "click"] {
            let original = Subject::new("test-object", "same-id", "target")
                .with_url(A)
                .with("instance", "first");
            let target = Arc::new(Mutex::new(original.clone()));
            let (mut host, recorded) =
                fixture("TARGET", Some(Box::new(Recogniser(target.clone()))));
            {
                let links = page_links(slack(&host));
                assert_eq!(links.len(), 1);
                assert_eq!(links[0].kind, "test-object");
                assert_eq!(links[0].url, original.url);
                assert_eq!(links[0].fields, original.fields);
            }
            popup(&mut host);
            {
                let mut target = target.lock().unwrap();
                match changed {
                    "url" => target.url = Some(B.into()),
                    "field" => {
                        target.fields.insert("instance".into(), "second".into());
                    }
                    "related" => target.related.push(Subject::link(B)),
                    _ => unreachable!(),
                }
                assert_eq!(
                    *target, original,
                    "public Subject equality is intentionally unchanged"
                );
            }
            to_text(&host, 160, 40);
            let effects = choose(&mut host, how);
            refused(&host, &effects);
            assert!(recorded.requests().is_empty());
        }
    }
}

#[test]
fn private_link_context_is_absent_from_host_records_and_persisted_history() {
    use radar_core::host::JournalRecord;
    use radar_core::journal::{ActionId, Journal, Outcome as JournalOutcome, Query};
    const PRIVATE: &str = "PRIVATE_MENU_ROUTING_CANARY";
    let original = Subject::new("test-object", "same-id", "target")
        .with_url(A)
        .with("instance", PRIVATE);
    let target = Arc::new(Mutex::new(original));
    let (mut host, recorded) = fixture("TARGET", Some(Box::new(Recogniser(target.clone()))));
    host.enable_journal();
    let mut effects = press(&mut host, KeyCode::Char('x'));
    to_text(&host, 160, 40);
    target.lock().unwrap().url = Some(B.into());
    to_text(&host, 160, 40);
    effects.extend(choose(&mut host, "copy"));
    let records: Vec<_> = effects
        .into_iter()
        .filter_map(|e| match e {
            HostEffect::Journal(record) => Some(record),
            _ => None,
        })
        .collect();
    let rendered = format!("{records:?}");
    for private in [PRIVATE, A, B] {
        assert!(!rendered.contains(private));
    }
    assert!(records.iter().any(|r| matches!(
        r,
        JournalRecord::Outcome {
            outcome: JournalOutcome::Blocked,
            ..
        }
    )));
    assert!(!records.iter().any(|r| matches!(
        r,
        JournalRecord::Outcome {
            outcome: JournalOutcome::Confirmed,
            ..
        }
    )));
    #[cfg(unix)]
    {
        let dir = std::env::temp_dir().join(format!(
            "radar-slack-menu-context-{}",
            ActionId::new().as_str()
        ));
        let mut journal = Journal::open(&dir.join("actions.sqlite3")).unwrap();
        let session = journal.start_session().unwrap();
        for record in &records {
            match record {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                } => journal
                    .begin_with_id(&session, id, descriptor.clone(), parent.as_ref())
                    .unwrap(),
                JournalRecord::Outcome {
                    id,
                    outcome,
                    metadata,
                } => {
                    journal.record(id, *outcome, metadata.clone()).unwrap();
                }
                JournalRecord::Gap { .. } => panic!("unexpected recording gap"),
            }
        }
        let events = journal
            .query(&Query {
                limit: 1000,
                ..Default::default()
            })
            .unwrap();
        let exported = serde_json::to_string(&events).unwrap();
        for private in [PRIVATE, A, B] {
            assert!(!exported.contains(private));
        }
        assert!(events.iter().any(|e| e.kind.as_str() == "ui.menu"));
        drop(journal);
        std::fs::remove_dir_all(dir).unwrap();
    }
    assert!(recorded.requests().is_empty());
}

#[test]
fn direct_keys_and_unrelated_popup_actions_keep_working_after_link_changes() {
    let (mut host, recorded) = fixture(&format!("<{A}|first>"), None);
    refresh(&mut host, &format!("<{B}|second>"));
    to_text(&host, 160, 40);
    assert_eq!(targets(&press(&mut host, KeyCode::Char('y'))), [B]);
    popup(&mut host);
    refresh(&mut host, &format!("<{A}|first>"));
    // The reaction row is not the selected link's action.
    press(&mut host, KeyCode::Char('e'));
    assert!(host.menu.as_ref().unwrap().menu.id.starts_with("react:"));
    press(&mut host, KeyCode::Char('e'));
    let confirm = host.menu.as_ref().unwrap().menu.id.clone();
    refresh(&mut host, &format!("<{B}|second>"));
    assert_eq!(host.menu.as_ref().unwrap().menu.id, confirm);
    press(&mut host, KeyCode::Char('n'));
    assert!(host.menu.is_none());
    to_text(&host, 160, 40);
    popup(&mut host);
    refresh(&mut host, &format!("<{A}|first>"));
    press(&mut host, KeyCode::Char('c'));
    assert!(slack(&host).replying.is_some());
    press(&mut host, KeyCode::Char('x'));
    assert!(host.menu.is_none());
    assert_eq!(slack(&host).replying.as_ref().unwrap().composer.text(), "x");
    assert!(recorded.requests().is_empty());
}

#[test]
fn changing_a_link_does_not_disable_the_popup_refresh_or_back_rows() {
    for action in ["refresh", "back"] {
        let (mut host, recorded) = fixture(&format!("<{A}|first>"), None);
        popup(&mut host);
        refresh(&mut host, &format!("<{B}|second>"));
        match action {
            "refresh" => {
                let effects = press(&mut host, KeyCode::Char('r'));
                assert!(
                    effects
                        .iter()
                        .any(|e| matches!(e, HostEffect::Spawn { .. }))
                );
                assert!(host.menu.is_none());
            }
            "back" => {
                let open = host.menu.as_mut().unwrap();
                open.row = open
                    .menu
                    .items
                    .iter()
                    .position(|i| i.label.starts_with("back to "))
                    .unwrap();
                press(&mut host, KeyCode::Enter);
                assert!(slack(&host).pages.is_empty());
                assert!(host.menu.is_none());
            }
            _ => unreachable!(),
        }
        assert!(recorded.requests().is_empty());
    }
}
