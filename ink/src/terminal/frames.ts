// The wire to the host's terminal (crates/radar-sessions-pty/src/frame.rs): `[type u8][length u32 BE][payload]`.
// Output is raw bytes; state is JSON. Keep this file in step with frame.rs.

export const OUT_OUTPUT = 1;
export const OUT_STATE = 2;
export const IN_INPUT = 1;
export const IN_RESIZE = 2;
export const IN_RESTART = 3;
export const IN_KILL = 4;

/** A frame's payload is at most this; a longer one closes the connection (frame.rs MAX_PAYLOAD). */
export const MAX_PAYLOAD = 1 << 20;

export interface TerminalState {
  state: "running" | "exited" | "failed";
  code?: number | null;
  signal?: string | null;
  message?: string | null;
  /** Raised on each (re)start; the screen is cleared when it changes. */
  generation: number;
  columns: number;
  rows: number;
}

export type Frame = { kind: "output"; bytes: Uint8Array } | { kind: "state"; state: TerminalState };

export function encodeFrame(kind: number, payload: Uint8Array): Uint8Array {
  const out = new Uint8Array(5 + payload.length);
  out[0] = kind;
  new DataView(out.buffer).setUint32(1, payload.length, false);
  out.set(payload, 5);
  return out;
}

export const inputFrame = (data: string | Uint8Array): Uint8Array =>
  encodeFrame(IN_INPUT, typeof data === "string" ? new TextEncoder().encode(data) : data);

export function resizeFrame(columns: number, rows: number): Uint8Array {
  const p = new Uint8Array(4);
  const v = new DataView(p.buffer);
  v.setUint16(0, Math.max(1, Math.min(65535, columns)), false);
  v.setUint16(2, Math.max(1, Math.min(65535, rows)), false);
  return encodeFrame(IN_RESIZE, p);
}

export const restartFrame = (): Uint8Array => encodeFrame(IN_RESTART, new Uint8Array());
export const killFrame = (): Uint8Array => encodeFrame(IN_KILL, new Uint8Array());

/** Splits a byte stream into frames; a partial frame waits for the next push. Throws on a frame that is too long. */
export class FrameReader {
  private buf: Uint8Array = new Uint8Array(0);

  push(chunk: Uint8Array): Frame[] {
    if (this.buf.length === 0) this.buf = chunk;
    else {
      const joined = new Uint8Array(this.buf.length + chunk.length);
      joined.set(this.buf);
      joined.set(chunk, this.buf.length);
      this.buf = joined;
    }
    const frames: Frame[] = [];
    for (;;) {
      if (this.buf.length < 5) break;
      const len = new DataView(this.buf.buffer, this.buf.byteOffset, this.buf.byteLength).getUint32(1, false);
      if (len > MAX_PAYLOAD) throw new Error("terminal frame too long");
      if (this.buf.length < 5 + len) break;
      const kind = this.buf[0]!;
      const payload = this.buf.slice(5, 5 + len);
      this.buf = this.buf.subarray(5 + len);
      if (kind === OUT_OUTPUT) frames.push({ kind: "output", bytes: payload });
      else if (kind === OUT_STATE) {
        try {
          frames.push({ kind: "state", state: JSON.parse(new TextDecoder().decode(payload)) as TerminalState });
        } catch {
          /* a state we cannot read is ignored; the next one replaces it */
        }
      }
    }
    return frames;
  }
}
