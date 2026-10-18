import { tokenize, type InputEvent } from "./tokens.js";

/**
 * Ink drops terminal reports it cannot map to a key (SGR mouse reports among them) and joins an ESC to the
 * next read as Alt-<key>. radar reads its own input instead: the tap wraps `stdin.read()`, splits each read into
 * keys and mouse reports in the order they arrived (`tokenize`), hands them to the bus as one ordered list,
 * and gives Ink nothing (Ink keeps the terminal in raw mode and the stream flowing).
 */
export class MouseBus {
  private handlers = new Set<(data: string) => void>();
  private listeners = new Set<(events: InputEvent[]) => void>();
  private rawListeners = new Set<(text: string) => void>();
  private rawClaim: ((text: string) => void) | undefined;
  /** Raw SGR mouse reports, for a listener that only wants the pointer. */
  on(fn: (data: string) => void): () => void {
    this.handlers.add(fn);
    return () => void this.handlers.delete(fn);
  }
  emit(data: string) {
    for (const h of [...this.handlers]) h(data);
  }
  /** Every key and mouse report of a read, in order. */
  onInput(fn: (events: InputEvent[]) => void): () => void {
    this.listeners.add(fn);
    return () => void this.listeners.delete(fn);
  }
  emitInput(events: InputEvent[]) {
    for (const l of [...this.listeners]) l(events);
  }
  /** Observe a claimed read once, before its terminal receives the unchanged bytes. Normal tokenized reads use onInput. */
  onRawInput(fn: (text: string) => void): () => void {
    this.rawListeners.add(fn);
    return () => void this.rawListeners.delete(fn);
  }
  /**
   * Takes every read whole and untokenized, for a surface that passes the terminal's bytes on to a program (the
   * dock's terminal tab): radar's key tokenizer drops what it has no chord for, and a program needs all of it. One
   * claim at a time; the newest wins, and the returned function releases it.
   */
  claimRaw(fn: (text: string) => void): () => void {
    this.rawClaim = fn;
    return () => {
      if (this.rawClaim === fn) this.rawClaim = undefined;
    };
  }
  /** Whether anyone reads the keys; without one the read passes through to Ink. */
  emitInputReady(): boolean {
    return this.listeners.size > 0 || !!this.rawClaim;
  }
  /** The text of a read, to the raw claim if there is one; false when nobody holds it. */
  emitRaw(text: string): boolean {
    const claim = this.rawClaim;
    if (!claim) return false;
    for (const fn of [...this.rawListeners]) fn(text);
    claim(text);
    return true;
  }
}

export const PARTIAL_FLUSH_MS = 30;

export function tapMouse(stdin: { read: (...a: any[]) => any }, bus: MouseBus): void {
  const read = stdin.read.bind(stdin);
  let rest = "";
  let timer: ReturnType<typeof setTimeout> | undefined;
  const take = (text: string) => {
    if (bus.emitRaw(rest + text)) {
      rest = "";
      if (timer) clearTimeout(timer);
      timer = undefined;
      return;
    }
    const t = tokenize(rest + text);
    rest = t.rest;
    if (t.events.length > 0) bus.emitInput(t.events);
    if (timer) clearTimeout(timer);
    timer = undefined;
    if (rest) {
      // A sequence cut off by the end of the read that no more bytes complete is Esc and its characters.
      timer = setTimeout(() => {
        const r = rest;
        rest = "";
        timer = undefined;
        bus.emitInput([{ kind: "key", chord: { key: "esc" } }, ...tokenize(r.slice(1)).events]);
      }, PARTIAL_FLUSH_MS);
    }
  };
  stdin.read = (...args: any[]) => {
    if (!bus.emitInputReady()) return read(...args);
    // Everything the stream holds is taken in this call and the stream is read until it says it has no more:
    // a `readable` listener that stops short of a null leaves the stream waiting to be read again and no
    // further `readable` comes (a real terminal; a PassThrough does not show it).
    let text = "";
    let got = false;
    for (let c = read(...args); c !== null && c !== undefined; c = read()) {
      got = true;
      text += typeof c === "string" ? c : Buffer.from(c).toString("utf8");
    }
    if (got) take(text);
    return null;
  };
}
