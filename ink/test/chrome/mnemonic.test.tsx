import { vi } from "vitest";
vi.hoisted(() => { process.env.FORCE_COLOR = "3"; });
import React from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { hintCells, hitAt, layoutMenu, layoutStatus, type Cell } from "../../src/chrome/layout.js";
import { historyFooter, layoutHistory } from "../../src/chrome/history.js";
import { paletteLines } from "../../src/chrome/palette.js";
import { cellWidth, clipSegments } from "../../src/chrome/width.js";
import { mnemonicPrefix, mnemonicRange, mnemonicRuns } from "../../src/text/mnemonic.js";
import { cellsOf, mnemonicLineCells, paragraphCells, widthOfCells } from "../../src/text/cells.js";
import { applyLook, bgOf, currentLook, defaultLook, slotStyle } from "../../src/text/look.js";
import { ActionList, invokeIntent } from "../../src/nodes/actions.js";
import { lineRows } from "../../src/nodes/index.js";
import { CellRow, frameOf } from "../../src/chrome/Chrome.js";
import { TextLine, textLineString } from "../../src/text/TextLine.js";
import { layoutTour, plainTourText } from "../../src/chrome/tour.js";
import { plainLines } from "../../src/workarea/plain.js";
import { frameAt, styledFrameAt } from "../nodes/helpers.js";
import { parseAnsi } from "../nodes/ansi.js";
import { booksChrome } from "./fixture.js";
import type { Choice, Hint, MenuView } from "../../src/chrome/types.js";
import type { HistoryView } from "../../src/protocol/generated/HistoryView.js";
import type { ActionSpec, Text } from "../../src/nodes/view.js";

const text = (cells: Cell[]) => cells.map((c) => c.text).join("");
const underline = (cells: Cell[]) => cells.filter((c) => c.style?.underline).map((c) => c.text).join("");
let before = currentLook();
beforeEach(() => { before = currentLook(); applyLook(defaultLook("off")); });
afterEach(() => applyLook(before));
const choice = (label: string, keys: string, at?: number): Choice => ({ label, keys, mnemonic_at: at, hint: "", value: keys, target: `chrome:menu:${keys}` });
const menu = (items: Choice[], question = false): MenuView => ({ id: "test", title: "actions", question, items, selected: 0 });
const hint = (label: string, keys: string, at?: number): Hint => ({ label, keys, mnemonic_at: at, keys_of: [{ label: keys, chord: { key: { char: keys } }, target: "chrome:key:0:0" }] });

describe("host scalar mnemonic contract", () => {
  it.each([
    ["refresh", "r", 0, [0, 1]], ["extra actions", "x", 1, [1, 2]],
    ["mirror", "r", 2, [2, 3]], ["İstanbul", "i", 0, [0, 1]],
    ["éclair", "É", 0, [0, 1]], ["I\u0307stanbul", "İ", 0, [0, 2]], ["e\u0301dit", "e", 0, [0, 2]],
    ["😀 edit", "e", 2, [2, 3]], ["👩‍💻 edit", "e", 4, [4, 5]],
    ["𝒜ction", "𝒜", 0, [0, 1]],
  ] as const)("validates %s / %s at scalar %i", (words, key, at, range) => {
    expect(mnemonicRange(words, key, at)).toEqual(range);
    expect(mnemonicPrefix(words, key, { mnemonic_at: at })).toBe("");
  });
  it.each([undefined, null, -1, 1.5, 999, Number.NaN, Number.POSITIVE_INFINITY, 2])("retains original keys for malformed or mismatched index %s", (at) => {
    const h = { ...hint("refresh", "r"), mnemonic_at: at, mnemonic_shift: true };
    expect(text(hintCells(h))).toBe("r refresh");
    expect(underline(hintCells(h))).toBe("");
  });
  it.each(["ctrl-r", "↑↓", "esc", "1", "?", "r/r"])("never suppresses named, chord, numeric or composite %s", (key) => {
    expect(mnemonicPrefix("refresh", key, { mnemonic_at: 0, mnemonic_shift: true })).toBe(key);
  });
  it("never searches for a better position and only adds underline to the existing style", () => {
    expect(mnemonicRange("refresh", "r", 1)).toBeUndefined();
    const style = { color: "#123456", backgroundColor: "#654321", bold: true, italic: true, inverse: true };
    const runs = mnemonicRuns("e\u0301dit", "e", { mnemonic_at: 0 }, style);
    expect(runs[0]).toEqual({ text: "e\u0301", style: { ...style, underline: true } });
    expect(runs[1]?.style).toEqual(style);
  });
  it("counts and clips complete emoji graphemes rather than their component scalars", () => {
    expect(cellWidth("👩‍💻 e\u0301")).toBe(4);
    expect(cellWidth("a\uFE0F")).toBe(1);
    expect(cellWidth("#\uFE0F")).toBe(2);
    expect(cellWidth("*\uFE0F")).toBe(2);
    expect(cellWidth("*\u0301\uFE0F")).toBe(1);
    expect(cellWidth("*\u200D\uFE0F")).toBe(1);
    expect(cellWidth("\uFE0F")).toBe(0);
    expect(cellWidth("1\uFE0F\u20E3")).toBe(2);
    expect(text(clipSegments([{ text: "👩‍💻e" }], 1))).toBe("");
    expect(text(clipSegments([{ text: "👩‍💻e" }], 2))).toBe("👩‍💻");
  });
});

