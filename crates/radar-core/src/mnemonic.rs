//! Host-owned key-in-word metadata. Positions count Unicode scalars, never
//! bytes, graphemes or terminal cells; drawers underline the containing grapheme.

use std::ops::Range;

use crate::view::{Seg, Text};

/// A single alphabetic scalar is a mnemonic. Named keys, actual modifier
/// chords, digits, punctuation and grouped aliases retain their key labels.
pub fn letter(keys: &str) -> Option<char> {
    let mut chars = keys.chars();
    let first = chars.next()?;
    (chars.next().is_none() && first.is_alphabetic()).then_some(first)
}

/// The first matching original scalar, with the palette's lowercase-expansion
/// convention. No accent stripping, normalization or locale-specific folding.
pub fn at(keys: &str, words: &str) -> Option<usize> {
    letter(keys)?;
    crate::palette::highlights(words, keys).into_iter().next()
}

/// Whether a mnemonic's unchanged input requires the explicit Shift modifier.
pub fn shifted(keys: &str) -> bool {
    letter(keys).is_some_and(char::is_uppercase)
}

/// Questions display a nonempty hint; ordinary and palette rows display label.
pub fn choice_words<'a>(question: bool, label: &'a str, hint: &'a str) -> &'a str {
    if question && !hint.is_empty() { hint } else { label }
}

/// A rich line uses source-scalar offsets in Text::words(), including icon
/// fallback text, but replaceable icon fallbacks cannot supply a mnemonic.
/// Chip captions remain eligible because their words are always displayed.
pub fn at_text(keys: &str, text: &Text) -> Option<usize> {
    letter(keys)?;
    let words = text.words();
    crate::palette::highlights(&words, keys)
        .into_iter()
        .find(|&at| eligible(text, at) && valid_at(keys, &words, Some(at)))
}

/// Whether this scalar belongs to words that remain present with icons enabled.
pub fn eligible(text: &Text, at: usize) -> bool {
    let mut offset = 0;
    for seg in &text.0 {
        let end = offset + seg.text.chars().count();
        if (offset..end).contains(&at) {
            return seg.icon.is_none() || seg.chip;
        }
        offset = end;
    }
    false
}

/// Validate only the supplied position, without searching for another letter.
/// Invalid or absent metadata must retain the original visible key label.
pub fn valid_at(keys: &str, words: &str, at: Option<usize>) -> bool {
    let Some(key) = letter(keys) else { return false };
    let Some((byte, _)) = at.and_then(|at| words.char_indices().nth(at)) else { return false };
    words[byte..].to_lowercase().starts_with(&key.to_lowercase().collect::<String>())
}

/// The visible key prefix. A valid lowercase mnemonic has none; a shifted one
/// has the named modifier. Legacy or malformed descriptions keep their key.
pub fn prefix<'a>(keys: &'a str, words: &str, at: Option<usize>, shift: bool) -> &'a str {
    if valid_at(keys, words, at) {
        if shift { "shift" } else { "" }
    } else {
        keys
    }
}

/// Source-scalar range of the complete grapheme containing the supplied index.
/// Every source scalar counts, including controls that do not paint a cell.
pub fn grapheme_range(words: &str, at: usize) -> Option<Range<usize>> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut offset = 0;
    for grapheme in words.graphemes(true) {
        let end = offset + grapheme.chars().count();
        if (offset..end).contains(&at) { return Some(offset..end); }
        offset = end;
    }
    None
}

