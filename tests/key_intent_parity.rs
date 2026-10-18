//! Keys reach the same place whichever renderer sends them. The Ratatui
//! runner sends terminal keys (`Event::Key`, an uppercase letter with
//! SHIFT, as crossterm reports it); Ink and Headless send the same chord as
//! `Intent::Key` (no SHIFT: the letter is the case). For every compiled-in
//! part's screens and every settings section (its navigation and the
//! section itself), every key the surface binds, the status line's keys and
//! the shared navigation keys give the same effects, journal, chrome and
//! view both ways; and every action invoked by its stable id does what its
//! key does. A defect of this class left History, Settings and actions
//! deaf under Ink while the terminal worked.
//!
//! One host pair per surface, driven in lockstep and put back on the
//! surface after each key, keeps this to one pass over every surface.

#![cfg(feature = "demo")]

use radar_core::headless::Headless;
use radar_core::host::{Event, HostEffect, JournalRecord, View};
use radar_core::journal::ActionId;
use radar_core::plugin::Pane;
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::view::{Chord, Intent, Key};

#[path = "support/mnemonic_acceptance.rs"]
mod mnemonic_acceptance;

const W: u16 = 120;
const H: u16 = 32;

/// Where a pair of hosts is put before each key.
#[derive(Clone, Copy, Debug)]
enum Surface {
    Screen(usize),
    /// A settings section, with the keys at its navigation (`Nav`) or in
    /// the section (`Middle`).
    Section(usize, Pane),
}

fn hosted() -> Headless {
    let mut host = radar_core::shot::demo_host(radar::parts()).expect("the parts fit together");
    host.enable_journal();
    Headless::new(host, W, H)
}

fn place(h: &mut Headless, at: Surface) {
    let host = h.host_mut();
    host.menu = None;
    host.bar = false;
    host.sources = false;
    host.tabs = false;
    host.quit = false;
    host.message = None;
    host.back.clear();
    match at {
        Surface::Screen(i) => {
            host.view = View::Screen(i);
            host.focus = Pane::Middle;
        }
        Surface::Section(i, pane) => {
            host.view = View::Settings;
            host.section = i;
            host.on_heading = false;
            host.focus = pane;
        }
    }
}

/// The terminal key a chord is, as crossterm reports it to the Ratatui
/// runner: an uppercase letter comes with SHIFT.
fn terminal(chord: Chord) -> Event {
    let mut key: KeyEvent = radar_core::view_tui::key_of(chord);
    if let KeyCode::Char(c) = key.code
        && c.is_uppercase()
    {
        key.modifiers |= KeyModifiers::SHIFT;
    }
    Event::Key(key)
}

/// The effects, journal records numbered by first appearance (their ids
/// are fresh per host).
fn shape(fx: &[HostEffect], ids: &mut Vec<ActionId>) -> Vec<String> {
    let mut n = |id: &ActionId| match ids.iter().position(|i| i == id) {
        Some(i) => i,
        None => {
            ids.push(id.clone());
            ids.len() - 1
        }
    };
    let mut out = Vec::new();
    for e in fx {
        let mut e = e;
        while let HostEffect::Tracked { effect, .. } = e {
            e = effect;
        }
        out.push(match e {
            HostEffect::Journal(JournalRecord::Begin { id, descriptor, parent }) => {
                let parent = parent.as_ref().map(&mut n);
                format!("begin #{} under {parent:?}: {descriptor:?}", n(id))
            }
            HostEffect::Journal(JournalRecord::Outcome { id, outcome, .. }) => {
                format!("#{} {outcome:?}", n(id))
            }
            HostEffect::Journal(JournalRecord::Gap { dropped }) => format!("gap {dropped}"),
            other => format!("{other:?}"),
        });
    }
    out
}

