//! Page-state and keyboard regressions; every service result is injected.

use super::*;
use crate::Via;
use crate::client::Recorded;
use crate::demo::{self, BOB, NOW, TEAM};
use radar_core::host::{Event, HostEffect};
use radar_core::view::Intent;
use crate::keys::Mods;
use crate::tests::shown;
use radar_core::{Host, Plugin};

const A: &str = "1791200000.000100";
const B: &str = "1791200001.000100";
const URL: &str = "https://example.com/first";

fn sample() -> SlackPlugin {
    let mut p = demo::sample();
    p.via = Via(Some(Recorded::new(vec![])));
    p
}

/// A pointer press on a target of the page view.
fn click(p: &mut SlackPlugin, target: &str) {
    p.intent(
        crate::SCREEN.id,
        Pane::Middle,
        &radar_core::view::Intent::Select {
            target: radar_core::view::Target::new(target),
        },
    );
}

#[test]
fn actions_are_invoked_by_their_stable_ids_through_headless() {
    use radar_core::headless::Headless;
    use radar_core::view::{Intent, PaneId};
    let mut headless = Headless::new(host(sample()), 160, 40);
    headless.focus(PaneId::Middle);
    let ids: Vec<String> = headless.screen().unwrap().actions.iter().map(|a| a.id.clone()).collect();
    let runnable: Vec<&str> = ids.iter().map(String::as_str).filter(|id| !id.is_empty()).collect();
    assert!(runnable.contains(&"slack.feed.filter"), "{ids:?}");
    // The agent actions carry the host's shared ids (`agent.send`), the same on every part.
    assert!(runnable.iter().all(|id| id.starts_with("slack.feed.") || id.starts_with("agent.")), "{ids:?}");
    let mut sorted = runnable.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), runnable.len(), "no id repeats: {ids:?}");
    assert!(slack(headless.host()).filter_input.is_none());
    headless.send(Intent::Invoke { action: "slack.feed.filter".into() });
    assert!(slack(headless.host()).filter_input.is_some(), "the id ran the filter row");
    // An id nobody listed does nothing.
    headless.send(Intent::Invoke { action: "slack.feed.nothing".into() });
}

