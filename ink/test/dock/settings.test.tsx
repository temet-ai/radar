import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { freezeClock, mount, tick, until } from "../app/harness.js";
import { parseAnsi } from "../nodes/ansi.js";
import { STATES } from "./states.js";

// While settings show the snapshot carries the dock beside the section (Snapshot.dock): a sign-in started in
// Settings → Agents runs in a dock tab drawn under the section, the detail keeping its height.
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

const fixtures = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures");
const read = (f: string) => JSON.parse(fs.readFileSync(path.join(fixtures, f), "utf8"));
const plain = (frame: string) => parseAnsi(frame).map((r) => r.map((c) => c.ch).join("").trimEnd());

describe("the dock's keys on settings", () => {
  // Live on Settings the host blurred the dock but Ink kept typing into it: the keys follow the dock the snapshot
  // carries beside the section, and change when the host's next snapshot says so.
  it("types into a focused dock, esc gives the keys back when idle, and after the blur keys reach the host", async () => {
    const section = read("git_settings-branches_160x40.json");
    const chrome = read("host_chrome_160x40.json");
    const m = mount(160, 40);
    live.push(m.stop);
    m.welcome();
    const idle = { ...STATES.empty!, composer: { ...STATES.empty!.composer, can_stop: false } };
    m.snapshot({ chrome, section, dock: { ...idle, focused: true } }, 1);
    await tick(40);
    const docks = () => m.intents().map((i) => i.intent).filter((i) => i.intent === "dock");
    m.type("hi");
    await until(() => plain(m.frame()).join("\n").includes("> hi"));
    m.type("\x1b");
    await until(() => docks().length > 0, 1000);
    expect(docks()).toEqual([{ intent: "dock", dock: "blur" }]);
    m.snapshot({ chrome, section, dock: { ...idle, focused: false, composer: { ...idle.composer, draft: "hi" } } }, 2);
    await tick(40);
    const before = m.intents().length;
    m.type("z");
    await until(() => m.intents().length > before, 1000);
    expect(m.intents().at(-1)!.intent).toEqual({ intent: "key", chord: { key: { char: "z" } } });
    expect(docks()).toHaveLength(1);
  });

  it("esc stops a running turn, and blurs once the host says it ended", async () => {
    const section = read("git_settings-branches_160x40.json");
    const chrome = read("host_chrome_160x40.json");
    const m = mount(160, 40);
    live.push(m.stop);
    m.welcome();
    const running = { ...STATES.empty!, composer: { ...STATES.empty!.composer, can_stop: true } };
    m.snapshot({ chrome, section, dock: running }, 1);
    await tick(40);
    const docks = () => m.intents().map((i) => i.intent).filter((i) => i.intent === "dock");
    m.type("\x1b");
    await until(() => docks().length > 0, 1000);
    m.snapshot({ chrome, section, dock: { ...running, composer: { ...running.composer, can_stop: false } } }, 2);
    await tick(40);
    m.type("\x1b");
    await until(() => docks().length > 1, 1000);
    expect(docks()).toEqual([
      { intent: "dock", dock: "stop" },
      { intent: "dock", dock: "blur" },
    ]);
  });
});

describe("the dock on settings", () => {
  for (const [cols, rows] of [
    [160, 40],
    [80, 24],
  ] as const) {
    it(`draws under the section at ${cols}x${rows}`, async () => {
      const section = read(`git_settings-branches_${cols}x${rows}.json`);
      const dock = read(`dock/approval_${cols}x${rows}.json`).dock;
      const chrome = read(`host_chrome_${cols}x${rows}.json`);
      const m = mount(cols, rows);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ chrome, section, dock: { ...dock, visible: true } }, 1);
      await tick(40);
      const text = plain(m.frame()).join("\n");
      expect(text).toContain("approve once");
      if (cols >= 120) {
        // Wide: the section is cut short above the dock and the detail keeps the column's full height.
        expect(text).toContain(section.detail_title);
        const lines = plain(m.frame());
        const approve = lines.findIndex((l) => l.includes("approve once"));
        expect(approve).toBeGreaterThan(rows / 2);
      }
      // Hidden, the section has the room again.
      m.snapshot({ chrome, section, dock: { ...dock, visible: false } }, 2);
      await tick(40);
      expect(plain(m.frame()).join("\n")).not.toContain("approve once");
    });
  }
});
