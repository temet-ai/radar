//! The work bridge uses only ready owners and the existing neutral host route.
use super::*;
use crate::journal::{SubjectKey, SubjectRole, SubjectSnapshot};
use crate::source_context::{ResolvedSources, WorkReply};
use crate::work_api::{ItemsRequest, Request};
use crate::{Owned, Setup};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    unavailable: bool,
    inverse_kind: Option<String>,
    roundtrip_id: Option<String>,
    inverse_calls: usize,
    shown: Vec<Subject>,
    replies: Vec<ResolvedSources>,
}

struct Source {
    id: &'static str,
    kind: &'static str,
    state: Arc<Mutex<State>>,
}

fn key(id: &str) -> SubjectKey {
    SubjectKey::new("shelf", "https://example.test", "record", id).unwrap()
}

impl Plugin for Source {
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
        if self.state.lock().unwrap().unavailable {
            Setup::Needed("connect this source".into())
        } else {
            Setup::Ready
        }
    }
    fn owns(&self) -> Vec<Owned> {
        vec![Owned::link(self.kind, "records")]
    }
    fn journal_subject(&self, subject: &Subject) -> Option<SubjectSnapshot> {
        let state = self.state.lock().unwrap();
        let mut source = key(state.roundtrip_id.as_deref().unwrap_or(&subject.id));
        match subject.id.as_str() {
            "missing" => return None,
            "unsafe" => source.instance = "https://example.test?token=private".into(),
            "wrong-kind" => source.kind = "another-record".into(),
            _ => {}
        }
        Some(SubjectSnapshot::new(source, SubjectRole::Primary))
    }
    fn subject_from_journal(&self, key: &SubjectKey) -> Option<Subject> {
        let mut state = self.state.lock().unwrap();
        state.inverse_calls += 1;
        Some(Subject::new(
            state.inverse_kind.as_deref().unwrap_or("record"),
            &key.id,
            "PRIVATE_LABEL",
        ))
    }
    fn show(&mut self, subject: &Subject) -> Showing {
        self.state.lock().unwrap().shown.push(subject.clone());
        Showing::Done(vec![Effect::OpenApp {
            link: "shelf://record/7".into(),
            web: "https://example.test/record/7".into(),
        }])
    }
}

struct Receiver(Arc<Mutex<State>>);
impl Plugin for Receiver {
    fn id(&self) -> &'static str {
        "notes"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "notes",
            name: "Notes",
            key: 'N',
        }]
    }
    fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::ScreenView> {
        use crate::view as v;
        Some(v::ScreenView::new(
            scope,
            vec![v::PaneView::new(
                v::PaneId::Middle,
                "Notes",
                v::Node::Document {
                    lines: vec![],
                    cut: false,
                },
            )],
        ))
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &crate::view::Intent) -> Outcome {
        if matches!(intent, crate::view::Intent::Select { .. }) {
            Outcome::Handled(vec![Effect::ResolveSources {
                id: 19,
                subjects: vec![Subject::new("record", "7", "PRIVATE_LABEL")],
            }])
        } else {
            Outcome::Ignored
        }
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        match msg.downcast::<ResolvedSources>() {
            Ok(reply) => self.0.lock().unwrap().replies.push(*reply),
            Err(msg) => {
                assert!(msg.is::<WorkReply>());
            }
        }
        vec![]
    }
}

fn fixture() -> (Host, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    let mut host = Host::new(
        vec![
            Box::new(Receiver(state.clone())),
            Box::new(Source {
                id: "shelf",
                kind: "record",
                state: state.clone(),
            }),
        ],
        0,
    )
    .unwrap();
    host.load_config(Some("")).unwrap();
    (host, state)
}

#[test]
fn work_sources_are_owner_validated_deduplicated_and_bounded_without_inference() {
    let (mut host, state) = fixture();
    let mut source = Subject::new("record", "7", "PRIVATE_LABEL");
    source.related = vec![Subject::new("record", "unrequested", "do not infer")];
    let effects = host.route(
        0,
        vec![Effect::ResolveSources {
            id: 1,
            subjects: vec![
                source.clone(),
                source,
                Subject::new("record", "missing", "missing"),
                Subject::new("record", "unsafe", "unsafe"),
                Subject::new("record", "wrong-kind", "wrong"),
                Subject::new("unowned", "7", "unowned"),
            ],
        }],
        0,
    );
    assert!(effects.is_empty());
    {
        let state = state.lock().unwrap();
        let reply = &state.replies[0];
        assert_eq!(reply.id, 1);
        assert_eq!(reply.omitted, 4);
        assert!(reply.action_id.is_none());
        assert_eq!(reply.sources.len(), 1);
        assert_eq!(reply.sources[0].key, key("7"));
    }
    host.route(
        0,
        vec![Effect::ResolveSources {
            id: 2,
            subjects: (0..40)
                .map(|n| Subject::new("record", n.to_string(), "PRIVATE_LABEL"))
                .collect(),
        }],
        0,
    );
    let state = state.lock().unwrap();
    assert_eq!(state.replies[1].sources.len(), 32);
    assert_eq!(state.replies[1].omitted, 8);
}

