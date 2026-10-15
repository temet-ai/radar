import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { freezeClock, mount, tick } from "./harness.js";
import { parseAnsi } from "../nodes/ansi.js";

// Live at 80x24 (an MR page): a pane's scrollbar on its right border is on the border's bg.surface in Ratatui; Ink drew
// it with no background. Every recorded screen at both sizes: no scrollbar cell (thumb █, track ║) is without one.
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
const screens = fs
  .readdirSync(dir)
  .filter((f) => /_(\d+)x(\d+)\.json$/.test(f) && !f.startsWith("host_chrome") && !f.startsWith("_"))
  .filter((f) => load(f).detail_title === undefined)
  .sort();

describe("a scrollbar is on a background", () => {
  let bars = 0;
  afterAll(() => expect(bars, "screens that draw a scrollbar").toBeGreaterThan(0));
  for (const f of screens) {
    it(f, async () => {
      const [, c, r] = /_(\d+)x(\d+)\.json$/.exec(f)!;
      const cols = Number(c);
      const rows = Number(r);
      const m = mount(cols, rows);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ chrome: load(`host_chrome_${cols}x${rows}.json`), screen: load(f) }, 1);
      await tick(40);
      const bare: string[] = [];
      parseAnsi(m.frame()).forEach((row, y) =>
        row.forEach((cell, x) => {
          if (cell.ch !== "█" && cell.ch !== "║") return;
          bars++;
          if (!cell.bg) bare.push(`${x},${y}`);
        }),
      );
      expect(bare, "scrollbar cells with no background").toEqual([]);
    });
  }
});