describe("status and menu mnemonic geometry", () => {
  it("uses the host word and exact whole-hint hit, including index zero and Shift", () => {
    const chrome = booksChrome();
    chrome.status = { scope: "WATCH", hints: [hint("quit", "q", 0), { ...hint("ask an agent", "A", 0), mnemonic_shift: true }], counts: [] };
    const laid = layoutStatus(chrome, 80, 23);
    expect(text(laid.cells).trimEnd()).toBe(" WATCH   quit   shift ask an agent");
    expect(underline(laid.cells)).toBe("qa");
    expect(text(laid.cells)).not.toContain("q quit");
    for (const hit of laid.hits) for (let x = hit.x; x < hit.x + hit.w; x++) expect(hitAt(laid.hits, x, 23)?.target).toBe(hit.target);
  });
  it.each([0, 1, 2, 8, 14, 20, 80, 160])("clips status text and targets to %i cells", (width) => {
    const chrome = booksChrome(); chrome.status.hints = [hint("👩‍💻 edit", "e", 4)]; chrome.status.counts = [];
    const laid = layoutStatus(chrome, width, 2);
    expect(cellWidth(text(laid.cells))).toBe(width);
    expect(laid.hits.every((h) => h.x >= 0 && h.x + h.w <= width && h.w > 0)).toBe(true);
  });
  it.each([false, true])("retains the whole mandatory status action and hit at width80 (plain=%s)", (plain) => {
    const chrome = booksChrome();
    const extra = { ...hint("extra actions", "x", 1), keys_of: [{ label: "x", chord: { key: { char: "x" } }, target: "chrome:extra" }] };
    chrome.status = {
      scope: "WATCH",
      hints: [hint("refresh all repositories", "r", 0), hint("check every working tree", "c", 0), hint("inspect the selected branch", "i", 0), extra],
      counts: [],
    };
    const laid = layoutStatus(chrome, 80, 23, plain);
    const label = plain ? "x extra actions" : "extra actions";
    const drawn = text(laid.cells);
    expect(drawn).toContain(label);
    expect(drawn).not.toContain("inspect the selected branch");
    expect(drawn.indexOf("refresh all repositories")).toBeLessThan(drawn.indexOf(label));
    const hit = laid.hits.find((h) => h.target === "chrome:extra")!;
    // Status hits retain their existing one-cell padding on both sides.
    expect(hit).toMatchObject({ x: cellWidth(drawn.slice(0, drawn.indexOf(label))) - 1, y: 23, w: cellWidth(label) + 2, h: 1 });
    for (let x = hit.x; x < hit.x + hit.w; x++) expect(hitAt(laid.hits, x, 23)?.target).toBe("chrome:extra");
    expect(hit.x + hit.w).toBeLessThanOrEqual(80);
    expect(underline(laid.cells).includes("x")).toBe(!plain);
  });
  it("protects exact unmodified x input even when the caption is renamed", () => {
    const chrome = booksChrome();
    chrome.status = { scope: "WATCH", hints: [hint("a very long earlier action", "a", 0), { ...hint("expert commands", "x", 1), keys_of: [{ label: "x", chord: { key: { char: "x" } }, target: "chrome:expert" }] }], counts: [] };
    const laid = layoutStatus(chrome, 32, 4);
    expect(text(laid.cells).trimEnd()).toBe(" WATCH   expert commands");
    expect(laid.hits).toContainEqual({ x: 8, y: 4, w: 17, h: 1, target: "chrome:expert" });
  });
  it.each([[40, false], [80, false], [40, true], [80, true]] as const)("matches native mandatory-action fitting at %i columns (plain=%s)", (width, plain) => {
    const chrome = booksChrome();
    chrome.status = {
      scope: "ALPHA",
      hints: [
        { ...hint("use it", "⏎"), keys_of: [{ label: "⏎", chord: { key: "enter" }, target: "chrome:use" }] },
        hint("retry status", "r", 0),
        { ...hint("ask an agent", "A", 0), mnemonic_shift: true },
        hint("bulk", "b", 0),
        { ...hint("extra actions", "x", 1), keys_of: [{ label: "x", chord: { key: { char: "x" } }, target: "chrome:extra" }] },
        hint("notices", "~"),
      ],
      least_needed: [], counts: [], health: [],
    };
    const laid = layoutStatus(chrome, width, 2, plain);
    const shown = text(laid.cells);
    const phrase = plain ? "x extra actions" : "extra actions";
    const at = shown.indexOf(phrase);
    expect(at).toBeGreaterThanOrEqual(0);
    for (let x = at; x < at + phrase.length; x++) expect(hitAt(laid.hits, x, 2)?.target).toBe("chrome:extra");
    expect(underline(laid.cells).includes("x")).toBe(!plain);
    expect(cellWidth(shown)).toBe(width);
  });
  it.each([
    { key: { char: "x" }, ctrl: true },
    { key: { char: "x" }, alt: true },
    { key: { char: "X" } },
  ])("does not reserve a similarly named status hint for another chord: %s", (chord) => {
    const chrome = booksChrome();
    chrome.status = { scope: "WATCH", hints: [hint("refresh the repository", "r", 0), { ...hint("extra actions", "x", 1), keys_of: [{ label: "x", chord, target: "chrome:not-extra" }] }], counts: [] };
    const laid = layoutStatus(chrome, 38, 4);
    expect(text(laid.cells)).toContain("refresh the repository");
    expect(laid.hits.some((h) => h.target === "chrome:not-extra")).toBe(false);
  });
  it.each(["off", "unicode", "nerd", "emoji"] as const)("keeps icon, Shift and action together with %s icons", (pack) => {
    applyLook(defaultLook(pack));
    const h = { ...hint("ask an agent", "A", 0), mnemonic_shift: true, icon: "agent" };
    const cells = hintCells(h);
    expect(text(cells)).toMatch(/shift ask an agent$/);
    expect(underline(cells)).toBe("a");
    const palette = paletteLines({ ...choice("ask an agent", "A", 0), mnemonic_shift: true, icon: "agent" }, true, [], 40, 0);
    expect(text(palette[0]!)).toContain("shift ask an agent");
    expect(underline(palette.flat())).toBe("a");
  });
  it("keeps its mnemonic visible in monochrome without discarding other modifiers", () => {
    const look = defaultLook("off");
    applyLook({ ...look, tier: "mono", slots: Object.fromEntries(Object.keys(look.slots).map((s) => [s, ""])) });
    expect(underline(hintCells(hint("quit", "q", 0)))).toBe("q");
    expect(underline(layoutMenu(menu([choice("refresh", "r", 0)]), 30, 1, 10).lines.flat())).toBe("r");
  });
  it("passes plain mode through chrome layout and retains exact original shortcut keys", () => {
    const chrome = booksChrome();
    chrome.status = { scope: "WATCH", hints: [{ ...hint("ask an agent", "A", 0), mnemonic_shift: true }], counts: [] };
    const view = menu([{ ...choice("ask an agent", "A", 0), mnemonic_shift: true }]);
    const frame = frameOf(chrome, view, 80, 24, true);
    expect(text(frame.status.cells)).toContain("A ask an agent");
    expect(frame.menu!.lines.map(text).join("\n")).toContain("A  ask an agent");
    expect(underline(frame.status.cells)).toBe("");
    expect(underline(frame.menu!.lines.flat())).toBe("");
    expect(chrome.status.hints![0]!.mnemonic_at).toBe(0);
  });
  it("removes only valid letter columns and retains display-only and chord controls", () => {
    const items = [choice("refresh", "r", 0), choice("extra actions", "x", 1), choice("open", "ctrl-o")];
    const laid = layoutMenu(menu(items), 80, 1, 22);
    expect(underline(laid.lines.flat())).toBe("rx");
    expect(laid.lines.map(text).join("\n")).toContain("ctrl-o");
    expect(laid.lines.map(text).join("\n")).not.toContain("r  refresh");
    expect(laid.hits.map((h) => h.target)).toEqual(items.map((i) => i.target));
  });
  it.each([8, 12, 20])("keeps wrapped ordinary action words, underline and targets at %i columns", (width) => {
    const laid = layoutMenu(menu([choice("refresh the watch records", "r", 0)]), width, 1, 40);
    expect(underline(laid.lines.flat())).toBe("r");
    expect(laid.lines.every((line) => cellWidth(text(line)) === laid.w)).toBe(true);
    expect(laid.hits.length).toBeGreaterThan(1);
    expect(laid.hits.every((h) => h.target === "chrome:menu:r" && h.x + h.w <= laid.x + laid.w - 1)).toBe(true);
  });
  it("composes palette search colour and selected shading with the mnemonic underline", () => {
    const c = { ...choice("e\u0301dit report", "e", 0), muted: true };
    const lines = paletteLines(c, true, [0, 1], 18, 0);
    const marked = lines.flat().find((r) => r.style?.underline)!;
    expect(marked.text).toBe("e\u0301");
    expect(marked.style).toMatchObject({ ...slotStyle("accent.primary"), bold: true, underline: true, backgroundColor: bgOf("bg.selection") });
    expect(lines.every((l) => cellWidth(text(l)) === 18)).toBe(true);
  });
  it("paints the actual underline through Ink's ANSI output", () => {
    const cells = hintCells(hint("e\u0301dit", "e", 0));
    const frame = parseAnsi(styledFrameAt(<CellRow cells={cells} />, 30));
    expect(frame.flat().filter((c) => c.underline).map((c) => c.ch).join("")).toBe("e\u0301");
  });
});

