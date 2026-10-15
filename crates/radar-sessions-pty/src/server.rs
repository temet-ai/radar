//! The socket a renderer attaches to.
//!
//! One Unix socket per terminal, in a folder only the user can enter. On
//! attach the renderer is sent the state and the ring of recent output, then
//! the live frames; it sends input, resizes, restart and kill (see
//! [`crate::frame`]). The socket outlives any renderer, so a restarted Ink
//! finds the program as it was.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::frame::{inp, out, read_frame, write_frame, write_output};
use crate::term::{Msg, Shared, Terminal, TerminalSpec};

/// A terminal and the socket it is served on. Dropping it ends the program
/// and removes the socket.
pub struct ServedTerminal {
    term: Terminal,
    socket: PathBuf,
    stop: Arc<AtomicBool>,
}

impl ServedTerminal {
    /// Starts `spec` and serves it on `<dir>/pty-<id>.sock`. `dir` is made
    /// with mode 0700 when missing.
    pub fn start(spec: TerminalSpec, dir: &Path, id: &str) -> Result<Self> {
        let term = Terminal::spawn(spec)?;
        Self::serve(term, dir, id)
    }

    /// Serves a terminal that is already made (also one that failed to start,
    /// so the renderer can show why and offer a restart).
    pub fn serve(term: Terminal, dir: &Path, id: &str) -> Result<Self> {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        let socket = dir.join(format!("pty-{}.sock", sanitise(id)));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).with_context(|| format!("bind {}", socket.display()))?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::clone(&term.shared);
        let spec = term.spec().clone();
        let flag = Arc::clone(&stop);
        std::thread::Builder::new().name("pty-accept".into()).spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let shared = Arc::clone(&shared);
                        let spec = spec.clone();
                        let _ = std::thread::Builder::new()
                            .name("pty-client".into())
                            .spawn(move || serve_client(stream, shared, spec));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
        })?;
        Ok(Self { term, socket, stop })
    }

    pub fn terminal(&self) -> &Terminal {
        &self.term
    }
    pub fn socket(&self) -> &Path {
        &self.socket
    }
}

impl Drop for ServedTerminal {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.term.kill();
        let _ = std::fs::remove_file(&self.socket);
    }
}

fn sanitise(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn serve_client(stream: UnixStream, shared: Arc<Shared>, spec: TerminalSpec) {
    let Ok(mut reader) = stream.try_clone() else { return };
    let mut writer = stream;
    let (state, ring, rx) = shared.attach();
    let hello = (|| -> std::io::Result<()> {
        write_frame(&mut writer, out::STATE, &serde_json::to_vec(&state).unwrap_or_default())?;
        write_output(&mut writer, &ring)
    })();
    if hello.is_err() {
        return;
    }
    // Input side: its own thread; ends when the renderer closes.
    let sh = Arc::clone(&shared);
    let closer = writer.try_clone().ok();
    let _ = std::thread::Builder::new().name("pty-client-in".into()).spawn(move || {
        while let Ok((kind, payload)) = read_frame(&mut reader) {
            match kind {
                inp::INPUT => {
                    let _ = sh.write_input(&payload);
                }
                inp::RESIZE if payload.len() == 4 => {
                    let c = u16::from_be_bytes([payload[0], payload[1]]);
                    let r = u16::from_be_bytes([payload[2], payload[3]]);
                    let _ = sh.resize(c, r);
                }
                inp::RESTART => {
                    let _ = sh.start(&spec);
                }
                inp::KILL => sh.kill(),
                _ => {}
            }
        }
        if let Some(c) = closer {
            let _ = c.shutdown(std::net::Shutdown::Both);
        }
    });
    // Output side: until the renderer goes or falls too far behind.
    while let Ok(msg) = rx.recv() {
        let r = match msg {
            Msg::Output(b) => write_output(&mut writer, &b),
            Msg::State(s) => write_frame(&mut writer, out::STATE, &serde_json::to_vec(&s).unwrap_or_default()),
        };
        if r.is_err() {
            break;
        }
    }
    let _ = writer.flush();
    let _ = writer.shutdown(std::net::Shutdown::Both);
}

