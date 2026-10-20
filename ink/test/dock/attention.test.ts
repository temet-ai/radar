import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { activityCells, duration, spinAt, tabRowCells, tabSpans, tabState, tokenWords } from "../../src/dock/rows.js";
import { splitDock } from "../../src/dock/layout.js";
import { layoutStatus, SPIN, chromeInk, type Cell } from "../../src/chrome/layout.js";
import { applyLook, defaultLook, iconOf, slotStyle } from "../../src/text/look.js";
import { cellWidth } from "../../src/chrome/width.js";
import type { DockActivity, DockTab } from "../../src/dock/model.js";
import type { ChromeView } from "../../src/chrome/types.js";

// The dock's attention states: the activity line above the composer, each tab's state mark, the full-height split, and
// the status line's chip while a session wants you out of sight (compared with the row Ratatui draws from the same
// host data, recorded beside it by radar-core's host_dock_tests).

const text = (cs: { ch: string }[]) => cs.map((c) => c.ch).join("");
const at = (a: Partial<DockActivity>): DockActivity => ({ doing: "thinking", elapsed: 0, ...a });
const DOINGS: DockActivity["doing"][] = ["thinking", "writing", "running", "approval", "question", "done", "stopped", "failed"];

describe("the activity line", () => {
  it.each([40, 78, 158])("says what the turn does, for how long and its tokens, in one row of %i cells", (w) => {
    for (const doing of DOINGS) {
      const row = activityCells(at({ doing, detail: doing === "running" ? "cargo test -p radar-core" : doing === "approval" ? "git push origin main" : "", elapsed: 72, tokens: 3400 }), w);
      expect(cellWidth(text(row)), doing).toBe(w);
      // The time is never cut; the words before it are.
      expect(text(row), doing).toContain("1m 12s");
    }
  });
  it("names the doing in plain words, with the command or the target", () => {
    const line = (a: Partial<DockActivity>) => text(activityCells(at(a), 120)).trimEnd();
    expect(line({ doing: "thinking", elapsed: 3 })).toBe(` ${spinAt(3)} thinking · 3s`);
    expect(line({ doing: "writing", elapsed: 14, tokens: 3400 })).toBe(` ${spinAt(14)} writing · 14s · 3.4k tokens`);
    // A message sent while the turn runs waits for after it.
    expect(line({ doing: "writing", elapsed: 14, queued: 2 })).toBe(` ${spinAt(14)} writing · 14s · 2 queued`);
    expect(line({ doing: "running", detail: "git diff", elapsed: 7 })).toBe(` ${spinAt(7)} running git diff · 7s`);
    expect(line({ doing: "approval", detail: "git push", elapsed: 65 })).toBe(` ${iconOf("warning")!.glyph} waiting for your approval · git push · 1m 05s`);
    expect(line({ doing: "question", elapsed: 40 })).toBe(` ${iconOf("warning")!.glyph} waiting for your answer · 40s`);
    expect(line({ doing: "done", elapsed: 42, tokens: 3400 })).toBe(` ${iconOf("run_passed")!.glyph} done in 42s · 3.4k tokens`);
    expect(line({ doing: "stopped", elapsed: 31 })).toBe(` ${iconOf("run_canceled")!.glyph} stopped after 31s`);
    expect(line({ doing: "failed", elapsed: 12 })).toBe(` ${iconOf("run_failed")!.glyph} failed after 12s`);
  });
  it("cuts the command first in a narrow dock", () => {
    const row = text(activityCells(at({ doing: "running", detail: "cargo nextest run -p radar-core -p radar-sessions", elapsed: 5 }), 30));
    expect(row).toMatch(/running cargo.*… · 5s/);
  });
  it("draws in theme slots: the spinner info, a wait warning, an end muted", () => {
    const cell = (a: Partial<DockActivity>, ch: string) => activityCells(at(a), 80).find((c) => c.ch === ch);
    expect(cell({ doing: "writing", elapsed: 1 }, spinAt(1))?.s.color).toBe(slotStyle("status.info").color);
    expect(cell({ doing: "approval" }, "w")?.s.color).toBe(slotStyle("status.warning").color);
    expect(cell({ doing: "done", elapsed: 2 }, "d")?.s.color).toBe(slotStyle("text.muted").color);
  });
  it("words durations and tokens", () => {
    expect([0, 59, 60, 72, 3599, 3600, 3725].map(duration)).toEqual(["0s", "59s", "1m 00s", "1m 12s", "59m 59s", "1h 00m", "1h 02m"]);
    expect([640, 1000, 3400, 12_345, 999_999, 1_250_000].map(tokenWords)).toEqual(["640 tokens", "1k tokens", "3.4k tokens", "12k tokens", "1M tokens", "1.3M tokens"]);
  });
});

