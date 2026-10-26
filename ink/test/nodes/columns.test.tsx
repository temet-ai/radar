import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  renderNode,
  visibleRows,
  type Node,
  type NodeContext,
} from "../../src/nodes/index.js";
import { fitTreeColumns, fitTableColumns } from "../../src/nodes/columns.js";
import { textWords } from "../../src/text/TextLine.js";
import { cellWidth } from "../../src/chrome/width.js";
import { frameAt } from "./helpers.js";

// Defect 5 of the live survey: a tree or table must draw its heading row (what is over the names, then each column's
// heading in capitals) and give the columns the room the reference gives them (ui::Columns::fit): the columns take their
// declared widths and the names get what is left, so "failed" and "main" are not cut to "fai…" and "m…".
const ctx = (width: number, over: Partial<NodeContext> = {}): NodeContext => ({
  width,
  focused: true,
  ...over,
});
const draw = (node: Node, width: number) =>
  frameAt(renderNode(node, ctx(width)), width);
const t = (text: string, ink?: string) =>
  [{ text, ...(ink ? { ink } : {}) }] as never;

describe("the tree's heading row", () => {
  const node: Node = {
    node: "tree",
    header: t("OPENED BY everyone"),
    columns: [
      { heading: "state", width: 20, keep: 6 },
      { heading: "updated", width: 8, keep: 4 },
    ],
    name_width: 30,
    open: [["g"]],
    rows: [
      {
        id: "g",
        name: t("acme-billing-service"),
        children: [
          {
            id: "r",
            name: t("billing-service-ui"),
            cells: [
              { heading: "state", text: t("✗ failed") },
              { heading: "updated", text: t("16m") },
            ],
          },
        ],
      },
    ],
  };

  it("draws what is over the names and each column's heading in capitals over its column", () => {
    const lines = draw(node, 62).split("\n");
    // a divider marks each edge a column is sized from (nodes/drag.ts withEdges)
    expect(lines[0]).toMatch(/^ OPENED BY everyone\s+│STATE\s+│UPDATED$/);
    // the headings stand exactly over the cells under them
    const at = (line: string, word: string) => line.indexOf(word);
    expect(at(lines[0], "STATE")).toBe(at(lines[2], "✗ failed"));
    expect(at(lines[0], "UPDATED")).toBe(at(lines[2], "16m"));
  });

  it("gives the columns their declared width, so a cell that fits is not cut", () => {
    const out = draw(node, 62);
    expect(out).toMatch(/✗ failed {2,}16m/);
    expect(out).not.toMatch(/…/);
  });

  it("drops the lowest-keep column whole when the names would get less than name_width (Columns::fit)", () => {
    // 44 - 2 (the cursor mark) = 42 cells; 20 + 8 + the 30 names want 58: `updated` (keep 4) goes, `state` stays.
    const narrow: Node = { ...(node as object), name_width: 30 } as Node;
    const out = draw(narrow, 44);
    expect(out.split("\n")[0]).toMatch(/STATE/);
    expect(out.split("\n")[0]).not.toMatch(/UPDATED/);
    expect(out).toMatch(/✗ failed/);
  });

  it("has no heading row without a header or a header target (the reference omits it), and none under a note", () => {
    const bare = { ...(node as object), header: [] } as Node;
    expect(draw(bare, 62).split("\n")[0]).toMatch(/acme-billing-service/);
    const noted = {
      ...(node as object),
      note: { state: "loading", text: "Loading" },
    } as Node;
    expect(draw(noted, 62)).not.toMatch(/STATE/);
  });

  it("an extra column comes after the own ones that fit, before the last, while the names keep their room", () => {
    const cols = [
      { heading: "a", width: 10, keep: 5 },
      { heading: "x", width: 12, keep: 1, extra: true },
      { heading: "b", width: 10, keep: 4 },
    ];
    expect(
      fitTreeColumns(cols, false, 70, 30).map((c) => c.column.heading),
    ).toEqual(["a", "x", "b"]);
    expect(
      fitTreeColumns(cols, false, 60, 30).map((c) => c.column.heading),
    ).toEqual(["a", "b"]);
    // ordered_columns: every column in declared order, fitted by keep
    expect(
      fitTreeColumns(cols, true, 60, 30).map((c) => c.column.heading),
    ).toEqual(["a", "b"]);
    expect(
      fitTreeColumns(cols, true, 70, 30).map((c) => c.column.heading),
    ).toEqual(["a", "x", "b"]);
  });

  it("drops the later of equal keeps first and always keeps the last one left", () => {
    const cols = [
      { heading: "a", width: 10, keep: 1 },
      { heading: "b", width: 10, keep: 1 },
    ];
    expect(
      fitTreeColumns(cols, true, 35, 20).map((c) => c.column.heading),
    ).toEqual(["a"]);
    expect(
      fitTreeColumns(cols, true, 5, 20).map((c) => c.column.heading),
    ).toEqual(["a"]);
  });
});

