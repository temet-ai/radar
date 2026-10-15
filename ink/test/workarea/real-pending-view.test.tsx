import React from "react";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { render, realNodes } from "./render.js";
import { WorkArea } from "../../src/workarea/WorkArea.js";
import { arrange } from "../../src/workarea/layout.js";
import { sectionsOf, type ScreenLike } from "../../src/workarea/model.js";

// The Work screen with a prepared change waiting for confirmation, recorded by the Rust side (T14).
const fixture = fileURLToPath(
  new URL("../../../crates/radar-workbench/tests/fixtures/work_view_pending.json", import.meta.url),
);
const screen = JSON.parse(readFileSync(fixture, "utf8")) as ScreenLike;

const caps = (columns: number, rows: number, plain_text = false) => ({ columns, rows, plain_text });
const frame = (columns: number, rows: number, plain = false) => {
  const r = render(<WorkArea screen={screen} capabilities={caps(columns, rows, plain)} renderNode={realNodes} />);
  const f = r.lastFrame() ?? "";
  r.unmount();
  // Each recording mints fresh item ids; keep the snapshots stable across re-recordings.
  return f.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g, "<uuid>");
};

describe("work area from the recorded real Work view with a pending question", () => {
  it("finds the question as the one pending agent section", () => {
    const s = sectionsOf(screen);
    const pending = s.filter((x) => x.pending);
    expect(pending.map((x) => [x.kind, x.title, x.status, x.target])).toEqual([
      ["agent", "question", "awaiting confirmation", "work-area:agent"],
    ]);
  });

  it("80x24: the question comes first, labelled, and nothing is hidden", () => {
    const f = frame(80, 24);
    expect(f).toMatchSnapshot();
    const lines = f.split("\n");
    expect(lines[0]).toContain("question");
    expect(lines[1]).toContain("waiting for you");
    expect(lines[2]).toContain("Waiting for your confirmation: apply the prepared local change?");
    expect(arrange(sectionsOf(screen), 80).columns[0]!.sections[0]!.kind).toBe("agent");
    // Under 84 columns Ratatui shows the focused pane alone; only the question is added to it.
    expect(f).toContain("╭ work items · page 1");
    for (const t of ["actions", "tabs", "timeline", "map"]) expect(f).not.toContain(`╭ ${t}`);
  });

  it("160x40: the question is drawn in its column beside the others", () => {
    const f = frame(160, 40);
    expect(f).toMatchSnapshot();
    expect(f).toContain("waiting for you");
    expect(f).toContain("Waiting for your confirmation: apply the prepared local change?");
  });

  it("plain text: PENDING first with its status", () => {
    const f = frame(80, 24, true);
    expect(f).toMatchSnapshot();
    const lines = f.split("\n");
    expect(lines[0]).toBe("[agent] question (PENDING, awaiting confirmation)");
    expect(lines[1]).toBe("  Waiting for your confirmation: apply the prepared local change?");
  });
});
