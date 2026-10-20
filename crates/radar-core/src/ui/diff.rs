//! A diff: one file's changes as unified lines (hunk headings, lines kept,
//! added and removed) with the old and new line numbers beside them, and a
//! cursor on a line. The lines and how they are read are `view::diff`'s;
//! [`DiffState`] is the cursor and the view's position, moved by keys.

use std::cell::Cell;

use crate::view::{Chord, Key};

pub use crate::view::diff::*;

/// The cursor and how far the view is scrolled, kept in `Cell`s so reading
/// through `&self` can keep the cursor in view.
#[derive(Debug, Default)]
pub struct DiffState {
    cursor: Cell<usize>,
    /// The first row shown (rows count the blocks under lines too).
    top: Cell<usize>,
    /// Rows visible in the last frame.
    height: Cell<usize>,
}

impl DiffState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The line with the cursor.
    pub fn cursor(&self) -> usize {
        self.cursor.get()
    }

    /// Put the cursor on line `i` (kept in view at the next frame).
    pub fn set(&self, i: usize) {
        self.cursor.set(i);
    }

    /// The first row shown.
    pub fn top(&self) -> usize {
        self.top.get()
    }

    /// The viewport size a renderer reported, for page movement.
    pub fn set_height(&self, height: usize) {
        self.height.set(height);
    }

    /// A renderer's local projection starts at this row.
    pub fn set_top(&self, top: usize) {
        self.top.set(top);
    }

    /// Back to the first line, scrolled to the top.
    pub fn reset(&self) {
        self.cursor.set(0);
        self.top.set(0);
    }

    /// `↑↓ j k PgUp PgDn g G Home End` over `n` lines; false for any other
    /// key, and for `↑` on the first line (the host takes it up).
    pub fn key(&self, key: Chord, n: usize) -> bool {
        let last = n.saturating_sub(1);
        let at = self.cursor.get().min(last);
        let page = self.height.get().saturating_sub(1).max(1);
        let to = match key.key {
            Key::Down | Key::Char('j') => (at + 1).min(last),
            Key::Up | Key::Char('k') if at == 0 => return false,
            Key::Up | Key::Char('k') => at - 1,
            Key::PageDown => (at + page).min(last),
            Key::PageUp => at.saturating_sub(page),
            Key::Home | Key::Char('g') => 0,
            Key::End | Key::Char('G') => last,
            _ => return false,
        };
        self.cursor.set(to);
        true
    }

    /// The same keys side by side ([`split_rows`]): the cursor moves by
    /// rows, staying on the side it is on where the row has a line there
    /// (the new side first).
    pub fn key_split(&self, key: Chord, lines: &[DiffLine]) -> bool {
        let rows = split_rows(lines);
        if rows.is_empty() {
            return false;
        }
        let at = self.cursor.get();
        let r = rows.iter().position(|(a, b)| *a == Some(at) || *b == Some(at)).unwrap_or(0);
        let last = rows.len() - 1;
        let page = self.height.get().saturating_sub(1).max(1);
        let to = match key.key {
            Key::Down | Key::Char('j') => (r + 1).min(last),
            Key::Up | Key::Char('k') if r == 0 => return false,
            Key::Up | Key::Char('k') => r - 1,
            Key::PageDown => (r + page).min(last),
            Key::PageUp => r.saturating_sub(page),
            Key::Home | Key::Char('g') => 0,
            Key::End | Key::Char('G') => last,
            _ => return false,
        };
        let left = rows[r].0 == Some(at) && rows[r].1 != Some(at);
        let (a, b) = rows[to];
        self.cursor.set(if left { a.or(b) } else { b.or(a) }.unwrap_or(at));
        true
    }

    /// The cursor to the next hunk's heading; false when there is none after it.
    pub fn next_hunk(&self, lines: &[DiffLine]) -> bool {
        let at = self.cursor.get();
        match hunks(lines).into_iter().find(|&h| h > at) {
            Some(h) => {
                self.cursor.set(h);
                true
            }
            None => false,
        }
    }

    /// The cursor to the hunk before (or to the start of its own hunk);
    /// false when there is none before it.
    pub fn previous_hunk(&self, lines: &[DiffLine]) -> bool {
        let at = self.cursor.get();
        match hunks(lines).into_iter().rev().find(|&h| h < at) {
            Some(h) => {
                self.cursor.set(h);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file's changes in two hunks, as a unified diff.
    const SAMPLE: &str = "@@ -18,6 +18,12 @@ public class Shelves {\n \
@RestController\n \
public class Shelves {\n\
-    // TODO: the dashboard\n\
+    private final Atlas atlas;\n\
+\n\
+    @GetMapping(\"/shelf/{id}\")\n\
+    public Shelf shelf(@PathVariable UUID id) {\n\
+        return atlas.find(id);\n\
+    }\n\
 \n\
 }\n\
\\ No newline at end of file\n\
@@ -40,3 +46,3 @@\n \
 void sort() {\n\
-    by(decade);\n\
+    by(decade, title);\n";

    #[test]
    fn a_unified_diff_is_read_with_both_sides_numbers() {
        let lines = parse(SAMPLE);
        assert_eq!(lines[0].kind, Kind::Hunk);
        assert_eq!((lines[1].old, lines[1].new), (Some(18), Some(18)));
        let removed = &lines[3];
        assert_eq!((removed.kind, removed.old, removed.new), (Kind::Removed, Some(20), None));
        assert_eq!(removed.text, "    // TODO: the dashboard");
        let added = &lines[4];
        assert_eq!((added.kind, added.old, added.new), (Kind::Added, None, Some(20)));
        assert_eq!(lines[5].text, "", "an added empty line");
        assert_eq!(lines[12].kind, Kind::Note);
        assert_eq!((lines[14].old, lines[14].new), (Some(40), Some(46)));
        assert_eq!(lines[14].text, "void sort() {");
        assert_eq!(hunks(&lines), [0, 13]);
        // File headings before the first hunk are not lines.
        let git = format!("diff --git a/x b/x\nindex 1..2\n--- a/x\n+++ b/x\n{SAMPLE}");
        assert_eq!(parse(&git), lines);
        assert!(parse("").is_empty());
    }

    #[test]
    fn the_cursor_moves_by_line_page_and_hunk() {
        let lines = parse(SAMPLE);
        let s = DiffState::new();
        let key = |c| Chord::new(c);
        assert!(s.key(key(Key::Down), lines.len()));
        assert_eq!(s.cursor(), 1);
        assert!(s.next_hunk(&lines));
        assert_eq!(s.cursor(), 13);
        assert!(!s.next_hunk(&lines), "none after the last");
        assert!(s.previous_hunk(&lines));
        assert_eq!(s.cursor(), 0);
        assert!(s.key(key(Key::Char('G')), lines.len()));
        assert_eq!(s.cursor(), lines.len() - 1);
        assert!(!s.key(key(Key::Char('x')), lines.len()));
    }
}
