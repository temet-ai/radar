import { beforeAll, describe, expect, it } from "vitest";
import { renderNode, type Node } from "../../src/nodes/index.js";
import { applyLook, defaultLook } from "../../src/text/look.js";
import { frameAt } from "./helpers.js";

// #40: a review chip on a merge request row looked dropped "with room to spare" at 228 columns. The room is not
// spare: the row's name cells hold the icon, "!264 ", the title, two spaces, the author icon and the author (19 cells
// beside the title), the chip is 19 cells, and the title must keep 12 (`chips.gives`, GUIDE "Chips"), so the chip needs
// 50 name cells. These tests pin that rule on the merge request row's real shape, at the widths of the report.
const t = (text: string) => [{ text }] as never;
const row = (title: string): unknown => ({
  id: "m:264",
  name: [
    { text: "● ", ink: "good" },
    { text: "!264 ", ink: "accent_secondary" },
    { text: title, ink: "primary" },
    { text: "  ", ink: "muted" },
    { text: "@ ", ink: "muted" },
    { text: "Alice E.", ink: "muted" },
  ],
  chips: { gives: 2, text: [{ text: " " }, { text: "reviewed def4567", chip: true, ink: "info" }] },
  cells: [
    { heading: "state", text: t("needs 1 approval") },
    { heading: "ticket", text: t("ABC-20") },
    { heading: "updated", text: t("2w") },
  ],
});
const tree = (title: string): Node =>
  ({
    node: "tree",
    header: t("OPENED BY everyone"),
    columns: [
      { heading: "state", width: 20, keep: 6 },
      { heading: "ticket", width: 21, keep: 3, extra: true },
      { heading: "updated", width: 8, keep: 4 },
    ],
    name_width: 30,
    open: [["t:watch"], ["t:watch", "r:api"]],
    rows: [{ id: "t:watch", name: t("ankh-morpork/watch"), children: [{ id: "r:api", name: t("api"), children: [row(title)] }] }],
  }) as unknown as Node;

const at = (title: string, width: number) =>
  frameAt(renderNode(tree(title), { width, height: 12, focused: true }), width, 12)
    .split("\n")
    .find((l) => l.includes("!264"))!;

describe("a review chip on a merge request row (#40)", () => {
  beforeAll(() => applyLook(defaultLook("off")));

  it("is drawn from the width where the title keeps 12 cells beside it, and at every width above", () => {
    const title = "Tidy the build";
    let first = 0;
    for (let w = 90; w <= 190; w++) {
      const line = at(title, w);
      if (line.includes("reviewed def4567") && !first) first = w;
      if (first) expect(line, `${w}: ${line}`).toContain("reviewed def4567");
    }
    expect(first).toBeGreaterThan(0);
    // one cell narrower the title would keep 11, so the chip gives way
    expect(at(title, first - 1)).not.toContain("reviewed def4567");
  }, 60000);

  it("never leaves the title with fewer than 12 cells beside the chip", () => {
    for (let w = 90; w <= 190; w++) {
      const line = at("Tidy the build and its docs", w);
      if (!line.includes("reviewed def4567")) continue;
      const shown = /!264 (\S[^@]*?)\s*…?\s+@ /.exec(line)?.[1] ?? "";
      expect(shown.replace("…", "").length, `${w}: ${line}`).toBeGreaterThanOrEqual(11);
    }
  }, 60000);

  it("a title shorter than 12 cells keeps the chip wherever the whole title fits beside it", () => {
    for (let w = 150; w <= 190; w++) {
      const line = at("Fix", w);
      if (line.includes("reviewed def4567")) expect(line).toContain("!264 Fix");
    }
    expect(at("Fix", 190)).toContain("reviewed def4567");
  }, 60000);
});
