//! The recovery menu keeps the host's navigation keys separate from draft actions.

use super::*;
use radar_core::Host;
use radar_core::host::{Hit, MouseKind};
use radar_core::shell::to_text;

mod setup_footer;

const ACTIVE: &str = "Active reply stays here.\nIncluding trailing spaces.  ";

fn slack(host: &Host) -> &SlackPlugin {
    assert_eq!(host.plugins[0].id(), "slack");
    // SAFETY: fixture installs exactly one SlackPlugin and never replaces it.
    unsafe { &*(host.plugins[0].as_ref() as *const dyn Plugin as *const SlackPlugin) }
}

fn slack_mut(host: &mut Host) -> &mut SlackPlugin {
    assert_eq!(host.plugins[0].id(), "slack");
    // SAFETY: fixture installs exactly one SlackPlugin, exclusively borrowed.
    unsafe { &mut *(host.plugins[0].as_mut() as *mut dyn Plugin as *mut SlackPlugin) }
}

fn fixture(settings: bool, configured: bool) -> (Host, Arc<Recorded>, ModeGuard) {
    let (mut p, recorded, mode) = sample(false);
    let menu = preview(&mut p);
    p.answer_reply(&menu, "no").expect("retain the first reply");
    p.begin_reply(demo::TEAM, OTHER_ROOT, Some(ACTIVE));
    if !configured {
        p.cfg.token.clear();
        p.via = Via(None);
    }
    let mut host = crate::tests::hosted();
    host.plugins[0] = Box::new(p);
    host.view = if settings {
        View::Settings
    } else {
        View::Screen(0)
    };
    if settings {
        host.section = host
            .sections
            .iter()
            .position(|(owner, section)| *owner == Owner::Plugin(0) && section.id == "connection")
            .expect("Slack connection settings");
    }
    host.focus = Pane::Middle;
    (host, recorded, mode)
}

fn state(host: &Host) -> Value {
    let p = slack(host);
    json!({
        "composer": p.replying.as_ref().map(|r| json!({
            "channel": r.channel,
            "thread": r.thread,
            "text": r.composer.text(),
            "cursor": r.composer.input.cursor(),
            "incarnation": r.stamp.incarnation,
            "revision": r.stamp.revision,
            "read": r.read,
            "team": r.team,
            "status": format!("{:?}", r.status),
        })),
        "kept": kept(p),
        "usage": p.reply_context.usage(),
        "capturing": p.capturing(),
    })
}

fn update(host: &mut Host, event: Event) -> Vec<HostEffect> {
    let effects = host.update(event);
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, HostEffect::Spawn { .. })),
        "recovery navigation and keeping must never start service work"
    );
    effects
}

fn press(host: &mut Host, code: KeyCode, modifiers: KeyModifiers) -> Vec<HostEffect> {
    update(host, Event::Key(KeyEvent::new(code, modifiers)))
}

fn open(host: &mut Host) -> String {
    press(host, KeyCode::Char('r'), KeyModifiers::CONTROL);
    let menu = &host.menu.as_ref().expect("Ctrl-R recovery menu").menu;
    assert!(menu.id.starts_with("reply-kept:"));
    menu.id.clone()
}

