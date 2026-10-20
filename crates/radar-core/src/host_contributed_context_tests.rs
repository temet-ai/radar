//! Opted-in owners keep contributed popup actions tied to their current subject.
use super::*;
use crate::{Action, Setup};
use std::sync::{Arc, Mutex};

struct State {
    subject: Option<Subject>,
    generation: u32,
    guarded: bool,
    typing: bool,
    ready: bool,
    offered: bool,
    calls: Vec<Subject>,
}

struct OwnerProbe(Arc<Mutex<State>>);
struct ContributorProbe(Arc<Mutex<State>>);
impl Plugin for OwnerProbe {
    fn id(&self) -> &'static str {
        "record-owner"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "records",
            name: "Records",
            key: 'I',
        }]
    }
    fn subject(&self, _: &str, _: Pane) -> Option<Subject> {
        self.0.lock().unwrap().subject.clone()
    }
    fn subject_menu_context(&self, _: &str, _: Pane) -> Option<String> {
        let state = self.0.lock().unwrap();
        state
            .guarded
            .then(|| format!("PRIVATE_CONTEXT_{}", state.generation))
    }
    fn capturing(&self) -> bool {
        self.0.lock().unwrap().typing
    }
}
impl Plugin for ContributorProbe {
    fn id(&self) -> &'static str {
        "record-contributor"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn set_up(&self) -> Setup {
        if self.0.lock().unwrap().ready {
            Setup::Ready
        } else {
            Setup::Needed("not available".into())
        }
    }
    fn contribute(&self, point: Point, _: &Subject) -> Option<Contribution> {
        (point == Point::Actions && self.0.lock().unwrap().offered)
            .then(|| Contribution::Actions(vec![Action::new('v', "open its folder", "folder")]))
    }
    fn act(&mut self, subject: &Subject, action: &str) -> Vec<Effect> {
        assert_eq!(action, "folder");
        self.0.lock().unwrap().calls.push(subject.clone());
        vec![Effect::Copy("opened".into())]
    }
}
fn fixture(guarded: bool) -> (Host, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State {
        subject: Some(
            Subject::new("record", "PRIVATE_ID_A", "record")
                .with_url("https://example.invalid/team/project/changes/7")
                .with(
                    "source_repository_url",
                    "https://example.invalid/source/project",
                )
                .with("head_sha", "aaaaaaaa"),
        ),
        generation: 1,
        guarded,
        typing: false,
        ready: true,
        offered: true,
        calls: vec![],
    }));
    let mut host = Host::new(
        vec![
            Box::new(OwnerProbe(state.clone())),
            Box::new(ContributorProbe(state.clone())),
        ],
        0,
    )
    .unwrap();
    host.load_config(Some("")).unwrap();
    (host, state)
}
fn press(host: &mut Host, c: char) -> Vec<HostEffect> {
    host.update(crate::host::chord_event(Chord::new(Key::Char(c))))
}
fn choose(host: &mut Host, enter: bool) -> Vec<HostEffect> {
    if enter {
        let row = host
            .menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .position(|i| i.label == "open its folder")
            .unwrap();
        host.menu.as_mut().unwrap().row = row;
        host.update(crate::host::chord_event(Chord::new(Key::Enter)))
    } else {
        press(host, 'v')
    }
}
#[test]
fn contributed_popup_blocks_changed_identity_fields_configuration_and_route() {
    for changed in [
        "id",
        "kind",
        "url",
        "source",
        "head",
        "missing",
        "config",
        "view",
        "focus",
        "bar",
        "tabs",
        "typing",
        "unready",
        "withdrawn",
    ] {
        for enter in [false, true] {
            let (mut host, state) = fixture(true);
            press(&mut host, 'x');
            {
                let mut s = state.lock().unwrap();
                match changed {
                    "id" => s.subject.as_mut().unwrap().id = "PRIVATE_ID_B".into(),
                    "kind" => s.subject.as_mut().unwrap().kind = "other-record".into(),
                    "url" => {
                        s.subject.as_mut().unwrap().url =
                            Some("https://example.invalid/other".into())
                    }
                    "source" => {
                        s.subject.as_mut().unwrap().fields.insert(
                            "source_repository_url".into(),
                            "https://example.invalid/other/project".into(),
                        );
                    }
                    "head" => {
                        s.subject
                            .as_mut()
                            .unwrap()
                            .fields
                            .insert("head_sha".into(), "bbbbbbbb".into());
                    }
                    "missing" => s.subject = None,
                    "config" => s.generation += 1,
                    "view" => host.view = View::Settings,
                    "focus" => host.focus = Pane::Detail,
                    "bar" => host.bar = true,
                    "tabs" => host.tabs = true,
                    "typing" => s.typing = true,
                    "unready" => s.ready = false,
                    "withdrawn" => s.offered = false,
                    _ => unreachable!(),
                }
            }
            assert!(choose(&mut host, enter).is_empty(), "{changed}");
            assert!(state.lock().unwrap().calls.is_empty(), "{changed}");
            assert!(host.menu.is_none());
            assert!(host.message.as_deref().unwrap().contains("press x"));
            assert!(!host.message.as_deref().unwrap().contains("PRIVATE"));
        }
    }
}
#[test]
fn unchanged_refresh_reopen_dismiss_and_direct_key_keep_expected_behavior() {
    let (mut host, state) = fixture(true);
    press(&mut host, 'x');
    // A fresh equivalent snapshot has the same semantic target and revision.
    let snapshot = state.lock().unwrap().subject.clone();
    state.lock().unwrap().subject = snapshot;
    assert!(matches!(choose(&mut host, false).first(), Some(HostEffect::Copy(s)) if s == "opened"));
    press(&mut host, 'x');
    host.update(crate::host::chord_event(Chord::new(Key::Esc)));
    assert_eq!(state.lock().unwrap().calls.len(), 1);
    state.lock().unwrap().generation += 1;
    press(&mut host, 'x');
    assert!(matches!(choose(&mut host, true).first(), Some(HostEffect::Copy(s)) if s == "opened"));
    assert!(matches!(press(&mut host, 'v').first(), Some(HostEffect::Copy(s)) if s == "opened"));
    assert_eq!(state.lock().unwrap().calls.len(), 3);
}
#[test]
fn legacy_contributor_popup_intentionally_retains_captured_subject() {
    let (mut host, state) = fixture(false);
    press(&mut host, 'x');
    state.lock().unwrap().subject.as_mut().unwrap().id = "PRIVATE_ID_B".into();
    choose(&mut host, false);
    assert_eq!(state.lock().unwrap().calls[0].id, "PRIVATE_ID_A");
}
#[test]
fn blocked_contribution_has_no_confirmation_or_private_context_in_history() {
    let (mut host, state) = fixture(true);
    host.enable_journal();
    press(&mut host, 'x');
    assert!(!format!("{:?}", host.menu_keys).contains("PRIVATE"));
    state.lock().unwrap().generation += 1;
    let effects = choose(&mut host, false);
    assert!(!format!("{effects:?}").contains("PRIVATE"));
    assert!(effects.iter().any(|e| matches!(
        e,
        HostEffect::Journal(JournalRecord::Outcome {
            outcome: JournalOutcome::Blocked,
            ..
        })
    )));
    assert!(!effects.iter().any(|e| matches!(
        e,
        HostEffect::Journal(JournalRecord::Outcome {
            outcome: JournalOutcome::Confirmed,
            ..
        })
    )));
    assert!(state.lock().unwrap().calls.is_empty());
}
