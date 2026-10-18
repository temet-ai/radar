// Pure cell contracts, with literal samples shared with motion/interaction_tests.rs.
// No host, renderer frame, timers, filesystem or recorded snapshots.
import { describe, expect, test } from "vitest";
import { Grid, copyCell, type Cell, type Rect } from "../../src/motion/grid.js";
import {
  FOLD_MS, FOCUS_MS, MAX_VIEWPORT_CELLS, bandColour, borderForegrounds, bounded,
  composeFold, focusDrain, focusLap, foldVisible, lapHead, lapOrigin, lapWidth,
  mixColour, perimeter, protectedCell, type Border, type FoldRows,
} from "../../src/motion/interaction-effects.js";

const stops = [0x010305, 0x0b0d0f, 0x151719, 0x1f2123, 0x292b2d, 0x333537, 0x3d3f41];
const styled = (ch: string, fg = 0x090a0b): Cell => ({ ch, fg, bg: 0x131415, attr: "1;3;4" });
function clean(width: number, height: number): Grid {
  const grid = new Grid(width, height);
  grid.cells.forEach((cell) => Object.assign(cell, styled(".")));
  return grid;
}
function rowCells(labels: string, width: number): Cell[] {
  return [...labels].flatMap((ch) => Array.from({ length: width }, () => styled(ch, ch.codePointAt(0)!)));
}
function put(grid: Grid, area: Rect, cells: readonly Cell[]) {
  cells.forEach((cell, i) => Object.assign(grid.at(area.x + i % area.w, area.y + Math.floor(i / area.w)), copyCell(cell)));
}
const labels = (grid: Grid, area: Rect) => Array.from({ length: area.h }, (_, y) => grid.at(area.x, area.y + y).ch).join("");
const border = (area: Rect): Border => ({ area, keep: [] });
function outsideUnchanged(before: readonly Cell[], after: Grid, area: Rect) {
  after.cells.forEach((cell, i) => {
    const x = i % after.width, y = Math.floor(i / after.width);
    if (x < area.x || x >= area.x + area.w || y < area.y || y >= area.y + area.h)
      expect(cell, `outside ${x},${y}`).toEqual(before[i]);
  });
}
function foregroundOnly(before: readonly Cell[], after: Grid, geometry: Border) {
  const points = new Set(perimeter(geometry.area).map(([x, y]) => y * after.width + x));
  after.cells.forEach((cell, i) => {
    const protected_ = protectedCell(geometry, i % after.width, Math.floor(i / after.width));
    expect(points.has(i) && !protected_ ? { ...cell, fg: before[i]!.fg } : cell, `cell ${i}`).toEqual(before[i]);
  });
}

