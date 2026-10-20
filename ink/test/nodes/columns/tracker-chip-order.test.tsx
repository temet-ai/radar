import { beforeAll, describe, expect, it } from "vitest";
import { renderNode, type Node } from "../../../src/nodes/index.js";
import { applyLook, defaultLook } from "../../../src/text/look.js";
import { frameAt } from "../helpers.js";

// #38: with a tracker chip and an extra (ticket) column, the chip gives way no later than the column and, once gone,
// stays gone at every narrower width, though the dropped column frees room for the name.
const t = (text: string) => [{ text }] as never;
const title = "WATCH-12: Add a stats endpoint for the dashboard";
const tree = (): Node =>
  ({
    node: "tree",
    header: t("OPENED BY everyone"),
    columns: [
      { heading: "state", width: 20, keep: 6 },
      { heading: "ticket", width: 19, keep: 3, extra: true },
      { heading: "updated", width: 8, keep: 4 },
    ],
    name_width: 30,
    open: [["t:watch"], ["t:watch", "r:api"]],
    rows: [
      {
        id: "t:watch",
        name: t("ankh-morpork/watch"),
        children: [
          {
            id: "r:api",
            name: t("api"),
            children: [
              {
                id: "m:263",
                name: [{ text: "!263 ", ink: "accent_secondary" }, { text: title, ink: "primary" }, { text: "  ", ink: "muted" }, { text: "Sam V.", ink: "muted" }],
                chips: { gives: 1, text: [{ text: " " }, { text: "tracked", chip: true, ink: "info" }] },
                cells: [
                  { heading: "state", text: t("◷ needs 1 approval") },
                  { heading: "ticket", text: t("WATCH-12 · Review"), target: "ticket:WATCH-12" },
                  { heading: "updated", text: t("16m") },
                ],
              },
            ],
          },
        ],
      },
    ],
  }) as unknown as Node;

const rowAt = (width: number) =>
  frameAt(renderNode(tree(), { width, height: 12, focused: true }), width, 12)
    .split("\n")
    .find((l) => l.includes("!263"))!;

describe("a tracker chip and an extra column give way in order (#38)", () => {
  beforeAll(() => applyLook(defaultLook("off")));

  it("the chip is gone from the width the ticket column goes, and at every width below", () => {
    let gone = false;
    const seen: string[] = [];
    for (let width = 160; width >= 50; width--) {
      const row = rowAt(width);
      const chip = row.includes("tracked");
      const ticket = row.includes("WATCH-12 ·") || row.includes("WATCH-12 · ");
      seen.push(`${width}:${chip ? "chip" : "-"}:${ticket ? "ticket" : "-"}`);
      if (!ticket) expect(chip, `${width}: ${row}`).toBe(false);
      if (gone) expect(chip, `${width}: ${row}`).toBe(false);
      if (!chip) gone = true;
    }
    // the sweep crosses both states
    expect(seen.some((s) => s.includes(":chip"))).toBe(true);
    expect(seen.some((s) => s.endsWith(":-:-"))).toBe(true);
  });

  it("at 76 columns the ticket column is dropped and the title is not cut to a stub beside a chip", () => {
    const row = rowAt(76);
    expect(row).not.toContain("tracked");
    expect(row).not.toMatch(/!263 Add …/);
  });
});
