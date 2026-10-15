// Navigation latency of the Ink renderer (T22): what a key costs inside the Ink process.
// Run the timings: `RADAR_PERF=1 npx vitest run test/perf` (the table goes to stderr).
//   key -> intent    stdin bytes in -> the Intent message handed to the connection
//   snapshot -> frame  Snapshot message in -> first byte of the frame written to stdout (and the frame's size)
// for Down on a 200-row list, Enter into an item page, and opening the x menu, at 160x40.
// Two modes: `throttled` is how ink/src/app/run.tsx renders (Ink's default 30 fps render throttle); `unthrottled`
// is Ink's debug mode (a write on every commit), the floor of what the tree itself costs.
// Budgets are on CPU time, not wall time, so they hold on a busy machine (see the comment above the cases).
import { afterEach, describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fixture, max, median, mount, settle } from "./harness.js";

const W = 160;
const H = 40;
const DOWN = "\x1b[B";
const ENTER = "\r";
const KEY_BUDGET_MS = 5;
const FRAME_CPU_BUDGET_MS = 60;
const N = 7;

let stops: Array<() => void> = [];
afterEach(() => {
  stops.forEach((f) => f());
  stops = [];
});

const chrome = fixture("host_chrome_160x40.json");
const withMenu = (items: number) => ({
  ...chrome,
  menu: {
    id: "actions",
    title: "Actions",
    question: false,
    items: Array.from({ length: items }, (_, i) => ({ value: `gitlab.act.${i}`, label: `action number ${i}`, hint: "does a thing", key: String.fromCharCode(97 + (i % 26)), target: `chrome:menu:${i}` })),
    selected: 0,
  },
});

/** The Repos screen with its middle pane replaced by a list of 200 merge-request-like rows. */
function listScreen(selected: number, rows = 200) {
  const s = JSON.parse(JSON.stringify(fixture("gitlab_repos_160x40.json")));
  s.panes[1].body = {
    node: "list",
    selected,
    rows: Array.from({ length: rows }, (_, i) => ({
      target: `row:${i}`,
      text: [{ icon: "change" }, { ink: "accent_secondary", text: `!${1000 + i} ` }, { text: `Fix the flaky thing number ${i} in the pipeline runner` }],
      right: [{ ink: "muted", text: `${i % 59}m` }, { ink: "number", text: ` ${i % 7} threads` }],
    })),
  };
  return s;
}
const page = fixture("gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket_160x40.json");

type Row = { step: string; mode: string; ms: number; maxMs: number; bytes?: number; writes?: number; react?: number; cpu?: number };
const rows: Row[] = [];

async function runAll(throttled: boolean) {
  const mode = throttled ? "throttled" : "unthrottled";
  const results: Record<string, { key: number; frame: number; bytes: number; writes: number; cpu: number }> = {};
  const scenarios: Array<{ name: string; keys: string; base: () => any; after: (i: number) => any; reset?: boolean }> = [
    { name: "Down on a 20-row list", keys: DOWN, base: () => ({ chrome, screen: listScreen(0, 20) }), after: (i) => ({ chrome, screen: listScreen(i + 1, 20) }) },
    { name: "Down on a 200-row list", keys: DOWN, base: () => ({ chrome, screen: listScreen(0) }), after: (i) => ({ chrome, screen: listScreen(i + 1) }) },
    { name: "Enter into an item page", keys: ENTER, base: () => ({ chrome, screen: listScreen(0, 20) }), after: () => ({ chrome, screen: page }) },
    { name: "x opens the menu", keys: "x", base: () => ({ chrome, screen: listScreen(0, 20) }), after: () => ({ chrome: withMenu(12), screen: listScreen(0, 20) }) },
  ];
  for (const sc of scenarios) {
    const m = mount(W, H, throttled);
    stops.push(m.stop);
    await m.frameAfter(sc.base());
    await settle();
    const keyMs: number[] = [];
    const frameMs: number[] = [];
    const bytes: number[] = [];
    const writes: number[] = [];
    const react: number[] = [];
    const cpus: number[] = [];
    for (let i = 0; i < N; i++) {
      const k = await m.key(sc.keys);
      keyMs.push(k.ms);
      const f = await m.frameAfter(sc.after(i));
      frameMs.push(f.ms);
      bytes.push(f.bytes);
      writes.push(f.writes);
      react.push(f.react);
      cpus.push(f.cpu);
      if (sc.name.startsWith("Enter") || sc.name.startsWith("x")) await m.frameAfter(sc.base()); // back to the list
      await settle(40);
    }
    m.stop();
    stops = stops.filter((f) => f !== m.stop);
    rows.push({ step: `${sc.name}: key -> intent`, mode, ms: median(keyMs), maxMs: max(keyMs) });
    rows.push({ step: `${sc.name}: snapshot -> frame`, mode, ms: median(frameMs), maxMs: max(frameMs), bytes: median(bytes), writes: median(writes), react: median(react), cpu: median(cpus) });
    results[sc.name] = { key: median(keyMs), frame: median(frameMs), bytes: median(bytes), writes: median(writes), cpu: median(cpus) };
  }
  return results;
}