describe("question effective words and native-style row packing", () => {
  const questions = () => menu([
    { ...choice("approve", "y", 0), hint: "yes, publish the watch report" },
    { ...choice("cancel", "n", 0), hint: "no, leave it" },
  ], true);
  it.each([18, 24, 80])("wraps complete captions and keeps button continuation padding clickable at %i columns", (width) => {
    const laid = layoutMenu(questions(), width, 1, 40);
    expect(underline(laid.lines.flat())).toBe("yn");
    expect(laid.hits).toHaveLength(2);
    expect(laid.lines.every((l) => cellWidth(text(l)) === laid.w)).toBe(true);
    for (const hit of laid.hits) {
      expect(hit.x + hit.w).toBeLessThanOrEqual(laid.x + laid.w - 1);
      expect(hit.y + hit.h).toBeLessThanOrEqual(laid.y + laid.h - 1);
      for (let y = hit.y; y < hit.y + hit.h; y++) for (let x = hit.x; x < hit.x + hit.w; x++) expect(hitAt(laid.hits, x, y)?.target).toBe(hit.target);
    }
    if (width === 18) expect(laid.hits[0]!.h).toBeGreaterThan(3);
    if (width < 80) expect(laid.hits[1]!.y).toBeGreaterThan(laid.hits[0]!.y);
  });
  it("matches native trim:false padding at a three-cell question face", () => {
    expect(paragraphCells(cellsOf(" yes "), 3).map((l) => l.map((c) => c.ch).join(""))).toEqual([" ye", "s "]);
    const laid = layoutMenu({ ...menu([choice("yes", "y", 0)], true), title: "" }, 5, 1, 20);
    expect(laid.hits).toHaveLength(1);
    expect(laid.hits[0]).toMatchObject({ w: 3, h: 4 });
    const hit = laid.hits[0]!;
    expect(hitAt(laid.hits, hit.x + 2, hit.y + 2)?.target).toBe(hit.target);
  });
  it("drops bevels before giving up, and hides all controls when even flat captions cannot fit", () => {
    const short = menu([choice("yes", "y", 0), choice("no", "n", 0)], true);
    const flat = layoutMenu(short, 80, 1, 3);
    expect(flat.hits).toHaveLength(2);
    expect(flat.hits.every((h) => h.h === 1)).toBe(true);
    expect(flat.lines.map(text).join("")).not.toMatch(/[▔▁]/);
    const hidden = layoutMenu(questions(), 18, 1, 4);
    expect(hidden.hits).toEqual([]);
    expect(hidden.h).toBe(4);
  });
});

