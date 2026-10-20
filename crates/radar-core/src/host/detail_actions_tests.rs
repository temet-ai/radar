use super::*;
use crate::plugin::Binding;
use crate::view::detail::{DetailContent, DetailHeader, FootAction};
use crate::view::{Node, PaneView};

fn subject(name: &str, connection: u64) -> Subject {
    Subject {
        owner: 0,
        scope: "books".into(),
        identity: DetailIdentity {
            connection,
            subject: vec![name.into()],
        },
    }
}

fn action(label: &str) -> ActionSpec {
    ActionSpec::named("books.copy", &Binding::new("y", label))
}

fn view(label: &str) -> ScreenView {
    let action = action(label);
    let foot = FootActions::new(vec![FootAction::new(action.clone()).caption("copy")]);
    let mut view = ScreenView::new(
        "books",
        vec![crate::view::detail::pane(
            "preview",
            DetailContent::new(
                DetailHeader::new("watch/books", "Guards! Guards!"),
                vec![],
                vec![],
            ),
            foot,
        )],
    );
    view.actions = vec![action];
    view
}

mod unit {
    use super::*;

    #[test]
    fn exact_targets_fail_closed_and_never_round_trip_alternate_spellings() {
        for slot in [Slot::Action(0), Slot::Action(4), Slot::Menu] {
            assert_eq!(
                parse(target_of(u64::MAX, slot).as_str()),
                Some((u64::MAX, slot))
            );
        }
        for target in [
            "",
            "detail-foot:",
            "detail-foot:0:0",
            "detail-foot:01:0",
            "detail-foot:1:00",
            "detail-foot:1:+1",
            "detail-foot:1:menu:0",
            "detail-foot:18446744073709551616:menu",
        ] {
            assert!(parse(target).is_none(), "{target}");
        }
    }

    #[test]
    fn repeated_projection_is_stable_and_subject_or_connection_aba_is_retired() {
        let mut state = State::default();
        let a = offered(subject("Vimes", 1), &view("copy original link")).unwrap();
        let serial = state.project(a.clone()).unwrap().serial;
        assert_eq!(state.project(a.clone()).unwrap().serial, serial);
        for replacement in [Some(subject("Carrot", 1)), Some(subject("Vimes", 2)), None] {
            state.reconcile(replacement.as_ref());
            assert!(state.active.is_none());
            let fresh = state.project(a.clone()).unwrap().serial;
            assert!(fresh > serial);
        }
    }

    #[test]
    fn canonical_semantics_and_availability_are_part_of_the_lease() {
        let a = offered(subject("Vimes", 1), &view("copy original link")).unwrap();
        let mut state = State::default();
        let old = state.project(a.clone()).unwrap().serial;
        let relabeled = offered(subject("Vimes", 1), &view("copy replacement link")).unwrap();
        assert_eq!(
            a.foot, relabeled.foot,
            "a compact caption can hide a changed canonical meaning"
        );
        assert!(state.project(relabeled).unwrap().serial > old);
        let mut disabled = a.clone();
        disabled.foot.actions[0].disabled = Some("waiting for held data".into());
        let old = state.project(a).unwrap().serial;
        assert!(state.project(disabled).unwrap().serial > old);
    }

    #[test]
    fn ambiguous_missing_heading_or_rebound_actions_are_inert_and_menu_is_host_owned() {
        let mut v = view("copy original link");
        let duplicate = v.actions[0].clone();
        v.actions.push(duplicate);
        assert_eq!(
            offered(subject("Vimes", 1), &v).unwrap().canonical,
            vec![None]
        );
        v.actions.clear();
        assert_eq!(
            offered(subject("Vimes", 1), &v).unwrap().canonical,
            vec![None]
        );
        v.actions = vec![action("copy original link")];
        v.actions[0].key = crate::view::Chord::parse("c");
        assert_eq!(
            offered(subject("Vimes", 1), &v).unwrap().canonical,
            vec![None]
        );
        v.actions[0].heading = true;
        assert_eq!(
            offered(subject("Vimes", 1), &v).unwrap().canonical,
            vec![None]
        );
        v.panes[0].foot_actions.as_mut().unwrap().menu.action.id = "books.copy".into();
        assert!(offered(subject("Vimes", 1), &v).is_none());
    }

