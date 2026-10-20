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
import { frameOf } from "../../src/chrome/Chrome.js";

// Live at 160x40 (an MR page): the reference's page head sits on the page's bg.base, so the cells after the line under the
// title and the tab row's last column are bg.base; Ink left them with no background. Every recorded page at both sizes:
// no cell of its head rows (crumbs, title, the line under it, the tabs, the notice) is without a background.
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
const pages = fs
  .readdirSync(dir)
  .filter((f) => /_(\d+)x(\d+)\.json$/.test(f) && !f.startsWith("host_chrome") && !f.startsWith("_"))
  .filter((f) => Array.isArray(load(f).page?.crumbs))
  .sort();

describe("an item page's head is on bg.base to the right edge", () => {
  it("there are pages at both sizes", () => {
    expect(pages.some((f) => f.endsWith("_80x24.json"))).toBe(true);
    expect(pages.some((f) => f.endsWith("_160x40.json"))).toBe(true);
  });

  for (const f of pages) {
    it(f, async () => {
      const [, c, r] = /_(\d+)x(\d+)\.json$/.exec(f)!;
      const cols = Number(c);
      const rows = Number(r);
      const chrome = load(`host_chrome_${cols}x${rows}.json`);
      const screen = load(f);
      const page = screen.page;
      const m = mount(cols, rows);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ chrome, screen }, 1);
      await tick(40);
      const cells = parseAnsi(m.frame());
      const top = frameOf(chrome, null, cols, rows).bodyTop;
      const head = 2 + (page.under ? 1 : 0) + ((page.tabs ?? []).length > 0 ? 1 : 0) + (page.notice ? 1 : 0);
      for (let y = top; y < top + head; y++) {
        const row = cells[y] ?? [];
        const bare = Array.from({ length: cols }, (_, x) => x).filter((x) => !row[x]?.bg);
        expect(bare, `row ${y}: columns with no background`).toEqual([]);
      }
    });
  }
});
