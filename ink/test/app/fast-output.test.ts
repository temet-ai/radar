// src/app/fast-output.ts replaces Ink's Output.get(). It must draw exactly what Ink's own does: the same character
// and the same colours and attributes in every cell. Both are run on recorded screens with colour on (FORCE_COLOR=3,
// set before chalk loads) and the frames compared cell by cell.
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});

import { freezeClock, mount, tick } from "./harness.js";
import Output from "ink-output";
import chalk from "chalk";
import { fallbackFrames, inkOutputGet, useFastOutput } from "../../src/app/fast-output.js";
import { charWidth } from "../../src/chrome/width.js";

const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures");
const load = (f: string) => JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));

type Cell = string; // "<char>|<fg>|<bg>|<attrs>"
/** A frame as a grid of cells: SGR state tracked the way a terminal does, trailing default blanks dropped. */
function grid(frame: string): Cell[][] {
  return frame.split("\n").map((line) => {
    const out: Cell[] = [];
    let fg = "",
      bg = "";
    const on = new Set<number>();
    const re = /\x1b\[([0-9;]*)m|([\s\S])/gu;
    for (const m of line.matchAll(re)) {
      if (m[2] === undefined) {
        const p = m[1] === "" ? ["0"] : m[1]!.split(";");
        for (let i = 0; i < p.length; i++) {
          const n = Number(p[i]);
          if (n === 0) {
            fg = bg = "";
            on.clear();
          } else if (n === 22) {
            on.delete(1);
            on.delete(2);
          } else if (n === 23) on.delete(3);
          else if (n === 24) on.delete(4);
          else if (n === 27) on.delete(7);
          else if (n === 29) on.delete(9);
          else if ([1, 2, 3, 4, 7, 9].includes(n)) on.add(n);
          else if (n === 39) fg = "";
          else if (n === 49) bg = "";
          else if (n === 38 || n === 48) {
            const k = p[i + 1] === "5" ? 3 : 5;
            const v = p.slice(i, i + k).join(";");
            i += k - 1;
            if (n === 38) fg = v;
            else bg = v;
          } else if ((n >= 30 && n <= 37) || (n >= 90 && n <= 97)) fg = String(n);
          else if ((n >= 40 && n <= 47) || (n >= 100 && n <= 107)) bg = String(n);
        }
      } else {
        const ch = m[2]!;
        const w = Math.max(1, charWidth(ch.codePointAt(0)!));
        const attrs = [...on].sort().join(",");
        out.push(`${ch}|${fg}|${bg}|${attrs}`);
        for (let k = 1; k < w; k++) out.push(`|${fg}|${bg}|${attrs}`);
      }
    }
    while (out.length && out[out.length - 1] === " |||") out.pop();
    return out;
  });
}

async function frameOf(fast: boolean, snap: Record<string, unknown>, cols: number, rows: number) {
  useFastOutput(fast);
  const m = mount(cols, rows);
  try {
    m.welcome();
    m.snapshot(snap, 1);
    await tick(80);
    return m.frame();
  } finally {
    m.stop();
    useFastOutput(true);
  }
}

let unfreeze = () => {};
beforeAll(() => {
  unfreeze = freezeClock(); // the busy throbber picks its glyph from the clock
});
afterAll(() => unfreeze());
afterEach(() => useFastOutput(true));

const SCREENS = ["gitlab_repos_160x40.json", "git_branch-screen_160x40.json", "gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket_160x40.json", "gitlab_repos_80x24.json"];

describe("fast Output draws what Ink's Output draws", () => {
  for (const name of SCREENS) {
    it(name, async () => {
      expect(fs.existsSync(path.join(dir, name)), `fixture ${name}`).toBe(true);
      const [cols, rows] = name.includes("80x24") ? [80, 24] : [160, 40];
      const chrome = load(`host_chrome_${cols}x${rows}.json`);
      const snap = { chrome, screen: load(name) };
      const slow = grid(await frameOf(false, snap, cols, rows));
      const fast = grid(await frameOf(true, snap, cols, rows));
      expect(slow.flat().some((c) => /\|\d/.test(c))).toBe(true); // colour really is on
      expect(fallbackFrames).toBe(0); // the fast path drew it
      expect(fast.length).toBe(slow.length);
      for (let y = 0; y < slow.length; y++) expect(fast[y]!.join("\n")).toBe(slow[y]!.join("\n"));
      // again, with every op's cells taken from what the first drawing decoded
      const warm = grid(await frameOf(true, snap, cols, rows));
      for (let y = 0; y < slow.length; y++) expect(warm[y]!.join("\n")).toBe(slow[y]!.join("\n"));
    });
  }

  it("clips that cut wide characters, combining marks and writes over wide characters, drawn cold and warm", () => {
    const draw = () => {
      const o = new Output({ width: 12, height: 3 }) as any;
      const red = (s: string) => chalk.red(s);
      o.write(0, 0, "a界b界c界d", { transformers: [red] });
      o.clip({ x1: 2, x2: 7, y1: 0, y2: 2 });
      o.write(1, 1, "界é界f界", { transformers: [] });
      o.write(0, 0, "xyz", { transformers: [red] });
      o.unclip();
      o.write(4, 2, "é\x1b[1m界\x1b[22m", { transformers: [] });
      o.write(5, 2, "q", { transformers: [] });
      return o.get();
    };
    useFastOutput(false);
    const slow = draw();
    useFastOutput(true);
    const before = fallbackFrames;
    expect(grid(draw().output)).toEqual(grid(slow.output));
    expect(grid(draw().output)).toEqual(grid(slow.output));
    expect(fallbackFrames).toBe(before);
  });

  it("a menu over a list", async () => {
    const chrome = load("host_chrome_160x40.json");
    const withMenu = {
      ...chrome,
      menu: { id: "actions", title: "Actions", question: false, selected: 1, items: Array.from({ length: 9 }, (_, i) => ({ value: `a.${i}`, label: `action number ${i}`, hint: "does a thing", key: String.fromCharCode(97 + i), target: `chrome:menu:${i}` })) },
    };
    const snap = { chrome: withMenu, screen: load("gitlab_repos_160x40.json") };
    const slow = grid(await frameOf(false, snap, 160, 40));
    const fast = grid(await frameOf(true, snap, 160, 40));
    for (let y = 0; y < slow.length; y++) expect(fast[y]!.join("\n")).toBe(slow[y]!.join("\n"));
  });
});

describe("the patch and its fallback", () => {
  it("replaces the Output.get that Ink has: an Ink upgrade that changes it breaks here, not on screen", () => {
    const ink = inkOutputGet as () => unknown;
    expect((Output.prototype as any).get).not.toBe(ink);
    // Ink 8.0.0's get(): a grid of styled characters built from `operations`, applied with applyWriteOperation.
    expect(String(ink)).toContain("applyWriteOperation");
    const o = new Output({ width: 4, height: 2 }) as any;
    expect(Array.isArray(o.operations)).toBe(true);
    expect([o.width, o.height]).toEqual([4, 2]);
    expect(typeof o.write).toBe("function");
    expect(typeof o.clip).toBe("function");
  });

  it("a style run shared by two Texts takes each Text's outer style, whichever is drawn first", () => {
    // text/cells.tsx interns one Info per style, so two Texts in the same style carry the same object; one under no outer
    // style, one under a background. Either order, each draws as the same style spelt out with chalk transformers.
    const red = "\x1b[38;2;255;0;0m";
    const onBlue = (s: string) => chalk.bgBlue(s);
    for (const order of [
      [0, 1],
      [1, 0],
    ]) {
      const info = { fg: "38;2;255;0;0", bg: "", attr: 0 };
      const runs = () => Object.assign((s: string) => s, { radarRows: [{ cols: [0], infos: [info], g: [] }] });
      const o = new Output({ width: 4, height: 2 });
      const spelt = new Output({ width: 4, height: 2 });
      for (const y of order) {
        o.write(0, y, "ab", { transformers: y === 0 ? [runs()] : [runs(), onBlue] } as never);
        spelt.write(0, y, "ab", { transformers: y === 0 ? [(s: string) => `${red}${s}\x1b[39m`] : [(s: string) => `${red}${s}\x1b[39m`, onBlue] } as never);
      }
      const got = grid(o.get().output);
      expect(got[1]![0]).toBe("a|38;2;255;0;0|44|");
      expect(got).toEqual(grid(spelt.get().output));
    }
  });

  it("a transformer that is not plain SGR around its text draws through Ink's own get() and gives the same frame", () => {
    const draw = () => {
      const o = new Output({ width: 12, height: 2 });
      o.write(1, 0, "hello", { transformers: [(s: string) => s.toUpperCase()] } as never);
      o.write(0, 1, "plain", { transformers: [] } as never);
      return o.get();
    };
    useFastOutput(false);
    const slow = draw();
    useFastOutput(true);
    const before = fallbackFrames;
    const fast = draw();
    expect(fallbackFrames).toBe(before + 1);
    expect(fast).toEqual(slow);
    expect(fast.output).toBe(" HELLO\nplain");
  });
});
