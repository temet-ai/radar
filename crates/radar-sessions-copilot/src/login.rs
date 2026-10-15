//! Copilot sign-in. The SDK's `account.login` takes a token, which radar never handles, and
//! its interactive flow has no device code; so sign-in runs `copilot login --device-code`
//! as a supervised child. The CLI stores the credential itself; radar reads the code and the
//! link from the child's output and nothing else (no other line is kept or logged).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use radar_sessions::{BoxFuture, LoginEvent, LoginHandle};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch};

/// Finds the link and the code in a line such as
/// `To authenticate, visit https://github.com/login/device and enter code 352A-43E7`.
pub fn parse_device_code(line: &str) -> Option<(String, String)> {
    let url_at = line.find("https://")?;
    let url: String = line[url_at..]
        .chars()
        .take_while(|c| !c.is_whitespace())
        .collect();
    let code_at = line.find("code ")? + "code ".len();
    let code: String = line[code_at..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    (code.len() >= 4 && !url.is_empty()).then_some((url, code))
}

/// A running `copilot login --device-code`.
pub struct LoginFlow {
    events: mpsc::UnboundedReceiver<LoginEvent>,
    cancel: watch::Sender<bool>,
    ended: bool,
}

impl LoginFlow {
    /// Starts the child. `host` is a GitHub Enterprise Cloud host (`https://x.ghe.com`);
    /// `home` is a scratch `COPILOT_HOME` for tests and recordings.
    pub fn start(program: &Path, host: Option<&str>, home: Option<&Path>) -> anyhow::Result<Self> {
        let mut cmd = Command::new(program);
        cmd.arg("login").arg("--device-code");
        if let Some(host) = host {
            cmd.arg("--host").arg(host);
        }
        cmd.env("COPILOT_AUTO_UPDATE", "false")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(home) = home {
            cmd.env("COPILOT_HOME", home);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("cannot start {}: {e}", program.display()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stdout"))?;
        let (tx, events) = mpsc::unbounded_channel();
        let (cancel, mut cancelled) = watch::channel(false);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut shown = false;
            loop {
                tokio::select! {
                    _ = cancelled.changed() => {
                        let _ = child.kill().await;
                        let _ = tx.send(LoginEvent::Cancelled);
                        return;
                    }
                    line = lines.next_line() => match line {
                        Ok(Some(line)) => {
                            if !shown
                                && let Some((url, code)) = parse_device_code(&line)
                            {
                                shown = true;
                                let _ = tx.send(LoginEvent::DeviceCode { url, code });
                            }
                        }
                        // Output closed: the child is ending.
                        _ => break,
                    },
                }
            }
            let end = tokio::select! {
                _ = cancelled.changed() => { let _ = child.kill().await; LoginEvent::Cancelled }
                status = child.wait() => match status {
                    Ok(s) if s.success() && shown => LoginEvent::Completed,
                    Ok(s) if shown => LoginEvent::Failed(format!("copilot login exited with {s}")),
                    Ok(_) => LoginEvent::Failed("copilot login ended before showing a device code".into()),
                    Err(e) => LoginEvent::Failed(e.to_string()),
                },
            };
            let _ = tx.send(end);
        });
        Ok(Self {
            events,
            cancel,
            ended: false,
        })
    }
}

impl LoginHandle for LoginFlow {
    fn next_event(&mut self) -> BoxFuture<'_, LoginEvent> {
        Box::pin(async move {
            if self.ended {
                return LoginEvent::Cancelled;
            }
            let ev = self
                .events
                .recv()
                .await
                .unwrap_or(LoginEvent::Failed("sign-in stopped".into()));
            self.ended = matches!(
                ev,
                LoginEvent::Completed | LoginEvent::Failed(_) | LoginEvent::Cancelled
            );
            ev
        })
    }

    /// Kills the child; idempotent. Nothing is signed in.
    fn cancel(&self) {
        let _ = self.cancel.send(true);
    }
}

/// `copilot` at the configured path, else the first on `PATH`. A sandbox
/// (`RADAR_SANDBOX`, set for every cargo test) only takes a configured one: the
/// real CLI is never started there, nor its sign-in.
pub fn resolve_program(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = configured {
        return p.is_file().then(|| p.to_path_buf());
    }
    if std::env::var_os("RADAR_SANDBOX").is_some_and(|v| !v.is_empty()) {
        return None;
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("copilot"))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn parses_the_cli_line() {
        let (url, code) = parse_device_code(
            "To authenticate, visit https://github.com/login/device and enter code 352A-43E7",
        )
        .unwrap();
        assert_eq!(url, "https://github.com/login/device");
        assert_eq!(code, "352A-43E7");
    }

    #[test]
    fn ignores_other_lines() {
        assert_eq!(parse_device_code("Waiting for authorization..."), None);
    }

    #[test]
    fn missing_program_is_none() {
        assert_eq!(
            resolve_program(Some(Path::new("/nonexistent/copilot"))),
            None
        );
    }

    /// A stand-in `copilot` script: prints the device-code lines then runs `tail`.
    fn script(dir: &Path, tail: &str) -> PathBuf {
        let p = dir.join("copilot");
        std::fs::write(
            &p,
            format!("#!/bin/sh\necho 'To authenticate, visit https://github.com/login/device and enter code ABCD-1234'\necho 'Waiting for authorization...'\n{tail}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn scratch(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("radar-copilot-login-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn shows_the_code_then_completes() {
        let d = scratch("ok");
        let mut f = LoginFlow::start(&script(&d, "exit 0"), None, None).unwrap();
        assert_eq!(
            f.next_event().await,
            LoginEvent::DeviceCode {
                url: "https://github.com/login/device".into(),
                code: "ABCD-1234".into()
            }
        );
        assert_eq!(f.next_event().await, LoginEvent::Completed);
        let _ = std::fs::remove_dir_all(d);
    }

    #[tokio::test]
    async fn cancel_kills_the_child_and_says_so() {
        let d = scratch("cancel");
        let mut f = LoginFlow::start(&script(&d, "exec sleep 60"), None, None).unwrap();
        assert!(matches!(
            f.next_event().await,
            LoginEvent::DeviceCode { .. }
        ));
        f.cancel();
        f.cancel();
        assert_eq!(f.next_event().await, LoginEvent::Cancelled);
        let _ = std::fs::remove_dir_all(d);
    }

    #[tokio::test]
    async fn a_failed_login_is_reported() {
        let d = scratch("fail");
        let mut f = LoginFlow::start(&script(&d, "exit 3"), None, None).unwrap();
        let _ = f.next_event().await;
        assert!(matches!(f.next_event().await, LoginEvent::Failed(_)));
        let _ = std::fs::remove_dir_all(d);
    }
}
