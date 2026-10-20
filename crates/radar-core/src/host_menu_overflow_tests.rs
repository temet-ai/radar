//! Reading a long menu never changes the action, drops text, or hides its field.

use super::*;
use crate::plugin::{Typing, Verdict};
use ratatui::layout::{Position, Rect};
use std::sync::{Arc, Mutex};

type MenuLog = Arc<Mutex<Vec<(String, String)>>>;

struct MenuProbe {
    log: MenuLog,
    typing: Option<Typing>,
}

impl Plugin for MenuProbe {
    fn id(&self) -> &'static str {
        "menu-probe"
    }

    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }

    fn config(&self) -> Option<toml::Value> {
        None
    }

    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "items",
            name: "items",
            key: 'I',
        }]
    }

    fn typing(&self, _: &str, _: &str) -> Typing {
        self.typing.clone().unwrap_or_default()
    }

    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        self.log.lock().unwrap().push((menu.into(), value.into()));
        vec![]
    }
}

fn fixture(menu: Menu, typing: Option<Typing>) -> (Host, MenuLog) {
    let log = Arc::new(Mutex::new(vec![]));
    let mut host = Host::new(
        vec![Box::new(MenuProbe {
            log: log.clone(),
            typing,
        })],
        0,
    )
    .unwrap();
    host.load_config(Some("")).unwrap();
    host.menu = Some(OpenMenu::open(Owner::Plugin(0), menu));
    host.typed_changed(true);
    (host, log)
}

fn press(host: &mut Host, code: Key) {
    host.update(crate::host::chord_event(Chord::new(code)));
}

fn draw(host: &Host, size: (u16, u16)) -> String {
    crate::shell::to_text(host, size.0, size.1)
}

fn assert_choice(log: &MenuLog, menu: &str, value: &str) {
    assert_eq!(
        *log.lock().unwrap(),
        vec![(menu.to_owned(), value.to_owned())]
    );
}

fn open(host: &Host) -> &OpenMenu {
    host.menu.as_ref().expect("reading keeps the menu open")
}

fn menu_rect(host: &Host) -> Rect {
    host.hits
        .borrow()
        .iter()
        .find_map(|(rect, hit)| matches!(hit, Hit::MenuBox).then_some(*rect))
        .expect("a drawn menu has a box")
}

fn item_rects(host: &Host, item: usize) -> Vec<Rect> {
    host.hits
        .borrow()
        .iter()
        .filter_map(|(rect, hit)| (*hit == Hit::MenuItem(item)).then_some(*rect))
        .collect()
}

fn wheel(host: &mut Host, down: bool) {
    let rect = menu_rect(host);
    host.update(Event::Mouse {
        kind: if down {
            MouseKind::ScrollDown
        } else {
            MouseKind::ScrollUp
        },
        column: rect.x + rect.width / 2,
        row: rect.y + rect.height / 2,
        at_ms: 100,
    });
}

fn click(host: &mut Host, at: Position) {
    host.update(Event::Mouse {
        kind: MouseKind::Press,
        column: at.x,
        row: at.y,
        at_ms: 200,
    });
}

fn text_position(screen: &str, needle: &str) -> Option<Position> {
    screen.lines().enumerate().find_map(|(y, line)| {
        line.find(needle).map(|at| Position {
            x: crate::ui::width(&line[..at]) as u16,
            y: y as u16,
        })
    })
}

/// The fixture needs only a few pages. Calculate that number from the drawn
/// geometry rather than spinning until a condition happens to change.
fn pages_left(host: &Host, down: bool) -> usize {
    let view = &open(host).viewport;
    let distance = if down {
        view.max().saturating_sub(view.at.get())
    } else {
        view.at.get()
    };
    let pages = distance.div_ceil(view.height.get().saturating_sub(1).max(1));
    assert!(pages <= 128, "fixture unexpectedly needs {pages} pages");
    pages
}

fn page_to_edge(host: &mut Host, size: (u16, u16), down: bool) -> String {
    let mut screen = draw(host, size);
    for _ in 0..pages_left(host, down) {
        press(
            host,
            if down {
                Key::PageDown
            } else {
                Key::PageUp
            },
        );
        screen = draw(host, size);
    }
    screen
}

