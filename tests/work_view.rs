//! Real owner composition over an isolated journal. Provider tasks are never run.
#![cfg(all(
    feature = "github",
    feature = "git",
    feature = "workbench",
    feature = "demo"
))]

use radar_core::headless::Headless;
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::work_map::{
    Basis, HandoffRecord, ItemFilter, ItemId, Mutation, RelationKind, Selection,
};
use radar_core::journal::{ActionId, Journal, SessionId};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::source_context::WorkReply;
use radar_core::view::{Intent, Node, PaneId};
use radar_core::{Host, Pane};
use serde_json::json;
use std::collections::VecDeque;
use std::path::PathBuf;

const TITLE: &str = "Ship explicit owner navigation";
const FOLDER: &str = "/work/source-api";

fn plugins() -> Host {
    let mut host = Host::new(
        vec![
            Box::<radar_github::GithubPlugin>::default(),
            Box::<radar_git::GitPlugin>::default(),
            Box::<radar_workbench::WorkbenchPlugin>::default(),
        ],
        radar_core::plugin::DEMO_NOW,
    )
    .unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0].demo();
    host.plugins[1].restore(json!({"index": {
        "checkouts": [{"dir": FOLDER, "remote": "git@github.com:alice/api.git", "project": "alice/api", "branch": "main", "files": [], "main": ""}],
        "at": radar_core::plugin::DEMO_NOW, "folders": 1, "cut": false, "missing": []
    }, "use": {}}));
    host
}

