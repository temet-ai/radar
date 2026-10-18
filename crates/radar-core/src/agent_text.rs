//! Credentials do not go into a brief. A token-shaped word is replaced by
//! `[redacted]`; a private key block is dropped whole. This is a net under the
//! rule that parts never contribute credentials, not a licence to.

/// Prefixes of well-known token shapes (a token is the prefix and at least
/// [`MIN_TOKEN`] bytes in all).
const PREFIXES: &[&str] = &[
    "xoxp-", "xoxb-", "xoxa-", "xoxs-", "xapp-", "xoxe-", "ghp_", "gho_", "ghu_", "ghs_", "ghr_",
    "github_pat_", "glpat-", "sk-", "AKIA", "ASIA", "AIza", "ya29.", "eyJ",
];
const MIN_TOKEN: usize = 20;

/// The text with secrets replaced, and how many were.
pub fn redact_secrets(text: &str) -> (String, usize) {
    let mut count = 0;
    let mut out = String::with_capacity(text.len());
    let mut in_key = false;
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 && !in_key {
            out.push('\n');
        }
        if in_key {
            if line.contains("-----END") {
                in_key = false;
                out.push('\n');
            }
            continue;
        }
        if line.contains("-----BEGIN") && line.contains("PRIVATE KEY") {
            count += 1;
            out.push_str("[redacted private key]");
            if !line.contains("-----END") {
                in_key = true;
            } else {
                // A key on one line: nothing else to drop.
            }
            continue;
        }
        out.push_str(&redact_line(line, &mut count));
    }
    (out, count)
}

fn redact_line(line: &str, count: &mut usize) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        let start = rest
            .find(|c: char| !is_break(c))
            .unwrap_or(rest.len());
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let end = rest.find(is_break).unwrap_or(rest.len());
        let word = &rest[..end];
        if looks_secret(word) {
            *count += 1;
            out.push_str("[redacted]");
        } else {
            out.push_str(word);
        }
        rest = &rest[end..];
    }
    // `Authorization: Bearer <x>` and `password=<x>` shapes.
    redact_assignments(&out, count)
}

fn is_break(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '(' | ')' | '<' | '>' | ',' | ';' | '[' | ']')
}

fn looks_secret(word: &str) -> bool {
    word.len() >= MIN_TOKEN
        && PREFIXES.iter().any(|p| word.starts_with(p))
        && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '='))
}

fn redact_assignments(line: &str, count: &mut usize) -> String {
    let lower = line.to_ascii_lowercase();
    for key in ["bearer ", "password=", "password:", "token=", "secret=", "api_key=", "apikey="] {
        if let Some(at) = lower.find(key) {
            let value_start = at + key.len();
            let value = &line[value_start..];
            let value = value.trim_start();
            let skipped = line.len() - value_start - value.len();
            let end = value.find(is_break).unwrap_or(value.len());
            if end >= 8 && !value.starts_with("[redacted") {
                *count += 1;
                let mut out = String::new();
                out.push_str(&line[..value_start + skipped]);
                out.push_str("[redacted]");
                out.push_str(&redact_assignments(&value[end..], count));
                return out;
            }
        }
    }
    line.to_owned()
}
