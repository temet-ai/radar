//! A diff's model: a unified diff's text as lines with their numbers on each
//! side, the lines side by side, the hunks, and the words that changed in a
//! replaced line. The drawing (cursor, scrolling, colours) is `ui::diff`.

use serde::{Deserialize, Serialize};

/// What a line of a diff is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[ts(rename = "DiffKind")]
pub enum Kind {
    /// `@@ -12,7 +12,9 @@ …`: where a hunk starts.
    Hunk,
    /// The same on both sides.
    Context,
    Added,
    Removed,
    /// `\ No newline at end of file` and the like.
    Note,
}

/// One line of a diff, with its numbers on each side.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
pub struct DiffLine {
    pub kind: Kind,
    /// Its number in the old file (kept and removed lines).
    pub old: Option<u32>,
    /// Its number in the new file (kept and added lines).
    pub new: Option<u32>,
    /// The text without its `+`, `-` or space; a hunk's whole heading.
    pub text: String,
}

/// The numbers in `@@ -12,7 +12,9 @@`: where each side starts.
fn hunk_starts(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("@@ ")?;
    let (ranges, _) = rest.split_once(" @@")?;
    let mut parts = ranges.split_whitespace();
    let start = |p: &str, sign: char| -> Option<u32> {
        p.strip_prefix(sign)?.split(',').next()?.parse().ok()
    };
    Some((start(parts.next()?, '-')?, start(parts.next()?, '+')?))
}

/// A unified diff's text as lines with their numbers. File headings
/// (`diff --git`, `---`, `+++`, `index`) are left out; text before the first
/// hunk is ignored.
pub fn parse(diff: &str) -> Vec<DiffLine> {
    let mut out = Vec::new();
    let (mut old, mut new) = (0u32, 0u32);
    let mut in_hunk = false;
    for raw in diff.lines() {
        if let Some((o, n)) = hunk_starts(raw) {
            (old, new, in_hunk) = (o, n, true);
            out.push(DiffLine {
                kind: Kind::Hunk,
                old: None,
                new: None,
                text: raw.to_string(),
            });
            continue;
        }
        if !in_hunk {
            continue;
        }
        let (kind, text) = match raw.chars().next() {
            Some('+') => (Kind::Added, &raw[1..]),
            Some('-') => (Kind::Removed, &raw[1..]),
            Some('\\') => (Kind::Note, raw),
            Some(' ') => (Kind::Context, &raw[1..]),
            // An empty kept line whose space was trimmed on the way.
            None => (Kind::Context, ""),
            Some(_) => (Kind::Context, raw),
        };
        let (o, n) = match kind {
            Kind::Added => (None, Some(new)),
            Kind::Removed => (Some(old), None),
            Kind::Context => (Some(old), Some(new)),
            Kind::Hunk | Kind::Note => (None, None),
        };
        match kind {
            Kind::Added => new += 1,
            Kind::Removed => old += 1,
            Kind::Context => {
                old += 1;
                new += 1;
            }
            Kind::Hunk | Kind::Note => {}
        }
        out.push(DiffLine {
            kind,
            old: o,
            new: n,
            text: text.to_string(),
        });
    }
    out
}

/// The lines side by side: each row's line on the left (the old file) and
/// on the right (the new). A run of removed lines faces the added lines
/// after it, in order; a side with nothing to face is `None`. A kept line,
/// a hunk's heading or a note is the same index on both sides.
pub fn split_rows(lines: &[DiffLine]) -> Vec<(Option<usize>, Option<usize>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if matches!(lines[i].kind, Kind::Removed | Kind::Added) {
            let r0 = i;
            while i < lines.len() && lines[i].kind == Kind::Removed {
                i += 1;
            }
            let a0 = i;
            while i < lines.len() && lines[i].kind == Kind::Added {
                i += 1;
            }
            let (nr, na) = (a0 - r0, i - a0);
            for k in 0..nr.max(na) {
                out.push(((k < nr).then_some(r0 + k), (k < na).then_some(a0 + k)));
            }
        } else {
            out.push((Some(i), Some(i)));
            i += 1;
        }
    }
    out
}

/// Where each hunk starts, as indexes into the lines.
pub fn hunks(lines: &[DiffLine]) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind == Kind::Hunk)
        .map(|(i, _)| i)
        .collect()
}

/// Where a removed line and the line that replaced it differ, word by
/// word, as byte ranges of each; nothing when they share too little for
/// the difference to help.
pub(crate) fn changed_words(old: &str, new: &str) -> (Ranges, Ranges) {
    use similar::{Algorithm, DiffTag, capture_diff_slices};
    // Words, runs of spaces, and each other character on its own.
    fn tokens(s: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut start = 0;
        let class = |c: char| if c.is_alphanumeric() || c == '_' { 0 } else if c.is_whitespace() { 1 } else { 2 };
        let mut prev: Option<u8> = None;
        for (i, c) in s.char_indices() {
            let k = class(c);
            if prev.is_some_and(|p| p != k || k == 2) {
                out.push(&s[start..i]);
                start = i;
            }
            prev = Some(k);
        }
        if start < s.len() {
            out.push(&s[start..]);
        }
        out
    }
    let (a, b) = (tokens(old), tokens(new));
    let offsets = |t: &[&str]| {
        let mut at = vec![0];
        for x in t {
            at.push(at.last().copied().unwrap_or(0) + x.len());
        }
        at
    };
    let (ao, bo) = (offsets(&a), offsets(&b));
    let (mut o, mut n) = (Vec::new(), Vec::new());
    let mut same = 0usize;
    for op in capture_diff_slices(Algorithm::Myers, &a, &b) {
        let (or, nr) = (op.old_range(), op.new_range());
        match op.tag() {
            DiffTag::Equal => {
                same += a[or].iter().filter(|t| !t.trim().is_empty()).map(|t| t.len()).sum::<usize>();
            }
            DiffTag::Delete => o.push((ao[or.start], ao[or.end])),
            DiffTag::Insert => n.push((bo[nr.start], bo[nr.end])),
            DiffTag::Replace => {
                o.push((ao[or.start], ao[or.end]));
                n.push((bo[nr.start], bo[nr.end]));
            }
        }
    }
    let longest = old.trim().len().max(new.trim().len()).max(1);
    if same * 4 < longest {
        return (vec![], vec![]);
    }
    (o, n)
}

/// The words that changed in each line, as byte ranges of its text (tabs
/// as four spaces): removed lines followed by added ones are paired up in
/// order and compared word by word; every other line has none.
pub fn changed_lines(lines: &[DiffLine]) -> Vec<Ranges> {
    let text_of = |l: &DiffLine| l.text.replace('\t', "    ");
    let mut out = vec![Ranges::new(); lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != Kind::Removed {
            i += 1;
            continue;
        }
        let r0 = i;
        while i < lines.len() && lines[i].kind == Kind::Removed {
            i += 1;
        }
        let a0 = i;
        while i < lines.len() && lines[i].kind == Kind::Added {
            i += 1;
        }
        for k in 0..(a0 - r0).min(i - a0) {
            let (r, a) = (r0 + k, a0 + k);
            (out[r], out[a]) = changed_words(&text_of(&lines[r]), &text_of(&lines[a]));
        }
    }
    out
}

/// Byte ranges of a line.
pub type Ranges = Vec<(usize, usize)>;
