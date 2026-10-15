//! Runs the Ink renderer as a child process and keeps it running: finds
//! the bundle and `node`, starts the child on a Unix socket, drives the
//! [`Bridge`] from its messages and the host's events, restores the
//! terminal whenever the child exits, restarts it under the policy, and
//! says when the Ratatui renderer must take over.
//!
//! The child owns the terminal (raw mode, the alternate screen, mouse
//! reporting, the cursor, resizes) from when it starts until it exits;
//! this process writes nothing to the terminal in that time. Between
//! children, and after the last, this process puts the terminal back.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::host::{Event, HostEffect};

use super::{Bridge, Lines, ToHost, ToRenderer, encode};

/// What leaves the terminal as it was, whatever the renderer turned on:
/// mouse reporting off (every mode), focus and paste reports off, the
/// alternate screen left, the cursor shown. Raw mode is the terminal's
/// line settings, not a sequence: see [`TtyModes`].
pub const RESTORE: &str = "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[?2004l\x1b[?1049l\x1b[?25h";

/// How long a started renderer has to connect.
pub const CONNECT_WITHIN: Duration = Duration::from_secs(10);

/// How long a renderer told to leave has before it is ended.
pub const LEAVE_WITHIN: Duration = Duration::from_secs(2);

/// The most lines waiting to be written to a renderer. Only one snapshot
/// can be among them (the rest wait for its ack), so a renderer that
/// reads keeps this nearly empty; one that stops reading fills it and is
/// ended as crashed.
pub const MAX_QUEUED: usize = 256;

/// How long a full queue may stay full before its renderer counts as
/// having stopped reading. A full queue alone is not enough: the task
/// that writes it to the socket may simply not have had its turn yet, and
/// ending a renderer that reads would restart it for nothing.
pub const STALLED_AFTER: Duration = Duration::from_secs(2);

/// Say on `err` (stderr) why the Ratatui renderer takes over; the same
/// words are already in the host's status line.
pub fn say_fall_back(err: &mut dyn Write, why: &str) {
    let _ = writeln!(err, "radar: {why}");
    crate::log::warn(why);
}

/// Restarts: at most `max` in any `window`; one more crash and the
/// Ratatui renderer takes over.
#[derive(Debug, Clone)]
pub struct RestartPolicy {
    pub max: usize,
    pub window: Duration,
    restarts: VecDeque<Instant>,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            max: 3,
            window: Duration::from_secs(60),
            restarts: VecDeque::new(),
        }
    }
}

impl RestartPolicy {
    /// The renderer crashed at `now`: true to start it again, false when
    /// it has already been restarted `max` times in the window.
    pub fn restart(&mut self, now: Instant) -> bool {
        while self
            .restarts
            .front()
            .is_some_and(|t| now.duration_since(*t) >= self.window)
        {
            self.restarts.pop_front();
        }
        if self.restarts.len() >= self.max {
            return false;
        }
        self.restarts.push_back(now);
        true
    }
}

/// Where the renderer's bundle is: `RADAR_INK_BUNDLE`, else
/// `<the radar executable's folder>/../share/radar/ink/radar-ink.js` (what
/// scripts/install puts there), else `ink/dist/radar-ink.js` in the checkout
/// the executable was built in (a folder above it holding `Cargo.toml`
/// and `ink/`). Only a file that exists counts.
pub fn find_bundle(var: &dyn Fn(&str) -> Option<String>, exe: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = var("RADAR_INK_BUNDLE").filter(|p| !p.is_empty()) {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    let dir = exe?.parent()?;
    let installed = dir.join("../share/radar/ink/radar-ink.js");
    if installed.is_file() {
        return Some(installed);
    }
    dir.ancestors()
        .find(|d| d.join("Cargo.toml").is_file() && d.join("ink").is_dir())
        .map(|d| d.join("ink/dist/radar-ink.js"))
        .filter(|p| p.is_file())
}

/// The command that starts the renderer: `node <bundle>` for a bundle that
/// `RADAR_INK_BUNDLE` names, else `<runtime> ink` for radar's runtime
/// ([`crate::runtime`]), else `node` with the bundle [`find_bundle`] finds.
/// The error says what is missing.
pub fn find_renderer(
    var: &dyn Fn(&str) -> Option<String>,
    exe: Option<&Path>,
) -> Result<(PathBuf, Vec<std::ffi::OsString>), &'static str> {
    let named = var("RADAR_INK_BUNDLE").is_some_and(|p| !p.is_empty());
    if !named && let Some(runtime) = crate::runtime::find(var) {
        return Ok((runtime, vec!["ink".into()]));
    }
    let bundle = find_bundle(var, exe)
        .ok_or("the Ink renderer is not installed (RADAR_RUNTIME, RADAR_INK_BUNDLE, share/radar/ink/radar-ink.js or ink/dist/radar-ink.js)")?;
    let node = find_node(var("PATH").as_deref()).ok_or("the Ink renderer's bundle needs `node` on PATH")?;
    Ok((node, vec![bundle.into()]))
}

