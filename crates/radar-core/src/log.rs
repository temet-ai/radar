//! The diagnostic log: what radar did and why, for when something looks wrong.
//!
//! A TUI cannot print to the terminal it draws on, so radar writes here
//! instead: `radar.log` in radar's state directory (`~/.local/state/radar/radar.log` on
//! Linux). It keeps itself small: past [`MAX_BYTES`] the file moves to
//! `radar.log.1` (replacing the one before) and a new one starts. Nothing is
//! written until the runner calls [`init`], so tests and the CLI's printing
//! stay quiet.
//!
//! `RADAR_DEBUG=1` adds [`debug`] lines. Every line is passed through
//! [`redact`] first: tokens never reach the file, whether they look like
//! one or were registered with [`secret`].

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// How big the file grows before it is rotated.
pub const MAX_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    const fn word(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// One log file, its rotation size and the lowest level it keeps.
pub struct Logger {
    pub path: PathBuf,
    pub max: u64,
    pub level: Level,
}

impl Logger {
    pub fn new(path: PathBuf, level: Level) -> Self {
        Self {
            path,
            max: MAX_BYTES,
            level,
        }
    }

    /// Append one line (redacted), rotating first if it would not fit.
    pub fn write(&self, level: Level, now: i64, msg: &str, known: &Known) {
        if level < self.level {
            return;
        }
        let line = format!(
            "{} {:<5} {}\n",
            crate::time::to_rfc3339(now),
            level.word(),
            redact(msg, known)
        );
        let size = std::fs::metadata(&self.path).map_or(0, |m| m.len());
        if size > 0 && size + line.len() as u64 > self.max {
            let _ = std::fs::rename(&self.path, rotated(&self.path));
        }
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

fn rotated(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".1");
    path.with_file_name(name)
}

/// What the log knows to hide, and what only looks like a secret.
#[derive(Debug, Default, Clone)]
pub struct Known {
    /// Values never written ([`secret`]).
    pub secrets: Vec<String>,
    /// Prefixes of a service's token format ([`token_shape`]).
    pub shapes: Vec<String>,
    /// Words that follow `token=` but name where a token comes from
    /// ([`not_secret`]).
    pub plain: Vec<String>,
}

struct State {
    logger: Option<Logger>,
    known: Known,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(State {
            logger: None,
            known: Known::default(),
        })
    })
}

/// Where the log goes: `<state dir>/radar/radar.log`.
pub fn default_path() -> PathBuf {
    crate::system::state_home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("radar")
        .join("radar.log")
}

/// Start logging to `path`; `RADAR_DEBUG=1` keeps debug lines too.
pub fn init(path: PathBuf) {
    let debug = std::env::var("RADAR_DEBUG").is_ok_and(|v| !v.is_empty() && v != "0");
    let level = if debug { Level::Debug } else { Level::Info };
    if let Ok(mut s) = state().lock() {
        s.logger = Some(Logger::new(path, level));
    }
}

/// Never write `value` (a token radar was given or looked up), whatever line
/// it turns up in.
pub fn secret(value: &str) {
    let value = value.trim();
    if value.len() < 8 {
        return;
    }
    if let Ok(mut s) = state().lock()
        && !s.known.secrets.iter().any(|v| v == value)
    {
        s.known.secrets.push(value.to_string());
    }
}

/// Whether bounded semantic metadata matches a registered secret, token shape,
/// or the redactor's generic patterns. Callers must bound input before scanning
/// and reject a matching identity rather than storing a rewritten one.
/// This only reads the existing registry: it resolves no credentials and does
/// no I/O. A poisoned registry fails closed; unknown secrets can remain undetected.
pub fn contains_secret(value: &str) -> bool {
    state()
        .lock()
        .map_or(true, |state| redact(value, &state.known) != value)
}

/// A service's tokens start with `prefix`: the prefix and what follows are
/// never written. A part registers its service's shapes when it starts.
pub fn token_shape(prefix: &str) {
    if let Ok(mut s) = state().lock()
        && !prefix.is_empty()
        && !s.known.shapes.iter().any(|v| v == prefix)
    {
        s.known.shapes.push(prefix.to_string());
    }
}

/// `word` after `token=` names where a token comes from (a CLI that holds
/// it), not the token: keep it.
pub fn not_secret(word: &str) {
    if let Ok(mut s) = state().lock()
        && !s.known.plain.iter().any(|v| v == word)
    {
        s.known.plain.push(word.to_string());
    }
}

pub fn log(level: Level, msg: &str) {
    if let Ok(s) = state().lock()
        && let Some(l) = &s.logger
    {
        l.write(level, crate::time::now_secs(), msg, &s.known);
    }
}

pub fn error(msg: impl AsRef<str>) {
    log(Level::Error, msg.as_ref());
}
pub fn warn(msg: impl AsRef<str>) {
    log(Level::Warn, msg.as_ref());
}
pub fn info(msg: impl AsRef<str>) {
    log(Level::Info, msg.as_ref());
}
/// Built only when `RADAR_DEBUG` is on.
pub fn debug(msg: impl FnOnce() -> String) {
    let on = state()
        .lock()
        .is_ok_and(|s| s.logger.as_ref().is_some_and(|l| l.level == Level::Debug));
    if on {
        log(Level::Debug, &msg());
    }
}