describe("fold cell composition", () => {
  test("integer quarters, last step and clamped endpoints are exact in both directions", () => {
    expect(FOLD_MS).toBe(160);
    for (const [ms, open, close] of [[-1, 0, 4], [0, 0, 4], [40, 1, 3], [80, 2, 2], [120, 3, 1], [159, 3, 1], [160, 4, 0], [9999, 4, 0]]) {
      expect(foldVisible(4, ms!, true), `open ${ms}`).toBe(open);
      expect(foldVisible(4, ms!, false), `close ${ms}`).toBe(close);
    }
    expect(foldVisible(0, 80, true)).toBe(0);
    expect(foldVisible(0, 80, false)).toBe(0);
    expect(foldVisible(1, 159, true)).toBe(0);
    expect(foldVisible(1, 159, false)).toBe(1);
  });

  test.each([
    { open: true, samples: [[0, "uvwxyz"], [40, "Auvwxy"], [80, "ABuvwx"], [120, "ABCuvw"], [159, "ABCuvw"], [160, "ABCDuv"]] as const },
    { open: false, samples: [[0, "ABCDuv"], [40, "ABCuvw"], [80, "ABuvwx"], [120, "Auvwxy"], [159, "Auvwxy"], [160, "uvwxyz"]] as const },
  ])("open=$open retains actual descendants and shifts the compact tail without duplicates", ({ open, samples }) => {
    const viewport = { x: 2, y: 2, w: 3, h: 7 }, area = { x: 2, y: 3, w: 3, h: 6 };
    const expanded = rowCells("ABCDuv", 3), compact = rowCells("uvwxyz", 3);
    for (const [ms, expected] of samples) {
      const grid = clean(8, 11);
      put(grid, area, open ? expanded : compact);
      const before = grid.cells.map(copyCell);
      expect(composeFold(grid, viewport, { area, expanded, compact, descendants: 4, open }, ms)).toBe(ms < 160);
      expect(labels(grid, area), `at ${ms} ms`).toBe(expected);
      outsideUnchanged(before, grid, area);
      if (!open && ms < 160) expect(grid.at(2, 3)).toEqual(expanded[0]);
      if (ms === 160) expect(grid.cells).toEqual(before);
    }
  });

  test("wide glyph continuations, combining marks and all cell styles survive a shifted row", () => {
    const area = { x: 1, y: 2, w: 4, h: 2 };
    const expanded = [styled("界", 20), styled("", 21), styled("e\u0301", 22), styled("x", 23), ...rowCells("B", 4)];
    const compact = rowCells("uv", 4), grid = clean(7, 5), before = grid.cells.map(copyCell);
    expect(composeFold(grid, grid.area, { area, expanded, compact, descendants: 2, open: false }, 80)).toBe(true);
    expect(grid.copyOf(area)).toEqual([...expanded.slice(0, 4), ...compact.slice(0, 4)]);
    outsideUnchanged(before, grid, area);
    grid.at(1, 2).attr = "9";
    expect(expanded[0]!.attr, "retained cells are copied, never aliased").toBe("1;3;4");
  });

  test("logical descendants are capped to the retained suffix height", () => {
    const area = { x: 1, y: 1, w: 1, h: 4 };
    for (const open of [true, false]) {
      const grid = clean(3, 6);
      expect(composeFold(grid, grid.area, { area, expanded: rowCells("ABCD", 1), compact: rowCells("uvwx", 1), descendants: Number.MAX_SAFE_INTEGER, open }, 80)).toBe(true);
      expect(labels(grid, area)).toBe("ABuv");
    }
  });

  test("incomplete, clipped or split-wide source rows fail atomically", () => {
    const area = { x: 1, y: 1, w: 2, h: 2 };
    const rows: FoldRows = { area, expanded: rowCells("AB", 2), compact: rowCells("uv", 2), descendants: 2, open: false };
    const wideEdge = rows.expanded.map(copyCell); wideEdge[1]!.ch = "界";
    const clippedStart = rows.expanded.map(copyCell); clippedStart[0]!.ch = "";
    const cases: FoldRows[] = [
      { ...rows, expanded: rows.expanded.slice(1) }, { ...rows, compact: rows.compact.slice(1) },
      { ...rows, descendants: 0 }, { ...rows, area: { ...area, w: 0 } },
      { ...rows, area: { ...area, x: 3, y: 3 } }, { ...rows, expanded: wideEdge },
      { ...rows, expanded: clippedStart },
    ];
    for (const invalid of cases) {
      const grid = clean(4, 4), before = grid.cells.map(copyCell);
      expect(composeFold(grid, grid.area, invalid, 80)).toBe(false);
      expect(grid.cells).toEqual(before);
    }
    const grid = clean(4, 4), before = grid.cells.map(copyCell);
    expect(composeFold(grid, { x: 1, y: 1, w: 1, h: 2 }, rows, 80)).toBe(false);
    expect(grid.cells).toEqual(before);
  });

  test("geometry caps reject malformed rectangles without allocating a retained frame", () => {
    expect(MAX_VIEWPORT_CELLS).toBe(65_536);
    expect(bounded({ x: 0, y: 0, w: 256, h: 256 })).toBe(true);
    for (const area of [
      { x: 0, y: 0, w: 257, h: 256 }, { x: -1, y: 0, w: 2, h: 2 },
      { x: 0, y: 0, w: 0, h: 1 }, { x: 0.5, y: 0, w: 2, h: 2 },
      { x: 0, y: 0, w: Infinity, h: 1 }, { x: NaN, y: 0, w: 1, h: 1 },
    ]) {
      expect(bounded(area)).toBe(false);
      expect(borderForegrounds(clean(2, 2), border(area))).toBeUndefined();
      expect(lapOrigin(border(area))).toBeUndefined();
    }
  });
});

