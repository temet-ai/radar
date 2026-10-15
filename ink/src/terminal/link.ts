import net from "node:net";
import { FrameReader, inputFrame, killFrame, resizeFrame, restartFrame, type Frame } from "./frames.js";

/** A terminal the host runs, as the surface sees it. The socket one is real; tests and stories give another. */
export interface TerminalLink {
  /** Called for each frame: the replayed ring first, then live output and state changes. */
  onFrame(fn: (f: Frame) => void): void;
  /** The connection ended (the host went away or the socket is gone). */
  onClose(fn: (why: string) => void): void;
  input(data: string | Uint8Array): void;
  resize(columns: number, rows: number): void;
  restart(): void;
  kill(): void;
  close(): void;
}

/** Attaches to the host's per-terminal Unix socket (crates/radar-sessions-pty/src/server.rs). */
export function connectSocket(path: string): TerminalLink {
  const sock = net.createConnection(path);
  const reader = new FrameReader();
  let frameFn: (f: Frame) => void = () => {};
  let closeFn: (why: string) => void = () => {};
  let closed = false;
  const end = (why: string) => {
    if (closed) return;
    closed = true;
    closeFn(why);
  };
  sock.on("data", (chunk) => {
    try {
      for (const f of reader.push(typeof chunk === "string" ? Buffer.from(chunk) : chunk)) frameFn(f);
    } catch (e) {
      end(String(e));
      sock.destroy();
    }
  });
  sock.on("error", (e) => end(e.message));
  sock.on("close", () => end("closed"));
  const send = (b: Uint8Array) => {
    if (!closed && sock.writable) sock.write(b);
  };
  return {
    onFrame: (fn) => void (frameFn = fn),
    onClose: (fn) => void (closeFn = fn),
    input: (d) => send(inputFrame(d)),
    resize: (c, r) => send(resizeFrame(c, r)),
    restart: () => send(restartFrame()),
    kill: () => send(killFrame()),
    close: () => sock.destroy(),
  };
}
