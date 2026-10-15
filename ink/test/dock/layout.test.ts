import { describe, expect, it } from "vitest";
import { ABOVE_MIN_ROWS, DOCK_MIN_ROWS, dragHeight, splitDock } from "../../src/dock/layout.js";

describe("splitDock", () => {
  it("takes about 40% of the middle column", () => {
    expect(splitDock(30, 160, true)).toEqual({ full: false, dock: 12, above: 18 });
  });
  it("uses the height the user dragged to, within bounds", () => {
    expect(splitDock(30, 160, true, 20).dock).toBe(20);
    expect(splitDock(30, 160, true, 99).dock).toBe(30 - ABOVE_MIN_ROWS);
    expect(splitDock(30, 160, true, 2).dock).toBe(DOCK_MIN_ROWS);
  });
  it("opens full height below 100 columns", () => {
    expect(splitDock(20, 80, true)).toEqual({ full: true, dock: 20, above: 0 });
    expect(splitDock(20, 99, true).full).toBe(true);
    expect(splitDock(20, 100, true).full).toBe(false);
  });
  it("opens full height when the column is too short to share", () => {
    expect(splitDock(10, 160, true).full).toBe(true);
  });
  it("draws nothing while hidden", () => {
    expect(splitDock(30, 160, false)).toEqual({ full: false, dock: 0, above: 30 });
  });
  it("shares 80x24's body and 160x40's body", () => {
    expect(splitDock(21, 80, true).full).toBe(true);
    const s = splitDock(37, 160, true);
    expect(s.dock + s.above).toBe(37);
  });
});

describe("dragHeight", () => {
  it("is the rows from the edge to the column's foot, clamped", () => {
    expect(dragHeight(2, 30, 20)).toBe(12);
    expect(dragHeight(2, 30, 3)).toBe(30 - ABOVE_MIN_ROWS);
    expect(dragHeight(2, 30, 31)).toBe(DOCK_MIN_ROWS);
  });
});
