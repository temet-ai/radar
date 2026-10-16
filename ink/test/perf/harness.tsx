// Timing harness for the Ink app: the App over a fake connection and a stdout stub that stamps every write.
// Self-contained (imports only src/ and the recorded fixtures) so it keeps running whatever the other tests do.
// Used by latency.test.tsx; the timing cases there run with RADAR_PERF=1.
import React, { Profiler } from "react";
import { EventEmitter } from "node:events";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { render } from "ink";
import { App } from "../../src/app/App.js";
import { Session, capabilitiesFrom } from "../../src/app/session.js";
import { FakeConnection } from "../../src/transport/fake.js";
import { MouseBus, tapMouse } from "../../src/input/stdin-tap.js";

export type Write = { at: number; bytes: number; text: string };

class Out extends EventEmitter {
  writes: Write[] = [];
  isTTY = true;
  constructor(
    public columns: number,
    public rows: number,
  ) {
    super();
  }
  write = (s: string) => {
    this.writes.push({ at: performance.now(), bytes: Buffer.byteLength(s), text: s });
    return true;
  };
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

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
export const settle = (ms = 80) => sleep(ms);

/** Wait on state, not on the clock. */
export async function until(ok: () => boolean, ms = 2000) {
  const end = performance.now() + ms;
  while (!ok() && performance.now() < end) await sleep(1);
}

const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures");
export const fixture = (name: string): any => JSON.parse(fs.readFileSync(path.join(dir, name), "utf8"));

/**
 * `throttled: true` renders the way ink/src/app/run.tsx does (Ink's default 30 fps render throttle);
 * false is Ink's `debug` mode, which writes on every commit - the floor of what the tree costs.
 */
export function mount(columns: number, rows: number, throttled: boolean) {
  const out = new Out(columns, rows);
  const stdin = new In();
  const conn = new FakeConnection();
  const mouse = new MouseBus();
  tapMouse(stdin, mouse);
  const session = new Session(conn, capabilitiesFrom({}, out));
  session.start();
  const commits: number[] = [];
  const instance = render(
    <Profiler id="app" onRender={(_id, _phase, actual) => commits.push(actual)}>
      <App session={session} mouse={mouse} />
    </Profiler>,
    {
    stdout: out as any,
    stdin: stdin as any,
    stderr: new Out(columns, rows) as any,
    debug: !throttled,
    exitOnCtrlC: false,
    patchConsole: false,
    },
  );
  conn.receive({ type: "welcome", protocol: 1, view: 1, cursor: 0 });
  let cursor = 0;
  return {
    out,
    conn,
    /** React render-phase time (ms) of each commit: the component tree alone, before Ink lays out and writes. */
    commits,
    stop: () => instance.unmount(),
    /** Deliver a snapshot; returns when it was handed over. */
    snapshot(over: { chrome: any; screen?: any; section?: any }): number {
      cursor += 1;
      const at = performance.now();
      conn.receive({ type: "snapshot", cursor, chrome: over.chrome, screen: over.screen ?? null, section: over.section ?? null, focus: "middle" });
      return at;
    },
    intents: () => (conn.sent as any[]).filter((x) => x.type === "intent"),
    /** Write `bytes` to stdin and return at once. */
    type: (bytes: string) => stdin.type(bytes),
    /** Write `bytes` to stdin; the time until an intent leaves the connection (ms), and the intent. */
    async key(bytes: string) {
      const before = conn.sent.length;
      const t0 = performance.now();
      stdin.type(bytes);
      await until(() => conn.sent.length > before, 500);
      const t1 = performance.now();
      return { ms: conn.sent.length > before ? t1 - t0 : NaN, sent: conn.sent.slice(before) };
    },
    /**
     * Deliver a snapshot and wait for the frame it causes: the time to the first write after it, and the
     * bytes written until the output has been quiet for 60 ms.
     */
    async frameAfter(over: { chrome: any; screen?: any; section?: any }) {
      const from = out.writes.length;
      const fromC = commits.length;
      const cpu0 = threadCpu();
      const t0 = this.snapshot(over);
      await until(() => out.writes.length > from, 1000);
      const first = out.writes[from];
      let n = out.writes.length;
      let cpu1 = threadCpu(cpu0);
      for (;;) {
        cpu1 = threadCpu(cpu0);
        await sleep(60);
        if (out.writes.length === n) break;
        n = out.writes.length;
      }
      const mine = out.writes.slice(from);
      // CPU time (user+system, ms) of this process from the snapshot to the last write: unlike wall time it does not
      // grow with the load of the machine, so a budget on it holds on a busy one.
      const cpu = (cpu1.user + cpu1.system) / 1000;
      return { cpu, ms: first ? first.at - t0 : NaN, react: commits.slice(fromC).reduce((a, c) => a + c, 0), writes: mine.length, bytes: mine.reduce((a, w) => a + w.bytes, 0) };
    },
  };
}

/** CPU time (user, system) of this thread: the test's own worker, where the App renders. `process.cpuUsage` counts the
 * whole process, vitest's other threads too, and doubled what a frame cost under the default thread pool. */
export const threadCpu = (prev?: NodeJS.CpuUsage): NodeJS.CpuUsage =>
  ((process as any).threadCpuUsage ?? process.cpuUsage).call(process, prev);

export const median = (xs: number[]) => {
  const s = xs.filter((x) => !Number.isNaN(x)).sort((a, b) => a - b);
  return s.length ? s[Math.floor(s.length / 2)]! : NaN;
};
export const max = (xs: number[]) => Math.max(...xs.filter((x) => !Number.isNaN(x)));
