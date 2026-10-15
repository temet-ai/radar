import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import { describe, expect, it } from "vitest";
import { renderNode, type Node } from "../../src/nodes/index.js";
import { styledFrameAt } from "./helpers.js";
import { parseAnsi } from "./ansi.js";

// ui::Columns::cell_lines pads a tree's column cell to the column's width, its air cell included, in the style of the
// cell's last span (live at 160x40: the blanks after "needs 1 approval" are bold like it). Ink padded with no style, so
// an underlined or shaded last word stopped at the word.
describe("a tree's column cell is padded in its last run's style", () => {
  const node = (last: object): Node =>
    ({
      node: "tree",
      columns: [{ heading: "state", width: 12, keep: 1 }],
      name_width: 10,
      rows: [{ id: "r", name: [{ text: "row" }], cells: [{ heading: "state", text: [{ text: "◷ ", ink: "warn" }, { text: "ok", ...last }] }] }],
    }) as never;

  it("an underlined last run underlines the column to its edge", () => {
    const width = 40;
    const row = parseAnsi(styledFrameAt(renderNode(node({ look: "accent.primary underline" }), { width, focused: false }), width))[0]!;
    // the column is the last 12 cells before the tree's scrollbar cell
    const col = row.slice(width - 1 - 12, width - 1);
    expect(col.map((c) => c.ch).join("")).toBe("◷ ok        ");
    expect(col.slice(2).every((c) => c.underline)).toBe(true);
    expect(col.slice(0, 2).some((c) => c.underline)).toBe(false);
  });

  it("a plain last run leaves the blanks plain", () => {
    const width = 40;
    const row = parseAnsi(styledFrameAt(renderNode(node({ ink: "primary" }), { width, focused: false }), width))[0]!;
    expect(row.slice(width - 1 - 12, width - 1).some((c) => c.underline || c.bold)).toBe(false);
  });
});
