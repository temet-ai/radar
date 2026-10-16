import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import { describe, expect, it } from "vitest";
import { renderNode, type Node } from "../../src/nodes/index.js";
import { bgOf } from "../../src/text/look.js";
import { styledFrameAt } from "./helpers.js";
import { parseAnsi } from "./ansi.js";

// view_tui render_section: the row being typed on is drawn as it is, then the field over it from `at` cells in to one
// cell short of the row's end. Radar's add rows (`  + add a repository`) keep their marker, the field starts after it.
const t = (text: string) => [{ text }] as never;
const node = (at: number): Node =>
  ({
    node: "list",
    selected: 1,
    editing: { row: 1, at, text: "acme", caret: 2 },
    rows: [{ target: "a", text: t("alpha") }, { target: "b", text: t("  + add a repository") }, { target: "c", text: t("gamma") }],
  }) as never;

describe("an inline edit starts `at` cells into its row", () => {
  const draw = (at: number) => parseAnsi(styledFrameAt(renderNode(node(at), { width: 30, focused: true }), 30, 6))[1]!;
  it("keeps the row's own cells before the field and the field from there on", () => {
    const row = draw(4);
    expect(row.slice(0, 4).map((c) => c.ch).join("")).toBe("   +");
    expect(row.slice(0, 4).every((c) => c.bg !== bgOf("bg.overlay"))).toBe(true);
    expect(row.slice(4, 8).map((c) => c.ch).join("")).toBe("acme");
    expect(row.slice(4, 29).every((c) => c.bg === bgOf("bg.overlay"))).toBe(true);
    // The caret is on the third typed character, one cell only.
    expect(row.filter((c) => c.inverse).map((c) => c.ch)).toEqual(["m"]);
    expect(row.findIndex((c) => c.inverse)).toBe(6);
    // The row's last cell stays the row's own.
    expect(row[29]!.bg).not.toBe(bgOf("bg.overlay"));
  });
  it("is the whole row but its last cell with `at` 0", () => {
    const row = draw(0);
    expect(row.slice(0, 4).map((c) => c.ch).join("")).toBe("acme");
    expect(row.findIndex((c) => c.inverse)).toBe(2);
    expect(row[29]!.bg).not.toBe(bgOf("bg.overlay"));
  });
});