struct Fixture {
    frontend: Headless,
    journal: Journal,
    session: SessionId,
    root: PathBuf,
    terminal: bool,
    cache: Option<serde_json::Value>,
    opened: Vec<String>,
    dropped_reads: usize,
}
impl Fixture {
    fn new(terminal: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "radar-work-provider-view-{}",
            ActionId::new().as_str()
        ));
        let mut journal = Journal::open(&root.join("journal.sqlite3")).unwrap();
        let session = journal.start_session().unwrap();
        let frontend =
            Headless::requiring_views(plugins(), 160, 40).unwrap_or_else(|gaps| panic!("{gaps:?}"));
        Self {
            frontend,
            journal,
            session,
            root,
            terminal,
            cache: None,
            opened: vec![],
            dropped_reads: 0,
        }
    }
    fn pump(&mut self, effects: Vec<HostEffect>) {
        let mut queue: VecDeque<_> = effects.into();
        let mut steps = 0;
        while let Some(effect) = queue.pop_front() {
            steps += 1;
            assert!(steps < 500);
            match effect {
                HostEffect::Journal(record) => match record {
                    JournalRecord::Begin {
                        id,
                        descriptor,
                        parent,
                    } => self
                        .journal
                        .begin_with_id_at(&self.session, &id, descriptor, parent.as_ref(), 1000)
                        .unwrap(),
                    JournalRecord::Outcome {
                        id,
                        outcome,
                        metadata,
                    } => {
                        self.journal
                            .record_at(&id, outcome, metadata, 1000)
                            .unwrap();
                    }
                    JournalRecord::Gap { .. } => panic!("unexpected recording gap"),
                },
                HostEffect::Work {
                    plugin,
                    id,
                    request,
                    mode,
                } => {
                    let result = radar_core::work_api::test_support::execute(
                        &mut self.journal,
                        &self.session,
                        *request,
                        mode,
                        1000,
                    );
                    queue.extend(self.frontend.host_mut().update(Event::Msg {
                        plugin,
                        msg: Box::new(WorkReply { id, result }),
                    }));
                }
                HostEffect::Tracked { effect, .. } => queue.push_front(*effect),
                HostEffect::WriteCache {
                    plugin: "workbench",
                    value,
                } => self.cache = Some(value),
                HostEffect::WriteCache { .. } | HostEffect::Save(_) => {}
                HostEffect::Open(url) => self.opened.push(url),
                HostEffect::Spawn { plugin, .. } => {
                    assert!(matches!(
                        self.frontend.host().plugins[plugin].id(),
                        "github" | "git"
                    ));
                    // Opening an owner's native page may request a read. Do not
                    // execute it: only the isolated local work engine runs here.
                    self.dropped_reads += 1;
                }
                other => panic!("unexpected external effect: {other:?}"),
            }
        }
    }
    fn key(&mut self, key: KeyCode) {
        let _ = radar_core::shell::to_text(self.frontend.host(), 160, 40);
        let effects = self
            .frontend
            .host_mut()
            .update(Event::Key(KeyEvent::new(key, KeyModifiers::NONE)));
        self.pump(effects);
    }
    fn invoke(&mut self, action: &str) {
        let effects = self.frontend.send(Intent::Invoke {
            action: action.into(),
        });
        self.pump(effects);
    }
    fn action(&mut self, key: char, action: &str) {
        if self.terminal {
            self.key(KeyCode::Char(key));
        } else {
            self.invoke(action);
        }
    }
    fn choose(&mut self, value: &str) {
        if self.terminal {
            let menu = self.frontend.host_mut().menu.as_mut().unwrap();
            menu.row = menu
                .menu
                .items
                .iter()
                .position(|item| item.value == value && !item.note && !item.heading)
                .unwrap();
            self.key(KeyCode::Enter);
        } else {
            self.invoke(value);
        }
    }
    fn paste(&mut self, text: &str) {
        let effects = self.frontend.host_mut().update(Event::Paste(text.into()));
        self.pump(effects);
        self.key(KeyCode::Enter);
    }
    fn selected_task(&self) -> ItemId {
        serde_json::from_value(
            self.frontend.host().plugins[2].snapshot().unwrap()["selected"].clone(),
        )
        .unwrap()
    }
    fn open_source(&mut self, label: &str) {
        self.key(KeyCode::Char('I'));
        self.frontend.focus(PaneId::Detail);
        let screen = self.frontend.screen().unwrap();
        let Node::Entries { rows, .. } = &screen.pane(PaneId::Detail).unwrap().body else {
            panic!()
        };
        let target = rows
            .iter()
            .find(|row| row.head.words().contains(label))
            .unwrap()
            .target
            .clone();
        let effects = self.frontend.send(Intent::Select { target });
        self.pump(effects);
        self.action('o', "work.open-source");
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn actual_github_and_git_sources_complete_review_group_navigation_and_restart() {
    for terminal in [true, false] {
        let mut fixture = Fixture::new(terminal);
        fixture.key(KeyCode::Char('G'));
        fixture.frontend.focus(PaneId::Middle);
        assert_eq!(
            fixture.frontend.host().plugins[0]
                .subject("github", Pane::Middle)
                .unwrap()
                .id,
            "https://github.com/acme/api/pull/42"
        );
        fixture.key(KeyCode::Char('x'));
        let add = fixture
            .frontend
            .menu()
            .unwrap()
            .items
            .iter()
            .find(|item| item.label == "add to work item…")
            .unwrap()
            .value
            .clone();
        fixture.choose(&add);
        fixture.choose("new");
        fixture.paste(TITLE);
        let menu = fixture.frontend.menu().unwrap();
        assert_eq!(menu.id, "work.sources");
        let candidates: Vec<_> = menu
            .items
            .iter()
            .filter(|item| item.value.starts_with("toggle:"))
            .collect();
        assert_eq!(candidates.len(), 4, "{menu:?}");
        assert_eq!(
            candidates
                .iter()
                .filter(|item| item.label.starts_with('✓'))
                .count(),
            1
        );
        assert!(
            candidates
                .iter()
                .any(|item| item.label.contains("base repository"))
        );
        assert!(
            candidates
                .iter()
                .any(|item| item.label.contains("source repository"))
        );
        assert!(candidates.iter().any(|item| item.hint.contains(FOLDER)));
        let extra: Vec<_> = candidates
            .iter()
            .filter(|item| item.label.starts_with('○'))
            .map(|item| item.value.clone())
            .collect();
        for value in extra {
            fixture.choose(&value);
        }
        fixture.choose("review");
        let preview = fixture.frontend.menu().unwrap();
        assert_eq!(preview.id, "work.confirm");
        assert!(
            preview
                .items
                .iter()
                .any(|item| item.label.contains("References local") && item.label.contains(FOLDER))
        );
        assert!(
            fixture
                .journal
                .query_items(&ItemFilter::default(), 100, None)
                .unwrap()
                .items
                .is_empty()
        );
        fixture.choose("yes");
        let task = fixture.selected_task();
        assert!(radar_core::shell::to_text(fixture.frontend.host(), 160, 40).contains(TITLE));
        let evidence = fixture
            .journal
            .handoff(
                &Selection {
                    items: vec![task.clone()],
                    actions: vec![],
                },
                Default::default(),
                None,
            )
            .unwrap();
        let references: Vec<_> = evidence
            .records
            .iter()
            .filter_map(|record| match record {
                HandoffRecord::Map { change } => match &change.request.mutation {
                    Mutation::RelateItems {
                        from,
                        to,
                        kind: RelationKind::References,
                    } => {
                        assert_eq!(from, &task);
                        assert_ne!(to, &task);
                        assert_eq!(change.request.provenance.basis, Basis::UserAsserted);
                        Some(to.clone())
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(references.len(), 4);
        assert_eq!(
            fixture
                .journal
                .query_items(&ItemFilter::default(), 100, None)
                .unwrap()
                .items
                .len(),
            5
        );
        fixture.action('t', "work.group");
        fixture.choose("new");
        fixture.paste("release candidate");
        fixture.choose("yes");
        assert!(
            serde_json::to_string(&fixture.frontend.screen().unwrap())
                .unwrap()
                .contains("release candidate")
        );
        fixture.open_source("pull-request");
        assert_eq!(fixture.frontend.host().screen().unwrap().1.id, "github");
        fixture.open_source("checkout");
        assert_eq!(
            fixture.frontend.host().plugins[fixture.frontend.host().screen().unwrap().0].id(),
            "git"
        );
        fixture.open_source("repository · alice/api");
        assert_eq!(
            fixture.opened.last().map(String::as_str),
            Some("https://github.com/alice/api")
        );
        let saved = fixture.cache.clone().unwrap();
        let mut restarted = plugins();
        restarted.plugins[2].restore(saved);
        fixture.frontend =
            Headless::requiring_views(restarted, 160, 40).unwrap_or_else(|gaps| panic!("{gaps:?}"));
        fixture.key(KeyCode::Char('I'));
        fixture.action('r', "work.refresh");
        assert_eq!(fixture.selected_task(), task);
        let view = fixture.frontend.screen().unwrap();
        let Node::Entries { rows, .. } = &view.pane(PaneId::Detail).unwrap().body else {
            panic!()
        };
        assert_eq!(rows.len(), 4);
        let encoded = serde_json::to_string(&view).unwrap();
        assert!(encoded.contains(TITLE) && encoded.contains("release candidate"));
    }
}

#[test]
fn work_screen_coexists_with_the_default_repository_and_workspace_groups() {
    let host = Host::new(radar::parts(), radar_core::plugin::DEMO_NOW)
        .expect("default parts have unique screen keys");
    assert!(
        host.screens
            .iter()
            .any(|(_, screen)| screen.id == "work" && screen.key == 'I')
    );
}
