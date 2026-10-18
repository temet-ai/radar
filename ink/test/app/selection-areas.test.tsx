import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { freezeClock, mount, tick } from "./harness.js";
import { HitRegistry } from "../../src/state/ui-state.js";
import { booksChrome, settingsNav } from "../chrome/fixture.js";
import { STATES } from "../dock/states.js";
import { parseAnsi } from "../nodes/ansi.js";

// Where a mouse selection may start and run (issue #51), for every recorded fixture at both sizes: each area is the
// inside of a frame Ink drew. The column either side of it is that frame's border (or the scrollbar on it), the row
// above a pane's is its title row and the row below its bottom border, so borders, scrollbars, titles, the top bar
// and the status line are never selected. Every fixture's panes register an area, so none is left unselectable.
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
const files = fs.readdirSync(dir).filter((f) => /_\d+x\d+\.json$/.test(f) && !f.startsWith("host_chrome")).sort();
const load = (f: string) => JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
const sizeOf = (f: string) => {
  const m = /_(\d+)x(\d+)\.json$/.exec(f)!;
  return { cols: Number(m[1]), rows: Number(m[2]) };
};
const EDGE = new Set(["│", "█", "║"]);

async function areasOf(view: any, size: { cols: number; rows: number }, dock?: any) {
  const registry = new HitRegistry();
  const m = mount(size.cols, size.rows, { registry });
  live.push(m.stop);
  m.welcome();
  const section = view.detail_title !== undefined;
  const chromeFile = `host_chrome_${size.cols}x${size.rows}.json`;
  const chrome = fs.existsSync(path.join(dir, chromeFile)) ? load(chromeFile) : booksChrome({ width: size.cols, height: size.rows });
  const shown = section ? { ...chrome, settings: { ...settingsNav, section_title: view.scope, detail_title: view.detail_title } } : chrome;
  m.snapshot({ chrome: shown, screen: section ? null : dock ? { ...view, dock } : view, section: section ? view : null }, 1);
  await tick(40);
  const grid = parseAnsi(m.frame()).map((r) => r.map((c) => c.ch));
  return { areas: registry.selectAreas(), grid, size };
}

function checkInside(r: Awaited<ReturnType<typeof areasOf>>, label: string) {
  const at = (x: number, y: number) => r.grid[y]?.[x] ?? "";
  for (const a of r.areas) {
    const name = `${label} ${a.id} ${JSON.stringify({ x: a.x, y: a.y, w: a.w, h: a.h })}`;
    expect(a.y, `${name}: under the top bar`).toBeGreaterThan(0);
    expect(a.y + a.h, `${name}: above the status line`).toBeLessThan(r.size.rows);
    for (let y = a.y; y < a.y + a.h; y++) {
      expect(EDGE.has(at(a.x - 1, y)), `${name}: the cell left of row ${y} is its frame, not ${JSON.stringify(at(a.x - 1, y))}`).toBe(true);
      expect(EDGE.has(at(a.x + a.w, y)), `${name}: the cell right of row ${y} is its frame or scrollbar, not ${JSON.stringify(at(a.x + a.w, y))}`).toBe(true);
    }
    if (a.id.endsWith(":body")) {
      // A structured detail excludes its pinned rule and controls from selectable prose.
      expect(at(a.x, a.y + a.h), `${name}: the next row is the footer rule`).toBe("─");
    } else if (a.id.startsWith("pane:")) {
      expect(at(a.x - 1, a.y - 1), `${name}: the row above is the title row`).toBe("╭");
      expect(at(a.x - 1, a.y + a.h), `${name}: the row below is the bottom border`).toBe("╰");
    }
  }
}

describe("selection areas are the insides of what Ink framed, in every fixture at both sizes", () => {
  for (const f of files) {
    it(f, async () => {
      const view = load(f);
      const r = await areasOf(view, sizeOf(f));
      // every framed pane on screen is selectable
      const frames = r.grid.flatMap((row, y) => row.map((c, x) => (c === "╭" && y > 0 ? 1 : 0))).reduce((n: number, v) => n + v, 0);
      expect(r.areas.length, `${f}: one area per framed pane`).toBe(frames);
      checkInside(r, f);
    });
  }

  for (const size of [{ cols: 80, rows: 24 }, { cols: 160, rows: 40 }]) {
    it(`the dock's transcript at ${size.cols}x${size.rows}: inside the dock's frame, under its tab row`, async () => {
      const r = await areasOf(load(`gitlab_repos_${size.cols}x${size.rows}.json`), size, STATES.unfocused);
      const dock = r.areas.find((a) => a.id === "dock");
      expect(dock).toBeDefined();
      checkInside(r, "dock");
    });
  }
});
