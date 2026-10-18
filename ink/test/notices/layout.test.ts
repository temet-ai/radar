import { afterEach, describe, expect, it } from "vitest";
import { layoutNotices, layoutNoticeHistory, noticeContent, NOTICE_OPEN } from "../../src/notices/layout.js";
import { noticeCells, wrapNotice } from "../../src/notices/text.js";
import { paintNoticeRows, noticeStyle } from "../../src/notices/paint.js";
import { Grid } from "../../src/motion/grid.js";
import { applyLook, defaultLook, currentLook } from "../../src/text/look.js";
import { widthOfCells } from "../../src/text/cells.js";
import { notice } from "./fixture.js";
afterEach(() => applyLook(defaultLook()));
const words = (cells: { ch: string }[]) => cells.map((c) => c.ch).join("");
describe("shared cell-safe notice layouts", () => {
  it.each([[80, 24], [160, 40]])("stacks inside panel borders at %dx%d, newest last and older dimmed", (w, h) => {
    const rows = layoutNotices([notice(1), notice(2, { tone: "error" })], 0, noticeContent(w, h, true), 0, true);
    expect(rows.map((r) => r.rect)).toEqual([{ x: w - 65, y: h - 4, w: 64, h: 1 }, { x: w - 65, y: h - 3, w: 64, h: 1 }]);
    expect(rows[0]!.cells.every((c) => c.s.dim)).toBe(true); expect(rows[1]!.cells.every((c) => !c.s.dim && c.s.bold)).toBe(true); expect(rows.map((r) => widthOfCells(r.cells))).toEqual([64, 64]);
  });
  it("uses ceil drain and theme tint, settles motion-off and expires exactly", () => {
    const rect = { x: 0, y: 0, w: 20, h: 2 }, row = layoutNotices([notice(1)], 0, rect, 2500, true)[0]!;
    expect(row.cells.filter((c) => c.s.backgroundColor !== currentLook().slots["bg.overlay"])).toHaveLength(10);
    const fixed = layoutNotices([notice(1)], 0, rect, 4999, false)[0]!; expect(fixed.cells.every((c) => c.s.backgroundColor === row.cells[0]!.s.backgroundColor)).toBe(true); expect(layoutNotices([notice(1)], 0, rect, 5000, true)).toEqual([]);
  });
  it("bounds overflow and retains the newest in tiny areas", () => {
    const active = [1, 2, 3, 4].map((id) => notice(id)), stack = layoutNotices(active, 6, { x: 0, y: 0, w: 64, h: 10 }, 0, true);
    expect(stack).toHaveLength(5); expect(stack[0]!.target).toBe(NOTICE_OPEN); expect(words(stack[0]!.cells)).toContain("+6 earlier notices · ~ history");
    const tiny = layoutNotices(active, 6, { x: 0, y: 0, w: 64, h: 1 }, 0, true); expect(tiny).toHaveLength(1); expect(tiny[0]!.target).toBe("chrome:notice:4"); expect(words(tiny[0]!.cells)).toContain("+9 more");
  });
  it("clips whole wide/combining/joined clusters and sanitizes terminal controls", () => {
    const row = layoutNotices([notice(1, { text: "界 e\u0301 👩‍💻\n\x1b[31m City Watch" })], 0, { x: 0, y: 0, w: 14, h: 1 }, 0, true)[0]!;
    expect(widthOfCells(row.cells)).toBe(14); expect(words(row.cells)).not.toMatch(/[\n\x1b]/); expect(row.cells.find((c) => c.ch === "👩‍💻")?.w).toBe(2); expect(row.cells.find((c) => c.ch === "e\u0301")?.w).toBe(1); expect(words(row.cells)).toContain("…");
    expect(noticeCells("👩‍💻")).toHaveLength(1); expect(wrapNotice("界\n\nCity Watch", 1)).toEqual(["…", "", "C", "i", "t", "y", "W", "a", "t", "c", "h"]);
  });
  it("snaps the drain before a wide glyph instead of splitting its ground", () => {
    applyLook(defaultLook("off")); const row = layoutNotices([notice(1, { text: "界界" })], 0, { x: 0, y: 0, w: 10, h: 1 }, 1500, true)[0]!;
    expect(row.cells.find((c) => c.ch === "界")!.s.backgroundColor).toBe(currentLook().slots["bg.overlay"]);
  });
  it.each(["16", "256", "mono"] as const)("uses the host %s tier without invented RGB", (tier) => {
    const look = defaultLook("off"); applyLook({ ...look, tier, slots: Object.fromEntries(Object.keys(look.slots).map((k) => [k, tier === "mono" ? "" : tier === "16" ? k === "status.error" ? "red" : "black" : "ansi256(236)"])) });
    const row = layoutNotices([notice(1, { tone: "error" })], 0, { x: 0, y: 0, w: 20, h: 1 }, 2500, true)[0]!;
    expect(new Set(row.cells.map((c) => c.s.backgroundColor))).toEqual(new Set([tier === "mono" ? undefined : tier === "16" ? "black" : "ansi256(236)"])); expect(row.cells.every((c) => tier !== "mono" || c.s.inverse)).toBe(true);
    const painted = noticeStyle(row.cells[0]!.s); expect(painted.bg).toBe(-1); expect(painted.attr).toContain(tier === "mono" ? "7" : tier === "16" ? "40" : "48;5;236");
  });
  it("erases underlying wide halves without changing their outside style", () => {
    const grid = new Grid(10, 1); Object.assign(grid.at(1, 0), { ch: "界", fg: 123, bg: 456, attr: "1" }); grid.at(2, 0).ch = ""; grid.at(7, 0).ch = "界"; grid.at(8, 0).ch = "";
    paintNoticeRows(grid, [{ rect: { x: 2, y: 0, w: 6, h: 1 }, target: "chrome:notice:1", cells: noticeCells("      ") }]);
    expect(grid.cells.every((c) => c.ch === " ")).toBe(true); expect(grid.at(1, 0)).toEqual({ ch: " ", fg: 123, bg: 456, attr: "1" });
  });
  it("draws separate history with full detail, omission count and stable inert row IDs", () => {
    const entries = Array.from({ length: 30 }, (_, i) => notice(i + 1, { text: `Watch report ${i + 1}` }));
    const laid = layoutNoticeHistory({ entries, omitted: 12, selected: 20, scroll: 0, detail_scroll: 4, detail: notice(10, { text: Array.from({ length: 30 }, (_, i) => `Detail ${i}`).join("\n") }) }, 80, 22), text = laid.lines.map(words).join("\n");
    expect(text).toContain("This session only · 30 retained · 12 older omitted"); expect(text).toContain("Notice details · info · lines 5–11 of 30"); expect(text).toContain("Detail 4"); expect(laid.hits.some((h) => h.target === "chrome:notices:id:10")).toBe(true); expect(laid.lines.every((line) => widthOfCells(line) === 80)).toBe(true);
  });
  it("keeps heading padding plain while selection still fills the whole list row", () => {
    const entry = notice(1), laid = layoutNoticeHistory({ entries: [entry], detail: entry, omitted: 0, selected: 0, scroll: 0, detail_scroll: 0 }, 80, 22);
    const look = currentLook(), plain = { color: look.slots["text.primary"], backgroundColor: look.slots["bg.base"] };
    expect(laid.lines[0]![1]!.s.bold).toBe(true);
    expect(laid.lines[1]![1]!.s.color).toBe(look.slots["text.muted"]);
    expect(laid.lines[14]![1]!.s.color).toBe(look.slots["text.secondary"]);
    for (const row of [0, 1, 14]) expect(laid.lines[row]!.at(-2)).toEqual({ ch: " ", w: 1, s: plain });
    expect(laid.lines[3]!.at(-2)!.s).toMatchObject({ color: look.slots["text.emphasis"], backgroundColor: look.slots["bg.selection"], bold: true });
  });

});
