//! A program in a pseudo-terminal, owned by the host.
//!
//! The host keeps the program, its last output (a bounded ring) and its exit
//! status. A renderer attaches through [`crate::server`], gets the ring back
//! and then the live bytes; restarting the renderer never ends the program.
//! The host draws nothing: the renderer parses the bytes into a screen.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{Context, Result};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::frame::StateFrame;

/// The last output kept for a renderer that attaches late.
pub const RING_BYTES: usize = 256 * 1024;
/// Frames a slow renderer may lag by before it is dropped (it reattaches and
/// is replayed the ring).
const SUBSCRIBER_QUEUE: usize = 1024;
/// How long after the program exits we wait for the reader to drain.
const DRAIN: Duration = Duration::from_millis(300);

/// What to run, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSpec {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Added to the environment (never read from radar's settings for secrets).
    pub env: Vec<(String, String)>,
    pub columns: u16,
    pub rows: u16,
}

impl TerminalSpec {
    pub fn new(command: impl Into<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            env: Vec::new(),
            columns: 80,
            rows: 24,
        }
    }
    pub fn args<I: IntoIterator<Item = S>, S: Into<String>>(mut self, args: I) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }
    pub fn size(mut self, columns: u16, rows: u16) -> Self {
        self.columns = columns.max(1);
        self.rows = rows.max(1);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TermState {
    Running,
    Exited { code: Option<i32>, signal: Option<String> },
    Failed { message: String },
}

impl TermState {
    pub fn is_running(&self) -> bool {
        matches!(self, TermState::Running)
    }
}

/// What a subscriber receives.
#[derive(Clone, Debug)]
pub enum Msg {
    Output(Vec<u8>),
    State(StateFrame),
}

struct Inner {
    generation: u64,
    state: TermState,
    size: (u16, u16),
    ring: Vec<u8>,
    subs: Vec<SyncSender<Msg>>,
}

/// Held by the terminal, its threads and the socket server.
pub(crate) struct Shared {
    inner: Mutex<Inner>,
    // Separate locks: a write that blocks on a full pty must not stop the
    // reader from draining it.
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    killer: Mutex<Option<Box<dyn ChildKiller + Send + Sync>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    fn frame(inner: &Inner) -> StateFrame {
        let (state, code, signal, message) = match &inner.state {
            TermState::Running => ("running", None, None, None),
            TermState::Exited { code, signal } => ("exited", *code, signal.clone(), None),
            TermState::Failed { message } => ("failed", None, None, Some(message.clone())),
        };
        StateFrame {
            state: state.into(),
            code,
            signal,
            message,
            generation: inner.generation,
            columns: inner.size.0,
            rows: inner.size.1,
        }
    }

    fn broadcast(inner: &mut Inner, msg: Msg) {
        inner.subs.retain(|s| s.try_send(msg.clone()).is_ok());
    }

    fn push_output(&self, generation: u64, bytes: &[u8]) {
        let mut inner = lock(&self.inner);
        if inner.generation != generation {
            return;
        }
        inner.ring.extend_from_slice(bytes);
        if inner.ring.len() > RING_BYTES {
            let cut = inner.ring.len() - RING_BYTES;
            inner.ring.drain(..cut);
        }
        Self::broadcast(&mut inner, Msg::Output(bytes.to_vec()));
    }

    fn set_state(&self, generation: u64, state: TermState) {
        let mut inner = lock(&self.inner);
        if inner.generation != generation {
            return;
        }
        inner.state = state;
        let frame = Self::frame(&inner);
        Self::broadcast(&mut inner, Msg::State(frame));
    }

    /// The state, the ring and a receiver for what follows, taken together so
    /// nothing is missed or doubled.
    pub(crate) fn attach(&self) -> (StateFrame, Vec<u8>, mpsc::Receiver<Msg>) {
        let mut inner = lock(&self.inner);
        let (tx, rx) = mpsc::sync_channel(SUBSCRIBER_QUEUE);
        inner.subs.push(tx);
        (Self::frame(&inner), inner.ring.clone(), rx)
    }

    pub(crate) fn write_input(&self, bytes: &[u8]) -> Result<()> {
        let mut w = lock(&self.writer);
        let w = w.as_mut().context("the program is not running")?;
        w.write_all(bytes)?;
        w.flush()?;
        Ok(())
    }

    pub(crate) fn resize(&self, columns: u16, rows: u16) -> Result<()> {
        let (columns, rows) = (columns.max(1), rows.max(1));
        if let Some(m) = lock(&self.master).as_ref() {
            m.resize(PtySize { rows, cols: columns, pixel_width: 0, pixel_height: 0 })?;
        }
        let mut inner = lock(&self.inner);
        if inner.size != (columns, rows) {
            inner.size = (columns, rows);
            let frame = Self::frame(&inner);
            Self::broadcast(&mut inner, Msg::State(frame));
        }
        Ok(())
    }

    pub(crate) fn kill(&self) {
        if let Some(k) = lock(&self.killer).as_mut() {
            let _ = k.kill();
        }
    }

    pub(crate) fn state(&self) -> (TermState, StateFrame) {
        let inner = lock(&self.inner);
        (inner.state.clone(), Self::frame(&inner))
    }

    pub(crate) fn start(self: &Arc<Self>, spec: &TerminalSpec) -> Result<()> {
        self.kill();
        let size = lock(&self.inner).size;
        let generation = {
            let mut inner = lock(&self.inner);
            inner.generation += 1;
            inner.ring.clear();
            inner.state = TermState::Running;
            let frame = Self::frame(&inner);
            Self::broadcast(&mut inner, Msg::State(frame));
            inner.generation
        };
        match self.launch(spec, size, generation) {
            Ok(()) => Ok(()),
            Err(e) => {
                let message = format!("{e:#}");
                self.set_state(generation, TermState::Failed { message: message.clone() });
                *lock(&self.writer) = None;
                *lock(&self.master) = None;
                *lock(&self.killer) = None;
                Err(anyhow::anyhow!(message))
            }
        }
    }

    fn launch(self: &Arc<Self>, spec: &TerminalSpec, size: (u16, u16), generation: u64) -> Result<()> {
        let pair = native_pty_system()
            .openpty(PtySize { rows: size.1, cols: size.0, pixel_width: 0, pixel_height: 0 })
            .context("no pseudo-terminal")?;
        let mut cmd = CommandBuilder::new(&spec.command);
        cmd.args(&spec.args);
        cmd.cwd(&spec.cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // The child is another program, not a radar renderer.
        cmd.env_remove("RADAR_IPC_SOCKET");
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("could not start {}", spec.command))?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        *lock(&self.writer) = Some(pair.master.take_writer()?);
        *lock(&self.killer) = Some(child.clone_killer());
        *lock(&self.master) = Some(pair.master);

        let (done_tx, done_rx) = mpsc::channel::<()>();
        let me = Arc::clone(self);
        std::thread::Builder::new().name("pty-reader".into()).spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => me.push_output(generation, &buf[..n]),
                }
            }
            let _ = done_tx.send(());
        })?;
        let me = Arc::clone(self);
        std::thread::Builder::new().name("pty-waiter".into()).spawn(move || {
            let status = child.wait();
            let _ = done_rx.recv_timeout(DRAIN);
            let state = match status {
                Ok(s) => TermState::Exited {
                    code: Some(s.exit_code() as i32),
                    signal: s.signal().map(str::to_string),
                },
                Err(_) => TermState::Exited { code: None, signal: None },
            };
            me.set_state(generation, state);
        })?;
        Ok(())
    }
}

