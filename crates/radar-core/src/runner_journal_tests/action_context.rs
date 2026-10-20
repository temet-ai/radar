//! Supplied references stay distinct from claims about what an action used.
use super::*;
use crate::compose::{Action, Contribution, Owned, Point, Setup, Subject};
use crate::journal::{SafeUrl, SubjectKey, SubjectRole, SubjectSnapshot};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Calls {
    related: Vec<String>,
    snapshots: Vec<String>,
    received: Vec<(Subject, Vec<Subject>)>,
}

struct Part {
    id: &'static str,
    kinds: Vec<&'static str>,
    primary: Option<Subject>,
    references: Vec<Subject>,
    ready: bool,
    calls: Arc<Mutex<Calls>>,
}

impl Plugin for Part {
    fn id(&self) -> &'static str {
        self.id
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn set_up(&self) -> Setup {
        if self.ready {
            Setup::Ready
        } else {
            Setup::Needed("Configure a source".into())
        }
    }
    fn owns(&self) -> Vec<Owned> {
        self.kinds
            .iter()
            .map(|kind| Owned::link(kind, "items"))
            .collect()
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        if self.primary.is_some() {
            vec![ScreenSpec {
                id: "main",
                name: "items",
                key: 'A',
            }]
        } else {
            vec![]
        }
    }
    fn subject(&self, _: &str, _: Pane) -> Option<Subject> {
        self.primary.clone()
    }
    fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::ScreenView> {
        use crate::view as v;
        Some(v::ScreenView::new(scope, vec![v::PaneView::new(
            v::PaneId::Middle, "items", v::Node::Document { lines: vec![], cut: false },
        )]))
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &crate::view::Intent) -> Outcome {
        if matches!(intent, crate::view::Intent::Key { chord } if chord.key == crate::view::Key::Char('d')) {
            Outcome::Handled(vec![Effect::Act {
                part: "receiver",
                subject: self.primary.clone().unwrap(),
                id: "SECRET action".into(),
            }])
        } else {
            Outcome::Ignored
        }
    }
    fn contribute(&self, point: Point, _: &Subject) -> Option<Contribution> {
        (self.id == "receiver" && point == Point::Actions).then(|| {
            Contribution::Actions(vec![Action::new(
                'a',
                "SECRET action label",
                "SECRET action",
            )])
        })
    }
    fn related(&self, subject: &Subject) -> Vec<Subject> {
        assert!(self.ready);
        assert_ne!(self.id, "receiver", "the receiving part is excluded");
        assert_eq!(
            subject.kind, "book",
            "nested references must never be queried"
        );
        let mut calls = self.calls.lock().unwrap();
        let again = calls.related.iter().any(|id| id == self.id);
        calls.related.push(self.id.into());
        if again {
            vec![reference("wrong-second-resolution")]
        } else {
            self.references.clone()
        }
    }
    fn journal_subject(&self, subject: &Subject) -> Option<SubjectSnapshot> {
        assert!(self.ready);
        assert!(self.kinds.contains(&subject.kind.as_str()));
        self.calls.lock().unwrap().snapshots.push(self.id.into());
        if subject.id == "unsupported" || subject.id == "bad-input" {
            return None;
        }
        let mut key = SubjectKey::new(
            subject.field("provider").unwrap_or("catalog"),
            subject.field("scope").unwrap_or("one"),
            &subject.kind,
            &subject.id,
        )
        .ok()?;
        match subject.id.as_str() {
            "bad-key" => key.id = "SECRET raw value".into(),
            "bad-kind" => key.kind = "other".into(),
            _ => {}
        }
        let url = if subject.id == "bad-url" {
            serde_json::from_str("\"https://identity.example.test/reset?token=SECRET\"").unwrap()
        } else {
            SafeUrl::parse(&format!(
                "https://identity.example.test/items/{}",
                subject.id
            ))
            .unwrap()
        };
        // The host must assign the invocation's role, regardless of this hint.
        Some(SubjectSnapshot::new(key, SubjectRole::Result).with_url(url))
    }
    fn act_with_related(&mut self, subject: &Subject, _: &str, related: &[Subject]) -> Vec<Effect> {
        assert_eq!(self.id, "receiver");
        self.calls
            .lock()
            .unwrap()
            .received
            .push((subject.clone(), related.to_vec()));
        vec![]
    }
}