describe("cut_names: the widest run gives way, a short run after it stays whole", () => {
  it("cuts the long repo name, keeps the branch word", () => {
    const node: Node = {
      node: "tree",
      cut_names: true,
      columns: [{ heading: "state", width: 12, keep: 1 }],
      name_width: 30,
      rows: [
        {
          id: "r",
          name: [
            { text: "acme-billing-admin-console-ui" },
            { text: "  main", ink: "muted" },
          ] as never,
          cells: [{ heading: "state", text: t("passed") }],
        },
      ],
    };
    const out = draw(node, 40);
    expect(out).toMatch(/…\s+main\s+passed/);
  });
});

describe("tables", () => {
  const columns = [
    { heading: "type", width: 8, keep: 5 },
    { heading: "title", width: 12, keep: 9, flex: true },
    { heading: "status", width: 12, keep: 4 },
    { heading: "updated", width: 9, keep: 3 },
  ];
  const rows = [
    {
      target: "a",
      cells: [
        { heading: "type", text: t("Story") },
        {
          heading: "title",
          text: t(
            "A title that is long enough to want the whole flexible column",
          ),
        },
        { heading: "status", text: t("In Progress") },
        { heading: "updated", text: t("2d") },
      ],
    },
  ];
  it("draws every heading in capitals over its column, the flex column taking the rest", () => {
    const out = draw({ node: "table", columns, rows } as Node, 70);
    const [head, row] = out.split("\n");
    expect(head).toMatch(/TYPE\s+│TITLE\s+│STATUS\s+│UPDATED/);
    expect(head.indexOf("STATUS")).toBe(row.indexOf("In Progress"));
    expect(head.indexOf("UPDATED")).toBe(row.indexOf("2d"));
    expect(cellWidth(row)).toBeLessThanOrEqual(70);
  });
  it("fits the flex column to what the fixed ones leave", () => {
    const placed = fitTableColumns(columns as never, 70 - 2);
    // ui::table: the flex column has `68 - fixed` cells for its value and its air follows it, so the row is one wider than the room.
    expect(placed.map((p) => p.width).reduce((a, b) => a + b, 0)).toBe(68 + 1);
    expect(placed.find((p) => p.column.flex)!.width).toBe(68 - 8 - 12 - 9 + 1);
  });
});

/** The heading as the reference draws it in its column: in capitals, whole, unless an icon before it leaves under `width - 1`
 * cells for the words (Column::heading_spans clips to the column less its air), when its first cells show. */
const headingShown = (c: {
  heading: string;
  width: number;
  icon?: string | null;
}) => {
  const h = c.heading.toUpperCase();
  return c.icon
    ? h.slice(
        0,
        Math.max(1, c.width - 1 - 2 - (h.length > c.width - 3 ? 1 : 0)),
      )
    : h;
};

// The class: every tree and every table in every recorded fixture, at the width a pane gets at 80 and 160 columns.
const dir = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../fixtures",
);
const files = fs
  .readdirSync(dir)
  .filter((f) => f.endsWith(".json") && !f.startsWith("_"));

function bodies(v: unknown, out: Node[] = []): Node[] {
  if (Array.isArray(v)) v.forEach((x) => bodies(x, out));
  else if (v && typeof v === "object") {
    const o = v as Record<string, unknown>;
    if ((o.node === "tree" || o.node === "table") && Array.isArray(o.columns))
      out.push(o as unknown as Node);
    else for (const x of Object.values(o)) bodies(x, out);
  }
  return out;
}