#[test]
fn host_recovery_navigation_and_escape_preserve_draft_and_retained_rows() {
    for (settings, configured) in [(false, true), (true, true), (true, false)] {
        let (mut host, recorded, _mode) = fixture(settings, configured);
        let before = state(&host);
        let id = open(&mut host);
        let rows = host.menu.as_ref().unwrap().menu.items.clone();
        assert_eq!(rows.len(), 3);
        for (code, row) in [
            (KeyCode::Char('j'), 1),
            (KeyCode::Char('j'), 2),
            (KeyCode::Char('k'), 1),
            (KeyCode::Up, 0),
            (KeyCode::Down, 1),
            (KeyCode::Char('k'), 0),
        ] {
            to_text(&host, 80, 24);
            press(&mut host, code, KeyModifiers::NONE);
            let menu = host.menu.as_ref().expect("navigation keeps the menu open");
            assert_eq!(menu.menu.id, id, "{settings}/{configured}/{code:?}");
            assert_eq!(menu.menu.items, rows);
            assert_eq!(menu.row, row, "{code:?}");
            assert_eq!(state(&host), before);
        }
        press(&mut host, KeyCode::Esc, KeyModifiers::NONE);
        assert!(host.menu.is_none());
        assert_eq!(state(&host), before);
        assert_ne!(
            open(&mut host),
            id,
            "reopening gets a fresh captured intent"
        );
        assert_eq!(host.menu.as_ref().unwrap().menu.items, rows);
        assert_eq!(state(&host), before);
        assert!(recorded.requests().is_empty());
    }
}

#[test]
fn host_recovery_keep_shortcut_is_explicit_and_single_use_without_credentials() {
    for (settings, configured) in [(false, true), (true, true), (true, false)] {
        let (mut host, recorded, _mode) = fixture(settings, configured);
        let id = open(&mut host);
        let keep = host.menu.as_ref().unwrap().menu.items.last().unwrap();
        assert_eq!(
            (keep.key, keep.keys.as_str(), keep.value.as_str()),
            (Some('s'), "s", "keep")
        );
        assert_eq!(keep.label, "keep current draft");
        to_text(&host, 80, 24);
        press(&mut host, KeyCode::Char('s'), KeyModifiers::NONE);
        assert!(!slack(&host).capturing());
        assert_eq!(kept_texts(slack(&host)), [TEXT, ACTIVE]);
        assert_eq!(
            slack(&host).reply_context.usage(),
            (2, TEXT.len() + ACTIVE.len())
        );
        assert_ne!(host.menu.as_ref().unwrap().menu.id, id);
        assert!(
            host.menu
                .as_ref()
                .unwrap()
                .menu
                .items
                .iter()
                .all(|item| item.key.is_none())
        );
        let after = state(&host);
        to_text(&host, 80, 24);
        press(&mut host, KeyCode::Char('s'), KeyModifiers::NONE);
        assert_eq!(state(&host), after, "repeated shortcut cannot keep twice");
        press(&mut host, KeyCode::Esc, KeyModifiers::NONE);
        open(&mut host);
        assert_eq!(state(&host), after);
        assert!(recorded.requests().is_empty());
    }
}