fn reference(id: &str) -> Subject {
    Subject::new("note", id, "SECRET label")
        .with("body", "SECRET field")
        .with_url("https://untrusted.example.test/SECRET?token=SECRET#SECRET")
}

fn fixture(existing: Vec<Subject>, discovered: Vec<Subject>) -> (Host, Arc<Mutex<Calls>>) {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let mut primary = Subject::new("book", "1", "SECRET primary").with("body", "SECRET body");
    primary.related = existing;
    let mut owner = Part {
        id: "owner",
        kinds: vec!["book"],
        primary: Some(primary),
        references: vec![],
        ready: true,
        calls: calls.clone(),
    };
    owner.references = discovered;
    let make = |id, kinds, ready| {
        Box::new(Part {
            id,
            kinds,
            primary: None,
            references: vec![],
            ready,
            calls: calls.clone(),
        }) as Box<dyn Plugin>
    };
    let mut host = Host::new(
        vec![
            Box::new(owner),
            make("notes", vec!["note"], true),
            make("later-owner", vec!["note"], true),
            make("unconfigured", vec!["hidden"], false),
            make("receiver", vec![], true),
        ],
        0,
    )
    .unwrap();
    host.load_config(Some("")).unwrap();
    host.enable_journal();
    (host, calls)
}

fn descriptor(effects: &[HostEffect]) -> ActionDescriptor {
    records(effects)
        .into_iter()
        .find_map(|record| match record {
            JournalRecord::Begin { descriptor, .. }
                if descriptor.kind.as_str() == "plugin.action" =>
            {
                Some(descriptor)
            }
            _ => None,
        })
        .unwrap()
}

fn assert_subject_input(actual: &Subject, expected: &Subject) {
    assert_eq!(actual, expected);
    assert_eq!(actual.label, expected.label);
    assert_eq!(actual.fields, expected.fields);
    assert_eq!(actual.url, expected.url);
    assert_eq!(actual.related.len(), expected.related.len());
    for (actual, expected) in actual.related.iter().zip(&expected.related) {
        assert_subject_input(actual, expected);
    }
}

#[test]
fn prepared_context_resolves_once_and_preserves_exact_delivered_input() {
    let first = reference("a").with("extra", "SECRET first");
    let duplicate = reference("a").with("extra", "SECRET second");
    let self_reference = Subject::new("book", "1", "SECRET self");
    let mut nested = reference("b");
    nested.related = vec![reference("not-top-level")];
    let existing = vec![first.clone(), duplicate.clone(), self_reference.clone()];
    let discovered = vec![
        reference("a"),
        nested.clone(),
        reference("b"),
        self_reference.clone(),
    ];
    let (mut host, calls) = fixture(existing.clone(), discovered);
    let effects = host.update(key(Key::Char('d')));
    let captured = descriptor(&effects);
    let calls = calls.lock().unwrap();
    assert_eq!(calls.related, ["owner", "notes", "later-owner"]);
    assert_eq!(calls.received.len(), 1);
    let expected = [first, duplicate, self_reference, nested];
    for delivered in [&calls.received[0].0.related, &calls.received[0].1] {
        assert_eq!(delivered.len(), expected.len());
        for (actual, expected) in delivered.iter().zip(&expected) {
            assert_subject_input(actual, expected);
        }
    }
    assert!(
        !calls
            .snapshots
            .iter()
            .any(|id| id == "later-owner" || id == "receiver")
    );
    assert_eq!(
        captured
            .subjects
            .iter()
            .map(|s| (s.key.kind.as_str(), s.key.id.as_str(), s.role))
            .collect::<Vec<_>>(),
        [
            ("book", "1", SubjectRole::Primary),
            ("note", "a", SubjectRole::Context),
            ("note", "b", SubjectRole::Context)
        ]
    );
    assert_eq!(captured.metadata.omitted_subjects, 0);
    assert!(!format!("{:?}", records(&effects)).contains("SECRET"));
}

