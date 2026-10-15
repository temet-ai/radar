//! The wire between the host's terminal and the renderer's drawing of it.
//!
//! A frame is `[type: u8][length: u32 big-endian][payload]`. Bytes are not
//! text, so they are not wrapped in JSON; the one JSON payload is the state.
//! `ink/src/terminal/frames.ts` is the other side; keep them in step.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

/// A frame's payload is at most this; a longer one closes the connection.
pub const MAX_PAYLOAD: usize = 1 << 20;

/// Host to renderer.
pub mod out {
    /// Raw bytes the program wrote.
    pub const OUTPUT: u8 = 1;
    /// A [`super::StateFrame`] as JSON.
    pub const STATE: u8 = 2;
}

/// Renderer to host.
pub mod inp {
    /// Bytes for the program's stdin (keys, mouse reports, paste).
    pub const INPUT: u8 = 1;
    /// `columns: u16, rows: u16`, big-endian.
    pub const RESIZE: u8 = 2;
    /// Start the program again (after it exited or failed).
    pub const RESTART: u8 = 3;
    /// End the program (SIGHUP through the pty, then kill).
    pub const KILL: u8 = 4;
}

/// What the renderer needs to say about the program besides its bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateFrame {
    /// `running`, `exited` or `failed`.
    pub state: String,
    pub code: Option<i32>,
    pub signal: Option<String>,
    pub message: Option<String>,
    /// Raised on each (re)start; the renderer clears its screen when it
    /// changes.
    pub generation: u64,
    pub columns: u16,
    pub rows: u16,
}

pub fn write_frame(w: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    debug_assert!(payload.len() <= MAX_PAYLOAD);
    let mut head = [0u8; 5];
    head[0] = kind;
    head[1..].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    w.write_all(&head)?;
    w.write_all(payload)?;
    w.flush()
}

/// Writes `bytes` as output frames of at most [`MAX_PAYLOAD`].
pub fn write_output(w: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    for chunk in bytes.chunks(MAX_PAYLOAD) {
        write_frame(w, out::OUTPUT, chunk)?;
    }
    Ok(())
}

pub fn read_frame(r: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut head = [0u8; 5];
    r.read_exact(&mut head)?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > MAX_PAYLOAD {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too long"));
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok((head[0], payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_round_trips_and_a_long_one_is_refused() {
        let mut buf = Vec::new();
        write_frame(&mut buf, inp::INPUT, b"ls\r").unwrap();
        let (k, p) = read_frame(&mut buf.as_slice()).unwrap();
        assert_eq!((k, p.as_slice()), (inp::INPUT, &b"ls\r"[..]));

        let mut bad = vec![1u8];
        bad.extend_from_slice(&((MAX_PAYLOAD as u32) + 1).to_be_bytes());
        assert!(read_frame(&mut bad.as_slice()).is_err());
    }

    #[test]
    fn output_longer_than_a_frame_is_split() {
        let mut buf = Vec::new();
        write_output(&mut buf, &vec![b'x'; MAX_PAYLOAD + 10]).unwrap();
        let mut r = buf.as_slice();
        assert_eq!(read_frame(&mut r).unwrap().1.len(), MAX_PAYLOAD);
        assert_eq!(read_frame(&mut r).unwrap().1.len(), 10);
    }
}
