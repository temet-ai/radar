//! Compact settings use the actual Jira fields, capture and host navigation.
use super::*;

fn key80(host: &mut radar_core::Host, code: Key) {
    crate::tests::shown(host, 80, 24);
    host.update(crate::key_event(Chord::new(code)));
}

#[test]
fn compact_settings_show_the_selected_value_and_live_verdict_with_the_cursor() {
    let mut host = hosted();
    section(&mut host, "connection");
    for _ in 0..4 {
        key80(&mut host, Key::Down);
    }
    let got = crate::tests::shown(&host, 80, 24);
    assert!(got.contains("https://acme.atlassian.net"), "{got}");
    insta::assert_snapshot!("settings_selected_site", got);
    key80(&mut host, Key::Enter);
    host.update(crate::key_event(Chord { ctrl: true, ..Chord::new(Key::Char('u')) }));
    host.update(Event::Paste("gamma".into()));
    assert!(host.capturing(), "the host is typing into the field");
    let got = crate::tests::shown(&host, 80, 24);
    assert!(got.contains(":error: not a web address"), "{got}");
    insta::assert_snapshot!("settings_invalid_site", got);
    host.update(crate::key_event(Chord { ctrl: true, ..Chord::new(Key::Char('u')) }));
    host.update(Event::Paste(
        "https://gamma.atlassian.net/jira/software/projects/GAM/boards/2".into(),
    ));
    let got = crate::tests::shown(&host, 80, 24);
    assert!(got.contains(":run_passed: the site gamma.atlassian.net"), "{got}");
    assert!(host.capturing());
    insta::assert_snapshot!("settings_valid_site", got);
}

#[test]
fn compact_settings_reset_between_two_sites_with_the_same_field_title() {
    let mut host = hosted();
    section(&mut host, "connection");
    for _ in 0..4 {
        key80(&mut host, Key::Down);
    }
    assert_eq!(host.section_view(0, "connection").unwrap().detail_title, "site");
    key80(&mut host, Key::Tab);
    key80(&mut host, Key::End);
    // The sheet ends with the help to connect Jira.
    let bottom = crate::tests::shown(&host, 80, 24);
    assert!(bottom.contains("radar guide: connect Jira"), "{bottom}");
    key80(&mut host, Key::Left);
    assert!(crate::tests::shown(&host, 80, 24).contains("radar guide: connect Jira"));
    // Past the first site's projects row and the second site's own row.
    for _ in 0..7 {
        key80(&mut host, Key::Down);
    }
    assert_eq!(jira(&host).section_rows[0], 11);
    assert_eq!(host.section_view(0, "connection").unwrap().detail_title, "site");
    key80(&mut host, Key::Right);
    let got = crate::tests::shown(&host, 80, 24);
    assert!(
        got.contains("CHECKED") && got.contains("https://beta.atlassian.net"),
        "{got}"
    );
}
