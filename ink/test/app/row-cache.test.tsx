// A right-click is a new snapshot whose body is the one on screen with a menu over it. Rows are kept between snapshots
// by their content (text/cells.tsx useRowCache), so drawing the body again costs nothing for a row that did not change;
// what is drawn must still be what a renderer that kept nothing draws.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { freezeClock, mount, tick } from "./harness.js";
import { rowCacheStats } from "../../src/text/cells.js";

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
const sizeOf = (f: string) => /_(\d+)x(\d+)\.json$/.exec(f);
// Screens under their size's recorded host chrome (sections draw under a settings chrome, tested in fixtures.test).
const screens = fs
  .readdirSync(dir)
  .filter((f) => f.endsWith(".json") && sizeOf(f) && !f.startsWith("host_chrome") && !f.startsWith("_"))
  .filter((f) => load(f).detail_title === undefined)
  .sort();

const menu = { id: "keys", title: "Actions", question: true, selected: 0, items: Array.from({ length: 6 }, (_, i) => ({ value: `a.${i}`, label: `action ${i}`, hint: "", key: String.fromCharCode(97 + i), target: `chrome:menu:${i}` })) };

/** The rows of the screen's lists and trees with the cursor moved: a list's to its next row, a tree's to its first. */
function moved(screen: any) {
  const s = structuredClone(screen);
  let n = 0;
  const walk = (o: any) => {
    if (!o || typeof o !== "object") return;
    if (o.node === "list" && (o.rows?.length ?? 0) > 1) {
      o.selected = ((o.selected ?? 0) + 1) % o.rows.length;
      n++;
    } else if (o.node === "tree" && (o.rows?.length ?? 0) > 0 && JSON.stringify(o.selected) !== JSON.stringify([o.rows[0].id])) {
      o.selected = [o.rows[0].id];
      n++;
    }
    for (const v of Object.values(o)) walk(v);
  };
  walk(s);
  return { screen: s, n };
}

async function fresh(cols: number, rows: number, over: Record<string, unknown>) {
  const m = mount(cols, rows);
  live.push(m.stop);
  m.welcome();
  m.snapshot(over, 1);
  await tick(40);
  return m.frame();
}

describe("a row that did not change is not drawn again", () => {
  it("there are screens with rows", () => {
    expect(screens.length).toBeGreaterThan(20);
  });

  for (const f of screens) {
    const [, c, r] = sizeOf(f)!;
    const cols = Number(c);
    const rows = Number(r);
    it(`${f}: the same body under a menu makes no row, and a moved cursor draws as a fresh renderer does`, async () => {
      const chrome = load(`host_chrome_${cols}x${rows}.json`);
      const screen = load(f);
      const m = mount(cols, rows);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ chrome, screen }, 1);
      await tick(40);
      const made = rowCacheStats.made;
      // the host's next snapshot: the same view, parsed anew, with the menu up
      const withMenu = { chrome: { ...structuredClone(chrome), menu }, screen: structuredClone(screen) };
      m.snapshot(withMenu, 2);
      await tick(40);
      expect(rowCacheStats.made - made, "rows made again for an unchanged body").toBe(0);
      expect(m.frame()).toBe(await fresh(cols, rows, withMenu));

      const next = moved(screen);
      m.snapshot({ chrome, screen: next.screen }, 3);
      await tick(40);
      expect(m.frame()).toBe(await fresh(cols, rows, { chrome, screen: next.screen }));
    });
  }
});