/// The work map's database id is fresh per host (it sits in the targets of
/// the workbench's rows); two hosts are compared with it blanked.
fn scrub_ids(text: String) -> String {
    let marks: Vec<char> = text.chars().collect();
    let shaped = |at: usize| {
        marks.get(at..at + 36).is_some_and(|w| {
            w.iter().enumerate().all(|(i, c)| match i {
                8 | 13 | 18 | 23 => *c == '-',
                _ => c.is_ascii_hexdigit(),
            })
        })
    };
    let (mut out, mut i) = (String::new(), 0);
    while i < marks.len() {
        if shaped(i) {
            out.push_str("<id>");
            i += 36;
        } else {
            out.push(marks[i]);
            i += 1;
        }
    }
    out
}

/// What a user and a renderer can see after a key.
fn seen(h: &Headless) -> String {
    let host = h.host();
    let chrome = host.chrome_view(W, H);
    let screen = h.screen();
    let section = h.section();
    let mut gaps = mnemonic_acceptance::chrome_gaps(&chrome);
    for value in screen
        .as_ref()
        .map(serde_json::to_value)
        .into_iter()
        .chain(section.as_ref().map(serde_json::to_value))
    {
        gaps.extend(mnemonic_acceptance::fixture_gaps(
            &value.expect("serialize neutral view"),
        ));
    }
    assert!(
        gaps.is_empty(),
        "first-party chrome mnemonic gaps: {gaps:#?}"
    );
    scrub_ids(format!(
        "{:?} {:?} bar={} sources={} tabs={} quit={} message={:?}\n{:?}\n{:?}\n{:?}",
        host.view,
        host.focus,
        host.bar,
        host.sources,
        host.tabs,
        host.quit,
        host.message.as_ref().map(|m| &m.text),
        chrome,
        screen,
        section,
    ))
}

/// Where two descriptions part, with a little on each side.
fn differs(a: &str, b: &str) -> String {
    let at = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    let from = at.saturating_sub(80);
    let cut = |s: &str| s.chars().skip(from).take(240).collect::<String>();
    format!("\n  terminal …{}\n  intent   …{}", cut(a), cut(b))
}

/// The keys to try on a surface: what it binds, what its status line
/// offers, and the keys every surface shares.
fn keys(h: &Headless) -> Vec<Chord> {
    let mut keys: Vec<Chord> = [
        Key::Up,
        Key::Down,
        Key::Left,
        Key::Right,
        Key::Enter,
        Key::Tab,
        Key::BackTab,
        Key::PageUp,
        Key::PageDown,
        Key::Home,
        Key::End,
    ]
    .map(Chord::new)
    .into();
    keys.extend("xH?/jkJKR".chars().map(Chord::char));
    let actions = h
        .screen()
        .map(|v| v.actions)
        .or_else(|| h.section().map(|v| v.actions))
        .unwrap_or_default();
    keys.extend(actions.iter().filter_map(|a| a.key));
    for hint in h.host().chrome_view(W, H).status.hints {
        keys.extend(hint.keys_of.iter().map(|k| k.chord));
    }
    // Esc last: it leaves the surface.
    keys.push(Chord::new(Key::Esc));
    let mut seen = Vec::new();
    keys.retain(|k| {
        let new = !seen.contains(k);
        seen.push(*k);
        new
    });
    keys
}

fn surfaces() -> Vec<Surface> {
    let h = hosted();
    let host = h.host();
    let mut all: Vec<Surface> = (0..host.screens.len()).map(Surface::Screen).collect();
    for i in 0..host.sections.len() {
        all.push(Surface::Section(i, Pane::Nav));
        all.push(Surface::Section(i, Pane::Middle));
    }
    all
}

/// History is the host's own overlay: closed between keys so each key
/// starts on the surface.
fn close_history(h: &mut Headless) {
    if h.host().chrome_view(W, H).history.is_some() {
        h.host_mut().update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    }
}

