use super::fixture::{self, ASK, PAGE, SYNC};
use super::*;
use crate::headless::Headless;
use crate::host::Event;
use crate::view::{Chord, Intent};

fn hello(protocol: u32) -> ToHost {
    ToHost::Hello {
        protocol,
        view: crate::view::VERSION,
        capabilities: Capabilities {
            columns: 80,
            rows: 24,
            mouse: true,
            true_colour: true,
            image: ImageProtocol::None,
            plain_text: false,
        },
    }
}

fn intent(seq: u64, intent: Intent) -> ToHost {
    ToHost::Intent {
        seq,
        cursor: 0,
        intent,
    }
}

fn invoke(action: &str) -> Intent {
    Intent::Invoke {
        action: action.into(),
    }
}

#[test]
fn the_bridge_names_no_terminal_library() {
    let source = include_str!("bridge.rs");
    for word in ["ratatui::", "crossterm", "Buffer", "Rect", "KeyEvent"] {
        let used = source
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .any(|l| l.contains(word));
        assert!(!used, "bridge.rs uses {word}");
    }
}

/// C2(f): the same intents through Headless and through the bridge record
/// the same actions with the same causal parents, and ask the same
/// question.
#[test]
fn an_intent_through_the_bridge_records_what_headless_records() {
    let mut headless = Headless::new(fixture::host(), 80, 24);
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    bridge.receive(hello(PROTOCOL));

    headless.send(invoke(ASK));
    bridge.receive(intent(1, invoke(ASK)));
    let asked = bridge.host().menu_view().expect("the bridge's host asks");
    assert_eq!(asked.title, "save it?");
    assert_eq!(
        headless.host().menu_view().map(|m| m.title),
        Some(asked.title.clone()),
        "the same question"
    );
    headless.send(invoke("yes"));
    bridge.receive(intent(2, invoke("yes")));

    let (a, b) = (fixture::shape(headless.records()), fixture::shape(bridge.records()));
    assert!(a.iter().any(|l| l.contains(ASK)), "{a:#?}");
    assert!(a.iter().any(|l| l.contains("probe.items.save")), "{a:#?}");
    assert_eq!(a, b, "the same journal actions with the same causality");
}

#[test]
fn a_renderer_of_another_protocol_is_refused_with_words_to_show() {
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    bridge.receive(hello(PROTOCOL + 1));
    let out = bridge.outgoing();
    let [ToRenderer::Refuse { wanted, got, message }] = out.as_slice() else {
        panic!("{out:?}")
    };
    assert_eq!((*wanted, *got), (PROTOCOL, PROTOCOL + 1));
    assert!(message.contains("RADAR_RENDERER=ratatui"), "{message}");
    // Nothing it sends is acted on.
    let fx = bridge.receive(intent(1, invoke(ASK)));
    assert!(fx.effects.is_empty() && bridge.host().menu.is_none());
}

#[test]
fn a_renderer_reading_another_view_version_is_refused() {
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    let ToHost::Hello { capabilities, .. } = hello(PROTOCOL) else {
        panic!()
    };
    bridge.receive(ToHost::Hello {
        protocol: PROTOCOL,
        view: crate::view::VERSION + 1,
        capabilities,
    });
    let out = bridge.outgoing();
    let [ToRenderer::Refuse { message, .. }] = out.as_slice() else {
        panic!("{out:?}")
    };
    assert!(message.contains("views of version"), "{message}");
}

#[test]
fn a_notice_is_cut_to_its_bound() {
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    bridge.receive(hello(PROTOCOL));
    let _ = bridge.outgoing();
    bridge.update(Event::Notice("é".repeat(MAX_NOTICE + 10)));
    let out = bridge.outgoing();
    let text = out
        .iter()
        .find_map(|m| match m {
            ToRenderer::Notice(n) => Some(n.text.clone()),
            _ => None,
        })
        .expect("a notice");
    assert_eq!(text.chars().count(), MAX_NOTICE);
}

fn snapshots(out: &[ToRenderer]) -> Vec<&Snapshot> {
    out.iter()
        .filter_map(|m| match m {
            ToRenderer::Snapshot(s) => Some(&**s),
            _ => None,
        })
        .collect()
}