describe("rich text and fallback action controls", () => {
  it.each([">", "⏎", "esc", "1", "ctrl-r", "alt-r", "↑↓"])("places nonletter rich key %s before its original styled word", (key) => {
    const source: Text = [{ text: "move WATCH-12", look: "good italic", marked: true }];
    const block = { text: source, key, mnemonic_at: 0, mnemonic_shift: true };
    for (const plain of [false, true]) {
      const rows = lineRows(block, 80, 80, plain);
      expect(rows.flat().map((cell) => cell.ch).join("")).toBe(`${key} move WATCH-12`);
      expect(rows.flat().some((cell) => cell.s.underline)).toBe(false);
      if (!plain) {
        expect(rows[0]![0]!.s).toMatchObject({ ...slotStyle("accent.secondary"), bold: true });
        expect(rows[0]!.find((cell) => cell.ch === "m")!.s).toMatchObject({ italic: true, inverse: true });
      }
    }
  });
  it("wraps a prefixed rich glyph and caption with native Paragraph geometry", () => {
    const block = { text: [{ text: "move" }], key: ">" };
    const rows = lineRows(block, 3, 3);
    expect(rows.map((row) => row.map((cell) => cell.ch).join(""))).toEqual([">", "mov", "e"]);
    expect(rows.every((row) => widthOfCells(row) <= 3)).toBe(true);
  });
  it.each(["off", "nerd"])("counts hidden source fallbacks while painting actual icon widths with icons %s", (pack) => {
    applyLook(defaultLook(pack));
    const source: Text = [{ icon: "open", text: "open " }, { text: "e", look: "accent.primary bold", link: "https://example.org/watch" }, { text: "\u0301dit", look: "text.secondary italic", marked: true }];
    const shown = mnemonicLineCells(source, "e", { mnemonic_at: 5 });
    expect(shown.valid).toBe(true);
    expect(shown.cells.filter((c) => c.s.underline).map((c) => c.ch).join("")).toBe("e\u0301");
    expect(shown.cells.find((c) => c.s.underline)?.s).toMatchObject({ bold: true, italic: true, underline: true });
    expect(shown.cells.find((c) => c.ch === "d")?.s.inverse).toBe(true);
    expect(mnemonicLineCells(source, "o", { mnemonic_at: 0 }).valid).toBe(false);
  });
  it("keeps keyed source captions literal when they contain an emoji shortcode", () => {
    const source: Text = [{ text: ":tada: edit", link: "https://example.org/watch" }];
    const shown = mnemonicLineCells(source, "e", { mnemonic_at: 7 });
    expect(shown.cells.map((c) => c.ch).join("")).toBe(":tada: edit");
    expect(shown.cells.filter((c) => c.s.underline).map((c) => c.ch).join("")).toBe("e");
    const plain = lineRows({ text: source, key: "e", mnemonic_at: 7 }, 80, 80, true);
    expect(plain.flat().map((c) => c.ch).join("")).toContain("<https://example.org/watch>");
    expect(plain.flat().map((c) => c.ch).join("")).toContain("[e]");
  });
  it("coalesces joined emoji split over source styles before measuring later letters", () => {
    const source: Text = [{ text: "👩", look: "accent.primary" }, { text: "‍", look: "text.secondary italic" }, { text: "💻 edit", look: "text.secondary" }];
    const shown = mnemonicLineCells(source, "e", { mnemonic_at: 4 });
    expect(shown.cells[0]).toMatchObject({ ch: "👩‍💻", w: 2 });
    expect(widthOfCells(shown.cells)).toBe(7);
    expect(shown.cells.filter((c) => c.s.underline).map((c) => c.ch).join("")).toBe("e");
  });
  it("underlines eligible chip text without styling the icon or padding", () => {
    applyLook(defaultLook("nerd"));
    const shown = mnemonicLineCells([{ icon: "open", chip: true, text: "open", ink: "good" }], "o", { mnemonic_at: 0 });
    expect(shown.valid).toBe(true);
    expect(shown.cells.filter((c) => c.s.underline).map((c) => c.ch).join("")).toBe("o");
  });
  it("keeps glyph clusters whole and exposes legacy and plain-text keys even when wrapping", () => {
    const source = [{ text: "👩‍💻 edit report", look: "accent.primary bold" }];
    const rows = lineRows({ text: source, key: "e", mnemonic_at: 4 }, 8, 8);
    expect(rows.flat().filter((c) => c.s.underline).map((c) => c.ch).join("")).toBe("e");
    expect(rows.every((r) => widthOfCells(r) <= 8)).toBe(true);
    expect(rows.flat().map((c) => c.ch).join("")).not.toContain("[e]");
    for (const plain of [false, true]) {
      const fallback = lineRows({ text: source, key: "x", mnemonic_at: 4 }, 8, 8, plain);
      expect(fallback.flat().map((c) => c.ch).join("")).toContain("[x]");
    }
  });
  it("renders action mnemonics without changing invoke IDs and preserves explicit plain shortcuts", () => {
    const action: ActionSpec = { id: "watch.refresh", label: "refresh", key: { key: { char: "r" } }, mnemonic_at: 0 };
    expect(invokeIntent(action)).toEqual({ intent: "invoke", action: "watch.refresh" });
    expect(frameAt(<ActionList actions={[action]} />, 40)).toBe("  refresh");
    expect(frameAt(<ActionList actions={[action]} plain />, 40)).toBe("  r  refresh");
    const rendered = parseAnsi(styledFrameAt(<ActionList actions={[action]} />, 40));
    expect(rendered.flat().filter((c) => c.underline).map((c) => c.ch).join("")).toBe("r");
  });
  it.each([
    [{ ctrl: true }, "ctrl-r"],
    [{ alt: true }, "alt-r"],
    [{ ctrl: true, alt: true }, "ctrl-alt-r"],
  ] as const)("retains complete modifier labels despite stale action mnemonic metadata: %s", (modifiers, key) => {
    const action: ActionSpec = { id: "watch.refresh", label: "refresh", key: { key: { char: "r" }, ...modifiers }, mnemonic_at: 0, mnemonic_shift: true };
    for (const plain of [false, true]) {
      expect(frameAt(<ActionList actions={[action]} plain={plain} />, 40)).toBe(`  ${key}  refresh`);
      const rendered = parseAnsi(styledFrameAt(<ActionList actions={[action]} plain={plain} />, 40));
      expect(rendered.flat().some((c) => c.underline)).toBe(false);
    }
  });
});