#[test]
fn host_recovery_modified_keys_do_not_borrow_keep_copy_or_navigation() {
    let (mut host, recorded, _mode) = fixture(false, true);
    let before = state(&host);
    let id = open(&mut host);
    to_text(&host, 160, 40);
    for (code, modifiers) in [
        (KeyCode::Char('s'), KeyModifiers::CONTROL),
        (KeyCode::Char('s'), KeyModifiers::ALT),
        (
            KeyCode::Char('s'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        (
            KeyCode::Char('s'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ),
        (KeyCode::Char('y'), KeyModifiers::CONTROL),
        (KeyCode::Char('j'), KeyModifiers::ALT),
        (KeyCode::Char('k'), KeyModifiers::CONTROL),
        (KeyCode::Up, KeyModifiers::CONTROL),
        (KeyCode::Down, KeyModifiers::ALT),
        (
            KeyCode::Char('r'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ),
    ] {
        assert!(press(&mut host, code, modifiers).is_empty());
        let menu = host.menu.as_ref().unwrap();
        assert_eq!(menu.menu.id, id);
        assert_eq!(menu.row, 0);
        assert_eq!(state(&host), before);
    }
    let copied = press(&mut host, KeyCode::Char('y'), KeyModifiers::NONE);
    assert!(
        copied
            .iter()
            .any(|effect| matches!(effect, HostEffect::Copy(text) if text == ACTIVE))
    );
    assert!(host.menu.is_none());
    assert_eq!(
        state(&host),
        before,
        "the existing copy choice is unchanged"
    );
    assert!(recorded.requests().is_empty());
}

#[test]
fn host_recovery_keep_rejects_newer_editor_or_connection_context() {
    for change in ["edit", "target", "connection"] {
        let (mut host, recorded, _mode) = fixture(false, true);
        open(&mut host);
        to_text(&host, 80, 24);
        let p = slack_mut(&mut host);
        match change {
            "edit" => {
                p.paste("newer text");
            }
            "target" => {
                p.begin_reply(demo::TEAM, ROOT, Some("new target composer"));
            }
            "connection" => {
                p.replace_token(String::new());
            }
            _ => unreachable!(),
        }
        let before = state(&host);
        press(&mut host, KeyCode::Char('s'), KeyModifiers::NONE);
        assert!(host.menu.is_none());
        assert_eq!(state(&host), before, "{change}");
        assert!(recorded.requests().is_empty());
    }
}

#[test]
fn host_recovery_keep_enter_and_repeated_click_retain_only_one_copy() {
    for (width, height, click) in [(80, 24, false), (80, 24, true), (160, 40, true)] {
        let (mut host, recorded, _mode) = fixture(false, true);
        open(&mut host);
        press(&mut host, KeyCode::Down, KeyModifiers::NONE);
        press(&mut host, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(
            host.menu.as_ref().unwrap().menu.items[host.menu.as_ref().unwrap().row].value,
            "keep"
        );
        to_text(&host, width, height);
        if click {
            let area = host
                .hits
                .borrow()
                .iter()
                .find_map(|(area, hit)| (*hit == Hit::MenuItem(2)).then_some(*area))
                .expect("rendered keep row");
            for at_ms in [1000, 1100] {
                // The second press deliberately precedes a redraw of the replacement menu.
                update(
                    &mut host,
                    Event::Mouse {
                        kind: MouseKind::Press,
                        column: area.x,
                        row: area.y,
                        at_ms,
                    },
                );
            }
            to_text(&host, width, height);
            update(
                &mut host,
                Event::Mouse {
                    kind: MouseKind::Press,
                    column: area.x,
                    row: area.y,
                    at_ms: 1200,
                },
            );
        } else {
            press(&mut host, KeyCode::Enter, KeyModifiers::NONE);
        }
        assert!(!slack(&host).capturing());
        assert_eq!(kept_texts(slack(&host)), [TEXT, ACTIVE]);
        assert!(recorded.requests().is_empty());
    }
}

#[test]
fn host_recovery_keep_menu_shows_its_non_navigation_shortcut_at_both_sizes() {
    let (mut host, recorded, _mode) = fixture(false, true);
    open(&mut host);
    press(&mut host, KeyCode::Down, KeyModifiers::NONE);
    press(&mut host, KeyCode::Down, KeyModifiers::NONE);
    for (width, height) in [(80, 24), (160, 40)] {
        let screen = to_text(&host, width, height);
        assert!(screen.contains("s  keep current draft"), "{screen}");
        insta::assert_snapshot!(format!("kept_reply_menu_{width}x{height}"), screen);
        if let Some(dir) = std::env::var_os("RADAR_SLACK_REPLY_MENU_SHOTS") {
            let dir = std::path::Path::new(&dir);
            std::fs::create_dir_all(dir).unwrap();
            radar_core::shell::to_svg(&host, width, height);
            let svg = radar_core::shell::to_svg(&host, width, height);
            std::fs::write(dir.join(format!("kept-replies-{width}x{height}.svg")), &svg).unwrap();
            let png =
                radar_core::shot::to_png(&svg).expect("visual export needs --features radar-core/png");
            std::fs::write(dir.join(format!("kept-replies-{width}x{height}.png")), png).unwrap();
        }
    }
    assert!(recorded.requests().is_empty());
}
