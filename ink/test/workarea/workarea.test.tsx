import React from "react";
import { describe, expect, it } from "vitest";
import { render, litTitles } from "./render.js";
import { WorkArea } from "../../src/workarea/WorkArea.js";
import { arrange, resize, columnWidths } from "../../src/workarea/layout.js";
import { sectionsOf } from "../../src/workarea/model.js";
import { workScreen } from "./fixture.js";

const caps = (columns: number, rows: number, plain_text = false) => ({ columns, rows, plain_text });
const frame = (el: React.ReactElement) => {
  const r = render(el);
  const f = r.lastFrame() ?? "";
  r.unmount();
  return f;
};
const raw = (el: React.ReactElement) => {
  const r = render(el);
  const f = r.raw();
  r.unmount();
  return f;
};

describe("work area layout", () => {
  it("wide: map and timeline, tabs, agent and rail side by side (160x40)", () => {
    const f = frame(<WorkArea screen={workScreen()} capabilities={caps(160, 40)} focus="pane:tabs" />);
    expect(f).toMatchSnapshot();
    const first = f.split("\n")[0]!;
    expect(first.indexOf("Map")).toBeLessThan(first.indexOf("Overview"));
    expect(first.indexOf("Overview")).toBeLessThan(first.indexOf("Agent"));
    expect(litTitles(raw(<WorkArea screen={workScreen()} capabilities={caps(160, 40)} focus="pane:tabs" />)).has("Overview")).toBe(true);
  });

  it("under 84 columns only the pending question is added to Ratatui's focused pane", () => {
    const f = frame(<WorkArea screen={workScreen({ pending: true })} capabilities={caps(80, 24)} />);
    expect(f.split("\n")[0]).toContain("Agent");
    for (const t of ["Map", "Timeline", "Overview", "Next"]) expect(f).not.toContain(`╭ ${t}`);
  });

  it("90 columns: one column, pending question first, nothing hidden", () => {
    const f = frame(<WorkArea screen={workScreen({ pending: true })} capabilities={caps(90, 30)} />);
    expect(f).toMatchSnapshot();
    const lines = f.split("\n");
    expect(lines[0]).toContain("Agent");
    expect(lines[1]).toContain("waiting for you");
    expect(lines[2]).toContain("Approve push to origin?");
    for (const t of ["Map", "Timeline", "Overview", "Next"]) expect(f).toContain(t);
  });

  it("80 columns without a pending question: rail, agent, tabs, timeline, map", () => {
    const order = arrange(sectionsOf(workScreen()), 80).columns[0]!.sections.map((s) => s.kind);
    expect(order).toEqual(["rail", "agent", "tabs", "timeline", "map"]);
  });

  it("a section the view lacks is absent, not an empty pane", () => {
    const s = workScreen();
    s.panes = s.panes!.filter((p) => p.pane !== "timeline" && p.pane !== "agent");
    const f = frame(<WorkArea screen={s} capabilities={caps(160, 40)} />);
    expect(f).not.toContain("Timeline");
    expect(f).not.toContain("Agent");
    expect(arrange(sectionsOf(s), 160).columns).toHaveLength(3);
  });

  it("plain-text mode: labelled lines in reading order, pending first, status kept", () => {
    const f = frame(<WorkArea screen={workScreen({ pending: true })} capabilities={caps(80, 24, true)} focus="pane:rail" />);
    expect(f).toMatchSnapshot();
    const lines = f.split("\n");
    expect(lines[0]).toBe("[agent] Agent (PENDING, running)");
    expect(lines[1]).toBe("  Approve push to origin?");
    expect(lines).toContain("[rail] Next (focused)");
  });

  it("focus is kept by target: other sections changing does not move it", () => {
    const a = raw(<WorkArea screen={workScreen()} capabilities={caps(160, 40)} focus="pane:rail" />);
    const s = workScreen({ pending: true });
    const b = raw(<WorkArea screen={s} capabilities={caps(160, 40)} focus="pane:rail" />);
    expect(litTitles(a).has("Next")).toBe(true);
    expect(litTitles(b).has("Next")).toBe(true);
    expect(litTitles(b).has("Agent")).toBe(false);
  });

  it("resizing moves weight between neighbours, never below 1", () => {
    expect(resize([1, 2, 1], 0, 1)).toEqual([2, 1, 1]);
    expect(resize([1, 2, 1], 0, 5)).toEqual([2, 1, 1]);
    expect(resize([1, 2, 1], 1, -9)).toEqual([1, 1, 2]);
    expect(columnWidths([1, 2, 1], 160)).toEqual([40, 80, 40]);
  });
});