    #[test]
    fn serial_exhaustion_revokes_instead_of_wrapping_or_reusing_a_target() {
        let a = offered(subject("Vimes", 1), &view("copy original link")).unwrap();
        let mut state = State {
            serial: u64::MAX - 1,
            active: None,
        };
        assert_eq!(state.project(a.clone()).unwrap().serial, u64::MAX);
        state.reconcile(None);
        assert!(state.project(a.clone()).is_none());
        assert!(state.project(a).is_none());
        assert_eq!(state.serial, u64::MAX);
    }

    #[test]
    fn frame_prefixes_keep_logical_numbers_and_missing_panes_unadorned() {
        use crate::view::chrome::{PaneJump, pane_title};
        let jumps = [PaneJump {
            pane: PaneId::Middle,
            key: '1',
        }];
        assert_eq!(pane_title(&jumps, PaneId::Middle, "book"), "1 book");
        assert_eq!(pane_title(&jumps, PaneId::Detail, "notes"), "notes");
        assert_eq!(pane_title(&[], PaneId::Middle, "setup"), "setup");
        let mut wire = serde_json::to_value(crate::view::chrome::ChromeView::new(80, 24)).unwrap();
        wire.as_object_mut().unwrap().remove("write_mode");
        let old: crate::view::chrome::ChromeView = serde_json::from_value(wire).unwrap();
        assert_eq!(old.write_mode, crate::view::chrome::WriteMode::ReadOnly);
        assert!(old.pane_jumps.is_empty());
    }
}

mod contract {
    use super::*;
    use crate::host::{Event, View};
    use crate::plugin::{Effect, Menu, Msg, Outcome, Plugin, ScreenSpec};
    use std::sync::{Arc, Mutex};

    struct Held {
        subject: Option<String>,
        connection: u64,
        label: String,
        disabled: Option<String>,
        hidden: bool,
        capturing: bool,
        overlay: bool,
        page: bool,
        ready: bool,
        measured_width: u16,
        write: Option<crate::writes::Change>,
        seen: Vec<(Pane, Intent)>,
        sent: usize,
        answered: Vec<String>,
    }

    struct Probe(Arc<Mutex<Held>>);
    type ChangeHeld = Box<dyn FnOnce(&mut Held) + Send>;

