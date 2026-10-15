import React from "react";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { render, litTitles, realNodes } from "./render.js";
import { WorkArea } from "../../src/workarea/WorkArea.js";
import { arrange } from "../../src/workarea/layout.js";
import { sectionsOf, type ScreenLike } from "../../src/workarea/model.js";

// The Work screen as radar-workbench emits it (recorded by the Rust side, T14).
const fixture = fileURLToPath(new URL("../../../crates/radar-workbench/tests/fixtures/work_view.json", import.meta.url));
const screen = JSON.parse(readFileSync(fixture, "utf8")) as ScreenLike;

const caps = (columns: number, rows: number, plain_text = false) => ({ columns, rows, plain_text });
const frame = (columns: number, rows: number, plain = false, focus?: string) => {
  const r = render(<WorkArea screen={screen} capabilities={caps(columns, rows, plain)} focus={focus} renderNode={realNodes} />);
  const f = r.lastFrame() ?? "";
  r.unmount();
  // Each recording mints fresh item ids; keep the snapshots stable across re-recordings.
  return f.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g, "<uuid>");
};

describe("work area from the recorded real Work view", () => {
  it("finds map, timeline, tabs and rail, plus the nav, middle and detail panes as other", () => {
    const s = sectionsOf(screen);
    expect(s.map((x) => x.kind)).toEqual(["other", "other", "other", "map", "timeline", "tabs", "rail"]);
    expect(s.filter((x) => x.kind !== "other").map((x) => x.target)).toEqual([
      "work-area:map",
      "work-area:timeline",
      "work-area:tabs",
      "work-area:rail",
    ]);
    expect(new Set(s.map((x) => x.key)).size).toBe(s.length);
    expect(new Set(s.map((x) => x.target)).size).toBe(s.length);
    expect(s.find((x) => x.kind === "timeline")!.status).toBe("complete");
    expect(s.some((x) => x.pending)).toBe(false);
  });

  it("160x40: columns side by side", () => {
    const f = frame(160, 40, false, "work-area:tabs");
    expect(f).toMatchSnapshot();
    const lit = render(<WorkArea screen={screen} capabilities={caps(160, 40)} focus="work-area:tabs" />);
    expect(litTitles(lit.raw()).has("tabs")).toBe(true);
    expect(litTitles(lit.raw()).has("timeline")).toBe(false);
    lit.unmount();
    for (const t of ["map", "timeline", "tabs", "actions", "Ship the work area"]) expect(f).toContain(t);
  });

  it("80x24: one column in reading order, nothing dropped", () => {
    const f = frame(80, 24);
    expect(f).toMatchSnapshot();
    const order = arrange(sectionsOf(screen), 80).columns[0]!.sections.map((x) => x.kind);
    expect(order).toEqual(["rail", "tabs", "timeline", "map", "other", "other", "other"]);
    for (const t of ["work items · page 1", "Ship the work area"]) expect(f).toContain(t);
  });

  it("plain-text mode keeps labels, order and status", () => {
    const f = frame(80, 24, true, "work-area:rail");
    expect(f).toMatchSnapshot();
    const lines = f.split("\n");
    expect(lines).toContain("[timeline] timeline (complete)");
    expect(lines).toContain("[rail] actions (focused)");
  });
});
