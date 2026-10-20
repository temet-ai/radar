import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import React from "react";
import { Box, Text } from "ink";
import { describe, expect, it } from "vitest";
import { renderNode, visibleRows, type Node } from "../../src/nodes/index.js";
import { PATH_SEP, hitSpecs } from "../../src/nodes/hits.js";
import { textWords } from "../../src/text/TextLine.js";
import { frameAt } from "./helpers.js";

// Q38-T11: body rows must be pointable. Every node the reference makes clickable registers its rectangles with the
// target ids view_tui registers (list/table/entries rows: the row's target; tree rows: the path joined by PATH_SEP,
// `node:<id>` under pointer_action; log lines: `log:<row>`), at the absolute cells the rows are drawn in.
type Hit = { x: number; y: number; w: number; h: number; target: string };
const OX = 3;
const OY = 2;
function hitsOf(node: Node, width: number): { hits: Hit[]; lines: string[] } {
  const hits: Hit[] = [];
  const registry = { add: (h: Hit) => hits.push(h) };
  const el = (
    <Box flexDirection="column">
      <Text>top</Text>
      <Text>top</Text>
      <Box marginLeft={OX}>
        {renderNode(node, { width, focused: true, registry })}
      </Box>
    </Box>
  );
  return { hits, lines: frameAt(el, width + OX).split("\n") };
}
const t = (text: string) => [{ text }] as never;

describe("the rows a press can reach", () => {
  it("list: a hit on each row's line, none on headings, at the node's absolute place", () => {
    const { hits, lines } = hitsOf(
      {
        node: "list",
        rows: [
          { target: "h", text: t("Views"), heading: true },
          { target: "row:a", text: t("alpha") },
          { target: "row:b", text: t("beta") },
        ],
      } as Node,
      30,
    );
    expect(hits.map((h) => h.target)).toEqual(["row:a", "row:b"]);
    for (const h of hits) {
      expect(h.x).toBe(OX);
      expect(h.w).toBe(30);
      expect(lines[h.y]).toContain(h.target === "row:a" ? "alpha" : "beta");
    }
  });
  it("table: rows sit under the heading row", () => {
    const { hits, lines } = hitsOf(
      {
        node: "table",
        columns: [{ heading: "name", width: 12, keep: 1 }],
        rows: [
          { target: "r1", cells: [{ heading: "name", text: t("one") }] },
          { target: "r2", cells: [{ heading: "name", text: t("two") }] },
        ],
      } as Node,
      30,
    );
    expect(hits.map((h) => h.target)).toEqual(["r1", "r2"]);
    expect(lines[hits[0].y]).toContain("one");
    expect(lines[hits[1].y]).toContain("two");
    expect(lines[hits[0].y - 1]).toContain("NAME");
  });
  it("tree: rows by path (ids joined by the separator), the header target and contributed cells' targets", () => {
    const node = {
      node: "tree",
      header: t("OPENED BY"),
      header_target: "authors",
      columns: [{ heading: "state", width: 10, keep: 1 }],
      name_width: 12,
      open: [["g"]],
      rows: [
        {
          id: "g",
          name: t("group"),
          children: [
            {
              id: "c",
              name: t("child"),
              cells: [{ heading: "state", text: t("ok"), target: "state:c" }],
            },
          ],
        },
      ],
    } as unknown as Node;
    const { hits, lines } = hitsOf(node, 40);
    const targets = hits.map((h) => h.target);
    expect(targets).toEqual(["g", `g${PATH_SEP}c`, "authors", "state:c"]);
    const row = hits.find((h) => h.target === `g${PATH_SEP}c`)!;
    expect(lines[row.y]).toContain("child");
    const cell = hits.find((h) => h.target === "state:c")!;
    expect(lines[cell.y].slice(cell.x, cell.x + 2)).toBe("ok");
    expect(lines[hits.find((h) => h.target === "authors")!.y]).toContain(
      "OPENED BY",
    );
  });
  it("tree with pointer_action sends node:<id>; heading rows are not pointable", () => {
    const node = {
      node: "tree",
      columns: [],
      pointer_action: true,
      name_width: 12,
      rows: [
        { id: "f1", name: t("file") },
        { id: "h", name: t("heading"), heading: true },
      ],
    } as unknown as Node;
    expect(hitsOf(node, 30).hits.map((h) => h.target)).toEqual(["node:f1"]);
  });
  it("entries: the whole entry; log: log:<row> per line", () => {
    const e = hitsOf(
      {
        node: "entries",
        empty: "",
        rows: [
          {
            target: "e1",
            before: [t("── May ──")],
            head: t("Ada"),
            body: [t("hi"), t("there")],
          },
          { target: "e2", head: t("Bob") },
        ],
      } as Node,
      30,
    );
    expect(e.hits.map((h) => [h.target, h.h])).toEqual([
      ["e1", 3],
      ["e2", 1],
    ]);
    expect(e.lines[e.hits[0].y]).toContain("Ada");
    const l = hitsOf(
      {
        node: "log",
        top: 0,
        total: 2,
        empty: "",
        lines: [
          { text: t("a"), row: 7, selected: false },
          { text: t("b"), row: 8, selected: false },
        ],
      } as Node,
      30,
    );
    expect(l.hits.map((h) => h.target)).toEqual(["log:7", "log:8"]);
  });
});

