//! One end-to-end host/runner/ordered-reader slice, with a private test database.
use super::*;
use crate::view::{Chord, Intent, Key};

fn key(chord: Chord) -> Event {
    Event::Intent(Intent::Key { chord })
}

fn open(host: &mut Host) -> HostEffect {
    let mut effects = host.update(key(Chord::char('H')));
    assert_eq!(effects.len(), 1);
    effects.pop().unwrap()
}
#[tokio::test]
async fn history_without_a_writer_returns_a_safe_disabled_state() {
    let mut host = Host::new(vec![], 0).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    perform(open(&mut host), &tx, None);
    let event = rx.recv().await.unwrap();
    assert!(matches!(
        event,
        Event::History {
            result: Err(crate::history::ReadError::Disabled),
            ..
        }
    ));
    assert!(host.update(event).is_empty());
    let image = crate::headless::text_of(&host, 80, 24);
    assert!(image.contains("disabled"));
}
#[cfg(unix)]
#[tokio::test]
async fn history_runner_renders_recorded_outcomes_without_creating_read_events() {
    let directory =
        std::env::temp_dir().join(format!("radar-history-runner-{}", ActionId::new().as_str()));
    let path = directory.join("actions.sqlite3");
    let mut recording = Recording::open_at(&path, SessionOrigin::Tui).unwrap();
    let action = recording.sink.begin("test.pipeline", Origin::Host, None);
    recording
        .sink
        .outcome(&action, JournalOutcome::Succeeded, None);
    let mut host = Host::new(vec![], 0).unwrap();
    host.enable_journal();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    perform(open(&mut host), &tx, Some(&recording.sink));
    let event = rx.recv().await.unwrap();
    assert!(matches!(&event, Event::History { result: Ok(page), .. } if page.events.len() == 2));
    assert!(host.update(event).is_empty());
    let image = crate::headless::text_of(&host, 160, 40);
    assert!(image.contains("test.pipeline"));
    assert!(image.contains("succeeded"));
    // From #26: the detail keeps its scroll through help, and a key with
    // ctrl or alt does not move it.
    let press = |host: &mut Host, k: Key| {
        assert!(host.update(key(Chord::new(k))).is_empty());
    };
    press(&mut host, Key::Enter);
    let detail_scroll = |host: &Host| host.chrome_view(80, 24).history.expect("the overlay").detail_scroll;
    press(&mut host, Key::PageDown);
    let scrolled = detail_scroll(&host);
    for modified in [Chord { ctrl: true, ..Chord::new(Key::Home) }, Chord { alt: true, ..Chord::new(Key::Home) }] {
        assert!(host.update(key(modified)).is_empty());
        assert_eq!(detail_scroll(&host), scrolled);
    }
    press(&mut host, Key::Char('?'));
    assert!(host.chrome_view(80, 24).history.expect("the overlay").help, "help shows");
    press(&mut host, Key::Esc);
    let back = host.chrome_view(80, 24).history.expect("the overlay");
    assert!(!back.help);
    assert_eq!(back.detail_scroll, scrolled, "back to the same scroll");
    let events = recording
        .sink
        .history(Default::default(), None, None)
        .await
        .unwrap();
    assert_eq!(events.events.len(), 2);
    recording.finish();
    std::fs::remove_dir_all(directory).unwrap();
}
