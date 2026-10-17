//! A real scratch checkout linked through Work's reviewed, durable local flow.
//! Git subprocesses belong to the owner-local isolated fixture; the host and
//! SQLite completions are production paths, with no fabricated replies.
#![cfg(all(feature = "git", feature = "workbench"))]

use radar_core::host::Event;
use radar_core::journal::work_map::{
    Basis, HandoffRecord, HistoryAssociation, ItemFilter, Mutation, RelationKind, Selection,
    TagTarget,
};
use radar_core::journal::{ActionEvent, ActionId, Journal, Outcome, Query, SubjectKey};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::scenario::{JournalRecord, Scenario};
use radar_core::view::{Intent, Node, PaneId, ScreenView, Target, Tree, TreeRow};
use radar_core::writes::Mode;
use radar_core::{Host, Pane};
use radar_git::test_support::{GitCommandPhase, local_checkout_fixture};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

const NOW: i64 = 1_791_271_000;
const TITLE: &str = "Review Watch patrol notes";
const BUDGET: usize = 128;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("radar-local-scenario-{}", ActionId::new().as_str()));
        std::fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn journal(&self) -> PathBuf {
        self.0.join("actions.sqlite3")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Resolve once without a shell, a helper subprocess, or global environment
/// changes. Relative PATH entries cannot redirect this test into its checkout.
fn installed_git() -> &'static Path {
    static GIT: OnceLock<PathBuf> = OnceLock::new();
    GIT.get_or_init(|| {
        let path = std::env::var_os("PATH").expect("installed Git requires PATH");
        std::env::split_paths(&path)
            .filter(|dir| dir.is_absolute())
            .filter_map(|dir| {
                let candidate = dir.join(if cfg!(windows) { "git.exe" } else { "git" });
                let resolved = candidate.canonicalize().ok()?;
                let metadata = std::fs::metadata(&resolved).ok()?;
                if !metadata.is_file() {
                    return None;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if metadata.permissions().mode() & 0o111 == 0 {
                        return None;
                    }
                }
                Some(resolved)
            })
            .next()
            .expect("the local Git scenario requires an installed Git executable")
    })
}

fn key(s: &mut Scenario, key: KeyCode) {
    s.dispatch(Event::Key(KeyEvent::new(key, KeyModifiers::NONE)))
        .unwrap();
}

fn invoke(s: &mut Scenario, action: &str) {
    s.dispatch(Event::Intent(Intent::Invoke {
        action: action.into(),
    }))
    .unwrap();
}

fn choose(s: &mut Scenario, menu: &str, value: &str) {
    let open = &s.host().menu.as_ref().expect("visible menu").menu;
    assert_eq!(open.id, menu);
    assert!(
        open.items
            .iter()
            .any(|item| { !item.heading && !item.note && item.value == value }),
        "{menu} offers {value}"
    );
    invoke(s, value);
}

fn select(s: &mut Scenario, target: Target) {
    s.dispatch(Event::Intent(Intent::Select { target }))
        .unwrap();
}

async fn settle(s: &mut Scenario) {
    s.settle_io(BUDGET, IO_TIMEOUT).await.unwrap();
}

fn view(s: &Scenario, plugin: usize, screen: &str) -> ScreenView {
    s.inspect(|host| host.screen_view_in(plugin, screen, 120, 40).unwrap())
}

fn branches(s: &Scenario) -> Tree {
    let screen = view(s, 0, "branch-screen");
    let Node::Tree(tree) = &screen.pane(PaneId::Middle).unwrap().body else {
        panic!("Branches exposes its actual checkout tree");
    };
    assert_eq!(
        tree.rows.len(),
        1,
        "only the scoped working clone is indexed"
    );
    assert!(tree.rows[0].name.words().contains("main"));
    let dirty = tree.rows[0]
        .cells
        .iter()
        .find(|cell| cell.heading == "dirty")
        .unwrap();
    assert!(dirty.text.words().contains("1 changed"), "{dirty:?}");
    tree.clone()
}

fn events(journal: &Journal) -> Vec<ActionEvent> {
    let events = journal
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    assert!(
        events.len() < 1000,
        "the bounded evidence query is complete"
    );
    events
}

