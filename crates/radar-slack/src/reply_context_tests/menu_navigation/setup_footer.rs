//! Setup never advertises keys for the hidden reply editor.

use super::*;
use crate::Target;

/// The status line: the keys it advertises and its counts.
fn status_of(screen: &str) -> &str {
    screen.split("── status").nth(1).expect("the status line")
}

#[test]
fn setup_footer_hides_unroutable_reply_keys_and_settings_recovers_the_text() {
    for (width, height) in [(80, 24), (160, 40)] {
        let (mut host, recorded, _mode) = fixture(false, false);
        let before = state(&host);
        assert!(host.setting_up().is_some());
        assert!(!host.capturing());
        assert!(slack(&host).capturing());
        let screen = shown(&host, width, height);
        assert!(!screen.contains("Active reply stays here."));
        let footer = status_of(&screen);
        assert!(footer.contains("extra actions"), "{footer}");
        for ghost in [
            "new line",
            "review and send",
            "stop",
            "kept replies",
            "ctrl-r",
            "ctrl-s",
        ] {
            assert!(!footer.contains(ghost), "{ghost}: {footer}");
        }
        for (code, modifiers) in [
            (Key::Char('r'), Mods::CONTROL),
            (Key::Char('s'), Mods::CONTROL),
            (
                Key::Char('r'),
                Mods::CONTROL | Mods::ALT,
            ),
        ] {
            press(&mut host, code, modifiers);
            assert!(host.menu.is_none());
            assert_eq!(state(&host), before);
        }
        press(&mut host, Key::Char('x'), Mods::NONE);
        let items = &host
            .menu
            .as_ref()
            .expect("advertised menu opens")
            .menu
            .items;
        assert_eq!(
            items
                .iter()
                .map(|item| (item.label.as_str(), item.keys.as_str()))
                .collect::<Vec<_>>(),
            [
                ("set up Slack in settings", "⏎"),
                ("recent notices (this session)", "~"),
                ("History", ""),
                ("recent activity (last 24 hours)", "H"),
                ("all activity history", ""),
            ]
        );
        assert!(items[2].heading);
        shown(&host, width, height);
        press(&mut host, Key::Esc, Mods::NONE);
        assert_eq!(state(&host), before);

        press(&mut host, Key::Enter, Mods::NONE);
        assert_eq!(host.view, View::Settings);
        assert_eq!(host.current_section().unwrap().1.id, "connection");
        assert_eq!(state(&host), before);
        let settings = shown(&host, width, height);
        assert!(status_of(&settings).contains("ctrl-r kept replies"), "{settings}");
        open(&mut host);
        assert_eq!(state(&host), before);
        shown(&host, width, height);
        press(&mut host, Key::Char('s'), Mods::NONE);
        assert_eq!(kept_texts(slack(&host)), [TEXT, ACTIVE]);
        assert!(slack(&host).replying.is_none());
        assert!(recorded.requests().is_empty());
    }
}

#[test]
fn ready_reply_and_settings_editor_keep_their_routable_footer_hints() {
    let (mut host, recorded, _mode) = fixture(false, true);
    let before = state(&host);
    for (width, height) in [(80, 24), (160, 40)] {
        assert!(host.capturing());
        let screen = shown(&host, width, height);
        let footer = status_of(&screen);
        for hint in ["⏎ new line", "ctrl-s review and send", "esc stop"] {
            assert!(footer.contains(hint), "{hint}: {footer}");
        }
        if width == 160 {
            assert!(footer.contains("ctrl-r kept replies"), "{footer}");
        }
        open(&mut host);
        press(&mut host, Key::Esc, Mods::NONE);
        assert_eq!(state(&host), before);
    }
    assert!(recorded.requests().is_empty());

    let (mut host, recorded, _mode) = fixture(true, false);
    let before = state(&host);
    let p = slack_mut(&mut host);
    p.section_rows[0] = crate::settings::ws_rows(p)
        .iter()
        .position(|row| *row == crate::settings::WsRow::Refresh)
        .unwrap();
    p.edit = Some((Target::Refresh, TextInput::new("90")));
    for (width, height) in [(80, 24), (160, 40)] {
        assert!(host.capturing());
        let screen = shown(&host, width, height);
        let footer = status_of(&screen);
        for hint in ["⏎ keep", "esc cancel", "ctrl-r kept replies"] {
            assert!(footer.contains(hint), "{hint}: {footer}");
        }
        open(&mut host);
        press(&mut host, Key::Esc, Mods::NONE);
        assert_eq!(state(&host), before);
        assert_eq!(slack(&host).edit.as_ref().unwrap().1.value(), "90");
    }
    press(&mut host, Key::Esc, Mods::NONE);
    assert!(
        slack(&host).edit.is_none(),
        "advertised cancel reaches settings"
    );
    assert_eq!(state(&host), before);
    assert!(recorded.requests().is_empty());
}