fn host(p: SlackPlugin) -> Host {
    let mut host = Host::new(vec![Box::new(SlackPlugin::new())], NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    host
}

fn slack(host: &Host) -> &SlackPlugin {
    assert_eq!(host.plugins[0].id(), "slack");
    // SAFETY: this fixture installs exactly one SlackPlugin.
    unsafe { &*(host.plugins[0].as_ref() as *const dyn Plugin as *const SlackPlugin) }
}

fn slack_mut(host: &mut Host) -> &mut SlackPlugin {
    assert_eq!(host.plugins[0].id(), "slack");
    // SAFETY: this fixture installs exactly one SlackPlugin, exclusively borrowed.
    unsafe { &mut *(host.plugins[0].as_mut() as *mut dyn Plugin as *mut SlackPlugin) }
}

/// Deliberately does not draw before the event: pending redraws are part of the test.
fn press(host: &mut Host, code: Key) -> Vec<HostEffect> {
    host.update(crate::keys::ev(crate::keys::chord(code, Mods::NONE)))
}

fn page(p: &mut SlackPlugin, ts: &str, words: Option<&str>) {
    if let Some(words) = words {
        on_thread(p, TEAM, ts, Ok(vec![demo::msg(ts, BOB, words)]));
    }
    p.pages.push(Page::Thread {
        channel: TEAM.into(),
        ts: ts.into(),
        tab: 0,
        row: 0,
    });
}

fn no_links(p: &mut SlackPlugin) {
    assert!(page_links(p).is_empty());
    assert!(
        !keys(p)
            .iter()
            .any(|b| matches!(b.keys.as_str(), "⏎" | "o" | "y"))
    );
    for code in [Key::Enter, Key::Char('o'), Key::Char('y')] {
        let out = key(p, crate::keys::chord(code, Mods::NONE));
        if let Outcome::Handled(effects) = out {
            assert!(
                !effects
                    .iter()
                    .any(|e| matches!(e, Effect::Show(_) | Effect::Open(_) | Effect::Copy(_)))
            );
        }
    }
}

#[test]
fn cold_failure_empty_and_retry_have_truthful_states() {
    let mut p = sample();
    page(&mut p, A, None);
    p.threads.insert(
        thread_key(TEAM, A),
        Fetched {
            loading: true,
            ..Default::default()
        },
    );
    let mut h = host(p);
    assert!(shown(&h, 80, 24).contains("loading…"));
    on_thread(slack_mut(&mut h), TEAM, A, Err("thread_not_found".into()));
    let wide = shown(&h, 160, 40);
    assert!(
        wide.contains("could not load: thread_not_found") && wide.contains("Press r to try again")
    );
    assert!(!wide.contains("loading…"));
    insta::assert_snapshot!("slack_thread_failed", wide);
    let retry = press(&mut h, Key::Char('r'));
    assert_eq!(
        retry
            .iter()
            .filter(|e| matches!(e, HostEffect::Spawn { .. }))
            .count(),
        1
    );
    let repeat = press(&mut h, Key::Char('r'));
    assert!(!repeat.iter().any(|e| matches!(e, HostEffect::Spawn { .. })));
    assert!(shown(&h, 80, 24).contains("loading…"));
    assert!(!shown(&h, 80, 24).contains("thread_not_found"));
    drop(retry); // No task is run, even against the empty recording.
    on_thread(slack_mut(&mut h), TEAM, A, Ok(vec![]));
    // The empty-state snapshot is independent of the duplicate-key notice.
    h.message = None;
    let empty = shown(&h, 80, 24);
    assert!(empty.contains("Slack returned no messages") && !empty.contains("loading…"));
    insta::assert_snapshot!("slack_thread_empty_80x24", empty);
    on_thread(
        slack_mut(&mut h),
        TEAM,
        A,
        Ok(vec![demo::msg(A, BOB, "The retry worked.")]),
    );
    assert!(shown(&h, 80, 24).contains("The retry worked."));
    assert!(!slack(&h).busy(crate::SCREEN.id));
}

#[test]
fn cached_reply_failure_never_claims_it_is_still_reading() {
    let mut p = sample();
    let ts = demo::ago(2 * 86_400 + 3600);
    p.data.channels.get_mut(TEAM).unwrap().replies.remove(&ts);
    p.pages.push(Page::Thread {
        channel: TEAM.into(),
        ts: ts.clone(),
        tab: 0,
        row: 0,
    });
    on_thread(&mut p, TEAM, &ts, Err("missing_scope".into()));
    let mut h = host(p);
    let text = shown(&h, 160, 40);
    assert!(text.contains("missing_scope") && text.contains("2 replies aren't loaded"));
    assert!(!text.contains("are being read"));
    press(&mut h, Key::Char(']'));
    let text = shown(&h, 160, 40);
    assert!(text.contains("missing_scope") && text.contains("Press r to try again"));
    assert!(
        text.contains(demo::MR_263),
        "cached links remain visible: {text}"
    );
}

#[test]
fn links_cannot_cross_threads_even_before_a_redraw_or_after_back_and_reentry() {
    let mut p = sample();
    page(&mut p, A, Some(&format!("<{URL}|first>")));
    let mut h = host(p);
    shown(&h, 160, 40);
    assert_eq!(page_links(slack(&h)).len(), 1);
    for _ in 0..2 {
        // Simulate a page push without clearing the derived cache: identity alone
        // must still refuse A's links before the next frame.
        page(slack_mut(&mut h), B, None);
        show_tab(slack_mut(&mut h), 1);
        no_links(slack_mut(&mut h));
        let text = shown(&h, 80, 24);
        assert!(!text.contains(URL));
        no_links(slack_mut(&mut h));
        press(&mut h, Key::Char('x'));
        assert!(
            !h.menu
                .as_ref()
                .unwrap()
                .menu
                .items
                .iter()
                .any(|i| i.label.contains("copy it"))
        );
        press(&mut h, Key::Esc);
        assert_eq!(slack(&h).pages.len(), 2, "dismiss only the menu");
        press(&mut h, Key::Esc);
        assert_eq!(slack(&h).pages.len(), 1);
        assert!(
            page_links(slack(&h)).is_empty(),
            "back waits for A's redraw"
        );
        shown(&h, 160, 40);
        assert_eq!(page_links(slack(&h)).len(), 1);
    }
}

#[test]
fn refreshed_links_are_invalid_until_rendered_then_cursor_and_actions_agree() {
    let mut p = sample();
    page(
        &mut p,
        A,
        Some(
            "<https://example.com/one|one> <https://example.com/two|two> <https://example.com/three|three>",
        ),
    );
    show_tab(&mut p, 1);
    let mut h = host(p);
    shown(&h, 160, 40);
    click(slack_mut(&mut h), "link:2");
    on_thread(
        slack_mut(&mut h),
        TEAM,
        A,
        Ok(vec![demo::msg(
            A,
            BOB,
            "<https://example.com/remaining|remaining>",
        )]),
    );
    assert!(
        page_links(slack(&h)).is_empty(),
        "old rows are unavailable before redraw"
    );
    assert!(!keys(slack(&h)).iter().any(|b| b.keys == "y"));
    let text = shown(&h, 80, 24);
    assert!(text.contains("remaining"));
    let copied = key(
        slack_mut(&mut h),
        crate::keys::chord(Key::Char('y'), Mods::NONE),
    );
    assert!(
        matches!(copied, Outcome::Handled(fx) if fx.iter().any(|e| matches!(e, Effect::Copy(s) if s == "https://example.com/remaining")))
    );
    assert!(matches!(
        slack(&h).pages.last(),
        Some(Page::Thread { row: 0, .. })
    ));
    for _ in 0..2 {
        click(slack_mut(&mut h), "link:99");
        assert!(matches!(
            slack(&h).pages.last(),
            Some(Page::Thread { row: 0, .. })
        ));
        click(slack_mut(&mut h), "tab:0");
        click(slack_mut(&mut h), "tab:1");
    }
    on_thread(
        slack_mut(&mut h),
        TEAM,
        A,
        Ok(vec![demo::msg(A, BOB, "No links left.")]),
    );
    no_links(slack_mut(&mut h));
    let empty = shown(&h, 80, 24);
    assert!(empty.contains("No links in these messages. Your browser gets a breather."));
    insta::assert_snapshot!("slack_empty_links_80x24", empty);
    no_links(slack_mut(&mut h));
    assert_eq!(page_tabs(slack(&h)).unwrap().names[1], "Links");
    slack_mut(&mut h)
        .threads
        .get_mut(&thread_key(TEAM, A))
        .unwrap()
        .loading = true;
    assert!(
        !shown(&h, 80, 24).contains("breather"),
        "no joke during a read"
    );
    on_thread(slack_mut(&mut h), TEAM, A, Err("missing_scope".into()));
    assert!(
        !shown(&h, 80, 24).contains("breather"),
        "no joke on an error"
    );
}

#[test]
fn rendered_links_do_not_survive_a_connection_revision() {
    let mut p = sample();
    page(&mut p, A, Some(&format!("<{URL}|first>")));
    show_tab(&mut p, 1);
    let mut h = host(p);
    shown(&h, 160, 40);
    slack_mut(&mut h).replace_token("env:RADAR_POLISH_UNSET_TOKEN".into());
    no_links(slack_mut(&mut h));
}

#[test]
fn back_words_name_the_actual_list_or_previous_page() {
    for (nav, expected) in [(0, "For you"), (1, "Discussions"), (2, "#team")] {
        let mut p = sample();
        p.nav_at = nav;
        page(&mut p, A, Some("Parent thread"));
        assert_eq!(back_to(&p), expected);
        page(&mut p, B, Some("Child thread"));
        assert_eq!(back_to(&p), "Bob Builder's thread in #team");
        key(&mut p, crate::keys::chord(Key::Esc, Mods::NONE));
        assert_eq!(back_to(&p), expected);
    }
    let mut p = sample();
    p.pages.push(Page::Draft);
    page(&mut p, A, None);
    assert_eq!(back_to(&p), "the review ask");
}

#[test]
fn committed_filter_escape_clears_before_leaving_the_list() {
    let mut h = host(sample());
    press(&mut h, Key::Char('/'));
    for c in "lunch".chars() {
        press(&mut h, Key::Char(c));
    }
    press(&mut h, Key::Enter);
    assert_eq!(slack(&h).filter, "lunch");
    assert_eq!(h.focus, Pane::Middle);
    press(&mut h, Key::Char('x'));
    press(&mut h, Key::Esc);
    assert_eq!(slack(&h).filter, "lunch", "first Escape only dismisses x");
    press(&mut h, Key::Esc);
    assert!(slack(&h).filter.is_empty());
    assert_eq!(h.focus, Pane::Middle);
    press(&mut h, Key::Esc);
    assert_eq!(h.focus, Pane::Nav);
}

#[test]
fn reply_capture_and_reentry_do_not_leak_page_or_write_shortcuts() {
    let mut p = sample();
    page(&mut p, A, Some("Reply here"));
    let mut h = host(p);
    shown(&h, 160, 40);
    // The Links tab, by the target the page view gives it.
    let tab = Intent::Select { target: radar_core::view::TabView::target(1) };
    assert!(shown(&h, 160, 40).contains(":link: Links"), "the page offers a Links tab");
    for attempt in 0..2 {
        press(&mut h, Key::Char('c'));
        assert!(slack(&h).replying.is_some());
        for c in "oxyernd".chars() {
            let fx = press(&mut h, Key::Char(c));
            assert!(!fx.iter().any(|e| matches!(e, HostEffect::Spawn { .. })));
        }
        assert!(h.menu.is_none());
        assert_eq!(slack(&h).pages.len(), 1);
        for _ in 0..2 {
            let fx = h.update(Event::Intent(tab.clone()));
            assert!(fx.is_empty(), "clicks do not escape the composer");
        }
        assert!(matches!(
            slack(&h).pages.last(),
            Some(Page::Thread { tab: 0, .. })
        ));
        assert!(
            slack(&h)
                .replying
                .as_ref()
                .unwrap()
                .composer
                .text()
                .contains("oxyernd")
        );
        press(&mut h, Key::Esc);
        assert!(h.menu.is_some(), "nonempty text asks before dropping");
        press(&mut h, Key::Esc);
        assert!(slack(&h).replying.is_some(), "dismissal keeps the composer");
        press(&mut h, Key::Esc);
        let fx = press(&mut h, Key::Char('y'));
        assert!(!fx.iter().any(|e| matches!(e, HostEffect::Spawn { .. })));
        assert_eq!(
            slack(&h).pages.len(),
            1,
            "drop text, not the page, attempt {attempt}"
        );
        assert!(slack(&h).replying.is_none());
    }
}

#[test]
fn rapid_repeat_tab_clicks_do_not_open_a_link_before_redraw() {
    let mut p = sample();
    page(&mut p, A, Some(&format!("<{URL}|first>")));
    let mut h = host(p);
    shown(&h, 160, 40);
    // The Links tab, by the target the page view gives it.
    let tab = Intent::Select { target: radar_core::view::TabView::target(1) };
    assert!(shown(&h, 160, 40).contains(":link: Links"), "the page offers a Links tab");
    fn opens(effect: &HostEffect) -> bool {
        match effect {
            HostEffect::Open(_) | HostEffect::OpenApp { .. } => true,
            HostEffect::Tracked { effect, .. } => opens(effect),
            _ => false,
        }
    }
    for _ in 0..2 {
        let effects = h.update(Event::Intent(tab.clone()));
        assert!(
            !effects.iter().any(opens),
            "a repeated tab click must not become Enter: {effects:?}"
        );
        assert!(matches!(
            slack(&h).pages.last(),
            Some(Page::Thread { tab: 1, .. })
        ));
    }
}

/// A watch merge request for a review draft.
fn ask_item(iid: &str, words: &str) -> crate::ask::Item {
    crate::ask::Item {
        id: format!("watch!{iid}"),
        url: format!("https://git.example/ankh-morpork/watch/-/merge_requests/{iid}"),
        label: format!("watch !{iid}"),
        words: words.into(),
        tickets: vec![],
    }
}

#[test]
fn a_press_on_another_item_leaves_the_open_edit_in_its_own_item() {
    use radar_core::headless::Headless;
    use radar_core::view::{Chord, Intent, Key, PaneId, Target};
    let mut p = sample();
    p.begin_ask_draft(crate::ask::Draft::new(&p.cfg, ask_item("12", "first")));
    assert!(p.draft.as_mut().unwrap().add(ask_item("13", "second")));
    p.pages = vec![Page::Draft];
    let mut headless = Headless::new(host(p), 160, 40);
    headless.focus(PaneId::Middle);
    headless.send(Intent::Key { chord: Chord::char('e') });
    headless.send(Intent::Key { chord: Chord::char('x') });
    let draft = |h: &Headless| {
        let d = slack(h.host()).draft.as_ref().expect("the draft");
        (d.row, d.edit.as_ref().map(|e| e.value()), d.items[0].words.clone(), d.items[1].words.clone())
    };
    let editing = (0, Some("firstx".to_string()), "first".to_string(), "second".to_string());
    assert_eq!(draft(&headless), editing);
    let other = Target::new("item:1");
    for intent in [
        Intent::Select { target: other.clone() },
        Intent::Context { target: other.clone() },
        Intent::Activate { target: other.clone() },
    ] {
        headless.send(intent.clone());
        assert_eq!(draft(&headless), editing, "{intent:?} waits for the edit");
        assert!(headless.host().menu.is_none(), "{intent:?} opens no menu over the edit");
    }
    // ⏎ commits the edit into the item it was opened on.
    headless.send(Intent::Key { chord: Chord::new(Key::Enter) });
    assert_eq!(draft(&headless), (0, None, "firstx".to_string(), "second".to_string()));
}

#[test]
fn a_press_on_a_feed_row_leaves_the_selection_while_the_filter_is_typed() {
    use radar_core::headless::Headless;
    use radar_core::view::{Intent, PaneId, TreeRow};
    let mut headless = Headless::new(host(sample()), 160, 40);
    headless.focus(PaneId::Middle);
    headless.send(Intent::Invoke { action: "slack.feed.filter".into() });
    assert!(slack(headless.host()).filter_input.is_some());
    let before = slack(headless.host()).tree.borrow().selected().to_vec();
    let other = slack(headless.host())
        .shown
        .borrow()
        .keys()
        .find(|path| **path != before)
        .cloned()
        .expect("a row other than the selected one");
    let target = TreeRow::target_of(&other);
    for intent in [
        Intent::Select { target: target.clone() },
        Intent::Context { target: target.clone() },
        Intent::Activate { target: target.clone() },
    ] {
        headless.send(intent.clone());
        assert_eq!(slack(headless.host()).tree.borrow().selected(), before, "{intent:?}");
        assert!(slack(headless.host()).filter_input.is_some(), "{intent:?} leaves the filter open");
        assert!(headless.host().menu.is_none());
    }
}
