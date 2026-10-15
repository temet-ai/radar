import React from "react";
import { vi } from "vitest";
import { EventEmitter } from "node:events";
import { render } from "ink";
import { App, type AppProps } from "../../src/app/App.js";
import { Session, capabilitiesFrom } from "../../src/app/session.js";
import { FakeConnection } from "../../src/transport/fake.js";
import { MouseBus, tapMouse } from "../../src/input/stdin-tap.js";
import { booksChrome } from "../chrome/fixture.js";

/** A stdout stub with a size (ink-testing-library fixes columns at 100). */
export class Out extends EventEmitter {
  frames: string[] = [];
  isTTY = true;
  constructor(public columns: number, public rows: number) {
    super();
  }
  write = (s: string) => {
    this.frames.push(s);
    return true;
  };
  last() {
    return this.frames.at(-1) ?? "";
  }
  resize(columns: number, rows: number) {
    this.columns = columns;
    this.rows = rows;
    this.emit("resize");
  }
}

class In extends EventEmitter {
  isTTY = true;
  data: string | null = null;
  setEncoding() {}
  setRawMode() {}
  resume() {}
  pause() {}
  ref() {}
  unref() {}
  read = () => {
    const d = this.data;
    this.data = null;
    return d;
  };
  type(s: string) {
    this.data = s;
    this.emit("readable");
  }
}

/** Wait on state, not on the clock: poll until `ok()` holds (the machine may be busy). */
export async function until(ok: () => boolean, ms = 3000) {
  const end = performance.now() + ms;
  while (!ok() && performance.now() < end) await new Promise((r) => setTimeout(r, 10));
}

/** Draw with the clock stopped: the busy throbber in the top bar picks its frame from Date.now(), and a snapshot of
 * it would change every 100 ms. Call it in beforeAll; the returned function puts the clock back (afterAll). */
export function freezeClock(): () => void {
  const spy = vi.spyOn(Date, "now").mockReturnValue(0);
  return () => spy.mockRestore();
}

export const tick = (ms = 20) => new Promise((r) => setTimeout(r, ms));

export function mount(columns: number, rows: number, props: Partial<AppProps> = {}, renderOptions: Record<string, unknown> = {}) {
  const out = new Out(columns, rows);
  const stdin = new In();
  const conn = new FakeConnection();
  const mouse = new MouseBus();
  tapMouse(stdin, mouse);
  const session = new Session(conn, capabilitiesFrom({}, out));
  session.start();
  const exits: Array<{ code: number; message?: string }> = [];
  const instance = render(<App session={session} mouse={mouse} onExit={(code, message) => exits.push({ code, message })} {...props} />, {
    stdout: out as any,
    stdin: stdin as any,
    stderr: new Out(columns, rows) as any,
    debug: true,
    exitOnCtrlC: false,
    patchConsole: false,
    ...renderOptions,
  });
  const snapshot = (over: Record<string, unknown> = {}, cursor = 1) =>
    conn.receive({ type: "snapshot", cursor, chrome: booksChrome({ width: columns, height: rows }), screen: null, section: null, focus: "middle", ...over });
  return {
    out, stdin, conn, session, exits, snapshot,
    welcome: () => conn.receive({ type: "welcome", protocol: 1, view: 1, cursor: 0 }),
    sent: (type: string) => (conn.sent as any[]).filter((m) => m.type === type),
    intents: () => (conn.sent as any[]).filter((m) => m.type === "intent"),
    type: (s: string) => stdin.type(s),
    frame: () => out.last(),
    stop: () => instance.unmount(),
  };
}
