import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import React from "react";
import { afterEach, describe, expect, it } from "vitest";
import { TabRow, tabWindow } from "../../src/app/TabRow.js";
import { styledFrameAt } from "../nodes/helpers.js";
import { parseAnsi } from "../nodes/ansi.js";
import { paintHover } from "../../src/app/hover.js";
import { applyLook, currentLook, defaultLook, slotStyle } from "../../src/text/look.js";
import { ATTENTION_DOT } from "../../src/text/icons.js";
import { cellWidth } from "../../src/chrome/width.js";
import type { TabView } from "../../src/protocol/generated/TabView.js";


// Live survey (T22): the reference's page tab row sits on bg.base (#0a0812) from its leading space to the end of the row, the
// showing title (bold accent) and the inactive titles included, until the row has the keys (the showing title is then
// on the accent). Ink left the leading space, the showing title and the earlier titles without one.
describe("the page tab row", () => {
  const BASE = "#0a0812";
  const row = (on: number) =>
    parseAnsi(styledFrameAt(<TabRow tabs={[{ label: "Overview" }, { label: "Changes" }, { label: "Threads" }]} on={on} focused={false} width={40} base={BASE} />, 42))[0]!;
  it.each([0, 1, 2])("puts every cell of the row on bg.base with tab %i showing", (on) => {
    expect(row(on).slice(0, 41).every((c) => c.bg === BASE)).toBe(true);
  });
  it("keeps the showing title bold in the accent", () => {
    const cells = row(1);
    const at = cells.map((c) => c.ch).join("").indexOf("[Changes]");
    expect([cells[at + 1]!.bold, cells[at + 1]!.fg]).toEqual([true, "#e135ff"]);
  });
});

describe("the page tab under the pointer", () => {
  const BASE = "#0a0812";
  it("is lit with bg.hover over its hit (the title and one cell either side), the others stay on bg.base", () => {
    const hits: Array<{ x: number; y: number; w: number; h: number; target: string }> = [];
    const frame = styledFrameAt(<TabRow tabs={[{ label: "Overview" }, { label: "Changes" }, { label: "Threads" }]} on={0} focused={false} width={40} base={BASE} registry={{ add: (h) => void hits.push(h) }} />, 42);
    const hit = hits.find((h) => h.target === "tab:1")!;
    const cells = parseAnsi(paintHover(frame, 42, hit))[0]!;
    const text = cells.map((c) => c.ch).join("");
    const at = text.indexOf("Changes");
    for (let i = at - 1; i < at + 8; i++) expect(cells[i]!.bg, `cell ${i}`).toBe("#2a263e");
    expect(cells[text.indexOf("Threads") + 2]!.bg).toBe(BASE);
    expect(cells[text.indexOf("Overview")]!.bg).toBe(BASE);
  });
});

const originalLook = currentLook();
afterEach(() => applyLook(originalLook));

describe("page tab attention", () => {
  it.each(["unicode", "nerd", "emoji", "off"] as const)("keeps one warning dot after an attentive name with the %s icon pack", (pack) => {
    applyLook(defaultLook(pack));
    const tabs: TabView[] = [{ label: "Overview", icon: "info" }, { label: "Changes", attention: true }, { label: "Threads", attention: false }];
    for (const focused of [false, true]) {
      const cells = parseAnsi(styledFrameAt(<TabRow tabs={tabs} on={1} focused={focused} width={60} base="#0a0812" />, 62))[0]!;
      const text = cells.map((c) => c.ch).join("");
      expect(text).toContain(`[Changes ${ATTENTION_DOT}]`);
      expect(cells.filter((c) => c.ch === ATTENTION_DOT)).toHaveLength(1);
      const dot = cells.find((c) => c.ch === ATTENTION_DOT)!;
      expect(dot.fg).toBe(slotStyle("status.warning").color);
      expect(dot.bg).toBe(focused ? slotStyle("accent.primary").color : "#0a0812");
      expect(text).not.toMatch(/\d/);
    }
  });

  it("styles the explicit dot independently of a literal dot in the name", () => {
    const cells = parseAnsi(styledFrameAt(<TabRow tabs={[{ label: "A ● B", attention: true }]} on={0} focused={true} width={20} base="#0a0812" />, 22))[0]!;
    const dots = cells.filter((c) => c.ch === ATTENTION_DOT);
    expect(dots).toHaveLength(2);
    expect(dots[0]!.fg).toBe(slotStyle("bg.base").color);
    expect(dots[1]!.fg).toBe(slotStyle("status.warning").color);
  });

  it("includes a wide name and its dot in mouse targets without selecting the active tab", () => {
    const tabs = [{ label: "Overview" }, { label: "図書 Cafe\u0301", attention: true }, { label: "Threads" }];
    const hits: Array<{ x: number; y: number; w: number; h: number; target: string }> = [];
    const frame = styledFrameAt(<TabRow tabs={tabs} on={0} focused={false} width={40} base="#0a0812" registry={{ add: (h) => void hits.push(h) }} />, 42);
    const cells = parseAnsi(frame)[0]!;
    const dotAt = cells.findIndex((c) => c.ch === ATTENTION_DOT);
    expect(dotAt).toBeGreaterThan(0);
    const hit = hits.find((h) => h.target === "tab:1")!;
    expect(hit.x).toBeLessThanOrEqual(dotAt);
    expect(hit.x + hit.w).toBeGreaterThan(dotAt);
    expect(hits.some((h) => h.target === "tab:0")).toBe(false);
    const hovered = parseAnsi(paintHover(frame, 42, hit))[0]!;
    expect(hovered[dotAt]!.fg).toBe(slotStyle("status.warning").color);
  });
});

