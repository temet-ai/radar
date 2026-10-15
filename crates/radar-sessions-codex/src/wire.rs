//! The line-oriented pipe to a `codex app-server`, and how one is started.
//!
//! The adapter only sees a [`Wire`]: lines out, lines in, and an end when the
//! process is gone. Tests start a replay of a recorded stream instead.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;

use radar_sessions::BoxFuture;
use radar_sessions::adapter::ProviderSetup;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

/// One running app-server. `rx` closing means the process exited (or the
/// replay ended); dropping `tx` asks it to stop.
pub struct Wire {
    pub tx: mpsc::UnboundedSender<String>,
    pub rx: mpsc::UnboundedReceiver<String>,
}

/// Starts app-servers and asks the CLI about itself.
pub trait Spawner: Send + Sync {
    /// Start a fresh `app-server`. Fails when the executable cannot start.
    fn spawn(&self) -> BoxFuture<'_, anyhow::Result<Wire>>;
    /// The CLI's version (`0.158.0`), or `None` when it is not installed.
    fn version(&self) -> BoxFuture<'_, Option<String>>;
    /// Where the CLI is and keeps its state (Settings → Agents). Applies from
    /// the next process started; the one running is left alone.
    fn configure(&self, _setup: &ProviderSetup) {}
}

/// Runs the operator's installed `codex`.
pub struct ChildSpawner {
    launch: Mutex<Launch>,
}

/// The program to run and the `CODEX_HOME` to give it.
#[derive(Clone)]
struct Launch {
    program: PathBuf,
    config_dir: Option<PathBuf>,
}

impl ChildSpawner {
    /// `program` is `codex` (found on `PATH`) or an explicit path from settings.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            launch: Mutex::new(Launch {
                program: program.into(),
                config_dir: None,
            }),
        }
    }

    fn command(&self) -> (Command, PathBuf) {
        let w = self.launch.lock().unwrap().clone();
        let mut cmd = Command::new(&w.program);
        if let Some(dir) = &w.config_dir {
            cmd.env("CODEX_HOME", dir);
        }
        (cmd, w.program)
    }
}

impl Default for ChildSpawner {
    fn default() -> Self {
        Self::new("codex")
    }
}

impl Spawner for ChildSpawner {
    fn spawn(&self) -> BoxFuture<'_, anyhow::Result<Wire>> {
        Box::pin(async move {
            let (mut cmd, program) = self.command();
            let mut child = cmd
                .arg("app-server")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| {
                    anyhow::anyhow!("cannot start `{} app-server`: {e}", program.display())
                })?;
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| anyhow::anyhow!("no stdin"))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| anyhow::anyhow!("no stdout"))?;
            let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
            let (in_tx, in_rx) = mpsc::unbounded_channel::<String>();
            // One task owns the child: it writes what the adapter sends,
            // forwards what the child prints, and ends the child when either
            // side is gone (kill_on_drop covers the task being dropped).
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                loop {
                    tokio::select! {
                        line = lines.next_line() => match line {
                            Ok(Some(l)) => { if in_tx.send(l).is_err() { break; } }
                            _ => break,
                        },
                        out = out_rx.recv() => match out {
                            Some(mut l) => {
                                l.push('\n');
                                if stdin.write_all(l.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                                    break;
                                }
                            }
                            None => break,
                        },
                    }
                }
                drop(stdin);
                let _ = child.kill().await;
            });
            Ok(Wire {
                tx: out_tx,
                rx: in_rx,
            })
        })
    }

    fn version(&self) -> BoxFuture<'_, Option<String>> {
        Box::pin(async move {
            let (mut cmd, _) = self.command();
            let out = cmd
                .arg("--version")
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .output()
                .await
                .ok()?;
            if !out.status.success() {
                return None;
            }
            parse_version(&String::from_utf8_lossy(&out.stdout))
        })
    }

    fn configure(&self, setup: &ProviderSetup) {
        let mut w = self.launch.lock().unwrap();
        w.program = setup
            .executable
            .clone()
            .unwrap_or_else(|| PathBuf::from("codex"));
        w.config_dir = setup.config_dir.clone();
    }
}

/// `codex-cli 0.158.0` → `0.158.0`.
pub fn parse_version(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_the_first_number() {
        assert_eq!(
            parse_version("codex-cli 0.158.0\n").as_deref(),
            Some("0.158.0")
        );
        assert_eq!(parse_version("").as_deref(), None);
    }
}

#[cfg(all(test, unix))]
mod launch_tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A stand-in `codex` that says which CODEX_HOME it was started with.
    #[tokio::test]
    async fn configure_sets_the_program_and_codex_home_the_child_gets() {
        let dir = std::env::temp_dir().join(format!("radar-codex-wire-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("codex");
        std::fs::write(
            &exe,
            "#!/bin/sh\nif [ \"$CODEX_HOME\" = /somewhere ]; then echo codex-cli 1.2.3; else echo codex-cli 0.0.1; fi\n",
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();

        let spawner = ChildSpawner::new("/definitely/not/codex");
        assert_eq!(spawner.version().await, None);
        spawner.configure(&ProviderSetup {
            executable: Some(exe.clone()),
            config_dir: Some("/somewhere".into()),
        });
        assert_eq!(spawner.version().await.as_deref(), Some("1.2.3"));
        spawner.configure(&ProviderSetup {
            executable: Some(exe),
            config_dir: Some("/elsewhere".into()),
        });
        assert_eq!(spawner.version().await.as_deref(), Some("0.0.1"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
