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
    reread_all(&[name]);
    var(name)
}

/// [`reread`] for several names, in one shell.
pub fn reread_all(names: &[&str]) {
    let names: Vec<&str> = names.iter().copied().filter(|n| valid_name(n)).collect();
    if names.is_empty() {
        return;
    }
    let Some(values) = from_shell(&names) else { return };
    if let Ok(mut m) = FRESH.write() {
        for (name, value) in names.iter().zip(values) {
            if let Some(v) = value {
                m.insert((*name).to_string(), v);
            }
        }
    }
}

/// The variable names among settings that may read `env:NAME`.
pub fn names_in<'a>(settings: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = settings
        .into_iter()
        .filter_map(|s| s.trim().strip_prefix("env:"))
        .filter(|n| valid_name(n))
        .map(str::to_owned)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Read every `env:NAME` among `settings` again from the shell, off the
/// async workers: what a part's check does first, so a token exported after
/// radar started is the one checked.
pub async fn reread_settings(settings: Vec<String>) {
    let names = names_in(settings.iter().map(String::as_str));
    if names.is_empty() || crate::system::sandboxed() {
        return;
    }
    let _ = tokio::task::spawn_blocking(move || {
        reread_all(&names.iter().map(String::as_str).collect::<Vec<_>>())
    })
    .await;
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Each name's value in the login shell (`None` when unset there); `None`
/// as a whole when the shell cannot be asked.
fn from_shell(names: &[&str]) -> Option<Vec<Option<String>>> {
    if names.is_empty() || !names.iter().all(|n| valid_name(n)) || crate::system::sandboxed() {
        return None;
    }
    let what = names.join(", ");
    let shell = std::env::var("SHELL").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "sh".into());
    // Interactive and login, so both ~/.zshrc-style and ~/.profile-style
    // exports are seen. `$NAME` (no braces) reads the same in sh, bash, zsh
    // and fish.
    let formats = "%s\\036".repeat(names.len());
    let args: String = names.iter().map(|n| format!(" \"${n}\"")).collect();
    let script = format!("printf '\\036radar\\036{formats}'{args}");
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
                crate::log::warn(format!("reading {what} from {shell}: no answer within {}s", WAIT.as_secs()));
                return None;
            }
        }
    }
    let values = parse(&reader.join().ok()?, names.len())?;
    values.iter().flatten().for_each(|v| crate::log::secret(v));
    crate::log::info(format!("read {what} again from {shell}"));
    Some(values)
}

/// The `n` values between the marks, each `None` when empty (unset);
/// `None` when the marks are missing.
fn parse(out: &str, n: usize) -> Option<Vec<Option<String>>> {
    let start = format!("{MARK}radar{MARK}");
    let mut rest = &out[out.find(&start)? + start.len()..];
    let mut values = Vec::with_capacity(n);
    for _ in 0..n {
        let end = rest.find(MARK)?;
        let value = &rest[..end];
        values.push((!value.is_empty()).then(|| value.to_string()));
        rest = &rest[end + MARK.len_utf8()..];
    }
    Some(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_value_is_found_among_start_up_noise() {
        let out = "welcome!\n\u{1e}radar\u{1e}glpat-new\u{1e}\u{1e}xoxp-1\u{1e}bye\n";
        let got = parse(out, 3).unwrap();
        assert_eq!(got, vec![Some("glpat-new".into()), None, Some("xoxp-1".into())]);
        assert_eq!(parse("\u{1e}radar\u{1e}\u{1e}", 1), Some(vec![None]));
        assert_eq!(parse("\u{1e}radar\u{1e}a\u{1e}", 2), None);
        assert_eq!(parse("no marks", 1), None);
    }

    #[test]
    fn only_env_settings_name_variables() {
        let got = names_in(["env:JIRA_TOKEN", "https://x.example", " env:JIRA_TOKEN", "env:bad name", "env:EMAIL"]);
        assert_eq!(got, vec!["EMAIL", "JIRA_TOKEN"]);
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
