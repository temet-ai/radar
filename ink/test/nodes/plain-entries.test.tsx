import React from "react";
import { describe, expect, it } from "vitest";
import { Framed, detailWidth } from "../../src/app/Panes.js";
import { renderNode, type Node, type Text } from "../../src/nodes/index.js";
import { entryLines } from "../../src/nodes/lists.js";
import { hitSpecs } from "../../src/nodes/hits.js";
import { lineCells } from "../../src/text/cells.js";
import { SegRun, TextLine, segRunWords, textLineString } from "../../src/text/TextLine.js";
import { frameAt } from "./helpers.js";

type Hit = { x: number; y: number; w: number; h: number; target: string };
const context: Text = [
  { text: "authored link · " },
  { text: "copy locator", key_hint: "c" },
  { text: " · " },
  { text: "extra actions", key_hint: "x" },
];
const entry = (n: number) => ({ target: `watch:${n}`, head: [{ text: `Watch item ${n}` }], body: [context] });
const at = (hits: Hit[], x: number, y: number) => hits.find((h) => x >= h.x && x < h.x + h.w && y >= h.y && y < h.y + h.h)?.target;

describe("plain Entries preserve expanded shortcut captions", () => {
  it.each([80, 120, 160])("paints complete context keys at terminal width%i with matching entry hits", (columns) => {
    const paneWidth = columns >= 120 ? detailWidth(columns) : columns;
    const width = paneWidth - 2;
    const node: Node = { node: "entries", rows: [entry(1)], selected: 0, empty: "" };
    const hits: Hit[] = [];
    const frame = frameAt(
      <Framed title="context" width={paneWidth} height={12} focused plain>
        {renderNode(node, { width, height: 10, focused: true, plainText: true, registry: { add: (hit) => hits.push(hit) } })}
      </Framed>, columns, 12,
    );
    expect(frame.match(/\[c\]/g)).toHaveLength(1);
    expect(frame.match(/\[x\]/g)).toHaveLength(1);
    const lines = frame.split("\n");
    const words = lines.slice(1, -1).map((line) => line.slice(1, paneWidth - 1).trim()).join(" ").replace(/\s+/g, " ");
    expect(words).toContain("authored link · copy locator [c] · extra actions [x]");
    for (const key of ["[c]", "[x]"]) {
      const y = lines.findIndex((line) => line.includes(key));
      const x = lines[y]!.indexOf(key);
      for (let offset = 0; offset < key.length; offset++) expect(at(hits, x + offset, y)).toBe("watch:1");
    }
    const plain = entryLines(node.rows, width, true);
    expect(hits.find((h) => h.target === "watch:1")?.h).toBe(plain.spans[0]![1]);
    // Styled Entries keep their original one-row-per-authored-line geometry.
    expect(entryLines(node.rows, width, false).lines).toHaveLength(2);
    expect(hitSpecs(node, width)).toEqual([{ dx: 0, dy: 0, w: width, h: 2, target: "watch:1" }]);
    const visual = frameAt(renderNode(node, { width, focused: true }), columns);
    expect(visual).not.toMatch(/\[[cx]\]/);
    expect(visual.split("\n")).toHaveLength(2);
  });

  it("scrolls expanded rows and keeps repeated keys attached to their original entry target", () => {
    const width = 46;
    const node: Node = { node: "entries", rows: [entry(0), entry(1), entry(2)], selected: 2, empty: "" };
    const hits: Hit[] = [];
    const frame = frameAt(renderNode(node, { width, height: 4, focused: true, plainText: true, registry: { add: (hit) => hits.push(hit) } }), width, 4);
    const lines = frame.split("\n");
    expect(lines).toHaveLength(4);
    expect(lines[0]).toContain("[x]");
    expect(at(hits, lines[0]!.indexOf("[x]"), 0)).toBe("watch:1");
    expect(lines[1]).toContain("Watch item 2");
    expect(lines[2]).toContain("[c]");
    expect(lines[3]).toContain("[x]");
    expect(at(hits, lines[2]!.indexOf("[c]"), 2)).toBe("watch:2");
    expect(at(hits, lines[3]!.indexOf("[x]"), 3)).toBe("watch:2");
    expect(hits).toEqual([
      { x: 0, y: 0, w: width, h: 1, target: "watch:1" },
      { x: 0, y: 1, w: width, h: 3, target: "watch:2" },
    ]);
  });
});

describe("zero-text presentation hints", () => {
  const text: Text = [{ text: "copy", key_hint: "c" }, { text: "", key_hint: "x" }, { text: "", key_hint: "c" }];
  it("keeps every supplied hint once in plain readers without adding visual text", () => {
    expect(segRunWords({ text: "", key_hint: "x" }, true)).toBe(" [x]");
    expect(segRunWords({ text: "", key_hint: "x" })).toBe("");
    expect(textLineString(text, true)).toBe("copy [c] [x] [c]");
    expect(textLineString(text)).toBe("copy");
    expect(lineCells(text, true).map((cell) => cell.ch).join("")).toBe("copy [c] [x] [c]");
    expect(frameAt(<TextLine text={text} plain />, 80)).toBe("copy [c] [x] [c]");
    expect(frameAt(<SegRun seg={{ text: "", key_hint: "x" }} plain />, 80)).toBe(" [x]");
  });
});