/// A running (or finished) program with its screen bytes.
pub struct Terminal {
    pub(crate) shared: Arc<Shared>,
    spec: TerminalSpec,
}

impl Terminal {
    /// Starts the program. An error is a program that could not start; the
    /// terminal then reports `Failed` and `restart` tries again.
    pub fn spawn(spec: TerminalSpec) -> Result<Self> {
        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner {
                generation: 0,
                state: TermState::Running,
                size: (spec.columns.max(1), spec.rows.max(1)),
                ring: Vec::new(),
                subs: Vec::new(),
            }),
            writer: Mutex::new(None),
            master: Mutex::new(None),
            killer: Mutex::new(None),
        });
        let term = Self { shared, spec };
        term.shared.start(&term.spec)?;
        Ok(term)
    }

    pub fn spec(&self) -> &TerminalSpec {
        &self.spec
    }
    pub fn state(&self) -> TermState {
        self.shared.state().0
    }
    pub fn frame(&self) -> StateFrame {
        self.shared.state().1
    }
    /// The bytes of the screen so far (bounded to [`RING_BYTES`]).
    pub fn replay(&self) -> Vec<u8> {
        lock(&self.shared.inner).ring.clone()
    }
    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        self.shared.write_input(bytes)
    }
    pub fn resize(&self, columns: u16, rows: u16) -> Result<()> {
        self.shared.resize(columns, rows)
    }
    pub fn restart(&self) -> Result<()> {
        self.shared.start(&self.spec)
    }
    pub fn kill(&self) {
        self.shared.kill();
    }
    /// For tests and tools: block until the program is no longer running.
    pub fn wait_exit(&self, timeout: Duration) -> Option<TermState> {
        let end = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < end {
            let s = self.state();
            if !s.is_running() {
                return Some(s);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.shared.kill();
    }
}