describe("plain host-styled text keeps exact original keys", () => {
  it("appends only the host-supplied key and leaves legacy text unchanged", () => {
    const source: Text = [{ text: "shift " }, { text: "a", look: "accent.secondary underline" }, { text: "sk an agent", key_hint: "A" }];
    expect(textLineString(source)).toBe("shift ask an agent");
    expect(textLineString(source, true)).toBe("shift ask an agent [A]");
    expect(frameAt(<TextLine text={source} plain />, 60)).toBe("shift ask an agent [A]");
    expect(textLineString([{ text: "old help line" }], true)).toBe("old help line");
    const card = { stop: 1, of: 2, title: "keys", lines: [source], keys: [{ text: "exit", key_hint: "x" }] };
    const laid = layoutTour(card, 80, 24, true)!;
    expect(laid.w).toBeGreaterThanOrEqual(cellWidth("shift ask an agent [A]") + 4);
  });
  it("retains a long tour caption's original shortcut at width40 instead of clipping it", () => {
    const source = [{ text: "extra actions open the complete list of available commands", key_hint: "x" }];
    const card = { stop: 1, of: 2, title: "keys", lines: [source], keys: [{ text: "esc leave" }] };
    const laid = layoutTour(card, 40, 24, true)!;
    const line = plainTourText(source, laid.w - 4);
    expect(line).toMatch(/… \[x\]$/);
    expect(cellWidth(line)).toBe(laid.w - 4);
  });
  it("keeps every tour shortcut next to its own caption on multi-key lines", () => {
    const source = [{ text: "refresh", key_hint: "r" }, { text: " · open", key_hint: "o" }, { text: " · copy link", key_hint: "y" }];
    expect(plainTourText(source, 80)).toBe("refresh [r] · open [o] · copy link [y]");
    expect(plainTourText(source, 24)).toBe("refresh [r] · open [o]…");
    expect(plainTourText(source, 18)).toBe("refresh [r]…");
    expect(plainTourText(source, 20)).toBe("refresh [r] · o… [o]");
    expect(plainTourText(source, 18)).not.toContain("[y]");
  });
  it.each([
    [{ ctrl: true }, "ctrl-r"],
    [{ alt: true }, "alt-r"],
    [{ ctrl: true, alt: true }, "ctrl-alt-r"],
  ] as const)("preserves all chord modifiers in plain fallback rail actions: %s", (modifiers, key) => {
    const node = { node: "extension", capability: "work.rail", data: { actions: [{ label: "refresh", key: { key: { char: "r" }, ...modifiers }, mnemonic_at: 0 }] } };
    expect(plainLines(node as never)).toEqual([`[${key}] refresh`]);
  });
});