fn all_pages(host: &mut Host, size: (u16, u16)) -> String {
    let mut screens = page_to_edge(host, size, false);
    for _ in 0..pages_left(host, true) {
        press(host, Key::PageDown);
        screens.push('\n');
        screens.push_str(&draw(host, size));
    }
    screens
}

fn long_question() -> Menu {
    let mut menu =
        Menu::confirm("review", "keep these notes?", "keep", "leave").saying("opening-note");
    for n in 0..46 {
        menu = menu.saying(format!("detail {n:02}: read this line before choosing"));
    }
    menu.saying("closing-note")
}

#[test]
fn paging_a_confirmation_covers_every_observation_and_wrapped_continuation() {
    let notes: Vec<String> = (0..48)
        .map(|n| format!("Observation {n:02}: start_{n:02} middle_{n:02} end_{n:02}"))
        .collect();
    let menu = Menu::write_confirm(
        "review",
        "keep these observations?",
        &notes.join("\n"),
        "keep",
        "leave",
    );
    for size in [(80, 24), (160, 40), (34, 14)] {
        let (mut host, log) = fixture(menu.clone(), None);
        let selected = open(&host).row;
        let screens = all_pages(&mut host, size);
        for (n, note) in notes.iter().enumerate() {
            assert!(
                screens.contains(&format!("Observation {n:02}:")),
                "lost observation {n} at {size:?}"
            );
            // At narrow widths these distinctive tokens occupy wrapped
            // continuation lines, including across a page boundary.
            for token in note.split_whitespace().skip(2) {
                assert!(screens.contains(token), "lost {token} at {size:?}");
            }
        }
        assert_eq!(open(&host).viewport.at.get(), open(&host).viewport.max());
        assert_eq!(open(&host).row, selected);
        assert_eq!(open(&host).menu, menu);
        assert!(log.lock().unwrap().is_empty());
    }
}

#[test]
fn every_confirmation_viewport_contains_consecutive_observation_rows() {
    let notes = (0..40)
        .map(|n| format!("Observation {n:02}: end{n:02}"))
        .collect();
    let menu = Menu::write_plain("review", "review observations?", notes, "keep", "leave");
    for size in [(80, 24), (160, 40), (34, 10)] {
        let (mut host, log) = fixture(menu.clone(), None);
        let _ = draw(&host, size);
        let pages = pages_left(&host, true);
        for page in 0..=pages {
            let screen = draw(&host, size);
            let view = &open(&host).viewport;
            assert_eq!(
                view.total.get(),
                40,
                "each source observation fits one line"
            );
            let found: Vec<(usize, usize)> = screen
                .lines()
                .enumerate()
                .filter_map(|(row, line)| {
                    let (_, tail) = line.split_once("Observation ")?;
                    let (number, _) = tail.split_once(':')?;
                    Some((row, number.parse().expect("a fixture observation number")))
                })
                .collect();
            assert_eq!(
                found.iter().map(|(_, n)| *n).collect::<Vec<_>>(),
                (view.at.get()..view.at.get() + view.height.get()).collect::<Vec<_>>(),
                "no skipped observations at {size:?}, page {page}\n{screen}"
            );
            assert!(
                found.windows(2).all(|pair| pair[1].0 == pair[0].0 + 1),
                "observations occupy consecutive screen rows"
            );
            if page < pages {
                press(&mut host, Key::PageDown);
            }
        }
        assert!(log.lock().unwrap().is_empty());
    }
}

fn offers() -> Vec<MenuItem> {
    (0..24)
        .map(|n| {
            MenuItem::pick(
                format!("choice {n:02}"),
                "a possible value",
                format!("value-{n:02}"),
            )
        })
        .collect()
}

fn assert_modal_hits_inside(host: &Host, size: (u16, u16)) {
    let hits = host.hits.borrow();
    let bounds = Rect::new(0, 0, size.0, size.1);
    for (rect, hit) in hits.iter() {
        if matches!(hit, Hit::MenuBox | Hit::MenuItem(_)) {
            assert_eq!(*rect, rect.intersection(bounds), "{hit:?}: {rect:?}");
        }
        if matches!(hit, Hit::MenuItem(_)) {
            assert!(rect.width > 0 && rect.height > 0, "empty action target");
            let popup = hits
                .iter()
                .find_map(|(r, h)| matches!(h, Hit::MenuBox).then_some(*r))
                .unwrap();
            assert_eq!(*rect, rect.intersection(popup), "action outside popup");
        }
    }
}

