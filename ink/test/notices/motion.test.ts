import { afterEach, describe, expect, it, vi } from "vitest";
import { Motion } from "../../src/motion/motion.js";
import * as frames from "../../src/motion/frame.js";
import { paintNoticeRows } from "../../src/notices/paint.js";
import { layoutNotices } from "../../src/notices/layout.js";
import { notice } from "./fixture.js";
import { applyLook, defaultLook } from "../../src/text/look.js";
afterEach(() => { vi.restoreAllMocks(); vi.useRealTimers(); applyLook(defaultLook()); });
const clean = Array.from({ length: 8 }, () => "clean base frame".padEnd(40)).join("\n");
function layer(m: Motion, now: () => number) {
  let visible = true, clears = 0;
  const rows = () => visible ? layoutNotices([notice(1, { text: "Notice after effects" })], 0, { x: 1, y: 1, w: 38, h: 5 }, now(), m.engine.level !== "off") : [];
  m.setPresentation({ visible: () => rows().length > 0, running: () => m.engine.level !== "off" && rows().length > 0, draw: (grid) => paintNoticeRows(grid, rows()), clear: () => { clears++; } });
  return { hide: () => { visible = false; m.refreshPresentation(); }, clears: () => clears };
}
describe("late notice composition and bounded scheduling", () => {
  it("paints after fold, legacy and focus while retention sees only the clean source", () => {
    vi.useFakeTimers(); const m = new Motion("full", () => 2500), order: string[] = [];
    vi.spyOn(m.interactions, "observe").mockImplementation((output) => { expect(output).toBe(clean); order.push("retain"); }); vi.spyOn(m.engine, "running").mockReturnValue(true);
    vi.spyOn(m.interactions, "draw").mockImplementation((_g, which) => { order.push(which); return []; }); vi.spyOn(m.engine, "draw").mockImplementation(() => { order.push("legacy"); return []; });
    m.setPresentation({ visible: () => true, running: () => true, clear() {}, draw: (grid) => { order.push("notice"); return paintNoticeRows(grid, layoutNotices([notice(1)], 0, { x: 1, y: 1, w: 38, h: 5 }, 2500, true)); } });
    expect(m.apply(clean, 40, 8).replace(/\x1b\[[0-9;]*m/g, "")).toContain("Watch updated"); expect(order).toEqual(["retain", "fold", "legacy", "focus", "notice"]); m.reset();
  });
  it("reuses one30ms timer only during visible draining, with no idle/hidden/teardown timer", () => {
    vi.useFakeTimers(); let now = 0; const m = new Motion("full", () => now); expect(vi.getTimerCount()).toBe(0); const l = layer(m, () => now); expect(vi.getTimerCount()).toBe(1); m.apply(clean, 40, 8); expect(vi.getTimerCount()).toBe(1);
    now = 5000; m.apply(clean, 40, 8); expect(vi.getTimerCount()).toBe(0); expect(m.apply(clean, 40, 8)).toBe(clean); l.hide(); expect(vi.getTimerCount()).toBe(0); m.reset(); expect(vi.getTimerCount()).toBe(0);
  });
  it("motion-off has static ground and no animation timer, but still expires", () => {
    vi.useFakeTimers(); let now = 0; const m = new Motion("off", () => now); layer(m, () => now); const first = m.apply(clean, 40, 8); now = 2500; expect(m.apply(clean, 40, 8)).toBe(first); expect(vi.getTimerCount()).toBe(0); now = 5000; expect(m.apply(clean, 40, 8)).toBe(clean); m.reset();
  });
  it("fails closed before parsing oversized/invalid frames and cannot rearm until valid", () => {
    vi.useFakeTimers(); const m = new Motion("full", () => 200), l = layer(m, () => 200), parse = vi.spyOn(frames, "parseFrame"); vi.spyOn(m.engine, "running").mockReturnValue(true);
    for (const [output, width, height] of [[clean, 1000, 1000], [clean, -1, 2], [clean, 1.5, 2], ["x".repeat(1_048_577), 40, 8]] as const) {
      expect(m.apply(output, width, height)).toBe(output); expect(m.active).toBe(false); expect(vi.getTimerCount()).toBe(0); m.refreshPresentation(); expect(vi.getTimerCount()).toBe(0);
    }
    expect(parse).not.toHaveBeenCalled(); expect(l.clears()).toBeGreaterThan(0); expect(m.apply(clean, 40, 8)).toContain("Notice after effects"); expect(parse).toHaveBeenCalledTimes(1); expect(vi.getTimerCount()).toBe(1); m.reset();
  });
});
