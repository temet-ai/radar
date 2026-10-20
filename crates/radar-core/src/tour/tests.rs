use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::host::Event;

fn press(host: &mut Host, code: KeyCode) {
    host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

fn host() -> Host {
    crate::shot::demo_host(vec![]).unwrap()
}

mod mnemonic_tests {
use super::*;

#[test]
fn tour_key_words_supply_whole_grapheme_styles_and_plain_shortcuts() {
    use ratatui::style::Modifier;
    let theme = crate::Theme::new(crate::design::theme::Tier::Mono);
    for (key, words, expected, plain) in [
        ("x", "extra actions", "extra actions", "extra actions [x]"),
        ("A", "ask an agent", "shift ask an agent", "shift ask an agent [A]"),
        ("e", "e\u{301}dit", "e\u{301}dit", "e\u{301}dit [e]"),
    ] {
        let text = keyed(key, words);
        assert_eq!(text.words(), expected);
        assert_eq!(text.plain_words(), plain);
        let line = crate::view_tui::line(&theme, &text);
        assert!(line.spans.iter().any(|span| span.style.add_modifier.contains(Modifier::UNDERLINED)));
    }
}

}

#[test]
fn the_tour_walks_its_stops_and_esc_ends_it() {
    let mut host = host();
    host.open_setup(Some("tour")).unwrap();
    assert!(host.setup.is_none());
    let card = host.tour_card().unwrap();
    assert_eq!((card.stop, card.title.as_str()), (1, "Every screen has a letter."));
    assert!(card.keys.words().contains("esc end the tour"));
    press(&mut host, KeyCode::Left);
    assert_eq!(host.tour.as_ref().unwrap().at, 0, "nothing before the first stop");
    press(&mut host, KeyCode::Right);
    let card = host.tour_card().unwrap();
    assert_eq!((card.stop, card.try_it.as_deref()), (2, Some("try it: press tab")));
    // The key a stop says to try goes on to the next stop, and the app has it.
    press(&mut host, KeyCode::Tab);
    assert_eq!(host.tour_card().unwrap().stop, 3);
    press(&mut host, KeyCode::Left);
    assert_eq!(host.tour_card().unwrap().stop, 2);
    press(&mut host, KeyCode::Esc);
    assert!(host.tour.is_none());
}

#[test]
fn the_last_stop_says_the_mode_and_enter_starts_working() {
    let mut host = host();
    host.open_tour();
    let n = host.tour.as_ref().unwrap().stops.len();
    for _ in 1..n {
        press(&mut host, KeyCode::Right);
    }
    let card = host.tour_card().unwrap();
    assert_eq!(card.title, "That's the tour.");
    assert!(card.lines[0].words().contains(host.mode().word()));
    assert!(card.keys.words().contains("start working"));
    press(&mut host, KeyCode::Enter);
    assert!(host.tour.is_none());
}

#[test]
fn the_card_is_given_over_the_app() {
    let mut host = host();
    host.open_tour();
    let text = crate::headless::host_text_of(&host, 80, 24);
    assert!(text.contains("tour · 1 of"), "{text}");
    assert!(text.contains("Every screen has a letter."), "{text}");
}