describe("History supplied action targets", () => {
  const history = (scroll = 0): HistoryView => ({
    version: 1, width: 22, height: 12, title: "History", boundary: "saved", scope: "all", notes: [], focus: "detail", split: false,
    events_title: "events", columns: [], rows: [], list_scroll: 0, state: "", detail_title: "detail",
    detail: [[{ text: "causal events", target: "chrome:history:key:c", look: "accent.secondary underline" }, { text: " · " }, { text: "action events", target: "chrome:history:key:i" }]],
    detail_keys_line: 0, detail_keys: [{ label: "c", chord: { key: { char: "c" } }, target: "chrome:history:key:c" }, { label: "i", chord: { key: { char: "i" } }, target: "chrome:history:key:i" }],
    detail_scroll: scroll, help_title: "help", help_lines: [], help_scroll: 0, footer: [hint("refresh", "r", 0)],
  } as HistoryView);
  it("uses explicit key hints in plain History detail and help", () => {
    const view = history();
    view.detail = [[{ text: "causal events", key_hint: "c", target: "chrome:history:key:c" }]];
    let laid = layoutHistory(view, 50, 12, 1, true);
    expect(laid.lines.map(text).join("\n")).toContain("causal events [c]");
    view.help = true;
    view.help_lines = [[{ text: "shift ask an agent", key_hint: "A" }]];
    laid = layoutHistory(view, 50, 12, 1, true);
    expect(laid.lines.map(text).join("\n")).toContain("shift ask an agent [A]");
  });
  it("drops low-priority whole hints before help and close, retaining Escape last", () => {
    const hints = [hint("move", "↑↓"), hint("details", "enter"), hint("next page", "n", 0), hint("previous page", "p", 0), hint("refresh", "r", 0), hint("extra keys", "x", 1), hint("close", "esc")];
    const full = historyFooter(hints, 160);
    expect(full).toEqual(hints);
    const narrow = historyFooter(hints, 24);
    expect(narrow.map((h) => h.label)).toEqual(["extra keys", "close"]);
    expect(historyFooter(hints, 10).map((h) => h.keys)).toEqual(["esc"]);
    expect(historyFooter(hints, 4)).toEqual([]);
  });
  it("retains original footer letters in plain mode", () => {
    const laid = layoutHistory(history(), 30, 12, 1, true);
    expect(text(laid.lines.at(-1)!)).toContain("r refresh");
    expect(underline(laid.lines.at(-1)!)).toBe("");
  });
  it("keeps targets through wrapping without searching for displayed key letters", () => {
    const laid = layoutHistory(history(), 22, 12, 1);
    const actionHits = laid.hits.filter((h) => h.target.includes("history:key:"));
    expect(new Set(actionHits.map((h) => h.target))).toEqual(new Set(["chrome:history:key:c", "chrome:history:key:i"]));
    expect(actionHits.filter((h) => h.target.endsWith(":i")).length).toBeGreaterThan(0);
    for (const hit of actionHits) expect(hitAt(laid.hits, hit.x, hit.y)?.target).toBe(hit.target);
    expect(underline(laid.lines.at(-1)!)).toBe("r");
  });
  it("rejects unknown targets and never leaves offscreen targets after clipping", () => {
    const view = history(); view.detail[0]!.push({ text: " unexpected", target: "chrome:history:key:unexpected" });
    const laid = layoutHistory(view, 10, 6, 1);
    expect(laid.hits.some((h) => h.target.endsWith(":unexpected"))).toBe(false);
    expect(laid.hits.every((h) => h.x >= 0 && h.x + h.w <= 10 && h.y >= 1 && h.y + h.h <= 7)).toBe(true);
  });
});
