//! Environment variables as the person's shell has them now.
//!
//! radar's own environment is fixed when it starts, so a token exported
//! again afterwards (a new `GITLAB_TOKEN` in `~/.zshrc`) never reaches it.
//! [`reread`] asks the person's login shell for one variable's current value
//! and keeps it; [`var`] returns that kept value before radar's own. Parts
//! read `env:NAME` settings through [`var`], and call [`reread`] when the
//! person asks for a check or the service turns a token down.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};

/// Values read again from the shell, by name.
static FRESH: LazyLock<RwLock<HashMap<String, String>>> = LazyLock::new(Default::default);

/// How long a login shell may take to start and answer.
const WAIT: Duration = Duration::from_secs(5);

/// Marks the value in the shell's output, so whatever its start-up files
/// print around it is ignored.
const MARK: char = '\u{1e}';

/// The variable's value: the one last read from the shell, else radar's own.
pub fn var(name: &str) -> Option<String> {
    if let Some(v) = FRESH.read().ok().and_then(|m| m.get(name).cloned()) {
        return Some(v);
    }
    std::env::var(name).ok()
}

/// Ask the login shell for `name` now, keep what it says, and return the
/// value [`var`] gives afterwards. When the shell cannot be asked (sandboxed,
/// no answer within a few seconds, the variable unset there), the value
/// already held stays. Blocks: call it off the UI thread.
pub fn reread(name: &str) -> Option<String> {
    if let Some(v) = from_shell(name)
        && let Ok(mut m) = FRESH.write()
    {
        m.insert(name.to_string(), v);
    }
    var(name)
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn from_shell(name: &str) -> Option<String> {
    if !valid_name(name) || crate::system::sandboxed() {
        return None;
    }
    let shell = std::env::var("SHELL").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "sh".into());
    // Interactive and login, so both ~/.zshrc-style and ~/.profile-style
    // exports are seen. `$NAME` (no braces) reads the same in sh, bash, zsh
    // and fish.
    let script = format!("printf '\\036radar\\036%s\\036' \"${name}\"");
    let mut child = Command::new(&shell)
        .args(["-l", "-i", "-c", &script])
        .env("RADAR_ENV_PROBE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                crate::log::warn(format!("reading {name} from {shell}: no answer within {}s", WAIT.as_secs()));
                return None;
            }
        }
    }
    let value = parse(&reader.join().ok()?)?;
    crate::log::secret(&value);
    crate::log::info(format!("read {name} again from {shell}"));
    Some(value)
}

/// The value between the marks; `None` when absent or empty (unset).
fn parse(out: &str) -> Option<String> {
    let start = format!("{MARK}radar{MARK}");
    let rest = &out[out.find(&start)? + start.len()..];
    let value = &rest[..rest.find(MARK)?];
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_value_is_found_among_start_up_noise() {
        let out = "welcome!\n\u{1e}radar\u{1e}glpat-new\u{1e}bye\n";
        assert_eq!(parse(out).as_deref(), Some("glpat-new"));
        assert_eq!(parse("\u{1e}radar\u{1e}\u{1e}"), None);
        assert_eq!(parse("no marks"), None);
    }

    #[test]
    fn only_plain_names_reach_a_shell() {
        assert!(valid_name("GITLAB_TOKEN"));
        assert!(!valid_name("A;rm -rf ~"));
        assert!(!valid_name("1A"));
        assert!(!valid_name(""));
    }

    #[test]
    fn a_sandbox_keeps_radars_own_value() {
        assert!(crate::system::sandboxed());
        assert_eq!(reread("RADAR_ENV_TEST_UNSET_VAR"), None);
        assert_eq!(reread("RADAR_SANDBOX"), std::env::var("RADAR_SANDBOX").ok());
    }
}