/// `node` on `PATH`.
pub fn find_node(path: Option<&str>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .map(|d| d.join("node"))
        .find(|p| p.is_file())
}

/// The socket's path: in radar's state folder, else the temporary folder
/// when that path would be too long for a Unix socket.
pub fn socket_path(state: Option<PathBuf>, pid: u32) -> PathBuf {
    let name = format!("ink-{pid}.sock");
    let short = |p: &Path| p.as_os_str().len() < 100;
    match state.map(|s| s.join("radar").join(&name)) {
        Some(p) if short(&p) => p,
        _ => std::env::temp_dir().join(format!("radar-{name}")),
    }
}

/// The terminal's line settings as they were before the renderer ran
/// (`stty -g`), put back after it exits: raw mode off.
#[derive(Debug, Clone, Default)]
pub struct TtyModes(Option<String>);

impl TtyModes {
    /// Read them from the terminal on stdin; nothing when stdin is not a
    /// terminal.
    pub fn save() -> Self {
        let out = std::process::Command::new("stty")
            .arg("-g")
            .stdin(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::null())
            .output();
        Self(
            out.ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|s| !s.is_empty()),
        )
    }

    pub fn restore(&self) {
        if let Some(modes) = &self.0 {
            let _ = std::process::Command::new("stty")
                .arg(modes)
                .stdin(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

/// How a run of the renderer ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// The user quit.
    Quit,
    /// The renderer cannot run (refused, or crashed past the policy): the
    /// Ratatui renderer takes over; the words say why, in one line.
    FallBack(String),
}

/// How one child's run ended.
enum Run {
    Quit,
    Crashed(String),
    Refused(String),
}

/// The renderer, run and run again.
pub struct Supervisor<W: Write> {
    /// The command that starts a renderer (`node <bundle>`); the socket
    /// is added to its environment as `RADAR_IPC_SOCKET`.
    pub command: Box<dyn FnMut() -> tokio::process::Command + Send>,
    pub socket: PathBuf,
    /// Where the terminal is put back ([`RESTORE`]): stdout, or a test's
    /// buffer.
    pub terminal: W,
    pub modes: TtyModes,
    pub policy: RestartPolicy,
    /// [`STALLED_AFTER`]; tests shorten it.
    pub stalled_after: Duration,
    /// The running renderer's process id, for the log and for tests that
    /// end it.
    pub pid: Option<u32>,
    /// Every renderer process started, in order.
    pub started: Vec<u32>,
    /// Why each renderer that crashed stopped, in order.
    pub stopped: Vec<String>,
}

impl<W: Write> Supervisor<W> {
    pub fn new(command: Box<dyn FnMut() -> tokio::process::Command + Send>, socket: PathBuf, terminal: W) -> Self {
        Self {
            command,
            socket,
            terminal,
            modes: TtyModes::default(),
            policy: RestartPolicy::default(),
            stalled_after: STALLED_AFTER,
            pid: None,
            started: Vec::new(),
            stopped: Vec::new(),
        }
    }

    /// Put the terminal back as it was before the renderer.
    pub fn restore(&mut self) {
        let _ = self.terminal.write_all(RESTORE.as_bytes());
        let _ = self.terminal.flush();
        self.modes.restore();
    }

    /// Run the renderer until the user quits or it cannot run. `events`
    /// are the host's own (ticks, finished work); every effect the host
    /// asks for goes to `perform`, and `filter` sees each event first (the
    /// runner takes the clipboard there).
    pub async fn run(
        &mut self,
        bridge: &mut Bridge,
        events: &mut UnboundedReceiver<Event>,
        perform: &mut dyn FnMut(HostEffect),
        filter: &mut dyn FnMut(Event, &mut Bridge) -> Option<Event>,
    ) -> Result<Ended> {
        loop {
            let run = match self.run_once(bridge, events, perform, filter).await {
                Ok(run) => run,
                Err(e) => Run::Crashed(format!("{e:#}")),
            };
            self.pid = None;
            self.restore();
            match run {
                Run::Quit => return Ok(Ended::Quit),
                Run::Refused(why) => {
                    bridge.host_mut().message = Some(crate::plugin::Message::new(None, why.clone()));
                    return Ok(Ended::FallBack(why));
                }
                Run::Crashed(why) if !bridge.host().quit => {
                    crate::log::warn(format!("Ink renderer stopped: {why}"));
                    self.stopped.push(why.clone());
                    if !self.policy.restart(Instant::now()) {
                        let why = format!(
                            "the Ink renderer stopped {} times in a minute ({why}); using the Ratatui renderer",
                            self.policy.max + 1
                        );
                        bridge.host_mut().message = Some(crate::plugin::Message::new(None, why.clone()));
                        return Ok(Ended::FallBack(why));
                    }
                    bridge.host_mut().message = Some(crate::plugin::Message::new(
                        None,
                        "the renderer stopped and was started again",
                    ));
                }
                _ => return Ok(Ended::Quit),
            }
        }
    }

    async fn run_once(
        &mut self,
        bridge: &mut Bridge,
        events: &mut UnboundedReceiver<Event>,
        perform: &mut dyn FnMut(HostEffect),
        filter: &mut dyn FnMut(Event, &mut Bridge) -> Option<Event>,
    ) -> Result<Run, anyhow::Error> {
        let _ = std::fs::remove_file(&self.socket);
        if let Some(dir) = self.socket.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let listener = UnixListener::bind(&self.socket)
            .with_context(|| format!("cannot listen on {}", self.socket.display()))?;
        let mut command = (self.command)();
        command
            .env("RADAR_IPC_SOCKET", &self.socket)
            .kill_on_drop(true);
        let mut child = command.spawn().context("cannot start the Ink renderer")?;
        self.pid = child.id();
        if let Some(pid) = self.pid {
            self.started.push(pid);
            crate::log::info(format!("Ink renderer started, pid {pid}"));
        }
        let accepted = tokio::select! {
            a = tokio::time::timeout(CONNECT_WITHIN, listener.accept()) => a,
            status = child.wait() => {
                let _ = std::fs::remove_file(&self.socket);
                return Ok(exit_of(status.ok()));
            }
        };
        let _ = std::fs::remove_file(&self.socket);
        let (stream, _) = match accepted {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Ok(Run::Crashed(format!("no connection: {e}"))),
            Err(_) => {
                let _ = child.kill().await;
                return Ok(Run::Crashed("it did not connect".into()));
            }
        };
        bridge.reconnect();
        let (mut read, mut write) = stream.into_split();
        // Lines go out through a queue of at most MAX_QUEUED: the host
        // never waits on the renderer, and a renderer that stops reading
        // fills it and is ended (then started again and told everything).
        let (out, mut queued) = tokio::sync::mpsc::channel::<Vec<u8>>(MAX_QUEUED);
        let writer = tokio::spawn(async move {
            while let Some(line) = queued.recv().await {
                if write.write_all(&line).await.is_err() {
                    break;
                }
            }
        });
        let mut lines = Lines::default();
        let mut buf = vec![0u8; 64 * 1024];
        let mut refused: Option<String> = None;
        let mut leaving = false;
        loop {
            // What the renderer must hear now.
            for message in bridge.outgoing() {
                if let ToRenderer::Refuse { message: why, .. } = &message {
                    refused = Some(why.clone());
                }
                if !queue(&out, encode(&message), self.stalled_after).await {
                    writer.abort();
                    let _ = child.kill().await;
                    return Ok(Run::Crashed(format!(
                        "it stopped reading ({MAX_QUEUED} messages waiting)"
                    )));
                }
            }
            if refused.is_some() || leaving {
                break;
            }
            tokio::select! {
                n = read.read(&mut buf) => {
                    let n = match n {
                        Ok(0) | Err(_) => {
                            // The renderer closed its end: it is leaving
                            // or gone; its exit says which.
                            let status = tokio::time::timeout(LEAVE_WITHIN, child.wait()).await;
                            return Ok(match status {
                                Ok(s) => exit_of(s.ok()),
                                Err(_) => {
                                    let _ = child.kill().await;
                                    Run::Crashed("it closed the connection".into())
                                }
                            });
                        }
                        Ok(n) => n,
                    };
                    let got = match lines.push(&buf[..n]) {
                        Ok(got) => got,
                        Err(_) => {
                            let _ = child.kill().await;
                            return Ok(Run::Crashed("a message over the line bound".into()));
                        }
                    };
                    for line in got {
                        let message: ToHost = match serde_json::from_slice(&line) {
                            Ok(m) => m,
                            Err(e) => {
                                crate::log::warn(format!("renderer sent what is not a message: {e}"));
                                continue;
                            }
                        };
                        let received = bridge.receive(message);
                        for effect in received.effects {
                            perform(effect);
                        }
                        leaving |= received.quit;
                    }
                }
                // The host's own events wait for the renderer's hello (a
                // handshake's milliseconds), so none folds into the first
                // snapshot unseen.
                e = events.recv(), if bridge.welcomed() => {
                    let Some(e) = e else { leaving = true; continue };
                    if let Some(e) = filter(e, bridge) {
                        for effect in bridge.update(e) {
                            perform(effect);
                        }
                    }
                    leaving |= bridge.host().quit;
                }
                // While the splash shows, its next frame (the loop's
                // `outgoing` sends it).
                () = splash_tick(bridge.next_frame()) => {}
                status = child.wait() => return Ok(exit_of(status.ok())),
            }
        }
        if let Some(why) = refused {
            drop(out);
            let _ = tokio::time::timeout(LEAVE_WITHIN, writer).await;
            let _ = tokio::time::timeout(LEAVE_WITHIN, child.wait()).await;
            let _ = child.kill().await;
            return Ok(Run::Refused(why));
        }
        let _ = queue(
            &out,
            encode(&ToRenderer::Shutdown {
                reason: "quit".into(),
            }),
            LEAVE_WITHIN,
        )
        .await;
        drop(out);
        let _ = tokio::time::timeout(LEAVE_WITHIN, writer).await;
        if tokio::time::timeout(LEAVE_WITHIN, child.wait()).await.is_err() {
            let _ = child.kill().await;
        }
        Ok(Run::Quit)
    }
}

/// Put `line` on the renderer's queue. When the queue is full, wait up to
/// `stalled_after` for the writer to make room (it may not have had its
/// turn yet); false when it made none: the renderer stopped reading.
async fn queue(out: &tokio::sync::mpsc::Sender<Vec<u8>>, line: Vec<u8>, stalled_after: Duration) -> bool {
    use tokio::sync::mpsc::error::TrySendError;
    match out.try_send(line) {
        Ok(()) => true,
        Err(TrySendError::Closed(_)) => false,
        Err(TrySendError::Full(line)) => matches!(tokio::time::timeout(stalled_after, out.send(line)).await, Ok(Ok(()))),
    }
}

/// After `every` while the splash shows; never when it does not.
async fn splash_tick(every: Option<Duration>) {
    match every {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
    }
}

fn exit_of(status: Option<std::process::ExitStatus>) -> Run {
    match status {
        Some(s) if s.success() => Run::Quit,
        Some(s) => Run::Crashed(match s.code() {
            Some(code) => format!("it exited with code {code}"),
            None => "it was ended by a signal".into(),
        }),
        None => Run::Crashed("its exit is unknown".into()),
    }
}
