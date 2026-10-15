import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { freezeClock, mount, tick } from "../app/harness.js";
import { parseAnsi } from "../nodes/ansi.js";

// Every dock state the host recorded (ink/test/fixtures/dock/<state>_<w>x<h>.json: a whole ScreenView with its dock,
// written by radar-core) drawn through the whole App at its own size under that size's recorded host chrome.
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

const dock = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures/dock");
const fixtures = path.resolve(dock, "..");
const files = fs.existsSync(dock) ? fs.readdirSync(dock).filter((f) => /_\d+x\d+\.json$/.test(f)).sort() : [];
const plain = (frame: string) => parseAnsi(frame).map((r) => r.map((c) => c.ch).join("").trimEnd());

describe("host-recorded dock states", () => {
  it("there are recorded states, at both sizes", () => {
    expect(files.length).toBeGreaterThan(0);
    const states = new Set(files.map((f) => f.replace(/_\d+x\d+\.json$/, "")));
    for (const s of states) for (const z of ["80x24", "160x40"]) expect(files, `${s} at ${z}`).toContain(`${s}_${z}.json`);
  });

  for (const f of files) {
    const [, c, r] = /_(\d+)x(\d+)\.json$/.exec(f)!;
    const cols = Number(c);
    const rows = Number(r);
    it(`${f}`, async () => {
      const screen = JSON.parse(fs.readFileSync(path.join(dock, f), "utf8"));
      const chrome = JSON.parse(fs.readFileSync(path.join(fixtures, `host_chrome_${cols}x${rows}.json`), "utf8"));
      const m = mount(cols, rows);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ chrome, screen }, 1);
      await tick(40);
      const frame = m.frame();
      const text = plain(frame).join("\n");
      expect(frame.split("\n").length).toBeLessThanOrEqual(rows);
      expect(text).toContain("agents");
      expect(text).not.toMatch(/[▶▸▾⌄]|\[\+\]|\[-\]/);
      expect(frame).toMatchSnapshot();
    });
  }
});
