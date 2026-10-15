// Where motion marks rows and waiting panels (view_tui.rs motion::row / motion::loading).
import { describe, expect, it } from "vitest";
import { nodeMarks } from "../../src/motion/node-marks.js";

const tree = (over: object = {}) =>
  ({
    node: "tree",
    columns: [],
    rows: [
      { id: "a", name: [], glow: "issue:A-1" },
      { id: "b", name: [] },
      { id: "c", name: [], glow: "issue:A-3" },
    ],
    ...over,
  }) as any;

describe("nodeMarks", () => {
  it("marks a tree row that has a glow key, one cell high at the node's width", () => {
    const m = nodeMarks(tree(), 3, 5, 40, 10);
    expect(m.rows).toEqual([
      ["issue:A-1", { x: 3, y: 5, w: 40, h: 1 }],
      ["issue:A-3", { x: 3, y: 7, w: 40, h: 1 }],
    ]);
    expect(m.loading).toEqual([]);
  });

  it("a header line pushes the rows down; a window shows only its rows, from the node's top", () => {
    const m = nodeMarks(
      tree({ header: [{ text: "H" }], header_target: "x" }),
      0,
      2,
      30,
      8,
    );
    expect(m.rows.map(([, r]) => r.y)).toEqual([3, 5]);
    const w = nodeMarks(tree(), 0, 2, 30, 1, { first: 2, count: 1 });
    expect(w.rows).toEqual([["issue:A-3", { x: 0, y: 2, w: 30, h: 1 }]]);
  });

  it("a State node that waits is a loading panel, one that is empty or failed is not", () => {
    expect(
      nodeMarks(
        { node: "state", state: "loading", text: "…" } as any,
        1,
        2,
        20,
        6,
      ).loading,
    ).toEqual([{ x: 1, y: 2, w: 20, h: 6 }]);
    expect(
      nodeMarks({ node: "state", state: "empty", text: "" } as any, 1, 2, 20, 6)
        .loading,
    ).toEqual([]);
    expect(
      nodeMarks(
        { node: "state", state: "failed", text: "" } as any,
        1,
        2,
        20,
        6,
      ).loading,
    ).toEqual([]);
  });

  it("a tree with a loading note is a loading panel, and its rows start under the note", () => {
    const m = nodeMarks(
      tree({ note: { state: "loading", text: "waiting" } }),
      0,
      0,
      30,
      9,
    );
    expect(m.loading).toEqual([{ x: 0, y: 0, w: 30, h: 9 }]);
    expect(m.rows[0]![1].y).toBe(1);
  });

  it("other nodes mark nothing", () => {
    expect(
      nodeMarks({ node: "document", lines: [] } as any, 0, 0, 10, 10),
    ).toEqual({ rows: [], loading: [] });
  });
});