/// Host-built styled key words for handwritten help and tour controls. `look`
/// includes its actual theme slot; the mnemonic adds underline to that look.
pub fn text(keys: &str, words: &str, look: &str) -> Text {
    let at = at(keys, words);
    let prefix = prefix(keys, words, at, at.is_some() && shifted(keys));
    let mut out = Text::new();
    if !prefix.is_empty() { out.add(Seg::styled(format!("{prefix} "), look)); }
    let Some(range) = at.and_then(|at| grapheme_range(words, at)) else {
        out.add(Seg::styled(words, look));
        return out;
    };
    let chars: Vec<_> = words.chars().collect();
    for (part, underlined) in [(&chars[..range.start], false), (&chars[range.clone()], true), (&chars[range.end..], false)] {
        if !part.is_empty() {
            out.add(Seg::styled(part.iter().collect::<String>(), if underlined { format!("{look} underline") } else { look.into() }));
        }
    }
    if let Some(last) = out.0.last_mut() { last.key_hint = Some(keys.into()); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ui::icon::Icon, view::Ink};

    #[test]
    fn first_matching_scalar_uses_palette_case_expansion_without_normalizing() {
        for (key, words, expected) in [
            ("r", "refresh", Some(0)), ("x", "extra actions", Some(1)),
            ("r", "mirror", Some(2)), ("i", "İstanbul", Some(0)),
            ("É", "éclair", Some(0)), ("e", "e\u{301}dit", Some(0)),
            ("é", "e\u{301}dit", None), ("e", "😀 edit", Some(2)),
            ("𐐀", "𐐨ction", Some(0)), ("z", "refresh", None),
            ("ctrl-r", "refresh", None), ("alt-a", "ask", None),
            ("r/s", "refresh", None), ("1", "1 task", None),
            ("↑↓", "move", None), ("esc", "escape", None),
        ] { assert_eq!(at(key, words), expected, "{key}: {words}"); }
        assert!(shifted("É"));
        assert!(shifted("A"));
        assert!(!shifted("ctrl-A"));
        assert!(!shifted("a"));
    }

    #[test]
    fn question_words_and_exact_position_validation_do_not_guess() {
        assert_eq!(choice_words(true, "yes", "yes, post it"), "yes, post it");
        assert_eq!(choice_words(false, "yes", "yes, post it"), "yes");
        assert_eq!(choice_words(true, "yes", ""), "yes");
        assert_eq!(prefix("r", "refresh", Some(0), false), "");
        assert_eq!(prefix("A", "ask", Some(0), true), "shift");
        for wrong in [None, Some(1), Some(99)] {
            assert_eq!(prefix("r", "refresh", wrong, true), "r");
        }
        assert_eq!(prefix("ctrl-r", "refresh", Some(0), false), "ctrl-r");
        assert!(valid_at("i", "İstanbul", Some(0)));
        assert_eq!(grapheme_range("e\u{301}dit", 0), Some(0..2));
        assert_eq!(grapheme_range("👨‍👩‍👧 edit", 6), Some(6..7));
        assert_eq!(grapheme_range("\tr", 1), Some(1..2));
        assert_eq!(grapheme_range("a\r\nr", 3), Some(3..4));
        assert_eq!(grapheme_range("\te\u{301}", 1), Some(1..3));
        assert_eq!(text("r", "refresh", "text.muted").plain_words(), "refresh [r]");
        assert_eq!(text("A", "ask", "text.muted").plain_words(), "shift ask [A]");
        assert_eq!(text("esc", "close", "text.muted").plain_words(), "esc close");
    }

    #[test]
    fn rich_offsets_count_hidden_fallback_but_only_visible_words_supply_keys() {
        let text = Text::new().push(Seg::icon_or(Icon::Link, "open ", Ink::Muted))
            .push(Seg::new("refresh", Ink::Plain));
        assert_eq!(at_text("r", &text), Some(5));
        assert_eq!(at_text("o", &text), None);
        let mut chip = Seg::new("copy", Ink::Accent);
        chip.icon = Some(Icon::Copy);
        chip.chip = true;
        assert_eq!(at_text("y", &Text::from(vec![chip])), Some(3));
        let split = Text::from(vec![Seg::new("e", Ink::Accent), Seg::new("\u{301}dit", Ink::Muted)]);
        assert_eq!(at_text("e", &split), Some(0));
        assert_eq!(grapheme_range(&split.words(), 0), Some(0..2));
    }
}
