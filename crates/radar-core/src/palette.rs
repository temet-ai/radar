//! Cheap cached navigation targets and pure command-palette matching.

use crate::{Subject, ui::icon::Icon};

/// One destination the owning part can show from its current cache. Enumerating
/// these must not perform I/O or discover credentials. Never include secrets in
/// search words. The host rechecks the complete subject before navigating.
#[derive(Debug, Clone)]
pub struct PaletteTarget {
    pub subject: Subject,
    /// Stable owner-scoped identity for frequency and duplicate suppression.
    pub identity: String,
    pub words: Vec<String>,
    pub icon: Option<Icon>,
}

impl PaletteTarget {
    pub fn new(subject: Subject) -> Self {
        Self::scoped(subject, "")
    }

    pub fn scoped(subject: Subject, scope: impl Into<String>) -> Self {
        let identity = serde_json::to_string(&(scope.into(), &subject.kind, &subject.id))
            .expect("strings serialize");
        Self {
            subject,
            identity,
            words: Vec::new(),
            icon: None,
        }
    }
}

/// Exact word, prefix, word-boundary prefix, then substring. Stable host ordering
/// resolves ties; no fuzzy fallback silently points at an unrelated item.
pub(crate) fn score(query: &str, words: &[String]) -> u32 {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return 0;
    }
    words
        .iter()
        .map(|word| {
            let word = word.to_lowercase();
            if word == query {
                100
            } else if word.starts_with(&query) {
                80
            } else if word
                .split(|c: char| !c.is_alphanumeric())
                .any(|part| part.starts_with(&query))
            {
                60
            } else if word.contains(&query) {
                40
            } else {
                0
            }
        })
        .max()
        .unwrap_or(0)
}

/// Unicode scalar positions, not byte offsets or terminal columns. Lowercase
/// expansion retains its original position, so renderers agree for non-ASCII text.
pub(crate) fn highlights(label: &str, query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut folded = String::new();
    let mut positions = Vec::new();
    for (index, character) in label.chars().enumerate() {
        for lower in character.to_lowercase() {
            let start = folded.len();
            folded.push(lower);
            positions.push((start, folded.len(), index));
        }
    }
    let mut found = Vec::new();
    for (start, matched) in folded.match_indices(&query) {
        let end = start + matched.len();
        for &(a, b, index) in &positions {
            if a < end && b > start && found.last() != Some(&index) {
                found.push(index);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_exact_prefix_boundary_then_substring() {
        let words = |word: &str| vec![word.to_string()];
        assert_eq!(score("watch", &words("WATCH")), 100);
        assert_eq!(score("wat", &words("watch")), 80);
        assert_eq!(score("wat", &words("city/watch")), 60);
        assert_eq!(score("atch", &words("watch")), 40);
        assert_eq!(score("  ", &words("watch")), 0);
        assert_eq!(score("missing", &words("watch")), 0);
    }

    #[test]
    fn matches_unicode_scalar_positions_and_lowercase_expansions() {
        assert_eq!(
            highlights("界 WATCH watch", "watch"),
            vec![2, 3, 4, 5, 6, 8, 9, 10, 11, 12]
        );
        assert_eq!(highlights("İstanbul", "i"), vec![0]);
        assert_eq!(highlights("éclair", "ÉC"), vec![0, 1]);
        assert!(highlights("watch", " ").is_empty());
        assert!(highlights("watch", "missing").is_empty());
    }
}
