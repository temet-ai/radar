import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick } from "../../app/harness.js";
import { parseAnsi, withoutEdgeDividers, type TCell } from "../../nodes/ansi.js";
import { applyLook, currentLook, defaultLook } from "../../../src/text/look.js";

// Ink's cells against the reference cells, for the same view data. `<screen>_<w>x<h>.json` is what the host sends for a
// demo-data screen ({width, height, chrome, screen}); `.cells.json` is the reference buffer drawn from the same host
// state (radar_core::shell::render into a TestBackend at truecolor, icons off): per cell `[symbol, fg, bg, modifiers]`
// with colours as `#rrggbb` (null = reset) and the modifier bits of the reference's Modifier. Both are written by a
// scratch test over the demo host (see README.md in this folder). The whole App draws the view data and every cell
// is compared: its character, colours and modifiers. A blank cell shows only its background, underline, reversal
// and strike, so only those are compared there.
const dir = path.dirname(fileURLToPath(import.meta.url));
const read = (f: string) => fs.readFileSync(path.join(dir, f), "utf8");

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

type RatCell = [string, string | null, string | null, number];
const BOLD = 1,
  DIM = 2,
  ITALIC = 4,
  UNDERLINE = 8,
  REVERSED = 64,
  STRIKE = 256;

/** The theme slots that have a colour, to name it in a failure. */
function nameOf(hex: string | null | undefined): string {
  if (!hex) return "-";
  const slots = currentLook().slots;
  const names = Object.keys(slots)
    .filter((n) => slots[n]!.toLowerCase() === hex.toLowerCase())
    .sort();
  return names.length ? names.slice(0, 2).join("/") : hex;
}

/** Colours equal as the terminal shows them: both unset, or the same `#rrggbb`. A cell the reference paints on bg.base
 * explicitly is not the terminal's own background, so it is not treated as unset. */
function sameColour(a: string | null | undefined, b: string | null | undefined, _slot?: string) {
  const norm = (c: string | null | undefined) => (c ? c.toLowerCase() : "");
  return norm(a) === norm(b);
}

/** The rows whose characters differ. */
export function textRows(ink: TCell[][], rat: RatCell[][]): number[] {
  return rat.flatMap((row, y) => {
    const a = row.map(([s]) => (s === "" ? " " : s)).join("");
    const b = (ink[y] ?? []).filter((c) => c.ch !== "").map((c) => c.ch).join("");
    const spin = (t: string) => t.replace(/[\u2800-\u28ff]/g, "*");
    return spin(a).trimEnd() === spin(b).trimEnd() ? [] : [y];
  });
}

export function compareCells(ink: TCell[][], rat: RatCell[][], skip: number[] = [], noBg: number[] = []): string[] {
  const found = new Map<string, { n: number; first: string }>();
  rat.forEach((row, y) => {
    if (skip.includes(y)) return;
    row.forEach(([sym, fg, bg, m], x) => {
      const c = ink[y]?.[x];
      if (c && c.ch === "") return; // the second cell of a wide character
      const ch = c?.ch ?? " ";
      const blank = sym.trim() === "";
      // the busy spinner in the top bar is a different frame of the same animation (U+2800 braille)
      if (/^[\u2800-\u28ff]$/.test(sym) && /^[\u2800-\u28ff]$/.test(ch)) return;
      const bad: string[] = [];
      if (ch !== (sym === "" ? " " : sym) && !(blank && ch.trim() === ""))
        bad.push(`char ${JSON.stringify(ch)} for ${JSON.stringify(sym)}`);
      if (!noBg.includes(y) && !sameColour(c?.bg, bg, "bg.base"))
        bad.push(`bg ${nameOf(c?.bg)} for ${nameOf(bg)}`);
      const flag = (f: number, v: boolean | undefined, w: string) => {
        if (!!v !== !!(m & f)) bad.push(`${w} ${!!v} for ${!!(m & f)}`);
      };
      flag(UNDERLINE, c?.underline, "underline");
      flag(REVERSED, c?.inverse, "reversed");
      flag(STRIKE, c?.strike, "strike");
      if (!blank) {
        if (!sameColour(c?.fg, fg, "text.primary"))
          bad.push(`fg ${nameOf(c?.fg)} for ${nameOf(fg)}`);
        flag(BOLD, c?.bold, "bold");
        flag(DIM, c?.dim, "dim");
        flag(ITALIC, c?.italic, "italic");
      }
      if (bad.length) {
        const key = bad.join(", ");
        const f = found.get(key);
        if (f) f.n++;
        else
          found.set(key, {
            n: 1,
            first: `row ${y} col ${x} ${JSON.stringify(sym)}`,
          });
      }
    });
  });
  return [...found].map(([k, v]) => `${v.n}× ${k} (first at ${v.first})`);
}

// Table geometry is not a styling matter and is not this file's to fix. At 80x24 the table's columns add up to more than the
// pane, and the reference (a table with Constraint::Length / Min) lets its layout solver squeeze them below their
// declared widths, so a cell cut by ui::table::cut to its declared width (ending in `…`) is then clipped again by the narrower
// column: "needs 1 approv", the title column empty. Ink cuts every cell at the declared width ("needs 1 appro…", "ABC-12…").
// The headings row (7) now agrees; the data row (8) is the one difference. The list must stay exactly this, so fixing the
// layout makes the test ask for the entry to go.
const KNOWN_TEXT_ROWS: Record<string, number[]> = {};

// The item page's title and tab rows: the buffer radar_core::shell::render paints has bg.base on every cell, but the bytes the
// live reference sends (read back through tmux, `ink-live colour`) leave the col-0 air, the title (and its padding) and the
// showing tab on the terminal's own background, and put bg.base only on the crumbs row, the gap and details after the
// title, and the rest of the tab row. Ink follows the live terminal (acceptance judges that), so the background of these
// two rows is not compared against the buffer; colours, bold and text still are. Why the two differ is not understood.
const LIVE_BG_ROWS: Record<string, number[]> = { "table 80x24": [3, 4, 5], "table 160x40": [3, 4, 5] };

const screens = ["tree", "table"];
describe("Ink's cells against the reference buffer for the same view data", () => {
  for (const name of screens)
    for (const size of ["80x24", "160x40"]) {
      it(`${name} ${size}`, async () => {
        applyLook(defaultLook("off"));
        const view = JSON.parse(read(`${name}_${size}.json`));
        const m = mount(view.width, view.height);
        live.push(m.stop);
        m.welcome();
        m.snapshot({ chrome: view.chrome, screen: view.screen, focus: view.focus ?? "middle" }, 1);
        await tick(60);
        const rat = JSON.parse(
          read(`${name}_${size}.cells.json`),
        ) as RatCell[][];
        // the reference buffer predates the column-edge dividers
        const ink = withoutEdgeDividers(parseAnsi(m.frame()), rat, (c) => c?.[0]);
        const known = KNOWN_TEXT_ROWS[`${name} ${size}`] ?? [];
        expect(textRows(ink, rat)).toEqual(known);
        expect(compareCells(ink, rat, known, LIVE_BG_ROWS[`${name} ${size}`] ?? [])).toEqual([]);
      });
    }
});