describe("every tree and table of every recorded fixture", () => {
  const cases: { file: string; node: any }[] = [];
  for (const f of files) {
    const v = JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
    bodies(v).forEach((node, i) => cases.push({ file: `${f}#${i}`, node }));
  }
  it("has trees and tables to check", () =>
    expect(cases.length).toBeGreaterThanOrEqual(16));

  for (const width of [52, 118]) {
    it(`draws its heading row and whole cells at ${width} cells`, () => {
      const bad: string[] = [];
      for (const { file, node } of cases) {
        if (node.rows.length === 0) continue;
        const tree = node.node === "tree";
        if (tree && node.note) continue;
        if (tree && (node.header ?? []).length === 0 && !node.header_target)
          continue;
        const lines = draw(node, width).split("\n");
        const head = lines[0] ?? "";
        const shown = (
          node.columns as {
            heading: string;
            width: number;
            icon?: string | null;
          }[]
        ).filter((c) => head.includes(headingShown(c)));
        if (shown.length === 0)
          bad.push(`${file}: no column heading in "${head}"`);
        // all columns fit with the names at their width → none may be dropped
        const want =
          (node.columns as { width: number }[]).reduce(
            (n, c) => n + c.width,
            0,
          ) + (tree ? node.name_width : 0);
        const extras =
          (node.columns as { extra?: boolean }[]).some((c) => c.extra) &&
          !node.ordered_columns;
        if (
          !extras &&
          want <= width - 2 &&
          shown.length !== node.columns.length
        )
          bad.push(
            `${file}: room for all columns at ${width} but ${shown.length}/${node.columns.length} headings`,
          );
        // a cell whose words fit its declared column is drawn whole when the column is shown; a table that does not
        // fit gives up width as the reference's constraint layout does (test/nodes/columns/), so only a fitting one is held to it
        const wide =
          tree ||
          (node.columns as { width: number }[]).reduce(
            (n, c) => n + c.width,
            0,
          ) <=
            width - 2;
        const rowsShown = tree
          ? visibleRows(node).map((s) => s.row)
          : node.rows;
        rowsShown.forEach((r: any, i: number) => {
          const line = lines[i + 1] ?? "";
          for (const c of shown as any[]) {
            const cell = r.cells?.find((x: any) => x.heading === c.heading);
            const words = cell ? textWords(cell.text).trimEnd() : "";
            if (
              wide &&
              words &&
              cellWidth(words) <= c.width - 1 &&
              !line.includes(words)
            )
              bad.push(
                `${file} row ${i} ${c.heading}: "${words}" cut in "${line}"`,
              );
          }
        });
      }
      expect(bad).toEqual([]);
    });
  }
});

describe("a column dragged wider than there is room for", () => {
  const col = (heading: string, width: number, keep: number, natural?: number) => ({ heading, width, keep, ...(natural ? { natural } : {}) });

  it("gives its extra back before any column of a tree is dropped, never more than it was given", () => {
    const columns = [col("state", 40, 6, 20), col("approvals", 11, 1), col("updated", 8, 4)];
    // 59 wanted beside names of 24 in 70: 13 too many, all from the dragged column
    const placed = fitTreeColumns(columns, true, 70, 24);
    expect(placed.map((p) => [p.column.heading, p.width])).toEqual([["state", 27], ["approvals", 11], ["updated", 8]]);
    // with less room still it is back at the part's width before APPROVALS goes
    expect(fitTreeColumns(columns, true, 63, 24).map((p) => [p.column.heading, p.width])).toEqual([["state", 20], ["approvals", 11], ["updated", 8]]);
    expect(fitTreeColumns(columns, true, 62, 24).map((p) => p.column.heading)).toEqual(["state", "updated"]);
  });

  it("gives its extra back in a table before the others are squeezed", () => {
    const placed = fitTableColumns([col("type", 6, 0), { ...col("title", 10, 0), flex: true }, col("status", 30, 0, 10)], 40);
    // 43 value cells wanted in 38 (40 less two spacers): STATUS gives the 5, the title keeps its least, TYPE its own
    expect(placed.map((p) => p.width)).toEqual([6, 10, 25]);
  });
});
