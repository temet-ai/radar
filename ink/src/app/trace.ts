/**
 * Latency tracing (`RADAR_TRACE_LATENCY=1`, the host's variable, inherited): how long a key takes to show, and how long a
 * snapshot takes from what caused it on the host (a key's intent, an agent's words) to the frame that draws it. Lines
 * go to radar.log through the host (`ink: latency ...`). Off, nothing is wrapped and every call returns at once.
 *
 * A measurement starts at a mark (a read from stdin, a snapshot line received), waits for the App to render after it,
 * and ends at the first write to stdout after that render: the frame that shows it.
 */
import type { SnapshotTrace } from "../protocol/generated/SnapshotTrace.js";

type Pending = { t0: number; rendered: boolean; done: (paintedAt: number) => string | null };

/** A key not drawn within this long changed nothing on screen: it is dropped, not timed against a later frame. */
const KEY_EXPIRES_MS = 1000;

export class Tracer {
  readonly on: boolean;
  private sink: (text: string) => void = () => {};
  private pending: Pending[] = [];
  /** When each intent's input was read, by its seq, until a snapshot answers it. */
  private inputs = new Map<number, number>();
  private lastInput: number | undefined;
  private line: { at: number; bytes: number } | undefined;

  constructor(on: boolean) {
    this.on = on;
  }

  /** Where the lines go (the session's log). */
  logTo(sink: (text: string) => void) {
    this.sink = sink;
  }

  /** Bytes were read from the terminal: a key or a mouse report. */
  input() {
    if (!this.on) return;
    const t0 = performance.now();
    this.lastInput = t0;
    this.pending.push({ t0, rendered: false, done: (at) => (at - t0 > KEY_EXPIRES_MS ? null : `latency key: painted ${(at - t0).toFixed(1)} ms after the read`) });
  }

  /** An intent left for the host, made from the last read. */
  intent(seq: number) {
    if (!this.on) return;
    this.inputs.set(seq, this.lastInput ?? performance.now());
    if (this.inputs.size > 256) this.inputs.delete(this.inputs.keys().next().value!);
  }

  /** A line arrived from the host (before it is parsed). */
  received(bytes: number) {
    if (!this.on) return;
    this.line = { at: performance.now(), bytes };
  }

  /** A snapshot was taken in: timed from its cause on the host to its frame. */
  snapshot(cursor: number, trace: SnapshotTrace | undefined) {
    if (!this.on) return;
    const t0 = this.line?.at ?? performance.now();
    const arrived = performance.timeOrigin + t0;
    const bytes = this.line?.bytes ?? 0;
    const input = trace?.seq !== undefined ? this.inputs.get(trace.seq) : undefined;
    if (trace?.seq !== undefined) for (const s of [...this.inputs.keys()]) if (s <= trace.seq) this.inputs.delete(s);
    this.pending.push({
      t0,
      rendered: false,
      done: (at) => {
        const ink = at - t0;
        if (!trace) return `latency snapshot ${cursor}: ink ${ink.toFixed(1)} ms (${bytes} bytes)`;
        const pipe = arrived - (trace.at_ms + trace.host_ms);
        const total = performance.timeOrigin + at - trace.at_ms;
        const key = input !== undefined ? `; key ${(at - input).toFixed(1)} ms (seq ${trace.seq})` : "";
        return `latency snapshot ${cursor} answers ${trace.cause}: host ${trace.host_ms.toFixed(1)} + pipe ${pipe.toFixed(1)} + ink ${ink.toFixed(1)} = ${total.toFixed(1)} ms${key}, ${bytes} bytes`;
      },
    });
  }

  /** The App rendered: whatever was marked before it is in the next frame. */
  rendered() {
    if (!this.on) return;
    for (const p of this.pending) p.rendered = true;
  }

  /** Something was written to the terminal: the frame for every mark rendered before it. */
  wrote() {
    if (!this.on || this.pending.length === 0) return;
    const at = performance.now();
    const keep: Pending[] = [];
    for (const p of this.pending) {
      if (!p.rendered) {
        if (at - p.t0 <= KEY_EXPIRES_MS) keep.push(p);
        continue;
      }
      const text = p.done(at);
      if (text) this.sink(text);
    }
    this.pending = keep;
  }
}

export const tracer = new Tracer(typeof process !== "undefined" && process.env.RADAR_TRACE_LATENCY === "1");

/** Wrap `stdout.write` and `stdin.read` so frames and reads are seen (only when tracing). */
export function traceTerminal(stdout: { write: (...a: any[]) => boolean }, stdin: { read: (...a: any[]) => any }) {
  if (!tracer.on) return;
  const write = stdout.write.bind(stdout);
  stdout.write = (...a: any[]) => {
    const r = write(...a);
    tracer.wrote();
    return r;
  };
  const read = stdin.read.bind(stdin);
  stdin.read = (...a: any[]) => {
    const d = read(...a);
    if (d != null) tracer.input();
    return d;
  };
}
