import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { freezeClock, mount, tick } from "../app/harness.js";
import { parseAnsi } from "../nodes/ansi.js";
import { STATES } from "./states.js";

// Every dock state drawn by the whole App under a recorded screen at both sizes: the dock sits under the middle pane
// (above 100 columns) or covers it (below), nav and detail keep their height, and the frame stays inside the terminal.
let unfreeze = () => {};
beforeAll(() => {
  unfreeze = freezeClock();
});
afterAll(() => unfreeze());
let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures");
const load = (f: string) => JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
// These dock views borrow generic chrome from another host state, so its pane routing map is not evidence.
// Actual paired native captures retain and verify their host-supplied pane_jumps separately.
const viewOnlyChrome = (cols: number, rows: number) => {
  const { pane_jumps: _unrelatedPaneJumps, ...chrome } = load(`host_chrome_${cols}x${rows}.json`);
  return chrome;
};
const SIZES: Array<[number, number]> = [
  [80, 24],
  [160, 40],
];

const plain = (frame: string) => parseAnsi(frame).map((row) => row.map((c) => c.ch).join("").trimEnd());

async function draw(name: string, cols: number, rows: number, dock = STATES[name]) {
  const m = mount(cols, rows);
  live.push(m.stop);
  m.welcome();
  const chrome = viewOnlyChrome(cols, rows);
  const screen = { ...load(`gitlab_repos_${cols}x${rows}.json`), dock };
  m.snapshot({ chrome, screen }, 1);
  await tick(40);
  return { m, frame: m.frame(), rows: plain(m.frame()) };
}

describe("the dock, every state, both sizes", () => {
  for (const name of Object.keys(STATES)) {
    for (const [cols, rows] of SIZES) {
      it(`${name} at ${cols}x${rows}`, async () => {
        const d = await draw(name, cols, rows);
        expect(d.frame.split("\n").length).toBeLessThanOrEqual(rows);
        expect(d.rows.join("\n")).toContain("agents");
        // no chevrons or [+]/[-] for folding
        expect(d.rows.join("\n")).not.toMatch(/[▶▸▾⌄]|\[\+\]|\[-\]/);
        expect(d.frame).toMatchSnapshot();
      });
    }
  }

  it("sits under the middle pane at 160 columns and covers it at 80", async () => {
    const wide = await draw("empty", 160, 40);
    const top = wide.rows.findIndex((r) => r.includes("agents"));
    expect(top).toBeGreaterThan(8);
    // nav and detail keep their frames to the foot of the body
    expect(wide.rows.slice(top, top + 3).join("\n")).toMatch(/╭/);
    const narrow = await draw("empty", 80, 24);
    const t2 = narrow.rows.findIndex((r) => r.includes("agents"));
    expect(t2).toBeLessThanOrEqual(3);
  });

  it("shows the exact command and keys inside their words in an approval", async () => {
    const d = await draw("approval", 160, 40);
    const text = d.rows.join("\n");
    expect(text).toContain("cargo test -p radar-core --no-fail-fast");
    expect(text).toContain("approve once  approve for session  deny  view diff");
  });

  // A terminal tab the host has given no socket yet draws what runs: the command, the directory (home as `~`), the state.
  for (const [cols, rows] of SIZES) {
    it(`a terminal tab without a socket names its command, directory and state at ${cols}x${rows}`, async () => {
      const home = process.env.HOME;
      process.env.HOME = "/home/vimes";
      try {
        const pty = STATES.pty!;
        const tab = pty.tabs[0]!;
        const bare = { ...pty, tabs: [{ ...tab, terminal: { ...tab.terminal!, socket: "", cwd: "/home/vimes/projects/radar" } }] };
        const d = await draw("pty", cols, rows, bare);
        expect(d.rows.join("\n")).toContain("codex login · ~/projects/radar · running");
      } finally {
        if (home === undefined) delete process.env.HOME;
        else process.env.HOME = home;
      }
    });
  }

  it("hidden dock draws nothing", async () => {
    const m = mount(160, 40);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: viewOnlyChrome(160, 40), screen: { ...load("gitlab_repos_160x40.json"), dock: { ...STATES.empty, visible: false } } }, 1);
    await tick(40);
    expect(plain(m.frame()).join("\n")).not.toContain("agents");
  });
});