#[test]
fn work_source_capture_keeps_the_headless_selection_action_without_extra_audit() {
    let (host, state) = fixture();
    let mut frontend = crate::headless::Headless::new(host, 80, 24);
    frontend.send(crate::view::Intent::Select {
        target: crate::view::Target::new("record:7"),
    });
    let state = state.lock().unwrap();
    let reply = &state.replies[0];
    let begins: Vec<_> = frontend
        .records()
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. } => Some((id, descriptor)),
            _ => None,
        })
        .collect();
    assert_eq!(begins.len(), 2);
    let (selection, descriptor) = begins.last().unwrap();
    assert_eq!(descriptor.kind.as_str(), "plugin.input");
    assert_eq!(reply.action_id.as_ref(), Some(*selection));
    assert_eq!(reply.sources[0].key, key("7"));
    assert!(!format!("{:?}", frontend.records()).contains("PRIVATE_LABEL"));
}

#[test]
fn work_show_source_roundtrips_with_exact_ready_owner_then_uses_native_route() {
    let (mut host, state) = fixture();
    let effects = host.route(0, vec![Effect::ShowSource(key("7"))], 0);
    assert!(matches!(&effects[..], [HostEffect::OpenApp { link, web }]
        if link == "shelf://record/7" && web == "https://example.test/record/7"));
    assert_eq!(state.lock().unwrap().shown[0].id, "7");
    for invalid in ["unavailable", "wrong-kind", "wrong-id"] {
        {
            let mut state = state.lock().unwrap();
            state.shown.clear();
            state.unavailable = invalid == "unavailable";
            state.inverse_kind = (invalid == "wrong-kind").then(|| "another-record".into());
            state.roundtrip_id = (invalid == "wrong-id").then(|| "8".into());
        }
        assert!(
            host.route(0, vec![Effect::ShowSource(key("7"))], 0)
                .is_empty()
        );
        assert!(state.lock().unwrap().shown.is_empty());
        assert!(host.message.as_deref().unwrap().contains("copy"));
        assert!(!host.message.as_deref().unwrap().contains("PRIVATE"));
    }
}

#[test]
fn work_show_source_never_asks_an_unrelated_inverse_hook() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut host = Host::new(
        vec![Box::new(Source {
            id: "unrelated",
            kind: "other",
            state: state.clone(),
        })],
        0,
    )
    .unwrap();
    assert!(
        host.route(0, vec![Effect::ShowSource(key("7"))], 0)
            .is_empty()
    );
    assert_eq!(state.lock().unwrap().inverse_calls, 0);
    assert!(host.message.as_deref().unwrap().contains("copy"));
}

#[test]
fn work_requests_capture_host_mode_and_do_not_self_audit_or_expose_payloads() {
    let (mut host, _) = fixture();
    host.enable_journal();
    host.set_env_mode(Some((crate::writes::Mode::ReadOnly, "read only".into())));
    let effects = host.route(
        0,
        vec![Effect::Work {
            id: 44,
            request: Box::new(Request::Items(ItemsRequest::default())),
        }],
        0,
    );
    assert!(matches!(
        &effects[..],
        [HostEffect::Work {
            plugin: 0,
            id: 44,
            mode: crate::writes::Mode::ReadOnly,
            ..
        }]
    ));
    assert!(host.trace.drain().is_empty());
    assert_eq!(format!("{effects:?}"), "[Work(..)]");
    assert_eq!(
        format!("{:?}", Effect::ShowSource(key("PRIVATE_ID"))),
        "ShowSource(..)"
    );
    assert_eq!(
        format!(
            "{:?}",
            Effect::ResolveSources {
                id: 1,
                subjects: vec![Subject::new("record", "PRIVATE_ID", "PRIVATE_LABEL")]
            }
        ),
        "ResolveSources(..)"
    );
}