/// Words after which the next word is a secret.
const SECRET_KEYS: [&str; 5] = [
    "bearer ",
    "token=",
    "token: ",
    "token = ",
    "private-token: ",
];

const REDACTED: &str = "[redacted]";

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '+' | '/' | '=')
}

/// `msg` with anything that is or looks like a secret replaced.
pub fn redact(msg: &str, known: &Known) -> String {
    let mut out = msg.to_string();
    for s in &known.secrets {
        if !s.is_empty() {
            out = out.replace(s.as_str(), REDACTED);
        }
    }
    // Token shapes the parts registered: the prefix and everything
    // token-like after it.
    for prefix in known.shapes.iter().map(String::as_str) {
        let mut from = 0;
        while let Some(at) = out[from..].find(prefix).map(|i| i + from) {
            let end = out[at..]
                .char_indices()
                .find(|(_, c)| !token_char(*c))
                .map_or(out.len(), |(i, _)| at + i);
            out.replace_range(at..end, REDACTED);
            from = at + REDACTED.len();
        }
    }
    // `Bearer x`, `token=x`: the word after the key.
    for key in SECRET_KEYS {
        let mut from = 0;
        loop {
            let lower = out.to_ascii_lowercase();
            let Some(mut at) = lower[from..].find(key).map(|i| i + from + key.len()) else {
                break;
            };
            if out[at..].starts_with('"') {
                at += 1;
            }
            let end = out[at..]
                .char_indices()
                .find(|(_, c)| !token_char(*c) || *c == '"')
                .map_or(out.len(), |(i, _)| at + i);
            let value = &out[at..end];
            let named = out[at..].starts_with("env:") || known.plain.iter().any(|w| w == value);
            if value.is_empty() || value == REDACTED || named {
                from = at;
            } else {
                out.replace_range(at..end, REDACTED);
                from = at + REDACTED.len();
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("radar-log-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("radar.log")
    }

    #[test]
    fn rotates_past_the_limit_and_keeps_one_old_file() {
        let path = scratch("rotate");
        let log = Logger {
            path: path.clone(),
            max: 200,
            level: Level::Info,
        };
        for i in 0..20 {
            log.write(
                Level::Info,
                0,
                &format!("line {i} of some length"),
                &Known::default(),
            );
        }
        let now = std::fs::metadata(&path).unwrap().len();
        let old = std::fs::metadata(rotated(&path)).unwrap().len();
        assert!(now <= 200 && old <= 200, "{now} {old}");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("line 19"), "{text}");
        assert!(text.starts_with("1970-01-01T00:00:00Z INFO "), "{text}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn debug_lines_only_at_debug_level() {
        let path = scratch("level");
        let log = Logger::new(path.clone(), Level::Info);
        log.write(Level::Debug, 0, "hidden", &Known::default());
        log.write(Level::Warn, 0, "shown", &Known::default());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("hidden") && text.contains("WARN  shown"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn semantic_identity_check_uses_existing_registered_values_shapes_and_patterns() {
        secret("semantic-value-canary-5cb8e481");
        token_shape("semantic-shape-canary-7ae034c9-");
        for value in [
            "scope/semantic-value-canary-5cb8e481/ref",
            "scope/semantic-shape-canary-7ae034c9-suffix",
            "Bearer semantic-generic-canary",
        ] {
            assert!(contains_secret(value));
        }
        assert!(!contains_secret("acme/project:feature/safe-identity"));
    }

    #[test]
    fn secrets_never_reach_the_file() {
        // Shaped like tokens but not real ones.
        let known = Known {
            secrets: vec!["not-a-real-secret-1234".to_string()],
            shapes: vec!["svc-".into()],
            plain: vec!["somecli".into()],
        };
        let cases = [
            (
                "token svc-AAAA.bbbb_cc-dd rejected",
                "token [redacted] rejected",
            ),
            (
                "Authorization: Bearer abc.def-123",
                "Authorization: Bearer [redacted]",
            ),
            (
                "url?private_token=1&token=xyz789 end",
                "url?private_token=[redacted]&token=[redacted] end",
            ),
            ("token = \"zzzzzzzzzz\"", "token = \"[redacted]\""),
            ("used not-a-real-secret-1234 here", "used [redacted] here"),
            ("token = env:SOME_TOKEN", "token = env:SOME_TOKEN"),
            ("token=somecli", "token=somecli"),
            ("token=othercli", "token=[redacted]"),
            ("plain words stay", "plain words stay"),
        ];
        for (input, want) in cases {
            assert_eq!(redact(input, &known), want, "{input}");
        }
        let path = scratch("secret");
        Logger::new(path.clone(), Level::Info).write(
            Level::Error,
            0,
            "sync failed with svc-0123456789abcdef",
            &known,
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("0123456789abcdef"), "{text}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
