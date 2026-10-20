//! Cell widths: how many terminal cells text takes, and cutting, padding and
//! wrapping it by them. Pure text, no renderer: every fit in the kit and the
//! view data's text form counts these, never characters.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// How many cells `text` takes on screen: an emoji or a CJK character two,
/// a combining mark none. Every fit, cut and pad in the kit counts this,
/// never characters, so wide text never moves a column.
pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// `text` as the terminal lays it out: each grapheme (`⚠` + U+FE0F, a
/// keycap, a ZWJ family, a letter and its marks) with its byte offset and
/// the cells it takes. Cut and wrap by these, never by `char`: a `char`
/// alone counts `⚠️` one cell where the terminal draws two. Ink counts the
/// same (ink/src/chrome/width.ts `clusters`).
pub fn clusters(text: &str) -> impl Iterator<Item = (usize, &str, usize)> {
    text.grapheme_indices(true)
        .map(|(i, g)| (i, g, UnicodeWidthStr::width(g)))
}

/// The longest start of `text` that fits in `w` cells, and the cells it
/// takes. A wide character that would cross the edge is left out whole,
/// never drawn half.
pub fn take(text: &str, w: usize) -> (&str, usize) {
    let mut used = 0;
    for (i, _, cw) in clusters(text) {
        if used + cw > w {
            return (&text[..i], used);
        }
        used += cw;
    }
    (text, used)
}

/// `text` with spaces after it to `w` cells (as it is when wider).
pub fn pad(text: &str, w: usize) -> String {
    let n = width(text);
    format!("{text}{}", " ".repeat(w.saturating_sub(n)))
}

/// `text` cut to `w` cells with an ellipsis, or padded to them; always
/// exactly `w` cells.
pub fn fit(text: &str, w: usize) -> String {
    if width(text) <= w {
        pad(text, w)
    } else if w == 0 {
        String::new()
    } else {
        let (head, used) = take(text, w - 1);
        format!("{head}…{}", " ".repeat(w - 1 - used))
    }
}

/// Word wrapping before any frontend chooses colors or cell styles.
pub fn wrap_words(text: &str, width: u16, prefix_width: usize) -> Vec<String> {
    let cells = self::width;
    let room = (width as usize).saturating_sub(prefix_width).max(8);
    let mut out = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && cells(&line) + 1 + cells(word) > room {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out
}
