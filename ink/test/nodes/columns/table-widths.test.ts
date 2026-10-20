import { describe, expect, it } from "vitest";
import { fitTableColumns, solveWidths } from "../../../src/nodes/columns.js";
import golden from "./table-widths.json" with { type: "json" };

// ui::table::Table gives the table `Min(width - 1)` for the flex column and `Length(width - 1)` for every
// other, one cell of spacing, and drops no column. table-widths.json holds the widths the constraint layout
// gave for those constraints (cases: the merge-request table and the repositories table at the pane widths of 80x24
// and 160x40 and others, then generated column sets); Ink must give the same widths.
type Case = { name: string; width: number; flex: number; columns: number[]; widths: number[] };

describe("table column widths follow the constraint layout", () => {
  for (const c of golden.cases as Case[]) {
    it(`${c.name} in ${c.width} cells`, () => {
      const values = c.columns.map((w) => w - 1);
      expect(solveWidths(values, c.flex < 0 ? undefined : c.flex, c.width)).toEqual(c.widths);
    });
  }

  it("keeps every column and counts each one's air cell (merge-request table, 80x24: 78 cells less the padding)", () => {
    const spec = (heading: string, width: number, flex = false) =>
      ({ heading, width, keep: 0, ...(flex ? { flex: true } : {}) }) as never;
    const columns = [
      spec("", 7), spec("for you", 9), spec("title", 8, true), spec("author", 12),
      spec("state", 19), spec("ticket", 23), spec("updated", 8),
    ];
    const placed = fitTableColumns(columns, 76);
    // measured from the recorded cells (ink/test/text/look/table_80x24.cells.json): 6 7 7 11 16 16 7 and the air cells
    expect(placed.map((p) => p.width - 1)).toEqual([6, 7, 7, 11, 16, 16, 7]);
    expect(placed).toHaveLength(7);
  });

  it("gives the flex column what is left when everything fits", () => {
    const spec = (heading: string, width: number, flex = false) =>
      ({ heading, width, keep: 0, ...(flex ? { flex: true } : {}) }) as never;
    const placed = fitTableColumns([spec("repo", 10, true), spec("main", 12), spec("mrs", 5), spec("last activity", 14)], 156);
    expect(placed.map((p) => p.width)).toEqual([126, 12, 5, 14]);
  });
});
