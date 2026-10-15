//! Compact settings use the actual Jira fields, capture and host navigation.
use super::*;

fn key80(host: &mut radar_core::Host, code: KeyCode) {
    text(host, 80, 24);
    host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

fn export(host: &radar_core::Host, name: &str) {
    let Some(dir) = std::env::var_os("RADAR_SETTINGS_POLISH_SHOTS") else {
        return;
    };
    let dir = std::path::Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();
    for (width, height) in [(80, 24), (160, 40)] {
        let svg = radar_core::shell::to_svg(host, width, height);
        std::fs::write(dir.join(format!("{name}-{width}x{height}.svg")), &svg).unwrap();
        let png = radar_core::shot::to_png(&svg).expect("visual export needs radar-core/png");
        std::fs::write(dir.join(format!("{name}-{width}x{height}.png")), png).unwrap();
    }
}

#[test]
fn compact_settings_show_the_selected_value_and_live_verdict_with_the_cursor() {
    let mut host = hosted();
    section(&mut host, "connection");
    for _ in 0..4 {
        key80(&mut host, KeyCode::Down);
    }
    let shown = text(&host, 80, 24);
    assert!(shown.contains("https://acme.atlassian.net"), "{shown}");
    insta::assert_snapshot!("settings_selected_site_80x24", shown);
    insta::assert_snapshot!("settings_selected_site_160x40", text(&host, 160, 40));
    export(&host, "selected-site");
    key80(&mut host, KeyCode::Enter);
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    host.update(Event::Paste("gamma".into()));
    let theme = host.build_theme(radar_core::design::theme::Tier::Mono);
    let (shown, _, cursor) = radar_core::shell::text_and_cursor(&host, &theme, 80, 24);
    assert!(shown.contains("✗ not a web address"), "{shown}");
    let cursor = cursor.expect("typing cursor visible");
    assert!(
        cursor.x >= 24 && cursor.x < 80 && cursor.y < 12,
        "{cursor:?}"
    );
    insta::assert_snapshot!("settings_invalid_site_80x24", shown);
    export(&host, "invalid-site");
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    host.update(Event::Paste(
        "https://gamma.atlassian.net/jira/software/projects/GAM/boards/2".into(),
    ));
    let shown = text(&host, 80, 24);
    assert!(shown.contains("✓ the site gamma.atlassian.net"), "{shown}");
    assert!(host.capturing());
    insta::assert_snapshot!("settings_valid_site_80x24", shown);
    export(&host, "valid-site");
}

#[test]
fn compact_settings_reset_between_two_sites_with_the_same_field_title() {
    let mut host = hosted();
    section(&mut host, "connection");
    for _ in 0..4 {
        key80(&mut host, KeyCode::Down);
    }
    assert_eq!(host.section_view(0, "connection").unwrap().detail_title, "site");
    key80(&mut host, KeyCode::Right);
    key80(&mut host, KeyCode::End);
    let bottom = text(&host, 80, 24);
    assert!(bottom.contains("config: [[jira.site]] url"), "{bottom}");
    key80(&mut host, KeyCode::Left);
    assert!(text(&host, 80, 24).contains("config: [[jira.site]] url"));
    export(&host, "scrolled-site");
    for _ in 0..6 {
        key80(&mut host, KeyCode::Down);
    }
    assert_eq!(jira(&host).section_rows[0], 10);
    assert_eq!(host.section_view(0, "connection").unwrap().detail_title, "site");
    key80(&mut host, KeyCode::Right);
    let shown = text(&host, 80, 24);
    assert!(
        shown.contains("CHECKED") && shown.contains("https://beta.atlassian.net"),
        "{shown}"
    );
    assert!(
        !shown.contains("config: [[jira.site]] url"),
        "new field starts at the top: {shown}"
    );
    export(&host, "other-site");
}
