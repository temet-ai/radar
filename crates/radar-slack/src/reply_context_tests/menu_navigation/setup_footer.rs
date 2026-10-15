//! Setup never advertises keys for the hidden reply editor.

use super::*;
use crate::Target;

fn render(host: &Host, place: &str, width: u16, height: u16) -> String {
    let screen = to_text(host, width, height);
    if let Some(dir) = std::env::var_os("RADAR_SLACK_SETUP_FOOTER_SHOTS") {
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();
        let svg = radar_core::shell::to_svg(host, width, height);
        std::fs::write(dir.join(format!("{place}-{width}x{height}.svg")), &svg).unwrap();
        let png = radar_core::shot::to_png(&svg).expect("visual export needs --features radar-core/png");
        std::fs::write(dir.join(format!("{place}-{width}x{height}.png")), png).unwrap();
    }
    screen
}

#[test]
fn setup_footer_hides_unroutable_reply_keys_and_settings_recovers_the_text() {
    for (width, height) in [(80, 24), (160, 40)] {
        let (mut host, recorded, _mode) = fixture(false, false);
        let before = state(&host);
        assert!(host.setting_up().is_some());
        assert!(!host.capturing());
        assert!(slack(&host).capturing());
        let screen = render(&host, "setup", width, height);
        assert!(!screen.contains("Active reply stays here."));
        let footer = screen.lines().last().unwrap();
        assert!(footer.contains("x menu"), "{footer}");
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
        assert!(host.hits.borrow().iter().all(|(_, hit)| {
            !matches!(hit, Hit::Key(key, _) if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('r' | 's')))
        }));
        for (code, modifiers) in [
            (KeyCode::Char('r'), KeyModifiers::CONTROL),
            (KeyCode::Char('s'), KeyModifiers::CONTROL),
            (
                KeyCode::Char('r'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ),
        ] {
            press(&mut host, code, modifiers);
            assert!(host.menu.is_none());
            assert_eq!(state(&host), before);
        }
        press(&mut host, KeyCode::Char('x'), KeyModifiers::NONE);
        let items = &host
            .menu
            .as_ref()
            .expect("advertised menu opens")
            .menu
            .items;
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].label, "set up Slack in settings");
        assert_eq!(items[0].keys, "⏎");
        assert_eq!(items[1].label, "History");
        assert_eq!(items[2].label, "recent activity (last 24 hours)");
        assert_eq!(items[2].keys, "H");
        assert_eq!(items[3].label, "all activity history");
        render(&host, "setup-menu", width, height);
        press(&mut host, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(state(&host), before);

        press(&mut host, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(host.view, View::Settings);
        assert_eq!(host.current_section().unwrap().1.id, "connection");
        assert_eq!(state(&host), before);
        let settings = render(&host, "settings-recovery", width, height);
        assert!(
            settings
                .lines()
                .last()
                .unwrap()
                .contains("ctrl-r kept replies")
        );
        open(&mut host);
        assert_eq!(state(&host), before);
        to_text(&host, width, height);
        press(&mut host, KeyCode::Char('s'), KeyModifiers::NONE);
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
        let screen = render(&host, "ready-reply", width, height);
        let footer = screen.lines().last().unwrap();
        for hint in ["⏎ new line", "ctrl-s review and send", "esc stop"] {
            assert!(footer.contains(hint), "{hint}: {footer}");
        }
        if width == 160 {
            assert!(footer.contains("ctrl-r kept replies"), "{footer}");
        }
        open(&mut host);
        press(&mut host, KeyCode::Esc, KeyModifiers::NONE);
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
        let screen = render(&host, "settings-editor", width, height);
        let footer = screen.lines().last().unwrap();
        for hint in ["⏎ keep", "esc cancel", "ctrl-r kept replies"] {
            assert!(footer.contains(hint), "{hint}: {footer}");
        }
        open(&mut host);
        press(&mut host, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(state(&host), before);
        assert_eq!(slack(&host).edit.as_ref().unwrap().1.value(), "90");
    }
    press(&mut host, KeyCode::Esc, KeyModifiers::NONE);
    assert!(
        slack(&host).edit.is_none(),
        "advertised cancel reaches settings"
    );
    assert_eq!(state(&host), before);
    assert!(recorded.requests().is_empty());
}