describe("focus border cell composition", () => {
  test("clockwise perimeter visits each corner once and supports collapsed rectangles", () => {
    expect(perimeter({ x: 2, y: 3, w: 4, h: 3 })).toEqual([[2, 3], [3, 3], [4, 3], [5, 3], [5, 4], [5, 5], [4, 5], [3, 5], [2, 5], [2, 4]]);
    expect(perimeter({ x: 2, y: 3, w: 1, h: 1 })).toEqual([[2, 3]]);
    expect(perimeter({ x: 2, y: 3, w: 1, h: 3 })).toEqual([[2, 3], [2, 4], [2, 5]]);
    expect(perimeter({ x: 2, y: 3, w: 3, h: 1 })).toEqual([[2, 3], [3, 3], [4, 3]]);
  });

  test("head, width and seven-stop interpolation have literal boundary and quarter samples", () => {
    expect(FOCUS_MS).toBe(300);
    for (const [ms, expected] of [[-1, 0], [0, 0], [75, 10], [150, 20], [225, 30], [299, 39], [300, 40], [9999, 40]])
      expect(lapHead(40, ms!)).toBe(expected);
    for (const [count, expected] of [[0, 0], [1, 1], [6, 6], [7, 7], [40, 7], [56, 7], [57, 8], [80, 10]])
      expect(lapWidth(count!)).toBe(expected);
    expect(Array.from({ length: 7 }, (_, age) => bandColour(stops, age, 7))).toEqual(stops);
    expect(bandColour(stops, 9999, 7)).toBe(0x3d3f41);
    expect(bandColour(stops, 9999, 1)).toBe(0x010305);
    expect(bandColour(stops, 0, 0)).toBe(0x010305);
    const odd = [0, 0xff6401, 0, 0, 0, 0, 0];
    expect(bandColour(odd, 1, 13)).toBe(0x803201);
    expect(bandColour(odd, 1, 19)).toBe(0x552100);
    expect(mixColour(0, 0xff6401, 500)).toBe(0x803201);
  });

  test("the lap visits literal quarter positions and leaves the exact clean ramp behind", () => {
    const geometry = border({ x: 2, y: 3, w: 15, h: 7 });
    const quiet = Array.from({ length: 40 }, (_, i) => 0x646566 + i);
    for (const [ms, head, x, y] of [[0, 0, 2, 3], [75, 10, 12, 3], [150, 20, 16, 9], [225, 30, 6, 9], [299, 39, 2, 4]]) {
      const grid = clean(19, 12);
      perimeter(geometry.area).forEach(([px, py], i) => { grid.at(px, py).fg = 0x202122 + i; });
      const before = grid.cells.map(copyCell);
      expect(focusLap(grid, geometry, geometry, quiet, stops, ms!)).toBe(true);
      expect(grid.at(x!, y!).fg).toBe(0x010305);
      perimeter(geometry.area).forEach(([px, py], i) => {
        if (i > head!) expect(grid.at(px, py).fg, `ahead ${i}`).toBe(quiet[i]);
        if (head! - i >= 7) expect(grid.at(px, py), `passed ${i}`).toEqual(before[py * grid.width + px]);
      });
      foregroundOnly(before, grid, geometry);
    }
  });

  test("the short trail does not wrap and exposes all seven colours in order", () => {
    const geometry = border({ x: 0, y: 0, w: 10, h: 2 }), quiet = Array(20).fill(0x646566);
    const first = clean(10, 2);
    expect(focusLap(first, geometry, geometry, quiet, stops, 0)).toBe(true);
    expect(first.at(0, 0).fg).toBe(0x010305);
    expect(first.at(0, 1).fg).toBe(0x646566);
    const sixth = clean(10, 2);
    expect(focusLap(sixth, geometry, geometry, quiet, stops, 90)).toBe(true);
    expect(Array.from({ length: 7 }, (_, x) => sixth.at(x, 0).fg)).toEqual([0x3d3f41, 0x333537, 0x292b2d, 0x1f2123, 0x151719, 0x0b0d0f, 0x010305]);
  });

  test("title and scrollbar gaps are protected but still consume physical lap distance", () => {
    const geometry: Border = {
      area: { x: 1, y: 1, w: 10, h: 4 }, title: { x: 2, y: 1, w: 4, h: 1 },
      keep: [{ x: 10, y: 2, w: 1, h: 2 }, { x: 3, y: 4, w: 3, h: 1 }],
    };
    expect(lapOrigin(geometry)).toBe(5);
    const grid = clean(13, 7), before = grid.cells.map(copyCell);
    expect(focusLap(grid, geometry, geometry, Array(24).fill(0x646566), stops, 75)).toBe(true);
    expect(grid.at(10, 3)).toEqual(before[3 * 13 + 10]);
    expect(grid.at(10, 4).fg, "the next physical position is still ahead").toBe(0x646566);
    foregroundOnly(before, grid, geometry);
    expect(lapOrigin({ ...geometry, title: { x: 1, y: 1, w: 10, h: 1 }, keep: [] })).toBe(10);
    expect(lapOrigin({ ...geometry, keep: [geometry.area] })).toBeUndefined();
  });

  test("outgoing drain uses half-up RGB interpolation at quarters and the clean endpoint", () => {
    const geometry = border({ x: 1, y: 1, w: 5, h: 3 });
    for (const [ms, expected] of [[0, 0], [75, 0x401900], [100, 0x552100], [150, 0x803201], [225, 0xbf4b01], [299, 0xfe6401], [300, 0xff6401]]) {
      const grid = clean(7, 5);
      perimeter(geometry.area).forEach(([x, y]) => { grid.at(x, y).fg = 0xff6401; });
      const before = grid.cells.map(copyCell);
      expect(focusDrain(grid, geometry, geometry, Array(12).fill(0), ms!)).toBe(ms! < 300);
      for (const [x, y] of perimeter(geometry.area)) expect(grid.at(x, y).fg).toBe(expected);
      foregroundOnly(before, grid, geometry);
    }
  });

  test("protected non-RGB cells survive drain while each clean target retains its own colour", () => {
    const geometry: Border = { area: { x: 1, y: 1, w: 6, h: 4 }, title: { x: 2, y: 1, w: 2, h: 1 }, keep: [{ x: 6, y: 2, w: 1, h: 2 }] };
    const grid = clean(9, 7), prior: number[] = [];
    perimeter(geometry.area).forEach(([x, y], i) => {
      const protect = protectedCell(geometry, x, y);
      grid.at(x, y).fg = protect ? -1 : ((20 + i) << 16) | 0x283c;
      prior.push(protect ? -1 : 0);
    });
    const before = grid.cells.map(copyCell);
    expect(focusDrain(grid, geometry, geometry, prior, 150)).toBe(true);
    expect(grid.at(1, 1).fg).toBe(0x0a141e);
    expect(grid.at(4, 1).fg).toBe(0x0c141e);
    foregroundOnly(before, grid, geometry);
  });

  test("geometry, non-RGB samples and incomplete captures reject the whole overlay", () => {
    const geometry = border({ x: 1, y: 1, w: 4, h: 3 }), prior = Array(10).fill(0x323334);
    const changes: Border[] = [
      { ...geometry, area: { ...geometry.area, w: 3 } },
      { ...geometry, title: { x: 2, y: 1, w: 1, h: 1 } },
      { ...geometry, keep: [{ x: 4, y: 2, w: 1, h: 1 }] },
      border({ x: 4, y: 4, w: 4, h: 3 }),
    ];
    for (const changed of changes) {
      const grid = clean(6, 5), before = grid.cells.map(copyCell);
      expect(focusLap(grid, geometry, changed, prior, stops, 150)).toBe(false);
      expect(focusDrain(grid, geometry, changed, prior, 150)).toBe(false);
      expect(grid.cells).toEqual(before);
    }
    for (const invalid of [prior.slice(1), [...prior.slice(0, 9), -1]]) {
      const grid = clean(6, 5), before = grid.cells.map(copyCell);
      expect(focusLap(grid, geometry, geometry, invalid, stops, 150)).toBe(false);
      expect(focusDrain(grid, geometry, geometry, invalid, 150)).toBe(false);
      expect(grid.cells).toEqual(before);
    }
    const grid = clean(6, 5); grid.at(1, 2).fg = -1;
    const before = grid.cells.map(copyCell);
    expect(focusDrain(grid, geometry, geometry, prior, 150)).toBe(false);
    expect(focusLap(grid, geometry, geometry, prior, stops.slice(1), 150)).toBe(false);
    expect(focusLap(grid, geometry, geometry, prior, [...stops.slice(0, 6), -1], 150)).toBe(false);
    expect(grid.cells).toEqual(before);
  });

  test("tiny or fully protected borders and elapsed endpoints never damage clean cells", () => {
    for (const [w, h] of [[1, 1], [1, 4], [4, 1]]) {
      const geometry = border({ x: 2, y: 2, w: w!, h: h! }), grid = clean(7, 7);
      const prior = borderForegrounds(grid, geometry)!;
      const before = grid.cells.map(copyCell);
      expect(focusLap(grid, geometry, geometry, prior, stops, 150)).toBe(true);
      foregroundOnly(before, grid, geometry);
      const protected_ = { ...geometry, keep: [geometry.area] }, protectedBefore = grid.cells.map(copyCell);
      expect(focusLap(grid, protected_, protected_, prior, stops, 150)).toBe(false);
      expect(focusDrain(grid, protected_, protected_, prior, 150)).toBe(false);
      for (const ms of [300, 9999]) {
        expect(focusLap(grid, geometry, geometry, prior, stops, ms)).toBe(false);
        expect(focusDrain(grid, geometry, geometry, prior, ms)).toBe(false);
      }
      expect(grid.cells).toEqual(protectedBefore);
    }
  });
});