#[test]
fn context_dedup_uses_full_scoped_identity_without_changing_delivery() {
    let one = reference("a").with("scope", "one");
    let two = reference("a").with("scope", "two");
    let other = reference("a").with("provider", "secondary");
    let (mut host, calls) = fixture(vec![one.clone(), two, one, other], vec![]);
    let captured = descriptor(&host.update(key(Key::Char('d'))));
    assert_eq!(captured.subjects.len(), 4);
    assert_eq!(captured.subjects[1].key.instance, "one");
    assert_eq!(captured.subjects[2].key.instance, "two");
    assert_eq!(captured.subjects[3].key.provider, "secondary");
    assert_eq!(calls.lock().unwrap().received[0].1.len(), 4);
}

#[test]
fn unsupported_and_invalid_owner_snapshots_are_omitted_without_fallback() {
    let mut subjects = ["unsupported", "bad-key", "bad-kind", "bad-url", "bad-input"]
        .map(reference)
        .to_vec();
    subjects.extend([
        Subject::new("missing", "a", "SECRET"),
        Subject::new("hidden", "a", "SECRET"),
        reference("safe"),
    ]);
    let (mut host, calls) = fixture(subjects, vec![]);
    let effects = host.update(key(Key::Char('d')));
    let captured = descriptor(&effects);
    assert_eq!(captured.subjects.len(), 2);
    assert_eq!(captured.subjects[1].key.id, "safe");
    assert_eq!(captured.metadata.omitted_subjects, 7);
    assert!(captured.metadata.redacted);
    assert!(!captured.metadata.truncated);
    assert_eq!(calls.lock().unwrap().received[0].1.len(), 8);
    assert!(
        !calls
            .lock()
            .unwrap()
            .snapshots
            .iter()
            .any(|id| id == "later-owner" || id == "unconfigured")
    );
    assert!(!format!("{:?}", records(&effects)).contains("SECRET"));
}

#[test]
fn context_bound_counts_unrepresented_occurrences_without_capping_delivery() {
    for (count, omitted, truncated) in [(30, 0, false), (31, 0, false), (32, 1, true)] {
        let subjects = (0..count).map(|i| reference(&format!("n{i}"))).collect();
        let (mut host, calls) = fixture(subjects, vec![]);
        let captured = descriptor(&host.update(key(Key::Char('d'))));
        assert_eq!(captured.subjects.len(), (count + 1).min(32));
        assert_eq!(captured.metadata.omitted_subjects, omitted);
        assert_eq!(captured.metadata.truncated, truncated);
        assert!(!captured.metadata.redacted);
        assert_eq!(calls.lock().unwrap().received[0].1.len(), count);
    }
    let mut subjects: Vec<_> = (0..31).map(|i| reference(&format!("n{i}"))).collect();
    let mut duplicates = subjects.clone();
    duplicates.extend([reference("n0"), reference("n30")]);
    let (mut host, _) = fixture(duplicates, vec![]);
    let captured = descriptor(&host.update(key(Key::Char('d'))));
    assert_eq!(captured.subjects.len(), 32);
    assert_eq!(captured.metadata.omitted_subjects, 0);
    assert!(!captured.metadata.truncated);
    subjects.extend([
        reference("n0"),
        reference("overflow"),
        reference("overflow"),
        reference("unsupported"),
    ]);
    let (mut host, _) = fixture(subjects, vec![]);
    let captured = descriptor(&host.update(key(Key::Char('d'))));
    assert_eq!(captured.subjects.len(), 32);
    assert_eq!(captured.metadata.omitted_subjects, 3);
    assert!(captured.metadata.truncated && captured.metadata.redacted);
}

