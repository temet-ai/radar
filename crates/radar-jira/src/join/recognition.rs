//! URL scope is stronger than a key-shaped substring. Mask the complete raw
//! URL-like token even if parsing fails; only a separately trimmed, strict browse
//! URL may yield a ticket. Never restart at a nested scheme inside that token.

use std::ops::Range;

use radar_core::Subject;

use crate::{JiraPlugin, subject_scope};

pub(super) fn recognise(p: &JiraPlugin, text: &str) -> Vec<(Range<usize>, Subject)> {
    let urls = candidates(text);
    let mut out = Vec::new();
    for raw in &urls {
        // Do not manufacture a supported URL from the middle of malformed text
        // such as `123https://...`; its key is still masked below.
        if text[..raw.start]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace() && !"([{<\"'`«“‘".contains(c))
        {
            continue;
        }
        let given = trim_prose(&text[raw.clone()]);
        let Some((_, key)) = subject_scope::browse_url(given) else {
            continue;
        };
        // Resolve the URL with no project-derived alias: a second site's URL
        // must win even when another site watches or holds this project/key.
        let url_only = Subject::new(super::TICKET, key, key).with_url(given);
        let Some(site) = subject_scope::resolve(p, &url_only) else {
            continue;
        };
        let subject = super::ticket(p, &site.name, key).with_url(given);
        // An invalid imported config may contain empty or shadowed aliases.
        // Never publish a subject that Show's explicit-alias gate would refuse.
        if subject_scope::resolve(p, &subject).is_some() {
            out.push((raw.start..raw.start + given.len(), subject));
        }
    }
    let mut url_at = 0;
    out.extend(
        super::keys_in(text, false)
            .into_iter()
            .filter(|(key, _)| {
                // keys_in and candidates are ordered: advance once rather than
                // re-scanning every URL range for each bare-key candidate.
                while urls.get(url_at).is_some_and(|url| url.end <= key.start) {
                    url_at += 1;
                }
                urls.get(url_at).is_none_or(|url| key.end <= url.start)
            })
            .filter(|(_, key)| super::known_project(p, key.split('-').next().unwrap_or_default()))
            .filter_map(|(range, key)| {
                let site = super::site_for(p, &key)?;
                Some((range, super::ticket(p, &site, &key)))
            }),
    );
    out.sort_by_key(|(range, _)| range.start);
    out
}

/// The first URL-like opener through the end of each whitespace-delimited token.
/// Delimiters inside the token (including quotes/wrappers) do not end exclusion:
/// malformed/nested/concatenated links must not leak their keys as bare mentions.
fn candidates(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut from = 0;
    for token in text.split_whitespace() {
        let start = from + text[from..].find(token).expect("token came from text");
        from = start + token.len();
        if let Some(at) = first_opener(token) {
            out.push(start + at..from);
        }
    }
    out
}

fn first_opener(token: &str) -> Option<usize> {
    let mut first = None;
    let mut scheme_start = None;
    for (at, c) in token.char_indices() {
        let tail = &token[at..];
        if tail.starts_with("//")
            || ["www.", "http:", "https:"].into_iter().any(|prefix| {
                tail.get(..prefix.len())
                    .is_some_and(|s| s.eq_ignore_ascii_case(prefix))
            })
        {
            first = Some(first.map_or(at, |old: usize| old.min(at)));
        }
        if c.is_ascii_alphabetic() {
            scheme_start.get_or_insert(at);
        } else if !c.is_ascii_digit() && !"+.-".contains(c) {
            if tail.starts_with("://")
                && let Some(start) = scheme_start
            {
                first = Some(first.map_or(start, |old| old.min(start)));
            }
            scheme_start = None;
        }
    }
    first
}

/// Trim clear prose punctuation and unmatched closing wrappers for recognition
/// only. Balanced query/fragment punctuation and IPv6 brackets stay verbatim.
fn trim_prose(mut url: &str) -> &str {
    let wrappers = [('(', ')'), ('[', ']'), ('{', '}'), ('<', '>')];
    let quotes = ['\'', '"', '`'];
    let mut balance = [0_isize; 4];
    let mut quote_counts = [0_usize; 3];
    for c in url.chars() {
        for (i, (open, close)) in wrappers.iter().enumerate() {
            balance[i] += isize::from(c == *close) - isize::from(c == *open);
        }
        for (i, quote) in quotes.iter().enumerate() {
            quote_counts[i] += usize::from(c == *quote);
        }
    }
    while let Some(last) = url.chars().next_back() {
        let unmatched = wrappers
            .iter()
            .enumerate()
            .find_map(|(i, (_, close))| (*close == last && balance[i] > 0).then_some(i));
        let quote = quotes
            .iter()
            .enumerate()
            .find_map(|(i, q)| (*q == last && quote_counts[i] % 2 == 1).then_some(i));
        if !matches!(last, '.' | ',' | ';' | '!') && unmatched.is_none() && quote.is_none() {
            break;
        }
        if let Some(i) = unmatched {
            balance[i] -= 1;
        }
        if let Some(i) = quote {
            quote_counts[i] -= 1;
        }
        url = &url[..url.len() - last.len_utf8()];
    }
    url
}

#[cfg(test)]
mod tests;