fn succeeded(events: &[ActionEvent], action: &ActionId) {
    assert_eq!(
        events
            .iter()
            .filter(|event| &event.action_id == action && event.outcome.is_terminal())
            .map(|event| event.outcome)
            .collect::<Vec<_>>(),
        [Outcome::Succeeded],
        "one real successful terminal result for {action:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn real_checkout_links_to_reviewed_work_and_reopens_exact_source() {
    let scratch = Scratch::new();
    let (git, fixture) = local_checkout_fixture(&scratch.0, installed_git()).unwrap();
    let root = fixture.root().to_str().unwrap().to_owned();
    assert_eq!(fixture.root(), fixture.root().canonicalize().unwrap());
    let expected_source = SubjectKey::new("local", &root, "checkout", &root).unwrap();
    let setup_commands = fixture.commands().len();
    assert!(setup_commands > 0);

    let mut host = Host::new(
        vec![
            Box::new(git),
            Box::<radar_workbench::WorkbenchPlugin>::default(),
        ],
        NOW,
    )
    .unwrap();
    host.load_config(Some("[theme]\nicons = 'off'\n")).unwrap();
    host.plugins[0].configure(Some(&fixture.config())).unwrap();
    host.set_env_mode(Some((Mode::Write, "RADAR_MODE".into())));
    host.focus = Pane::Middle;
    let mut s = Scenario::with_journal(host, &scratch.journal()).unwrap();

    // Let actual checkout discovery and the initial Work queries complete before
    // asking Branches to read. Its initial view may predate discovery.
    s.advance_time(NOW).unwrap();
    settle(&mut s).await;
    key(&mut s, KeyCode::Char('B'));
    let _ = view(&s, 0, "branch-screen");
    invoke(&mut s, "git.branch.read_again");
    settle(&mut s).await;
    let tree = branches(&s);
    let checkout_target = TreeRow::target_of(&[tree.rows[0].id.clone()]);
    select(&mut s, Target::new("pane:middle"));
    select(&mut s, checkout_target.clone());
    let selected = s.inspect(|host| {
        let subject = host.plugins[0]
            .subject("branch-screen", Pane::Middle)
            .unwrap();
        host.plugins[0].journal_subject(&subject).unwrap().key
    });
    assert_eq!(selected, expected_source);
    let first_read = s
        .records()
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. }
                if descriptor.kind.as_str() == "git.branches.read" =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .expect("actual branch read was dispatched");

    key(&mut s, KeyCode::Char('x'));
    let add = s
        .host()
        .menu
        .as_ref()
        .unwrap()
        .menu
        .items
        .iter()
        .find(|item| item.label == "add to work item…")
        .expect("Work contributes to the selected real checkout")
        .value
        .clone();
    invoke(&mut s, &add);
    settle(&mut s).await;
    choose(&mut s, "work.choose", "new");
    s.dispatch(Event::Paste(TITLE.into())).unwrap();
    key(&mut s, KeyCode::Enter);
    let sources = &s.host().menu.as_ref().unwrap().menu;
    assert_eq!(sources.id, "work.sources");
    let candidates: Vec<_> = sources
        .items
        .iter()
        .filter(|item| item.value.starts_with("toggle:"))
        .collect();
    assert_eq!(candidates.len(), 1, "{sources:?}");
    assert!(candidates[0].label.starts_with("✓ checkout · "));
    assert_eq!(candidates[0].hint, format!("local · {root} · {root}"));
    choose(&mut s, "work.sources", "review");
    settle(&mut s).await;
    let confirm = &s.host().menu.as_ref().unwrap().menu;
    assert_eq!(confirm.id, "work.confirm");
    assert!(
        confirm.items.iter().any(|item| {
            item.label == format!("References local · {root} · checkout · {root}")
        })
    );
    {
        let journal = Journal::open(&scratch.journal()).unwrap();
        assert!(
            journal
                .query_items(&ItemFilter::default(), 100, None)
                .unwrap()
                .items
                .is_empty()
        );
        assert!(
            !events(&journal)
                .iter()
                .any(|event| event.kind.as_str() == "work.apply"),
            "review alone must not apply a batch"
        );
    }

    choose(&mut s, "work.confirm", "yes");
    settle(&mut s).await;
    assert_eq!(s.host().screen().unwrap().1.id, "work");
    let work = view(&s, 1, "work");
    let Node::List { rows, selected, .. } = &work.pane(PaneId::Middle).unwrap().body else {
        panic!("Work contains the committed items");
    };
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[selected.unwrap()].text.words(), TITLE);
    let Node::Entries { rows: sources, .. } = &work.pane(PaneId::Detail).unwrap().body else {
        panic!("the task exposes its persisted source binding");
    };
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].head.words(), format!("checkout · {root}"));
    select(&mut s, Target::new("pane:detail"));
    select(&mut s, sources[0].target.clone());
    invoke(&mut s, "work.open-source");
    settle(&mut s).await;
    assert_eq!(s.host().screen().unwrap().1.id, "branch-screen");
    assert_eq!(branches(&s).selected, TreeRow::path_of(&checkout_target));
    assert_eq!(
        s.inspect(|host| {
            host.plugins[0]
                .subject("branch-screen", Pane::Middle)
                .unwrap()
                .id
        }),
        root
    );

    // The binding begins at the link selection. A later real read, rather than
    // the earlier discovery, supplies evidence inside that explicit interval.
    invoke(&mut s, "git.branch.read_again");
    settle(&mut s).await;
    let _ = branches(&s);
    assert!(s.desktop_effects().is_empty());
    assert!(s.is_idle());
    assert_eq!(
        s.costs().shell_frames,
        0,
        "neutral-view checkpoint: no owned-host Ratatui frames"
    );
    s.finish_journal().unwrap();
    fixture.assert_finished();
    fixture.assert_unchanged();

    let commands = fixture.commands();
    assert!(commands.len() > setup_commands);
    assert!(
        commands[..setup_commands]
            .iter()
            .all(|command| command.phase == GitCommandPhase::Setup)
    );
    assert!(
        commands[setup_commands..]
            .iter()
            .all(|command| command.phase == GitCommandPhase::Read)
    );
    assert!(
        commands
            .iter()
            .all(|command| command.completed && command.success.is_some())
    );
    assert!(commands[setup_commands..].iter().any(|command| {
        command.dir == fixture.root() && command.args == ["status", "--porcelain=2", "--branch"]
    }));
    assert!(fixture.violations().is_empty());

    let journal = Journal::open(&scratch.journal()).unwrap();
    let items = journal
        .query_items(&ItemFilter::default(), 100, None)
        .unwrap();
    assert!(!items.truncated && items.next.is_none());
    assert_eq!(items.items.len(), 2);
    let task = &items
        .items
        .iter()
        .find(|item| item.kind.as_str() == "task")
        .unwrap()
        .item_id;
    let checkout = &items
        .items
        .iter()
        .find(|item| item.kind.as_str() == "checkout")
        .unwrap()
        .item_id;
    let events = events(&journal);
    let applies: Vec<_> = events
        .iter()
        .filter(|event| event.kind.as_str() == "work.apply" && event.outcome == Outcome::Requested)
        .collect();
    assert_eq!(
        applies.len(),
        1,
        "one reviewed batch, never a duplicate mutation"
    );
    succeeded(&events, &applies[0].action_id);

    let authored = journal
        .handoff(
            &Selection {
                items: vec![task.clone(), checkout.clone()],
                actions: vec![],
            },
            Default::default(),
            None,
        )
        .unwrap();
    assert!(!authored.truncated && authored.next.is_none() && !authored.known_recording_gaps);
    let changes: Vec<_> = authored
        .records
        .iter()
        .filter_map(|record| match record {
            HandoffRecord::Map { change } => Some(change.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(changes.len(), 6);
    for change in &changes {
        assert!(!change.retracted);
        assert_eq!(change.request.provenance.basis, Basis::UserAsserted);
        assert_eq!(
            change.request.provenance.recording_action_id,
            applies[0].action_id
        );
        assert_eq!(change.evidence.action.action_id, applies[0].action_id);
        assert_eq!(change.evidence.outcome, Outcome::Requested);
    }
    assert!(matches!(&changes[0].request.mutation,
        Mutation::CreateItem { item_id, kind } if item_id == task && kind.as_str() == "task"));
    let tag = match &changes[1].request.mutation {
        Mutation::CreateTag {
            tag_id,
            namespace,
            key,
            label,
        } => {
            assert_eq!(namespace.as_str(), "work.title");
            assert_eq!(key.as_str(), task.as_str());
            assert_eq!(label.as_str(), TITLE);
            tag_id
        }
        other => panic!("expected authored title, got {other:?}"),
    };
    assert!(matches!(&changes[2].request.mutation,
        Mutation::AssignTag { tag_id, target: TagTarget::Item(item_id) }
            if tag_id == tag && item_id == task));
    assert!(matches!(&changes[3].request.mutation,
        Mutation::CreateItem { item_id, kind } if item_id == checkout && kind.as_str() == "checkout"));
    let binding_start = match &changes[4].request.mutation {
        Mutation::BindSubject {
            item_id,
            subject,
            from_sequence,
            through_sequence,
        } => {
            assert_eq!(item_id, checkout);
            assert_eq!(subject, &expected_source);
            assert_eq!(*through_sequence, None);
            *from_sequence
        }
        other => panic!("expected exact source binding, got {other:?}"),
    };
    assert!(matches!(&changes[5].request.mutation,
        Mutation::RelateItems { from, to, kind: RelationKind::References }
            if from == task && to == checkout));

    let reads: Vec<_> = events
        .iter()
        .filter(|event| {
            event.kind.as_str() == "git.branches.read" && event.outcome == Outcome::Requested
        })
        .collect();
    assert_eq!(
        reads.len(),
        2,
        "the two explicit refreshes complete real reads"
    );
    assert_eq!(reads[0].action_id, first_read);
    assert!(reads[0].sequence < binding_start && reads[1].sequence >= binding_start);
    for read in &reads {
        assert_eq!(read.subjects.len(), 1);
        assert_eq!(read.subjects[0].key, expected_source);
        succeeded(&events, &read.action_id);
        let tasks: Vec<_> = events
            .iter()
            .filter(|event| {
                event.kind.as_str() == "runtime.task"
                    && event.outcome == Outcome::Requested
                    && event.parent_action_id.as_ref() == Some(&read.action_id)
            })
            .collect();
        assert_eq!(tasks.len(), 1, "each read owns its actual runtime task");
        succeeded(&events, &tasks[0].action_id);
    }
    for (kind, count) in [
        ("git.branches.read.branches", 1),
        ("git.branches.read.worktrees", 1),
        ("git.branches.read.stashes", 0),
    ] {
        let observations: Vec<_> = events
            .iter()
            .filter(|event| event.kind.as_str() == kind && event.outcome == Outcome::Requested)
            .collect();
        assert_eq!(observations.len(), 2);
        for observed in observations {
            assert_eq!(observed.metadata.count, Some(count));
            assert!(!observed.metadata.truncated);
            succeeded(&events, &observed.action_id);
        }
    }
    let evidence = journal
        .handoff(
            &Selection {
                items: vec![checkout.clone()],
                actions: vec![],
            },
            Default::default(),
            None,
        )
        .unwrap();
    assert!(!evidence.truncated && evidence.next.is_none() && !evidence.known_recording_gaps);
    assert_eq!(evidence.authority, "observations_only");
    let bound_reads: Vec<_> = evidence
        .records
        .iter()
        .filter_map(|record| match record {
            HandoffRecord::Action {
                event, association, ..
            } if event.kind.as_str() == "git.branches.read" => {
                assert_eq!(*association, HistoryAssociation::ExplicitBinding);
                Some(event.as_ref())
            }
            _ => None,
        })
        .collect();
    assert!(!bound_reads.is_empty());
    assert!(
        bound_reads
            .iter()
            .all(|event| event.action_id == reads[1].action_id)
    );
    assert!(
        bound_reads
            .iter()
            .any(|event| event.outcome == Outcome::Succeeded)
    );
}
