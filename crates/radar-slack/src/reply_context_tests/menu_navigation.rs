//! The recovery menu keeps the host's navigation keys separate from draft actions.

use super::*;
use radar_core::Host;
use radar_core::view::Intent;
use crate::tests::shown;

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

fn press(host: &mut Host, code: Key, modifiers: crate::keys::Mods) -> Vec<HostEffect> {
    update(host, crate::keys::ev(crate::keys::chord(code, modifiers)))
}

fn open(host: &mut Host) -> String {
    press(host, Key::Char('r'), Mods::CONTROL);
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
            (Key::Char('j'), 1),
            (Key::Char('j'), 2),
            (Key::Char('k'), 1),
            (Key::Up, 0),
            (Key::Down, 1),
            (Key::Char('k'), 0),
        ] {
            shown(&host, 80, 24);
            press(&mut host, code, Mods::NONE);
            let menu = host.menu.as_ref().expect("navigation keeps the menu open");
            assert_eq!(menu.menu.id, id, "{settings}/{configured}/{code:?}");
            assert_eq!(menu.menu.items, rows);
            assert_eq!(menu.row, row, "{code:?}");
            assert_eq!(state(&host), before);
        }
        press(&mut host, Key::Esc, Mods::NONE);
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
        assert_eq!(keep.label, "stay with current draft");
        shown(&host, 80, 24);
        press(&mut host, Key::Char('s'), Mods::NONE);
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
        shown(&host, 80, 24);
        press(&mut host, Key::Char('s'), Mods::NONE);
        assert_eq!(state(&host), after, "repeated shortcut cannot keep twice");
        press(&mut host, Key::Esc, Mods::NONE);
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
    shown(&host, 160, 40);
    for (code, modifiers) in [
        (Key::Char('s'), Mods::CONTROL),
        (Key::Char('s'), Mods::ALT),
        (
            Key::Char('s'),
            Mods::CONTROL | Mods::SHIFT,
        ),
        (
            Key::Char('s'),
            Mods::CONTROL | Mods::ALT,
        ),
        (Key::Char('y'), Mods::CONTROL),
        (Key::Char('j'), Mods::ALT),
        (Key::Char('k'), Mods::CONTROL),
        (Key::Up, Mods::CONTROL),
        (Key::Down, Mods::ALT),
        (
            Key::Char('r'),
            Mods::CONTROL | Mods::ALT,
        ),
    ] {
        assert!(press(&mut host, code, modifiers).is_empty());
        let menu = host.menu.as_ref().unwrap();
        assert_eq!(menu.menu.id, id);
        assert_eq!(menu.row, 0);
        assert_eq!(state(&host), before);
    }
    let copied = press(&mut host, Key::Char('y'), Mods::NONE);
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
        shown(&host, 80, 24);
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
        press(&mut host, Key::Char('s'), Mods::NONE);
        assert!(host.menu.is_none());
        assert_eq!(state(&host), before, "{change}");
        assert!(recorded.requests().is_empty());
    }
}

#[test]
fn host_recovery_keep_enter_and_repeated_choice_retain_only_one_copy() {
    // A press on the menu's third row, by the value the menu view gives it.
    fn press_keep_row(host: &mut Host) {
        let action = host.menu.as_ref().and_then(|m| m.menu.items.get(2)).map(|item| item.value.clone());
        if let Some(action) = action {
            update(host, Event::Intent(Intent::Invoke { action }));
        }
    }
    for click in [false, true] {
        let (mut host, recorded, _mode) = fixture(false, true);
        open(&mut host);
        press(&mut host, Key::Down, Mods::NONE);
        press(&mut host, Key::Down, Mods::NONE);
        assert_eq!(
            host.menu.as_ref().unwrap().menu.items[host.menu.as_ref().unwrap().row].value,
            "keep"
        );
        shown(&host, 160, 40);
        if click {
            // The second press comes before the replacement menu was shown.
            press_keep_row(&mut host);
            press_keep_row(&mut host);
            shown(&host, 160, 40);
            press_keep_row(&mut host);
        } else {
            press(&mut host, Key::Enter, Mods::NONE);
        }
        assert!(!slack(&host).capturing());
        assert_eq!(kept_texts(slack(&host)), [TEXT, ACTIVE]);
        assert!(recorded.requests().is_empty());
    }
}

#[test]
fn host_recovery_keep_menu_shows_its_non_navigation_shortcut() {
    let (mut host, recorded, _mode) = fixture(false, true);
    open(&mut host);
    press(&mut host, Key::Down, Mods::NONE);
    press(&mut host, Key::Down, Mods::NONE);
    let screen = shown(&host, 160, 40);
    assert!(screen.contains("stay with current draft"), "{screen}");
    insta::assert_snapshot!("kept_reply_menu", screen);
    assert!(recorded.requests().is_empty());
}
