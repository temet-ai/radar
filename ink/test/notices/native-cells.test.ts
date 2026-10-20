// Independent expansion of actual native Buffer captures, compared to production Ink layout and painting.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { applyHostLook, applyLook, defaultLook, type HostLook } from "../../src/text/look.js";
import { Grid } from "../../src/motion/grid.js";
import { parseHex } from "../../src/motion/color.js";
import { charWidth } from "../../src/chrome/width.js";
import { layoutNotices, layoutNoticeHistory } from "../../src/notices/layout.js";
import { paintNoticeRows } from "../../src/notices/paint.js";
import type { NoticeView } from "../../src/protocol/generated/NoticeView.js";
import type { NoticeHistoryView } from "../../src/protocol/generated/NoticeHistoryView.js";
type Run = [string, number, string, string, string];
type Capture = { format: number; name: string; width: number; height: number; look: HostLook; content: number[]; body: number[]; now_ms: number; motion: boolean; notices: NoticeView[]; hidden: number; history?: NoticeHistoryView | null; before: Run[][]; rows: Run[][]; toasts: Array<{ id: number; area: number[] }>; summary?: { area: number[] } | null };
const directory = fileURLToPath(new URL("../fixtures/notices/", import.meta.url));
const fixtures = fs.readdirSync(directory).filter((name) => name.endsWith(".json")).sort().map((name) => JSON.parse(fs.readFileSync(`${directory}/${name}`, "utf8")) as Capture);
const names = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "blackBright", "redBright", "greenBright", "yellowBright", "blueBright", "magentaBright", "cyanBright", "whiteBright"];
const modifiers: Record<string, string> = { bold: "1", dim: "2", italic: "3", underline: "4", reversed: "7", strike: "9" };
function colour(word: string, background: boolean): string {
  const indexed = /^ansi256\((\d+)\)$/.exec(word); if (indexed) return `${background ? 48 : 38};5;${indexed[1]}`;
  const n = names.indexOf(word); return n < 0 ? "" : String((background ? 40 : 30) + n % 8 + (n >= 8 ? 60 : 0));
}
function expand(rows: Run[][], width: number, height: number): Grid {
  const grid = new Grid(width, height), segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" }); expect(rows).toHaveLength(height);
  rows.forEach((runs, y) => {
    let x = 0;
    for (const [text, count, fg, bg, mods] of runs) {
      const attr = [...mods.split("+").map((mod) => modifiers[mod]), colour(fg, false), colour(bg, true)].filter(Boolean).join(";"), style = { fg: parseHex(fg), bg: parseHex(bg), attr }, start = x;
      for (const { segment } of segmenter.segment(text)) {
        const cp = segment.codePointAt(0)!, cells = segment.includes("\u200d") || segment.includes("\ufe0f") || cp >= 0x1f1e6 && cp <= 0x1f1ff ? 2 : charWidth(cp); if (!cells) continue;
        Object.assign(grid.at(x++, y), style, { ch: segment }); for (let tail = 1; tail < cells; tail++) Object.assign(grid.at(x++, y), style, { ch: "" });
      }
      // Cell count comes from the actual Buffer, not this parser.
      expect(x - start, `${y}:${start} captured cell count`).toBe(count);
    }
    expect(x, `row ${y} width`).toBe(width);
  });
  return grid;
}
const rect = ([x = 0, y = 0, w = 0, h = 0]: number[]) => ({ x, y, w, h });
afterEach(() => applyLook(defaultLook()));
describe("actual Rust notice cells", () => {
  it("keeps the reviewed native capture matrix complete", () => expect(fixtures).toHaveLength(21));
  it.each(fixtures.map((f) => [f.name, f] as const))("matches every glyph, ground, modifier and wide tail: %s", (_name, f) => {
    expect(f.format).toBe(1); applyHostLook(f.look); const actual = expand(f.before, f.width, f.height);
    if (f.history) {
      const laid = layoutNoticeHistory(f.history, f.body[2]!, f.body[3]!); paintNoticeRows(actual, laid.lines.map((cells, y) => ({ cells, rect: { x: f.body[0]!, y: f.body[1]! + y, w: f.body[2]!, h: 1 }, target: "" })));
    } else {
      const laid = layoutNotices(f.notices, f.hidden, rect(f.content), f.now_ms, f.motion); expect(laid.map((r) => r.rect)).toEqual([...(f.summary ? [rect(f.summary.area)] : []), ...f.toasts.map((t) => rect(t.area))]); paintNoticeRows(actual, laid);
    }
    const expected = expand(f.rows, f.width, f.height), differences = actual.cells.flatMap((cell, i) => JSON.stringify(cell) === JSON.stringify(expected.cells[i]) ? [] : [{ x: i % f.width, y: Math.floor(i / f.width), actual: cell, expected: expected.cells[i] }]);
    expect(differences.slice(0, 12), `${differences.length} mismatched cells`).toEqual([]);
  });
});