// The class: every list, table, tree, entries and log in every recorded fixture, at a pane's width.
const dir = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../fixtures",
);
function nodes(v: unknown, out: Node[] = []): Node[] {
  if (Array.isArray(v)) v.forEach((x) => nodes(x, out));
  else if (v && typeof v === "object") {
    const o = v as Record<string, unknown>;
    if (
      ["list", "table", "tree", "entries", "log"].includes(String(o.node)) &&
      (o.rows || o.lines)
    )
      out.push(o as unknown as Node);
    else for (const x of Object.values(o)) nodes(x, out);
  }
  return out;
}
describe("every pointable node of every recorded fixture", () => {
  const cases: Node[] = [];
  for (const f of fs
    .readdirSync(dir)
    .filter((f) => f.endsWith(".json") && !f.startsWith("_")))
    nodes(JSON.parse(fs.readFileSync(path.join(dir, f), "utf8")), cases);
  it("registers one hit per drawn row, each on the line that shows the row", () => {
    expect(cases.length).toBeGreaterThan(30);
    const bad: string[] = [];
    cases.forEach((node: any, n) => {
      const { hits, lines } = hitsOf(node, 52);
      const expected = hitSpecs(node, 52).filter((s) => s.dx === 0).length;
      if (
        hits.length === 0 &&
        (node.rows ?? node.lines).length > 0 &&
        !(node.rows ?? []).every((r: any) => r.heading)
      )
        bad.push(`#${n} ${node.node}: no hits`);
      if (hits.length < expected)
        bad.push(
          `#${n} ${node.node}: ${hits.length} of ${expected} registered`,
        );
      const rowHits = hits.filter((h) => h.x === OX && h.w === 52);
      const names: string[] =
        node.node === "tree"
          ? visibleRows(node)
              .filter((s) => !s.row.heading)
              .map((s) => textWords(s.row.name).trim().slice(0, 4))
          : node.node === "list"
            ? node.rows
                .filter((r: any) => !r.heading)
                .map((r: any) => textWords(r.text).trim().slice(0, 4))
            : [];
      if (node.node === "list" || node.node === "tree")
        rowHits.forEach((h, i) => {
          if (names[i] && !lines[h.y]?.includes(names[i]))
            bad.push(
              `#${n} ${node.node} hit ${i} at y=${h.y} "${lines[h.y]}" lacks "${names[i]}"`,
            );
        });
    });
    expect(bad).toEqual([]);
  });
});
