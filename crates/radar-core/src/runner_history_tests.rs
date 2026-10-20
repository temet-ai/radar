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
    let image = crate::shell::to_svg(&host, 80, 24);
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
    let image = crate::shell::to_svg(&host, 160, 40);
    assert!(image.contains("test.pipeline"));
    assert!(image.contains("succeeded"));
    // From #26: the detail keeps its scroll through help, and a key with
    // ctrl or alt does not move it.
    let press = |host: &mut Host, k: Key| {
        assert!(host.update(key(Chord::new(k))).is_empty());
    };
    press(&mut host, Key::Enter);
    let first_detail = crate::shell::to_text(&host, 80, 24);
    press(&mut host, Key::PageDown);
    let scrolled = crate::shell::to_text(&host, 80, 24);
    assert_ne!(first_detail, scrolled);
    for modified in [Chord { ctrl: true, ..Chord::new(Key::Home) }, Chord { alt: true, ..Chord::new(Key::Home) }] {
        assert!(host.update(key(modified)).is_empty());
        assert_eq!(crate::shell::to_text(&host, 80, 24), scrolled);
    }
    press(&mut host, Key::Char('?'));
    let help_top = crate::shell::to_text(&host, 80, 24);
    assert_ne!(help_top, scrolled, "help shows");
    press(&mut host, Key::Esc);
    assert_eq!(crate::shell::to_text(&host, 80, 24), scrolled, "back to the same scroll");
    let events = recording
        .sink
        .history(Default::default(), None, None)
        .await
        .unwrap();
    assert_eq!(events.events.len(), 2);
    recording.finish();
    std::fs::remove_dir_all(directory).unwrap();
}