/// Runs `per_surface` for every surface, a few at a time on their own
/// threads (each builds its own hosts), and returns the gaps in surface
/// order.
fn across_surfaces(per_surface: fn(Surface, &mut Vec<String>)) -> Vec<String> {
    let all = surfaces();
    let lanes = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    let mut found: Vec<(usize, Vec<String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..lanes)
            .map(|lane| {
                let mine: Vec<(usize, Surface)> =
                    all.iter().copied().enumerate().filter(|(i, _)| i % lanes == lane).collect();
                scope.spawn(move || {
                    mine.into_iter()
                        .map(|(i, at)| {
                            let mut gaps = Vec::new();
                            per_surface(at, &mut gaps);
                            (i, gaps)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("a surface's walk panicked")).collect()
    });
    found.sort_by_key(|(i, _)| *i);
    found.into_iter().flat_map(|(_, g)| g).collect()
}

#[test]
fn every_key_on_every_surface_does_the_same_as_a_terminal_key_or_an_intent() {
    let gaps = across_surfaces(keys_on_surface);
    assert!(gaps.is_empty(), "{} keys differ by renderer:\n{}", gaps.len(), gaps.join("\n"));
}

fn keys_on_surface(at: Surface, gaps: &mut Vec<String>) {
    {
        let (mut tui, mut ink) = (hosted(), hosted());
        let (mut tui_ids, mut ink_ids) = (Vec::new(), Vec::new());
        place(&mut tui, at);
        let chords = keys(&tui);
        place(&mut ink, at);
        let _ = keys(&ink);
        for chord in chords {
            place(&mut tui, at);
            place(&mut ink, at);
            let before = seen(&tui);
            let other = seen(&ink);
            assert!(before == other, "{at:?}: the pair starts equal: {}", differs(&before, &other));
            let a = tui.host_mut().update(terminal(chord));
            let b = ink.send(Intent::Key { chord });
            let (a, b) = (shape(&a, &mut tui_ids), shape(&b, &mut ink_ids));
            let (sa, sb) = (seen(&tui), seen(&ink));
            if a != b || sa != sb {
                gaps.push(format!(
                    "{at:?} {chord:?}:\n  terminal {a:?}\n  intent   {b:?}{}",
                    if sa == sb { String::new() } else { differs(&sa, &sb) }
                ));
                // Back in step for the next key.
                (tui, ink) = (hosted(), hosted());
                (tui_ids, ink_ids) = (Vec::new(), Vec::new());
                continue;
            }
            close_history(&mut tui);
            close_history(&mut ink);
        }
    }
}

#[test]
fn every_action_invoked_by_its_id_does_what_its_key_does() {
    let gaps = across_surfaces(actions_on_surface);
    assert!(gaps.is_empty(), "{} actions differ by id:\n{}", gaps.len(), gaps.join("\n"));
}

fn actions_on_surface(at: Surface, gaps: &mut Vec<String>) {
    {
        let (mut tui, mut ink) = (hosted(), hosted());
        let (mut tui_ids, mut ink_ids) = (Vec::new(), Vec::new());
        place(&mut tui, at);
        let resting = seen(&tui);
        let actions = tui
            .screen()
            .map(|v| v.actions)
            .or_else(|| tui.section().map(|v| v.actions))
            .unwrap_or_default();
        for action in actions.iter().filter(|a| !a.heading) {
            let Some(chord) = action.key else { continue };
            place(&mut tui, at);
            place(&mut ink, at);
            let a = tui.host_mut().update(terminal(chord));
            let b = ink.send(Intent::Invoke { action: action.id.clone() });
            let (a, b) = (shape(&a, &mut tui_ids), shape(&b, &mut ink_ids));
            let (sa, sb) = (seen(&tui), seen(&ink));
            if a != b || sa != sb {
                gaps.push(format!(
                    "{at:?} {} ({chord:?}):\n  key    {a:?}\n  invoke {b:?}{}",
                    action.id,
                    if sa == sb { String::new() } else { differs(&sa, &sb) }
                ));
                (tui, ink) = (hosted(), hosted());
                (tui_ids, ink_ids) = (Vec::new(), Vec::new());
                continue;
            }
            close_history(&mut tui);
            close_history(&mut ink);
            // An action that opened a page, a menu or a field leaves the
            // pair somewhere the next id is not offered (a typed key goes
            // into the field): start the next from the surface again.
            if seen(&tui) != resting {
                (tui, ink) = (hosted(), hosted());
                (tui_ids, ink_ids) = (Vec::new(), Vec::new());
            }
        }
    }
}
