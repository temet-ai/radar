//! The one conversion between a terminal key (crossterm's) and radar's own
//! [`Chord`], for the Ratatui runner and the parts' tests that still build
//! terminal keys. Temporary: removed with the runner's terminal path (T15).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::view::{Chord, Key};

/// A terminal key as a chord; `None` for keys the contract has no name for.
pub fn chord_of(key: &KeyEvent) -> Option<Chord> {
    let k = match key.code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        _ => return None,
    };
    Some(Chord {
        key: k,
        ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
        alt: key.modifiers.contains(KeyModifiers::ALT),
        shift: !matches!(key.code, KeyCode::Char(_) | KeyCode::BackTab)
            && key.modifiers.contains(KeyModifiers::SHIFT),
    })
}

/// A chord as the terminal key it stands for.
pub fn key_of(chord: Chord) -> KeyEvent {
    let code = match chord.key {
        Key::Char(c) => KeyCode::Char(c),
        Key::Enter => KeyCode::Enter,
        Key::Esc => KeyCode::Esc,
        Key::Tab => KeyCode::Tab,
        Key::BackTab => KeyCode::BackTab,
        Key::Backspace => KeyCode::Backspace,
        Key::Delete => KeyCode::Delete,
        Key::Up => KeyCode::Up,
        Key::Down => KeyCode::Down,
        Key::Left => KeyCode::Left,
        Key::Right => KeyCode::Right,
        Key::Home => KeyCode::Home,
        Key::End => KeyCode::End,
        Key::PageUp => KeyCode::PageUp,
        Key::PageDown => KeyCode::PageDown,
    };
    let mut mods = KeyModifiers::NONE;
    if chord.ctrl {
        mods |= KeyModifiers::CONTROL;
    }
    if chord.alt {
        mods |= KeyModifiers::ALT;
    }
    if chord.shift { mods |= KeyModifiers::SHIFT; }
    KeyEvent::new(code, mods)
}
