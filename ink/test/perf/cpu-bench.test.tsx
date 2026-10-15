// CPU cost of a frame, many times over, for comparing a change with the code before it (T7).
//   RADAR_PERF=1 NODE_ENV=production npx vitest run test/perf/cpu-bench
// Each case alternates two snapshots 40 times (selection moves, as on Down) on a recorded screen at 160x40 and prints
// the process CPU ms per frame (user+system, so it holds on a busy machine better than wall time). Run it twice and
// take the lower number; the first run of a process also pays for JIT warm-up (the first 5 frames are not counted).
import { afterEach, describe, it } from "vitest";
import { fixture, mount, settle } from "./harness.js";

const stops: Array<() => void> = [];
afterEach(() => {
  stops.forEach((f) => f());
  stops.length = 0;
});

const chrome = fixture("host_chrome_160x40.json");
const SCREENS = ["gitlab_repos_160x40.json", "gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket_160x40.json", "git_branch-screen_160x40.json"];

function moved(screen: any, n: number) {
  const s = JSON.parse(JSON.stringify(screen));
  const walk = (o: any) => {
    if (o && typeof o === "object") {
      if (o.node === "list" || o.node === "tree") o.selected = Math.min((o.rows?.length ?? 1) - 1, n % 2);
      for (const v of Object.values(o)) walk(v);
    }
  };
  walk(s);
  return s;
}

describe.skipIf(!process.env.RADAR_PERF)("CPU per frame at 160x40", () => {
  // A right-click: the host's snapshot with the menu up, then (Esc) the same screen without it. Nothing under the
  // menu changed, so what the rows cost here is what a frame pays for drawing again what it drew before.
  it("a menu opened and closed over the repositories", async () => {
    const screen = fixture("gitlab_repos_160x40.json");
    const menu = { id: "keys", title: "Actions", question: true, selected: 0, items: Array.from({ length: 12 }, (_, i) => ({ value: `a.${i}`, label: `action number ${i}`, hint: "does a thing", key: String.fromCharCode(97 + i), target: `chrome:menu:${i}` })) };
    // RADAR_PERF_SIZE=280x75: the same view on a bigger terminal (the chrome is laid out for the size it is drawn at)
    const [w, h] = (process.env.RADAR_PERF_SIZE ?? "160x40").split("x").map(Number) as [number, number];
    const sized = { ...chrome, width: w, height: h };
    const at = (i: number) => ({ chrome: i % 2 ? { ...sized, menu } : sized, screen });
    const m = mount(w, h, false);
    stops.push(m.stop);
    for (let i = 0; i < 6; i++) await m.frameAfter(at(i));
    // RADAR_PERF_N: more frames, so that a count over the whole run (perf stat -e instructions:u) is mostly frames
    const N = Number(process.env.RADAR_PERF_N ?? 40);
    const from = m.commits.length;
    const cpu0 = process.cpuUsage();
    for (let i = 0; i < N; i++) {
      m.snapshot(at(i + 6));
      await new Promise((r) => setTimeout(r, 0));
      await settle(1);
    }
    await settle(100);
    const c = process.cpuUsage(cpu0);
    const react = m.commits.slice(from).reduce((a, x) => a + x, 0) / N;
    process.stderr.write(`menu open/close: ${((c.user + c.system) / 1000 / N).toFixed(1)} ms CPU per frame, React ${react.toFixed(1)} ms\n`);
  }, 60_000);

  for (const name of SCREENS) {
    it(name, async () => {
      let screen: any;
      try {
        screen = fixture(name);
      } catch {
        return;
      }
      const m = mount(160, 40, false); // debug mode: one write per commit, no throttle in the way
      stops.push(m.stop);
      await m.frameAfter({ chrome, screen });
      for (let i = 0; i < 5; i++) await m.frameAfter({ chrome, screen: moved(screen, i) });
      const N = 40;
      const cpu0 = process.cpuUsage();
      for (let i = 0; i < N; i++) {
        m.snapshot({ chrome, screen: moved(screen, i + 5) });
        await new Promise((r) => setTimeout(r, 0));
        await settle(1);
      }
      await settle(100);
      const c = process.cpuUsage(cpu0);
      // The whole frame Ink composes (debug mode writes it whole): how many styled runs it holds.
      const frame = m.out.writes.at(-1)!.text;
      const sgr = (frame.match(/\x1b\[[0-9;]*m/g) ?? []).length;
      process.stderr.write(`${name}: ${((c.user + c.system) / 1000 / N).toFixed(1)} ms CPU per frame; frame ${frame.length} chars, ${sgr} SGR sequences\n`);
    }, 60_000);
  }
});