#[test]
fn missing_primary_leaves_all_slots_for_valid_context() {
    let subjects = (0..32).map(|i| reference(&format!("n{i}"))).collect();
    let (mut host, _) = fixture(subjects, vec![]);
    // The kind has an owner, but that owner explicitly declines this locator.
    let input = host.plugins[0].subject("main", Pane::Nav).unwrap();
    let calls = Arc::new(Mutex::new(Calls::default()));
    let mut invalid = input;
    invalid.id = "unsupported".into();
    host.plugins[0] = Box::new(Part {
        id: "owner",
        kinds: vec!["book"],
        primary: Some(invalid),
        references: vec![],
        ready: true,
        calls,
    });
    let captured = descriptor(&host.update(key(Key::Char('d'))));
    assert_eq!(captured.subjects.len(), 32);
    assert!(
        captured
            .subjects
            .iter()
            .all(|s| s.role == SubjectRole::Context)
    );
    assert_eq!(captured.metadata.omitted_subjects, 1);
    assert!(captured.metadata.redacted);
    assert!(!captured.metadata.truncated);
}

#[test]
fn contributed_key_and_menu_capture_the_same_context() {
    for menu in [false, true] {
        let (mut host, calls) = fixture(vec![], vec![reference("a")]);
        let effects = if menu {
            host.update(key(Key::Char('x')));
            let state = host.menu.as_mut().unwrap();
            state.row = state
                .menu
                .items
                .iter()
                .position(|item| item.label == "SECRET action label")
                .unwrap();
            host.update(key(Key::Enter))
        } else {
            host.update(key(Key::Char('a')))
        };
        let captured = descriptor(&effects);
        assert_eq!(captured.subjects.len(), 2);
        assert_eq!(captured.subjects[1].role, SubjectRole::Context);
        assert_eq!(calls.lock().unwrap().received.len(), 1);
    }
}

#[test]
fn public_act_wrapper_keeps_legacy_resolution_and_dispatch_contract() {
    let (mut host, calls) = fixture(vec![reference("a")], vec![reference("b")]);
    let input = host.plugins[0].subject("main", Pane::Nav).unwrap();
    crate::compose::act(&mut host.plugins, 4, &input, "read");
    let calls = calls.lock().unwrap();
    assert_eq!(calls.related, ["owner", "notes", "later-owner"]);
    assert_eq!(
        calls.received[0]
            .1
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert!(calls.snapshots.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn supplied_context_round_trips_without_creating_map_facts() {
    use crate::journal::work_map::{EventFilter, HandoffRecord, Limits, Selection};
    let (mut host, _) = fixture(vec![reference("a"), reference("bad-url")], vec![]);
    let temp = TestDir::new();
    let mut recording = Recording::open_at(&temp.path(), SessionOrigin::Tui).unwrap();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    for effect in host.update(key(Key::Char('d'))) {
        perform(effect, &tx, Some(&recording.sink));
    }
    recording.finish();
    assert_eq!(recording.sink.take_warning(), None);
    let journal = Journal::open(&temp.path()).unwrap();
    let snapshot = journal.work_map_snapshot().unwrap();
    assert_eq!(snapshot.map_sequence(), 0);
    let subject = SubjectKey::new("catalog", "one", "note", "a").unwrap();
    let page = journal
        .query_events(
            &EventFilter {
                subject: Some(subject.clone()),
                ..Default::default()
            },
            Limits::default(),
            None,
        )
        .unwrap();
    assert!(!page.events.is_empty());
    let action = page.events[0].event.action_id.clone();
    let handoff = journal
        .handoff(
            &Selection {
                items: vec![],
                actions: vec![action],
            },
            Limits::default(),
            None,
        )
        .unwrap();
    assert!(
        handoff
            .records
            .iter()
            .all(|record| !matches!(record, HandoffRecord::Map { .. }))
    );
    let events: Vec<_> = handoff
        .records
        .iter()
        .filter_map(|record| match record {
            HandoffRecord::Action { event, .. } => Some(event),
            _ => None,
        })
        .collect();
    assert!(events.iter().all(|event| {
        event
            .subjects
            .iter()
            .any(|s| s.key == subject && s.role == SubjectRole::Context)
    }));
    let requested = events
        .iter()
        .find(|event| event.outcome == JournalOutcome::Requested)
        .unwrap();
    assert_eq!(requested.metadata.omitted_subjects, 1);
    assert!(requested.metadata.redacted);
    assert!(!serde_json::to_string(&handoff).unwrap().contains("SECRET"));
    assert_eq!(journal.work_map_snapshot().unwrap(), snapshot);
}