#[test]
fn long_questions_open_at_the_first_note_and_keep_choices_pinned_while_reading() {
    for (size, answer) in [((80, 24), Key::Enter), ((160, 40), Key::Char('y'))] {
        let (mut host, log) = fixture(long_question(), None);
        let original = open(&host).menu.clone();
        let selected = open(&host).row;
        let first = draw(&host, size);
        assert!(first.contains("opening-note"), "{first}");
        assert!(!first.contains("closing-note"), "fixture must overflow");
        assert_eq!(open(&host).viewport.at.get(), 0);
        assert!(open(&host).viewport.more());
        let yes = item_rects(&host, selected);
        let no = item_rects(&host, selected + 1);
        assert!(
            !yes.is_empty() && !no.is_empty(),
            "both choices are visible"
        );

        wheel(&mut host, true);
        let _ = draw(&host, size);
        assert!(open(&host).viewport.at.get() > 0);
        assert_eq!(open(&host).row, selected);
        assert_eq!(item_rects(&host, selected), yes);
        wheel(&mut host, false);
        let _ = draw(&host, size);
        assert_eq!(open(&host).viewport.at.get(), 0);

        let last = page_to_edge(&mut host, size, true);
        assert!(last.contains("closing-note"), "{last}");
        assert_eq!(open(&host).viewport.at.get(), open(&host).viewport.max());
        assert_eq!(item_rects(&host, selected), yes);
        assert_eq!(item_rects(&host, selected + 1), no);
        let first_again = page_to_edge(&mut host, size, false);
        assert!(first_again.contains("opening-note"), "{first_again}");
        assert_eq!(open(&host).viewport.at.get(), 0);
        assert_eq!(open(&host).row, selected);
        assert_eq!(open(&host).menu, original, "reading must keep all the text");
        assert!(log.lock().unwrap().is_empty());

        press(&mut host, answer);
        assert!(host.menu.is_none());
        assert_choice(&log, "review", "yes");
    }
}

#[test]
fn reading_a_narrow_question_preserves_an_explicit_no_default_and_accelerators() {
    for answer in [Key::Enter, Key::Char('y')] {
        let (mut host, log) = fixture(long_question().at("no"), None);
        let size = (34, 10);
        let screen = draw(&host, size);
        let selected = open(&host).row;
        assert_eq!(open(&host).menu.items[selected].value, "no");
        assert!(screen.contains("opening-note"), "{screen}");
        assert!(!item_rects(&host, selected).is_empty());
        assert!(!item_rects(&host, selected - 1).is_empty());
        wheel(&mut host, true);
        let _ = draw(&host, size);
        press(&mut host, Key::PageDown);
        let _ = draw(&host, size);
        assert!(open(&host).viewport.at.get() > 0);
        assert_eq!(open(&host).row, selected);
        assert!(log.lock().unwrap().is_empty());
        press(&mut host, answer);
        assert!(host.menu.is_none());
        assert_choice(
            &log,
            "review",
            if answer == Key::Enter {
                "no"
            } else {
                "yes"
            },
        );
    }
}

#[test]
fn overflow_exposes_scroll_keys_and_a_scrollbar_on_its_first_render() {
    let (host, _) = fixture(long_question(), None);
    let screen = draw(&host, (80, 24));
    assert!(screen.contains("scroll"), "{screen}");
    assert!(
        screen.contains('█'),
        "overflow needs a visible scrollbar thumb\n{screen}"
    );
    for code in [Key::PageUp, Key::PageDown] {
        assert!(
            host.hits
                .borrow()
                .iter()
                .any(|(_, hit)| { matches!(hit, Hit::Key(key, _) if key.key == code) }),
            "{code:?} is advertised and clickable on the first frame"
        );
    }
}