    impl Plugin for Probe {
        fn id(&self) -> &'static str {
            "detail-probe"
        }
        fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
            Ok(())
        }
        fn config(&self) -> Option<toml::Value> {
            None
        }
        fn screens(&self) -> Vec<ScreenSpec> {
            vec![
                ScreenSpec {
                    id: "books",
                    name: "books",
                    key: 'B',
                },
                ScreenSpec {
                    id: "shelves",
                    name: "shelves",
                    key: 'S',
                },
            ]
        }
        fn set_up(&self) -> crate::compose::Setup {
            if self.0.lock().unwrap().ready {
                crate::compose::Setup::Ready
            } else {
                crate::compose::Setup::Needed("choose a shelf".into())
            }
        }
        fn depth(&self, _: &str) -> usize {
            usize::from(self.0.lock().unwrap().page)
        }
        fn capturing(&self) -> bool {
            self.0.lock().unwrap().capturing
        }
        fn panes(&self, _: &str) -> Vec<Pane> {
            let held = self.0.lock().unwrap();
            if held.page {
                if held.measured_width >= 120 {
                    vec![Pane::Nav, Pane::Middle]
                } else {
                    vec![Pane::Middle]
                }
            } else {
                vec![Pane::Nav, Pane::Middle, Pane::Detail]
            }
        }
        fn detail_identity(&self, scope: &str, pane: Pane) -> Option<DetailIdentity> {
            let held = self.0.lock().unwrap();
            if scope != "books" || pane != Pane::Detail || held.page {
                return None;
            }
            Some(DetailIdentity {
                connection: held.connection,
                subject: vec![held.subject.clone()?],
            })
        }
        fn view(&self, scope: &str, cx: &crate::view::ViewCx<'_>) -> Option<ScreenView> {
            let mut held = self.0.lock().unwrap();
            held.measured_width = cx.width;
            let mut view = super::view(&held.label);
            view.scope = scope.into();
            let foot = view.panes[0].foot_actions.as_mut().unwrap();
            foot.actions[0].disabled = held.disabled.clone();
            foot.actions[0].action.change = held.write.map(Into::into);
            view.actions[0].change = held.write.map(Into::into);
            if held.hidden {
                foot.actions.clear();
            }
            if cx.focus != PaneId::Detail {
                view.actions.clear();
            }
            if held.overlay {
                view.overlay = Some(Box::new(PaneView::new(
                    PaneId::Middle,
                    "editing",
                    Node::Document {
                        lines: vec![],
                        cut: false,
                    },
                )));
            }
            Some(view)
        }
        fn intent(&mut self, _: &str, pane: Pane, intent: &Intent) -> Outcome {
            let mut held = self.0.lock().unwrap();
            held.seen.push((pane, intent.clone()));
            if let Intent::Invoke { action } = intent
                && action == "books.copy"
            {
                if let Some(change) = held.write {
                    let mut menu = Menu::write_plain(
                        "held",
                        "keep this note?",
                        vec![held.subject.clone().unwrap()],
                        "keep",
                        "leave",
                    );
                    if change == crate::writes::Change::Add {
                        menu = menu.adds();
                    }
                    return Outcome::Handled(vec![Effect::Menu(menu)]);
                }
                return Outcome::Handled(vec![Effect::Copy(held.subject.clone().unwrap())]);
            }
            Outcome::done()
        }
        fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
            let change = *msg.downcast::<ChangeHeld>().unwrap();
            change(&mut self.0.lock().unwrap());
            vec![]
        }
        fn on_menu(&mut self, _: &str, value: &str) -> Vec<Effect> {
            let mut held = self.0.lock().unwrap();
            held.answered.push(value.into());
            if value == "yes"
                && let Some(change) = held.write
            {
                if let Some(reason) = crate::writes::not_sent(change) {
                    return vec![Effect::notify(None, reason)];
                }
                held.sent += 1;
            }
            vec![]
        }
    }

    fn fixture() -> (Host, Arc<Mutex<Held>>) {
        let held = Arc::new(Mutex::new(Held {
            subject: Some("Vimes".into()),
            connection: 1,
            label: "copy original link".into(),
            disabled: None,
            hidden: false,
            capturing: false,
            overlay: false,
            page: false,
            ready: true,
            measured_width: 0,
            write: None,
            seen: vec![],
            sent: 0,
            answered: vec![],
        }));
        let mut host = Host::new(vec![Box::new(Probe(held.clone()))], 0).unwrap();
        host.load_config(Some("")).unwrap();
        host.view = View::Screen(0);
        (host, held)
    }

    fn change(host: &mut Host, f: impl FnOnce(&mut Held) + Send + 'static) {
        host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(Box::new(f) as ChangeHeld),
        });
    }

    fn target(host: &Host) -> Target {
        host.displayed_screen(160, 40).unwrap().panes[0]
            .foot_actions
            .as_ref()
            .unwrap()
            .actions[0]
            .target
            .clone()
            .unwrap()
    }

    fn activate(host: &mut Host, target: &Target) -> Vec<HostEffect> {
        host.update(Event::Intent(Intent::Activate {
            target: target.clone(),
        }))
    }

    fn press(host: &mut Host, key: &str) {
        host.update(crate::host::chord_event(crate::plugin::parse_chord(key).unwrap()));
    }

    #[test]
    fn native_controls_and_renderer_intents_share_admission_and_never_synthesize_enter() {
        let (mut host, held) = fixture();
        host.focus = Pane::Nav;
        host.bar = true;
        let raw = host.screen_view(0, "books").unwrap();
        assert!(
            raw.panes[0].foot_actions.as_ref().unwrap().actions[0]
                .target
                .is_none()
        );
        let first = target(&host);
        assert_eq!(target(&host), first);
        assert_eq!(host.screen_view(0, "books").unwrap(), raw);
        host.screen_view_in(0, "shelves", 20, 5);
        assert_eq!(
            target(&host),
            first,
            "inactive enumeration cannot rotate a lease"
        );
        assert!(
            matches!(activate(&mut host, &first).as_slice(), [HostEffect::Copy(value)] if value == "Vimes")
        );
        assert_eq!(host.focus, Pane::Detail);
        assert!(!host.bar && !host.sources && !host.tabs);
        for _ in 0..2 {
            let out = activate(&mut host, &first);
            assert!(matches!(out.as_slice(), [HostEffect::Copy(value)] if value == "Vimes"));
        }
        let count = held.lock().unwrap().seen.len();
        assert_eq!(count, 3);
        assert!(
            held.lock()
                .unwrap()
                .seen
                .iter()
                .all(|(pane, intent)| *pane == Pane::Detail
                    && matches!(intent, Intent::Invoke { action } if action == "books.copy"))
        );
        for intent in [
            Intent::Select {
                target: first.clone(),
            },
            Intent::Context {
                target: first.clone(),
            },
            Intent::Activate {
                target: Target::new("detail-foot:bad:menu"),
            },
        ] {
            assert!(host.update(Event::Intent(intent)).is_empty());
        }
        assert_eq!(held.lock().unwrap().seen.len(), count);
        let live = host.displayed_screen(160, 40).unwrap();
        assert_eq!(crate::headless::foot_controls(&live).len(), 2);
        assert!(crate::headless::offered(&live).is_empty());
        let menu = live.panes[0]
            .foot_actions
            .as_ref()
            .unwrap()
            .menu
            .target
            .clone()
            .unwrap();
        activate(&mut host, &menu);
        assert_eq!(host.menu.as_ref().unwrap().menu.id, crate::host::KEYS_MENU);
        assert_eq!(host.focus, Pane::Detail);
    }

    #[test]
    fn events_revoke_selection_config_surface_and_capture_aba_before_any_intermediate_paint() {
        let (mut host, held) = fixture();
        for mutate in [
            (
                Box::new(|s: &mut Held| s.subject = Some("Carrot".into())) as ChangeHeld,
                Box::new(|s: &mut Held| s.subject = Some("Vimes".into())) as ChangeHeld,
            ),
            (
                Box::new(|s: &mut Held| s.subject = None),
                Box::new(|s: &mut Held| s.subject = Some("Vimes".into())),
            ),
            (
                Box::new(|s: &mut Held| s.connection = 2),
                Box::new(|s: &mut Held| s.connection = 3),
            ),
            (
                Box::new(|s: &mut Held| s.page = true),
                Box::new(|s: &mut Held| s.page = false),
            ),
            (
                Box::new(|s: &mut Held| s.capturing = true),
                Box::new(|s: &mut Held| s.capturing = false),
            ),
            (
                Box::new(|s: &mut Held| s.ready = false),
                Box::new(|s: &mut Held| s.ready = true),
            ),
        ] {
            let old = target(&host);
            change(&mut host, mutate.0);
            change(&mut host, mutate.1);
            host.focus = Pane::Nav;
            assert!(activate(&mut host, &old).is_empty());
            assert_eq!(host.focus, Pane::Nav);
            let fresh = target(&host);
            assert_ne!(fresh, old);
            assert!(!activate(&mut host, &fresh).is_empty());
        }
        let old = target(&host);
        press(&mut host, "S");
        press(&mut host, "B");
        assert!(activate(&mut host, &old).is_empty());
        let old = target(&host);
        host.notify("held note".into());
        host.open_notice_history();
        // Notice handling returns early from update; its epilogue must still retire.
        activate(&mut host, &old);
        press(&mut host, "esc");
        assert!(activate(&mut host, &old).is_empty());
        assert_eq!(held.lock().unwrap().seen.len(), 6);
    }

    #[test]
    fn pane_numbers_follow_responsive_dispatch_and_exclude_setup_and_auxiliary_panes() {
        let (mut host, _) = fixture();
        let normal = host.chrome_view(80, 24).pane_jumps;
        assert_eq!(
            normal.iter().map(|jump| jump.key).collect::<Vec<_>>(),
            vec!['1', '2', '3']
        );
        press(&mut host, "3");
        assert_eq!(host.focus, Pane::Detail);
        change(&mut host, |held| held.page = true);
        let narrow = host.chrome_view(80, 24).pane_jumps;
        assert_eq!(
            narrow,
            vec![crate::view::chrome::PaneJump {
                pane: PaneId::Middle,
                key: '1'
            }]
        );
        // Raw enumeration at another width cannot poison the active map.
        host.screen_view_in(0, "shelves", 160, 40);
        assert_eq!(host.pane_jumps(), narrow);
        press(&mut host, "1");
        assert_eq!(host.focus, Pane::Middle);
        let wide = host.chrome_view(160, 40).pane_jumps;
        assert_eq!(
            wide.iter().map(|jump| jump.pane).collect::<Vec<_>>(),
            vec![PaneId::Nav, PaneId::Middle]
        );
        assert!(!wide.iter().any(|jump| jump.pane == PaneId::Detail));
        press(&mut host, "2");
        assert_eq!(host.focus, Pane::Middle);
        host.view = View::Settings;
        assert_eq!(host.chrome_view(80, 24).pane_jumps.len(), 3);
        host.view = View::Screen(0);
        change(&mut host, |held| held.ready = false);
        assert_eq!(host.chrome_view(80, 24).pane_jumps, narrow);
        host.open_setup(None).unwrap();
        assert!(host.chrome_view(80, 24).pane_jumps.is_empty());
    }

    #[test]
    fn fresh_semantics_overlay_and_original_write_confirmation_are_checked_before_dispatch() {
        let saved_mode = crate::writes::mode();
        let (mut host, held) = fixture();
        let old = target(&host);
        change(&mut host, |s| s.label = "copy replacement link".into());
        assert!(activate(&mut host, &old).is_empty());
        let old = target(&host);
        change(&mut host, |s| s.disabled = Some("waiting".into()));
        assert!(activate(&mut host, &old).is_empty());
        change(&mut host, |s| s.disabled = None);
        let old = target(&host);
        change(&mut host, |s| s.hidden = true);
        assert!(activate(&mut host, &old).is_empty());
        change(&mut host, |s| s.hidden = false);
        let old = target(&host);
        change(&mut host, |s| s.overlay = true);
        host.focus = Pane::Nav;
        assert!(activate(&mut host, &old).is_empty());
        assert_eq!(host.focus, Pane::Nav);
        change(&mut host, |s| s.overlay = false);
        assert!(held.lock().unwrap().seen.is_empty());
        for (mode, change_kind, sends) in [
            (
                crate::writes::Mode::ReadOnly,
                crate::writes::Change::Add,
                false,
            ),
            (
                crate::writes::Mode::Write,
                crate::writes::Change::Edit,
                false,
            ),
            (crate::writes::Mode::Edit, crate::writes::Change::Edit, true),
        ] {
            host.set_env_mode(Some((mode, "TEST_MODE".into())));
            change(&mut host, move |s| s.write = Some(change_kind));
            let old = target(&host);
            let before = held.lock().unwrap().sent;
            activate(&mut host, &old);
            let menu = &host.menu.as_ref().unwrap().menu;
            assert_eq!(menu.title, "keep this note?");
            assert!(menu.items.iter().any(|item| item.label.contains("Vimes")));
            assert_eq!(
                menu.items
                    .iter()
                    .any(|item| item.label.contains("nothing will be sent")),
                !sends
            );
            assert!(
                activate(&mut host, &old).is_empty(),
                "a background foot cannot answer the question"
            );
            assert_eq!(held.lock().unwrap().sent, before);
            press(&mut host, "y");
            assert_eq!(held.lock().unwrap().sent, before + usize::from(sends));
            assert!(
                activate(&mut host, &old).is_empty(),
                "closing a question never revives its background lease"
            );
        }
        crate::writes::set_mode(saved_mode.0, saved_mode.1);
    }
}