/// C2(c): a renderer that acknowledges late is sent one snapshot at a
/// time, the newest state when it catches up, and every question and
/// notice as it happens.
#[test]
fn a_slow_renderer_sees_every_question_and_notice_while_snapshots_coalesce() {
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    bridge.receive(hello(PROTOCOL));
    let first = bridge.outgoing();
    assert!(matches!(first[0], ToRenderer::Welcome { .. }), "{first:?}");
    let shown = snapshots(&first);
    assert_eq!(shown.len(), 1);
    let unacked = shown[0].cursor;

    // Progress while the renderer is busy: no second snapshot.
    let mut sent = Vec::new();
    for n in 0..5 {
        bridge.update(Event::Tick(n));
        bridge.update(Event::Notice(format!("step {n}")));
        sent.extend(bridge.outgoing());
    }
    bridge.receive(intent(1, invoke(ASK)));
    sent.extend(bridge.outgoing());
    assert!(snapshots(&sent).is_empty(), "coalesced while one is in flight");
    let notices: Vec<_> = sent
        .iter()
        .filter_map(|m| match m {
            ToRenderer::Notice(n) => Some(n.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(notices, ["step 0", "step 1", "step 2", "step 3", "step 4"]);
    let questions: Vec<_> = sent
        .iter()
        .filter_map(|m| match m {
            ToRenderer::Question(q) => Some(q),
            _ => None,
        })
        .collect();
    assert_eq!(questions.len(), 1, "{sent:?}");
    assert_eq!(questions[0].menu.title, "save it?");

    // It catches up: one snapshot, the newest state, the question in it.
    bridge.receive(ToHost::Ack { cursor: unacked });
    let caught = bridge.outgoing();
    let shown = snapshots(&caught);
    assert_eq!(shown.len(), 1, "{caught:?}");
    assert!(shown[0].cursor > questions[0].cursor);
    assert_eq!(
        shown[0].chrome.menu.as_ref().map(|m| m.title.as_str()),
        Some("save it?")
    );
    // Nothing changed since: nothing more to send.
    bridge.receive(ToHost::Ack {
        cursor: shown[0].cursor,
    });
    assert!(bridge.outgoing().is_empty());
}

#[test]
fn a_restarted_renderer_is_told_everything_again() {
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    bridge.receive(hello(PROTOCOL));
    bridge.receive(intent(1, invoke(ASK)));
    let _ = bridge.outgoing();
    bridge.reconnect();
    assert!(bridge.outgoing().is_empty(), "nothing before its hello");
    bridge.receive(hello(PROTOCOL));
    let again = bridge.outgoing();
    assert!(again.iter().any(|m| matches!(m, ToRenderer::Question(_))), "{again:?}");
    assert_eq!(snapshots(&again).len(), 1);
}

/// T8: the renderer has no highlighter, so a diff is sent painted the way
/// Ratatui's `paint` feeds its highlighters.
#[test]
fn a_diff_is_painted_for_the_renderer() {
    use crate::ui::diff::{DiffLine, Kind};
    use crate::view::{Node, PaintRun};
    let line = |kind, old, new, text: &str| DiffLine { kind, old, new, text: text.into() };
    let mut node = Node::Diff {
        lines: vec![
            line(Kind::Hunk, None, None, "@@ -1,2 +1,2 @@"),
            line(Kind::Context, Some(1), Some(1), "/* open"),
            line(Kind::Removed, Some(2), None, "still open */ let x = 1;"),
            line(Kind::Added, None, Some(2), "\tlet y = 2;"),
        ],
        under: vec![],
        selected: 0,
        top: 0,
        split: false,
        language: Some("rs".into()),
        empty: String::new(),
        painted: vec![],
        words: vec![],
    };
    node.paint_diffs();
    let Node::Diff { painted, .. } = &node else { unreachable!() };
    assert_eq!(painted.len(), 4);
    assert!(painted[0].is_empty(), "a hunk heading has no runs");
    let text = |l: &Vec<PaintRun>| l.iter().map(|r| r.text.as_str()).collect::<String>();
    assert_eq!(text(&painted[2]), "still open */ let x = 1;");
    assert_eq!(text(&painted[3]), "    let y = 2;", "tabs are four spaces, as Ratatui paints them");
    // The context line opened a comment on both sides; the removed line
    // closed it on the old side only, so the added line is still inside it.
    assert_eq!(painted[2][0].token, "comment");
    assert!(painted[2].iter().any(|r| r.token == "keyword" && r.text == "let"), "{:?}", painted[2]);
    assert_eq!(painted[3], [PaintRun { text: "    let y = 2;".into(), token: "comment".into() }]);
    // An unknown language sends nothing.
    if let Node::Diff { language, .. } = &mut node {
        *language = Some("no-such-language".into());
    }
    node.paint_diffs();
    let Node::Diff { painted, .. } = &node else { unreachable!() };
    assert!(painted.is_empty());
}

/// The words that changed in a replaced pair are sent with the diff, in
/// characters, as Ratatui emphasises them; the renderer compares nothing.
#[test]
fn a_diff_is_sent_with_the_words_that_changed() {
    use crate::ui::diff::{DiffLine, Kind};
    use crate::view::Node;
    let line = |kind, old, new, text: &str| DiffLine { kind, old, new, text: text.into() };
    let diff = |lines| Node::Diff {
        lines,
        under: vec![],
        selected: 0,
        top: 0,
        split: false,
        language: None,
        empty: String::new(),
        painted: vec![],
        words: vec![],
    };
    let mut node = diff(vec![
        line(Kind::Context, Some(1), Some(1), "fn main() {"),
        line(Kind::Removed, Some(2), None, "\tlet café = old_value;"),
        line(Kind::Added, None, Some(2), "\tlet café = new_value;"),
    ]);
    node.paint_diffs();
    let Node::Diff { words, .. } = &node else { unreachable!() };
    // "    let café = " is 15 characters (17 bytes); the changed word follows it.
    assert_eq!(words, &vec![vec![], vec![(15, 24)], vec![(15, 24)]]);
    // No pair, no words.
    let mut node = diff(vec![line(Kind::Added, None, Some(1), "new")]);
    node.paint_diffs();
    let Node::Diff { words, .. } = &node else { unreachable!() };
    assert!(words.is_empty());
}

/// T6: a context menu the renderer's right-click opens is anchored where
/// the pointer pressed, as the Ratatui runner anchors one.
#[test]
fn a_context_menu_opens_where_the_pointer_pressed() {
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    bridge.receive(hello(PROTOCOL));
    let _ = bridge.outgoing();
    let context = || Intent::Context { target: crate::view::Target::new("row") };
    // Without a pointer, the menu has no anchor.
    bridge.receive(intent(1, context()));
    assert_eq!(bridge.host().menu.as_ref().map(|m| m.anchor), Some(None));
    bridge.host_mut().menu = None;
    // With one, it opens there; and the pointer is spent by that intent.
    bridge.receive(ToHost::Pointer { column: 60, row: 5 });
    bridge.receive(intent(2, context()));
    assert_eq!(bridge.host().menu.as_ref().and_then(|m| m.anchor), Some((60, 5).into()));
    assert_eq!(bridge.host().menu_view().and_then(|m| m.at), Some(crate::view::chrome::CellAt { column: 60, row: 5 }));
    bridge.host_mut().menu = None;
    bridge.receive(intent(3, context()));
    assert_eq!(bridge.host().menu.as_ref().map(|m| m.anchor), Some(None));
}

/// #32: what the renderer laid out reaches the part's scroll, so PgDn
/// pages it by the rows the renderer has; where the part moved it is sent
/// back as where to scroll, and the renderer's own position is not.
#[test]
fn the_rows_a_renderer_laid_out_reach_the_parts_scroll_and_its_page_comes_back() {
    use crate::plugin::Pane;
    use crate::view::{Chord, Key, PaneId, PaneRows};
    let rows = |scope: &str, at| ToHost::Rows {
        scope: scope.into(),
        panes: vec![PaneRows { pane: PaneId::Middle, rows: 10, content: 25, at }],
    };
    let scroll = |b: &Bridge| {
        let s = b.host().plugins[0].scroll("items", Pane::Middle).unwrap();
        (s.at(), s.max(), s.height())
    };
    let told = |out: &[ToRenderer]| {
        snapshots(out).last().and_then(|s| s.screen.as_ref()).and_then(|s| s.pane(PaneId::Middle)).map(|p| p.scroll)
    };
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    // Before the hello nothing is taken.
    bridge.receive(rows("items", 0));
    assert_eq!(bridge.host().pane_rows("items", PaneId::Middle), None);
    bridge.receive(hello(PROTOCOL));
    let out = bridge.outgoing();
    settle(&mut bridge, &out);
    // Nothing laid out: PgDn has nowhere to go.
    let pgdn = || Intent::Key { chord: Chord::new(Key::PageDown) };
    bridge.receive(intent(1, pgdn()));
    assert_eq!(scroll(&bridge), (0, 0, 0));
    let out = bridge.outgoing();
    settle(&mut bridge, &out);
    bridge.receive(rows("items", 3));
    assert_eq!(bridge.host().pane_rows("items", PaneId::Middle).map(|r| (r.rows, r.content, r.at)), Some((10, 25, 3)));
    assert_eq!(scroll(&bridge), (3, 15, 10));
    let out = bridge.outgoing();
    assert_eq!(told(&out).flatten(), None, "the renderer's own place is not sent back: {out:?}");
    settle(&mut bridge, &out);
    // A page keeps one row of the last.
    bridge.receive(intent(2, pgdn()));
    assert_eq!(scroll(&bridge).0, 12);
    let out = bridge.outgoing();
    assert_eq!(told(&out), Some(Some(12)), "the renderer is told where");
    settle(&mut bridge, &out);
    bridge.receive(rows("items", 12));
    let out = bridge.outgoing();
    assert_eq!(told(&out), Some(None));
    settle(&mut bridge, &out);
    // A report about a screen not showing changes nothing.
    bridge.receive(rows("elsewhere", 0));
    assert_eq!(scroll(&bridge).0, 12);
    assert_eq!(bridge.host().pane_rows("items", PaneId::Middle).map(|r| r.at), Some(12));
}

#[test]
fn a_rows_report_is_cut_to_its_bounds() {
    use crate::view::{PaneId, PaneRows};
    let pane = PaneRows { pane: PaneId::Detail, rows: 1, content: 2, at: 0 };
    let long = ToHost::Rows { scope: "s".repeat(MAX_SCOPE + 5), panes: vec![pane; MAX_PANES + 3] };
    let ToHost::Rows { scope, panes } = long.bounded() else { panic!() };
    assert_eq!(scope.chars().count(), MAX_SCOPE);
    assert_eq!(panes.len(), MAX_PANES);
}

/// T2: a part that is not set up shows the host's set-up page, as the
/// Ratatui shell draws it, not the part's own screen.
#[test]
fn a_part_not_set_up_is_sent_as_its_set_up_page() {
    let mut bridge = Bridge::new(fixture::unready_host(), 80, 24);
    bridge.receive(hello(PROTOCOL));
    let screen = bridge.snapshot_now(1).screen.expect("a screen");
    assert_eq!(screen.panes.len(), 1);
    assert_eq!(screen.panes[0].title, "set up probe");
    let crate::view::Node::Document { lines, .. } = &screen.panes[0].body else { panic!("{:?}", screen.panes[0].body) };
    let words: Vec<String> = lines.iter().map(|l| l.words()).collect();
    assert_eq!(words[0], "probe is not set up yet.");
    assert_eq!(words[2], "Add a tenancy.");
    // Set up, the part's own screen shows.
    let ready = Bridge::new(fixture::host(), 80, 24).snapshot_now(1).screen.unwrap();
    assert_eq!(ready.panes[0].title, "items");
}

/// T20: views are built for the icon column the theme draws, as the Ratatui
/// shell sets it before every view; under Ink no frame is drawn to set it.
#[test]
fn views_are_built_for_the_icon_column_of_the_theme() {
    let mut host = fixture::host();
    host.colours.cfg.icons = "nerd".into();
    let want = crate::ui::icon::lead_width(&host.build_theme(crate::design::theme::Tier::TrueColor));
    assert!(want > 0, "the nerd pack leads icons");
    let mut bridge = Bridge::new(host, 80, 24);
    bridge.receive(hello(PROTOCOL));
    assert_eq!(bridge.host().frontend.lead(), 0);
    let _ = bridge.snapshot_now(1);
    assert_eq!(bridge.host().frontend.lead(), want);
}

fn motion_of(out: &[ToRenderer]) -> Vec<MotionView> {
    snapshots(out).into_iter().map(|s| s.motion.clone()).collect()
}

/// A bridge whose motion is not held back (not over SSH, full colour).
fn motion_bridge(level: &str) -> Bridge {
    let mut host = fixture::host();
    host.colours.cfg.motion = level.into();
    let mut bridge = Bridge::new(host, 80, 24);
    bridge.set_ssh(false);
    bridge.receive(hello(PROTOCOL));
    let first = bridge.outgoing();
    settle(&mut bridge, &first);
    bridge
}

fn settle(bridge: &mut Bridge, out: &[ToRenderer]) {
    if let Some(s) = snapshots(out).last() {
        bridge.receive(ToHost::Ack { cursor: s.cursor });
    }
}

/// T28: a screen switch gives one switch cue with the body region.
#[test]
fn a_screen_switch_sends_one_screen_cue_for_the_body() {
    let mut bridge = motion_bridge("subtle");
    bridge.update(crate::host::chord_event(Chord::char(crate::host::SETTINGS_KEY)));
    let out = bridge.outgoing();
    let motion = motion_of(&out);
    assert_eq!(motion.len(), 1, "{out:?}");
    assert_eq!(motion[0].level, MotionLevel::Subtle);
    assert_eq!(motion[0].cues, [MotionCue::Screen { region: MotionRegion::Body }]);
    settle(&mut bridge, &out);
    assert!(bridge.outgoing().is_empty(), "the cue is sent once");
}

/// T28: a sync that changed rows gives a Changed cue with their keys.
#[test]
fn a_sync_that_changes_rows_sends_a_changed_cue_with_their_keys() {
    let mut bridge = motion_bridge("subtle");
    bridge.send(invoke(SYNC));
    let out = bridge.outgoing();
    let motion = motion_of(&out);
    assert_eq!(
        motion[0].cues,
        [MotionCue::Changed {
            changed: vec!["probe:1".into(), "probe:2".into()],
            added: vec!["probe:3".into()],
        }]
    );
}

/// T28: cues raised while a snapshot is in flight arrive in the next
/// one, in order.
#[test]
fn cues_wait_for_the_snapshot_in_flight_and_arrive_in_order() {
    let mut bridge = motion_bridge("full");
    bridge.send(invoke(SYNC));
    let first = bridge.outgoing();
    assert!(matches!(motion_of(&first)[0].cues.as_slice(), [MotionCue::Changed { .. }]));
    bridge.send(invoke(ASK));
    assert!(snapshots(&bridge.outgoing()).is_empty(), "one in flight");
    settle(&mut bridge, &first);
    let next = bridge.outgoing();
    assert_eq!(
        motion_of(&next)[0].cues,
        [MotionCue::Popup { region: MotionRegion::Popup }]
    );
}

/// T28: motion off still sends the level, and no cues.
#[test]
fn motion_off_sends_the_level_and_no_cues() {
    let mut bridge = motion_bridge("off");
    bridge.update(crate::host::chord_event(Chord::char(crate::host::SETTINGS_KEY)));
    let out = bridge.outgoing();
    let motion = motion_of(&out);
    assert_eq!(motion.len(), 1);
    assert_eq!(motion[0].level, MotionLevel::Off);
    assert!(motion[0].cues.is_empty());
    assert_eq!(motion[0].held_back, None);
}

/// T28: over SSH the level is off and says why, as the Ratatui runner has it.
#[test]
fn over_ssh_motion_is_held_back_and_says_why() {
    let mut host = fixture::host();
    host.colours.cfg.motion = "full".into();
    let mut bridge = Bridge::new(host, 80, 24);
    bridge.set_ssh(true);
    bridge.receive(hello(PROTOCOL));
    let out = bridge.outgoing();
    let m = &motion_of(&out)[0];
    assert_eq!(m.level, MotionLevel::Off);
    assert_eq!(m.held_back.as_deref(), Some("over SSH"));
    assert_eq!(m.timings.screen, crate::motion::SCREEN_MS);
}

fn key(code: crate::view::Key) -> Event {
    crate::host::chord_event(Chord::new(code))
}

/// T28: a menu appearing is one popup cue for the popup region; moving in
/// it sends none.
#[test]
fn a_menu_sends_one_popup_cue_and_moving_in_it_none() {
    use crate::view::Key as Key;
    let mut bridge = motion_bridge("subtle");
    bridge.send(invoke(ASK));
    let out = bridge.outgoing();
    assert_eq!(
        motion_of(&out)[0].cues,
        [MotionCue::Popup { region: MotionRegion::Popup }]
    );
    settle(&mut bridge, &out);
    bridge.update(key(Key::Down));
    let moved = bridge.outgoing();
    assert!(motion_of(&moved).iter().all(|m| m.cues.is_empty()), "{moved:?}");
}

/// T28: a message appearing is a message cue, and leaving a message-gone
/// cue, both for the message region, each once. Message cues run at full.
#[test]
fn a_message_comes_and_goes_as_two_cues_at_full() {
    use crate::view::Key as Key;
    let mut bridge = motion_bridge("full");
    bridge.update(Event::Notice("copied 12 characters".into()));
    let out = bridge.outgoing();
    assert_eq!(
        motion_of(&out)[0].cues,
        [MotionCue::Message { region: MotionRegion::Message }]
    );
    settle(&mut bridge, &out);
    bridge.update(Event::Notice("copied 12 characters".into()));
    let repeated = bridge.outgoing();
    assert_eq!(motion_of(&repeated)[0].cues, [MotionCue::Message { region: MotionRegion::Message }]);
    settle(&mut bridge, &repeated);
    bridge.update(key(Key::Char('a')));
    let retained = bridge.outgoing();
    assert!(motion_of(&retained).iter().all(|m| m.cues.is_empty()));
    settle(&mut bridge, &retained);
    bridge.update(key(Key::Esc));
    let gone = bridge.outgoing();
    assert_eq!(
        motion_of(&gone)[0].cues,
        [MotionCue::MessageGone { region: MotionRegion::Message }]
    );
}

/// T28: at subtle a message shows no motion, as in the Ratatui engine.
#[test]
fn a_message_sends_no_cue_at_subtle() {
    let mut bridge = motion_bridge("subtle");
    bridge.update(Event::Notice("copied 12 characters".into()));
    let out = bridge.outgoing();
    assert!(motion_of(&out)[0].cues.is_empty());
}

/// T28: an item's page opening is one page cue for the body, at full only.
#[test]
fn a_page_opening_sends_one_page_cue_for_the_body_at_full() {
    let mut bridge = motion_bridge("full");
    bridge.send(invoke(PAGE));
    let out = bridge.outgoing();
    assert_eq!(
        motion_of(&out)[0].cues,
        [MotionCue::Page { region: MotionRegion::Body }]
    );
    let mut subtle = motion_bridge("subtle");
    subtle.send(invoke(PAGE));
    assert!(motion_of(&subtle.outgoing()).iter().all(|m| m.cues.is_empty()));
}

/// A bridge at a real start: the splash shows (`show_splash`), on a clock
/// the test moves.
fn splash_bridge(level: &str) -> Bridge {
    let mut host = fixture::host();
    host.colours.cfg.motion = level.into();
    let mut bridge = Bridge::new(host, 80, 24);
    bridge.set_ssh(false);
    bridge.show_splash();
    bridge.set_clock_ms(0);
    bridge.receive(hello(PROTOCOL));
    bridge
}

/// The splash is sent in the first snapshot, a new frame on each frame
/// tick, and at the end of its hold it hands off: no splash and one screen
/// cue, so Ink fades the first screen in as Ratatui does.
#[test]
fn the_splash_is_sent_frame_by_frame_then_hands_off_with_the_screen_cue() {
    let mut bridge = splash_bridge("full");
    let out = bridge.outgoing();
    let first = snapshots(&out)[0].splash.clone().expect("the splash shows first");
    assert_eq!((first.width, first.height, first.rows.len()), (80, 24, 24));
    assert!(snapshots(&out)[0].screen.is_some(), "the screen under it is sent too: keys act on it");
    assert_eq!(bridge.next_frame(), Some(crate::splash::FRAME));
    settle(&mut bridge, &out);
    bridge.set_clock_ms(200);
    let mid = bridge.outgoing();
    let later = snapshots(&mid)[0].splash.clone().expect("still showing");
    assert_ne!(later, first, "the assemble moved on");
    settle(&mut bridge, &mid);
    bridge.set_clock_ms(crate::splash::HOLD_FULL_MS as u64);
    let end = bridge.outgoing();
    let snap = snapshots(&end)[0];
    assert!(snap.splash.is_none(), "handed off");
    assert_eq!(snap.motion.cues, [MotionCue::Screen { region: MotionRegion::Body }]);
    assert_eq!(bridge.next_frame(), None, "no more frame ticks");
}

/// The splash blocks nothing: what reaches the host while it shows (a
/// sync's result, a tick) is in the very next snapshot with the splash's
/// frame, and frames in between reuse the rest of the snapshot rather
/// than build the screen under the splash again.
#[test]
fn the_host_keeps_changing_under_the_splash_and_each_change_is_sent_at_once() {
    let mut bridge = splash_bridge("full");
    let out = bridge.outgoing();
    let before = snapshots(&out)[0].chrome.clone();
    settle(&mut bridge, &out);
    // At 80 x 24 the splash has no room for the scope: the mark paints in from 100 ms.
    bridge.set_clock_ms(200);
    let frame = bridge.outgoing();
    assert_eq!(snapshots(&frame)[0].chrome, before, "only the splash moved");
    settle(&mut bridge, &frame);
    bridge.update(Event::Saved(Err("the disk is full".into())));
    bridge.set_clock_ms(300);
    let after = bridge.outgoing();
    let snap = snapshots(&after)[0];
    assert!(snap.splash.is_some(), "still showing");
    assert_ne!(snap.chrome, before, "the host's change is sent while the splash shows");
}

/// A key during the splash ends it at once and acts on the screen under
/// it (here: settings open), with no hand-off fade of its own.
#[test]
fn a_key_during_the_splash_skips_it_and_acts() {
    let mut bridge = splash_bridge("subtle");
    let out = bridge.outgoing();
    assert!(snapshots(&out)[0].splash.is_some());
    settle(&mut bridge, &out);
    bridge.receive(intent(1, Intent::Key { chord: crate::view::Chord::char(crate::host::SETTINGS_KEY) }));
    let after = bridge.outgoing();
    let snap = snapshots(&after)[0];
    assert!(snap.splash.is_none(), "skipped");
    assert!(snap.section.is_some(), "the key acted: settings show");
}

/// With motion off and the first screen ready at once, there is no splash.
#[test]
fn with_motion_off_there_is_no_splash_when_the_screen_is_ready() {
    let mut bridge = splash_bridge("off");
    let out = bridge.outgoing();
    assert!(snapshots(&out)[0].splash.is_none());
    assert_eq!(bridge.next_frame(), None);
}

/// RADAR_TRACE_LATENCY=1: a snapshot names the intent it answers, by the
/// renderer's seq, so Ink can time the key to the frame; off, it carries
/// nothing.
#[test]
fn a_traced_snapshot_names_the_intent_it_answers() {
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    bridge.set_trace(true);
    bridge.receive(hello(PROTOCOL));
    let out = bridge.outgoing();
    let first = snapshots(&out).pop().expect("the first snapshot").cursor;
    bridge.receive(ToHost::Ack { cursor: first });
    bridge.receive(intent(7, invoke(ASK)));
    let out = bridge.outgoing();
    let trace = snapshots(&out).pop().expect("the question's snapshot").trace.clone().expect("traced");
    assert_eq!((trace.cause.as_str(), trace.seq), ("intent invoke", Some(7)));
    assert!(trace.bytes > 0 && trace.at_ms > 0.0);

    let mut quiet = Bridge::new(fixture::host(), 80, 24);
    quiet.set_trace(false);
    quiet.receive(hello(PROTOCOL));
    assert!(snapshots(&quiet.outgoing()).iter().all(|s| s.trace.is_none()));
}

mod notice_history_tests {
    use super::*;
    #[test]
    fn identical_notice_occurrences_cross_backpressure_and_reconnect_without_replay() {
        let mut bridge = Bridge::new(fixture::host(), 80, 24);
        bridge.set_clock_ms(0); bridge.receive(hello(PROTOCOL));
        let initial = bridge.outgoing(); let first_cursor = snapshots(&initial)[0].cursor;
        bridge.update(Event::Notice("same".into())); bridge.update(Event::Notice("same".into()));
        let immediate = bridge.outgoing();
        let ids: Vec<_> = immediate.iter().filter_map(|message| match message { ToRenderer::Notice(n) => Some(n.occurrence.as_ref().unwrap().id), _ => None }).collect();
        assert_eq!(ids, [1, 2]); assert!(snapshots(&immediate).is_empty());
        bridge.set_clock_ms(crate::notices::INFO_MS); bridge.receive(ToHost::Ack { cursor: first_cursor });
        let expired = bridge.outgoing();
        assert!(snapshots(&expired)[0].chrome.notices.active.is_empty()); assert_eq!(snapshots(&expired)[0].chrome.notices.through_id, 2);
        bridge.reconnect(); bridge.receive(hello(PROTOCOL)); let reconnected = bridge.outgoing();
        assert!(reconnected.iter().all(|m| !matches!(m, ToRenderer::Notice(_))));
        assert!(snapshots(&reconnected)[0].chrome.notices.active.is_empty()); assert_eq!(bridge.host().notices.len(), 2);
    }
    #[test]
    fn an_open_notice_history_freezes_the_wire_clock_but_expiry_remains_authoritative() {
        let mut bridge = Bridge::new(fixture::host(), 80, 24);
        bridge.set_clock_ms(0); bridge.receive(hello(PROTOCOL)); let initial = bridge.outgoing(); settle(&mut bridge, &initial);
        bridge.update(Event::Notice("read this session notice".into()));
        bridge.send(Intent::Key { chord: crate::view::Chord::new(crate::view::Key::Char('~')) });
        let opened = bridge.outgoing(); assert!(snapshots(&opened)[0].chrome.notice_history.is_some());
        assert_eq!(snapshots(&opened)[0].chrome.notices.now_ms, 0); settle(&mut bridge, &opened);
        bridge.set_clock_ms(1_000); assert!(snapshots(&bridge.outgoing()).is_empty(), "hidden progress never republishes history");
        bridge.set_clock_ms(crate::notices::INFO_MS); let expired = bridge.outgoing();
        assert!(snapshots(&expired)[0].chrome.notices.active.is_empty());
        assert_eq!(snapshots(&expired)[0].chrome.notice_history.as_ref().unwrap().entries.len(), 1);
    }
    #[test]
    fn native_fallback_resets_notice_input_capability_for_captured_back_clicks() {
        let mut bridge = Bridge::new(fixture::host(), 80, 24); bridge.set_clock_ms(0);
        bridge.update(Event::Notice("still active".into())); let through_id = bridge.host().notices.through_id();
        bridge.send(Intent::NoticeInput { escape: false, through_id });
        bridge.send(Intent::Key { chord: crate::view::Chord::new(crate::view::Key::Char('~')) });
        assert!(bridge.host().chrome_view(80, 24).notice_history.is_some());
        let mut host = bridge.into_host();
        host.update(Event::Intent(Intent::Activate { target: crate::view::Target::new("chrome:notices:close") }));
        assert!(host.chrome_view(80, 24).notice_history.is_none()); assert!(!host.notices.has_active()); assert_eq!(host.notices.len(), 1);
    }
}