describe("tab states", () => {
  const tab = (t: Partial<DockTab>): DockTab => ({ id: "s", title: "review", state: "idle", ...t });
  it("a running tab spins with its turn's seconds, in the kit's frames", () => {
    expect(SPIN).toContain(tabState(tab({ state: "streaming", running: 4 }))!.glyph);
    expect(tabState(tab({ state: "streaming", running: 4 }))!.glyph).not.toBe(tabState(tab({ state: "streaming", running: 5 }))!.glyph);
  });
  it("needs you, failed, done unread and seen each look their own; a seen idle tab shows nothing", () => {
    expect(tabState(tab({ state: "approval", unread: true }))).toEqual({ glyph: iconOf("warning")!.glyph, slot: "status.warning" });
    expect(tabState(tab({ state: "error", unread: true }))).toEqual({ glyph: iconOf("run_failed")!.glyph, slot: "status.error" });
    expect(tabState(tab({ unread: true }))).toEqual({ glyph: iconOf("unread")!.glyph, slot: "accent.primary" });
    expect(tabState(tab({ state: "stopped", unread: true }))!.slot).toBe("accent.primary");
    expect(tabState(tab({}))).toBeUndefined();
  });
  it("the mark keeps its own slot inside a bold accent tab, the name is cut before it, and the pointer spans agree", () => {
    const tabs = [tab({ id: "a", title: "a very long session name indeed", unread: true }), tab({ id: "b", title: "codex", state: "approval" })];
    for (const w of [40, 78, 158]) {
      const row = tabRowCells(tabs, 0, w);
      expect(cellWidth(text(row))).toBe(w);
      const unread = row.find((c) => c.ch === iconOf("unread")!.glyph)!;
      expect(unread.s.color).toBe(slotStyle("accent.primary").color);
      const warn = row.find((c) => c.ch === iconOf("warning")!.glyph)!;
      expect(warn.s.color).toBe(slotStyle("status.warning").color);
      // Names, not counts: no digit is added to a tab.
      expect(text(row)).not.toMatch(/\d/);
      const spans = tabSpans(tabs, w, 0);
      const s = text(row);
      expect(s.slice(spans[1]!.x, spans[1]!.x + spans[1]!.w)).toContain("codex");
    }
  });
});

describe("full height", () => {
  it("takes the whole middle column at every width, and hidden takes none", () => {
    for (const cols of [80, 120, 160]) expect(splitDock(30, cols, true, 9, true)).toEqual({ full: true, dock: 30, above: 0 });
    expect(splitDock(30, 160, false, 9, true)).toEqual({ full: false, dock: 0, above: 30 });
  });
});

describe("the status line's attention chip", () => {
  const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures/dock/status");
  const recorded = fs.existsSync(dir) ? fs.readdirSync(dir).filter((f) => f.endsWith(".json")).sort() : [];
  const line = (cells: Cell[]) => cells.map((c) => c.text).join("");
  it("is recorded at both sizes", () => expect(recorded).toEqual(["attention_160x40.json", "attention_80x24.json"]));
  it.each(recorded)("%s: Ink draws the row Ratatui draws, the chip square on its state's ground, one pointer target", (f) => {
    const { chrome, ratatui } = JSON.parse(fs.readFileSync(path.join(dir, f), "utf8")) as { chrome: ChromeView; ratatui: string };
    const w = chrome.width;
    // The host's test theme draws with icons off: so does Ink here, then with the pack's glyph.
    try {
      applyLook(defaultLook("off"));
      expect(line(layoutStatus(chrome, w, chrome.height - 1).cells).trimEnd()).toBe(ratatui.trimEnd());
    } finally {
      applyLook(defaultLook());
    }
    const laid = layoutStatus(chrome, w, chrome.height - 1);
    const a = chrome.status.attention!;
    const chip = laid.cells.find((c) => c.text.includes(a.text))!;
    expect(chip.text).toBe(` ${iconOf(a.icon)!.glyph} ${a.text} `);
    expect(chip.style?.backgroundColor).toBe(chromeInk(a.ink).color);
    expect(chip.style?.color).toBe(slotStyle("bg.base").color);
    const key = laid.cells.find((c) => c.text === a.keys)!;
    expect(key.style?.color).toBe(slotStyle("accent.secondary").color);
    const hit = laid.hits.find((h) => h.target === a.target)!;
    expect(hit.w).toBe(cellWidth(`${a.keys}  ${iconOf(a.icon)!.glyph} ${a.text} `));
  });
  it("never gives way: at 50 cells the keys go first", () => {
    const { chrome } = JSON.parse(fs.readFileSync(path.join(dir, recorded[1]!), "utf8")) as { chrome: ChromeView };
    const s = line(layoutStatus(chrome, 50, 23).cells);
    expect(s).toContain(chrome.status.attention!.text);
    expect(cellWidth(s)).toBeLessThanOrEqual(50);
  });
});