describe("page tab windows measure names and attention in cells", () => {
  const tabs: TabView[] = [{ label: "Overview" }, { label: "Changes", attention: true }, { label: "Threads" }];
  it.each([80, 160])("keeps all names and original indices at %i cells", (width) => {
    expect(tabWindow(tabs, 1, width).map(({ index, text, x }) => ({ index, text, x }))).toEqual([
      { index: 0, text: "Overview", x: 0 },
      { index: 1, text: "[Changes ●]", x: 10 },
      { index: 2, text: "Threads", x: 23 },
    ]);
  });
  it("accounts for the attention footprint before choosing the first visible tab", () => {
    expect(tabWindow(tabs, 1, 20).map(({ index, text }) => ({ index, text }))).toEqual([
      { index: 1, text: "[Changes ●]" },
      { index: 2, text: "Threads" },
    ]);
    expect(tabWindow(tabs.map((t) => ({ ...t, attention: false })), 1, 20)[0]!.index).toBe(0);
  });
  it.each([
    [0, ""], [1, "…"], [2, "…"], [3, "[…]"], [4, "[C…]"], [5, "[… ●]"], [8, "[Cha… ●]"],
  ])("clips a long selected name safely at %i cells", (width, want) => {
    const shown = tabWindow([{ label: "Changes", attention: true }], 0, width);
    expect(shown.map((t) => t.text).join("")).toBe(want);
    for (const t of shown) expect(t.x + cellWidth(t.text)).toBeLessThanOrEqual(width);
    if (width >= 5) expect(shown[0]!.pieces.filter((p) => p.attention).map((p) => p.text)).toEqual([ATTENTION_DOT]);
  });
  it.each([5, 7, 9, 12, 18])("keeps wide and combining characters intact at %i cells", (width) => {
    const shown = tabWindow([{ label: "Hidden" }, { label: "図書 Cafe\u0301", attention: true }], 1, width);
    expect(shown.map((t) => t.index)).toContain(1);
    for (const t of shown) {
      expect(t.x + cellWidth(t.text)).toBeLessThanOrEqual(width);
      expect(t.text).not.toContain("�");
    }
    expect(shown.find((t) => t.index === 1)!.text).toMatch(/ ●\]$/);
  });
  it("pads the spare cell after clipping a wide glyph before the dot or closing bracket", () => {
    expect(tabWindow([{ label: "日本語の長い名前", attention: true }], 0, 10)[0]!.text).toBe("[日本…  ●]");
    expect(tabWindow([{ label: "日本語の長い名前" }], 0, 10)[0]!.text).toBe("[日本語… ]");
    const partial = tabWindow([{ label: "A" }, { label: "日本語の長い名前", attention: true }], 0, 11);
    expect(partial[1]!.text).toBe("日本… ");
    expect(partial[1]!.x + cellWidth(partial[1]!.text)).toBe(11);
  });
  it.each([20, 80, 160])("matches the inset pointer target on a wide name, dot and padding at %i cells", (width) => {
    const rowTabs = [...tabs, { label: "Pipeline", attention: true }, { label: "図", attention: true }];
    const hits: Array<{ x: number; y: number; w: number; h: number; target: string }> = [];
    const cells = parseAnsi(styledFrameAt(<TabRow tabs={rowTabs} on={3} focused={false} width={width} registry={{ add: (h) => void hits.push(h) }} />, width + 2))[0]!;
    const titleAt = cells.findIndex((c) => c.ch === "図");
    const dotAt = cells.findIndex((c, i) => i > titleAt && c.ch === ATTENTION_DOT);
    const hit = hits.find((h) => h.target === "tab:4")!;
    expect(titleAt).toBeGreaterThan(0);
    expect(dotAt).toBe(titleAt + 3);
    expect(hit.x).toBe(titleAt - 1);
    expect(hit.x + hit.w).toBe(dotAt + 2);
    for (const x of [titleAt - 1, titleAt, titleAt + 1, dotAt, dotAt + 1]) {
      expect(hits.find((h) => x >= h.x && x < h.x + h.w)?.target).toBe("tab:4");
    }
    expect(hits.some((h) => h.target === "tab:3")).toBe(false);
    if (width === 20) expect(hits.map((h) => h.target)).toEqual(["tab:4"]);
  });
  it("does not create a pointer target for an empty inactive label", () => {
    const hits: Array<{ target: string }> = [];
    styledFrameAt(<TabRow tabs={[{ label: "" }, { label: "Changes" }]} on={1} focused={false} width={20} registry={{ add: (h) => void hits.push(h) }} />, 22);
    expect(hits).toEqual([]);
  });
  it("preserves the original indices and bounds of visible pointer targets", () => {
    const hits: Array<{ x: number; y: number; w: number; h: number; target: string }> = [];
    const rowTabs = [...tabs, { label: "Pipeline", attention: true }, { label: "Commits" }];
    styledFrameAt(<TabRow tabs={rowTabs} on={3} focused={false} width={20} registry={{ add: (h) => void hits.push(h) }} />, 24);
    expect(hits.map((h) => h.target)).toEqual(["tab:4"]);
    for (const h of hits) {
      expect(h.x).toBeGreaterThanOrEqual(0);
      expect(h.x + h.w).toBeLessThanOrEqual(24);
      expect(h.w).toBeGreaterThan(0);
    }
  });
});