let throttled: Awaited<ReturnType<typeof runAll>>;
let unthrottled: Awaited<ReturnType<typeof runAll>>;

// Wall-clock budgets flake on a busy machine, so the timing cases run only when asked: `RADAR_PERF=1 npx vitest run test/perf`
// (add NODE_ENV=production to see React's production build). `npm test` runs the deterministic case only.
const timed = describe.skipIf(!process.env.RADAR_PERF);

timed("Ink navigation latency at 160x40 (median of 7)", () => {
  it("measures", async () => {
    unthrottled = await runAll(false);
    throttled = await runAll(true);
    // eslint-disable-next-line no-console
    process.stderr.write(
      "\nstep | mode | median ms | worst ms | frame bytes | writes | React ms | CPU ms\n" +
        rows.map((r) => `${r.step} | ${r.mode} | ${r.ms.toFixed(1)} | ${r.maxMs.toFixed(1)} | ${r.bytes ?? ""} | ${r.writes ?? ""} | ${r.react?.toFixed(1) ?? ""} | ${r.cpu?.toFixed(1) ?? ""}`).join("\n") + "\n",
    );
    expect(Object.keys(throttled).length).toBe(4);
    process.stderr.write(`React build: NODE_ENV=${process.env.NODE_ENV}\n`);
  }, 120_000);

  for (const name of ["Down on a 20-row list", "Down on a 200-row list", "Enter into an item page", "x opens the menu"]) {
    it(`${name}: key -> intent under ${KEY_BUDGET_MS} ms`, () => {
      expect(throttled[name]!.key).toBeLessThan(KEY_BUDGET_MS);
    });
  }
  // Snapshot -> frame the way run.tsx renders, as CPU time of the process (a wall-clock budget flakes with the load
  // of the machine: the same frame took 11-40 ms wall in the same run). Vitest runs React's development build unless
  // NODE_ENV=production is set; the shipped bundle is built with it, so measure with it. Measured 2026-10-12, machine at
  // load ~30 on 16 cores, NODE_ENV=production, throttled: 17-31 ms CPU per frame (unthrottled 34-66 ms: two writes).
  // The budget is what leaves room in the 100 ms the operator asked for (a key or a screen switch drawn within 100 ms
  // of the key, host share included); it stays far above today's number so only a regression of the frame's cost
  // (re-rendering whole trees, writing whole frames) trips it.
  for (const name of ["Down on a 20-row list", "Down on a 200-row list", "Enter into an item page", "x opens the menu"]) {
    it(`${name}: snapshot -> frame under ${FRAME_CPU_BUDGET_MS} ms of CPU`, () => {
      expect(throttled[name]!.cpu).toBeLessThan(FRAME_CPU_BUDGET_MS);
    });
  }
});

describe("the shipped bundle", () => {
  it("is built with NODE_ENV=production, so React's production build runs (T2: ink/package.json `bundle`)", () => {
    const pkg = JSON.parse(readFileSync(new URL("../../package.json", import.meta.url), "utf8"));
    expect(pkg.scripts.bundle).toMatch(/NODE_ENV/);
  });
});