#[test]
fn wrapped_label_and_hint_continuations_choose_the_original_item() {
    let menu = Menu::new(
        "pick",
        "choose an item",
        vec![
            MenuItem::heading("available items"),
            MenuItem::note("read before choosing"),
            MenuItem::new(
                'a',
                "A careful label with several words ending label-end",
                "A complete explanation for this action ending hint-end",
                "original-value",
            ),
            MenuItem::new('c', "other item", "another action", "other-value"),
        ],
    );
    for needle in ["label-end", "hint-end"] {
        let (mut host, log) = fixture(menu.clone(), None);
        let size = (34, 10);
        let mut screen = draw(&host, size);
        let first_row = item_rects(&host, 2).first().copied().unwrap();
        let mut at = text_position(&screen, needle);
        for _ in 0..pages_left(&host, true) {
            if at.is_some() {
                break;
            }
            press(&mut host, Key::PageDown);
            screen = draw(&host, size);
            at = text_position(&screen, needle);
        }
        let at = at.unwrap_or_else(|| panic!("missing {needle}\n{screen}"));
        assert!(
            open(&host).viewport.at.get() > 0 || at.y > first_row.y,
            "the test must click a wrapped continuation"
        );
        assert!(item_rects(&host, 2).iter().any(|r| r.contains(at)));
        assert_modal_hits_inside(&host, size);
        assert_eq!(open(&host).menu, menu);
        assert!(log.lock().unwrap().is_empty());
        click(&mut host, at);
        assert!(host.menu.is_none());
        assert_choice(&log, "pick", "original-value");
    }
}

#[test]
fn narrow_question_button_continuations_keep_their_original_choice() {
    let (mut host, log) = fixture(
        Menu::confirm(
            "ask",
            "keep these notes?",
            "Keep these notes until review-end",
            "Leave these notes until leave-end",
        ),
        None,
    );
    let size = (34, 10);
    let screen = draw(&host, size);
    assert!(open(&host).viewport.usable.get(), "{screen}");
    assert!(screen.contains("review-end"), "{screen}");
    let at = text_position(&screen, "leave-end").unwrap_or_else(|| panic!("{screen}"));
    let choice = item_rects(&host, 1);
    assert!(choice.iter().any(|r| r.contains(at)));
    assert!(at.y > choice[0].y, "click the button's continuation line");
    assert_modal_hits_inside(&host, size);
    click(&mut host, at);
    assert!(host.menu.is_none());
    assert_choice(&log, "ask", "no");
}

#[test]
fn ordinary_menu_paging_changes_reading_position_without_changing_selection() {
    let menu = Menu::new("pick", "available items", offers());
    let (mut host, log) = fixture(menu.clone(), None);
    let size = (34, 10);
    let _ = draw(&host, size);
    let selected = open(&host).row;
    press(&mut host, Key::PageDown);
    let screen = draw(&host, size);
    assert!(open(&host).viewport.at.get() > 0, "{screen}");
    assert_eq!(open(&host).row, selected);
    assert_eq!(open(&host).menu, menu);
    assert!(log.lock().unwrap().is_empty());
    press(&mut host, Key::PageUp);
    let _ = draw(&host, size);
    assert_eq!(open(&host).viewport.at.get(), 0);
    wheel(&mut host, true);
    let _ = draw(&host, size);
    assert_eq!(open(&host).row, selected);
    assert!(open(&host).viewport.at.get() > 0);
    wheel(&mut host, false);
    let _ = draw(&host, size);
    assert_eq!(open(&host).viewport.at.get(), 0);
    press(&mut host, Key::Enter);
    assert_choice(&log, "pick", "value-00");
}

#[test]
fn a_long_title_can_be_read_in_full_by_paging() {
    let title = (0..24)
        .map(|n| format!("title{n:02}"))
        .collect::<Vec<_>>()
        .join(" ");
    let menu = Menu::new(
        "pick",
        title.clone(),
        vec![MenuItem::new('a', "keep", "", "kept")],
    );
    let (mut host, log) = fixture(menu, None);
    let first = draw(&host, (34, 10));
    assert!(
        !item_rects(&host, 0).is_empty(),
        "the selected action opens in sight\n{first}"
    );
    assert!(
        open(&host).viewport.at.get() > 0,
        "the long title cannot fit before the action"
    );
    assert!(
        !first.contains("title00"),
        "the earlier title is reached by paging back"
    );
    let screens = all_pages(&mut host, (34, 10));
    for word in title.split_whitespace() {
        assert!(screens.contains(word), "lost title word {word}\n{screens}");
    }
    assert!(
        open(&host).viewport.at.get() > 0,
        "the title requires paging"
    );
    assert_eq!(open(&host).menu.title, title);
    assert_eq!(open(&host).row, 0);
    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn a_questions_long_title_opens_at_its_start_with_its_choice_still_visible() {
    let title = (0..24)
        .map(|n| format!("title{n:02}"))
        .collect::<Vec<_>>()
        .join(" ");
    let menu = Menu::confirm("review", title.clone(), "keep", "leave").saying("draft note");
    let (mut host, log) = fixture(menu, None);
    let selected = open(&host).row;
    let first = draw(&host, (34, 10));
    assert!(
        first.contains("title00"),
        "a question starts with what it asks\n{first}"
    );
    assert_eq!(open(&host).viewport.at.get(), 0);
    assert!(
        !item_rects(&host, selected).is_empty(),
        "its choice remains pinned"
    );
    let screens = all_pages(&mut host, (34, 10));
    for word in title.split_whitespace() {
        assert!(screens.contains(word), "lost title word {word}");
    }
    assert_eq!(open(&host).row, selected);
    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn typed_menu_paging_keeps_the_field_offers_and_normal_editing() {
    let (mut host, log) = fixture(
        Menu::typed("name", "name the item", "kept text"),
        Some(Typing {
            verdict: None,
            offers: offers(),
        }),
    );
    let size = (34, 10);
    let _ = draw(&host, size);
    let original = open(&host).menu.items.clone();
    let cursor = open(&host).input.as_ref().unwrap().cursor();
    press(&mut host, Key::PageDown);
    let screen = draw(&host, size);
    assert!(
        screen.contains("kept text"),
        "the field stays visible\n{screen}"
    );
    assert!(open(&host).viewport.at.get() > 0);
    assert_eq!(open(&host).row, NO_ROW);
    assert_eq!(open(&host).input.as_ref().unwrap().value(), "kept text");
    assert_eq!(open(&host).input.as_ref().unwrap().cursor(), cursor);
    assert_eq!(open(&host).menu.items, original);
    assert!(log.lock().unwrap().is_empty());
    press(&mut host, Key::PageUp);
    let _ = draw(&host, size);
    assert_eq!(open(&host).viewport.at.get(), 0);

    press(&mut host, Key::Left);
    press(&mut host, Key::Char('!'));
    assert_eq!(open(&host).input.as_ref().unwrap().value(), "kept tex!t");
    host.update(crate::host::chord_event(Chord::ctrl('u')));
    assert_eq!(open(&host).input.as_ref().unwrap().value(), "t");
    let _ = draw(&host, size);
    press(&mut host, Key::Enter);
    assert_choice(&log, "name", "t");
}

#[test]
fn long_refusal_reasons_are_readable_without_losing_the_typed_value() {
    let words = (0..28).map(|n| format!("reason{n:02}")).collect::<Vec<_>>();
    let reason = words.join(" ");
    let (mut host, log) = fixture(
        Menu::typed("name", "name the item", "kept text"),
        Some(Typing {
            verdict: Some(Verdict::Bad(reason.clone())),
            offers: vec![],
        }),
    );
    let screens = all_pages(&mut host, (34, 10));
    for word in words {
        assert!(
            screens.contains(&word),
            "lost refusal word {word}\n{screens}"
        );
    }
    assert_eq!(open(&host).refused.as_deref(), Some(reason.as_str()));
    press(&mut host, Key::Enter);
    assert_eq!(open(&host).input.as_ref().unwrap().value(), "kept text");
    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn unicode_wrapping_keeps_complete_wide_and_combining_text_and_its_hit() {
    let label = "界界 cafe\u{301} alpha beta gamma delta epsilon zeta unicode-end";
    let menu = Menu::new(
        "pick",
        "choose an item",
        vec![MenuItem::new('a', label, "keep it", "unicode")],
    );
    let (mut host, log) = fixture(menu, None);
    let size = (34, 10);
    let first = draw(&host, size);
    assert!(
        first.contains("界界") && first.contains("cafe\u{301}"),
        "{first}"
    );
    let last = page_to_edge(&mut host, size, true);
    let at = text_position(&last, "unicode-end").unwrap_or_else(|| panic!("{last}"));
    assert!(item_rects(&host, 0).iter().any(|r| r.contains(at)));
    assert_modal_hits_inside(&host, size);
    assert_eq!(open(&host).menu.items[0].label, label);
    click(&mut host, at);
    assert_choice(&log, "pick", "unicode");
}

#[test]
fn resize_clamps_the_reading_position_and_preserves_the_original_choice() {
    let (mut host, log) = fixture(long_question(), None);
    let original = open(&host).menu.clone();
    let selected = open(&host).row;
    let _ = page_to_edge(&mut host, (80, 24), true);
    let before = open(&host).viewport.at.get();
    let _ = draw(&host, (160, 40));
    assert!(
        open(&host).viewport.at.get() < before,
        "a taller screen clamps the old bottom"
    );
    assert_eq!(open(&host).viewport.at.get(), open(&host).viewport.max());
    for size in [(34, 10), (160, 40), (80, 24)] {
        let _ = draw(&host, size);
        assert!(open(&host).viewport.at.get() <= open(&host).viewport.max());
        assert_eq!(open(&host).row, selected);
        assert_eq!(open(&host).menu, original);
        assert_modal_hits_inside(&host, size);
    }
    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn tiny_terminals_do_not_expose_hidden_actions_and_escape_still_closes() {
    for size in [(1, 1), (2, 3), (8, 4), (20, 4)] {
        for menu in [
            Menu::confirm("ask", "keep the item?", "keep", "leave"),
            Menu::new(
                "pick",
                "choose an item",
                vec![MenuItem::new('a', "keep", "", "kept")],
            ),
            Menu::typed("name", "name the item", "kept text"),
        ] {
            let (mut host, log) = fixture(menu, None);
            let _ = draw(&host, size);
            assert_modal_hits_inside(&host, size);
            assert!(
                !open(&host).viewport.usable.get(),
                "{size:?} cannot show this menu safely"
            );
            assert!(
                !host
                    .hits
                    .borrow()
                    .iter()
                    .any(|(_, h)| matches!(h, Hit::MenuItem(_)))
            );
            press(&mut host, Key::Enter);
            assert!(host.menu.is_some(), "a hidden choice must not be submitted");
            if open(&host).input.is_none() {
                press(&mut host, Key::Char('y'));
                press(&mut host, Key::Char('a'));
            }
            assert!(host.menu.is_some());
            assert!(log.lock().unwrap().is_empty());
            press(&mut host, Key::Esc);
            assert!(host.menu.is_none());
        }
    }
}

#[test]
fn paging_and_wheeling_a_write_confirmation_emit_no_effects_and_keep_edit_selected() {
    let text = (0..36)
        .map(|n| format!("note {n:02}: keep this text while reading"))
        .collect::<Vec<_>>()
        .join("\n");
    let menu =
        Menu::write_confirm("review", "keep these notes?", &text, "keep", "leave").at("edit");
    let (mut host, log) = fixture(menu.clone(), None);
    let size = (80, 24);
    let _ = draw(&host, size);
    let selected = open(&host).row;
    assert_eq!(open(&host).menu.items[selected].value, "edit");
    assert!(open(&host).viewport.more());

    assert!(
        host.update(crate::host::chord_event(Chord::new(Key::PageDown)))
        .is_empty(),
        "paging only changes the reading position"
    );
    let _ = draw(&host, size);
    let after_page = open(&host).viewport.at.get();
    assert!(after_page > 0);
    assert_eq!(open(&host).row, selected);

    let rect = menu_rect(&host);
    assert!(
        host.update(Event::Mouse {
            kind: MouseKind::ScrollDown,
            column: rect.x + rect.width / 2,
            row: rect.y + rect.height / 2,
            at_ms: 100,
        })
        .is_empty(),
        "the wheel only changes the reading position"
    );
    let _ = draw(&host, size);
    assert!(open(&host).viewport.at.get() > after_page);
    assert_eq!(open(&host).row, selected);
    assert_eq!(open(&host).menu, menu);
    assert!(log.lock().unwrap().is_empty());

    press(&mut host, Key::Enter);
    assert!(host.menu.is_none());
    assert_choice(&log, "review", "edit");
}
