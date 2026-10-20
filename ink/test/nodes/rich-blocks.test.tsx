import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import { describe, expect, it } from "vitest";
import { renderNode, type Node, type NodeContext } from "../../src/nodes/index.js";
import { frameAt, styledFrameAt } from "./helpers.js";
import { parseAnsi } from "./ansi.js";
import { fitTreeColumns } from "../../src/nodes/columns.js";
import { slotStyle } from "../../src/text/look.js";

// view_tui rich_lines: a `doc` block is ui::rich's layout (a column short of the room), `most` cuts it to that many
// lines and a muted `…`, a link's address follows its words unless `urls` is off (the settings sheet's default), a
// `wrapped` block is drawn in its ink.
const ctx = (width: number, over: Partial<NodeContext> = {}): NodeContext => ({ width, focused: true, ...over });
const rich = (...blocks: unknown[]): Node => ({ node: "rich", blocks }) as never;
const doc = (inlines: unknown[], over: Record<string, unknown> = {}) => ({ block: "doc", doc: [{ t: "para", text: inlines }], ...over });
const draw = (node: Node, width = 80, over: Partial<NodeContext> = {}) => frameAt(renderNode(node, ctx(width, over)), width);
const link = (words: string, url: string) => ({ text: words, kind: { link: { url } } });

describe("a doc block's most, urls and bar", () => {
  const paras = ["a", "b", "c", "d"].map((x) => ({ t: "para", text: [{ text: `DOC${x}` }] }));

  it("most cuts the document to that many lines and ends in a muted …", () => {
    const out = draw(rich({ block: "doc", doc: paras, most: 1 }));
    expect(out).toContain("DOCa");
    expect(out).not.toContain("DOCb");
    expect(out).toContain("…");
    expect(draw(rich({ block: "doc", doc: paras, most: null }))).toContain("DOCd");
  });

  it("a link's address follows its words where they differ, unless urls is false or the sheet draws it", () => {
    const words = [{ text: "see " }, link("the docs", "https://example.test/docs")];
    expect(draw(rich(doc(words)))).toContain("see the docs (https://example.test/docs)");
    expect(draw(rich(doc(words, { urls: false })))).not.toContain("https://");
    expect(draw(rich(doc(words)), 80, { sheet: true })).not.toContain("https://");
    expect(draw(rich(doc(words, { urls: true })), 80, { sheet: true })).toContain("(https://example.test/docs)");
    // words that are the address say it once
    expect(draw(rich(doc([link("https://example.test/x", "https://example.test/x")])))).not.toContain("(https://");
  });

  it("the bar is set as it comes before every line, with no gap of Ink's own", () => {
    const out = draw(rich({ block: "doc", doc: paras.slice(0, 2), bar: { text: "│ ", ink: "muted" } }));
    expect(out.split("\n").filter((l) => l.trimStart().startsWith("│ DOC")).length).toBe(2);
  });
});

describe("a wrapped block is drawn in its ink", () => {
  const colourOf = (ink: string) => {
    const row = parseAnsi(styledFrameAt(renderNode(rich({ block: "wrapped", text: "words", prefix: { text: "> " }, ink }), ctx(40)), 40))[0]!;
    return row.find((c) => c.ch === "w")?.fg;
  };

  it("an ink draws in its slot's colour and plain in the terminal's own", () => {
    expect(colourOf("bad")).toBe(slotStyle("status.error").color);
    expect(colourOf("good")).toBe(slotStyle("status.success").color);
    expect(colourOf("plain")).toBeUndefined();
  });
});

describe("a tree's columns at 50 and 160 columns", () => {
  const cols = [
    { heading: "x", width: 10, keep: 0 },
    { heading: "y", width: 10, keep: 0, extra: true },
    { heading: "z", width: 10, keep: 0 },
  ];
  const names = (width: number, ordered: boolean, name: number) => fitTreeColumns(cols, ordered, width, name).map((c) => c.column.heading);

  it("an extra column is added before the part's last only where the names keep their room", () => {
    expect(names(47, false, 20)).toEqual(["x", "z"]);
    expect(names(157, false, 20)).toEqual(["x", "y", "z"]);
  });

  it("ordered_columns fits every column in declared order, the later of equal keeps going first", () => {
    expect(names(47, true, 20)).toEqual(["x", "y"]);
    expect(names(157, true, 20)).toEqual(["x", "y", "z"]);
  });

  it("name_width decides which columns drop", () => {
    const two = [{ heading: "x", width: 18, keep: 0 }, { heading: "y", width: 18, keep: 1 }];
    const kept = (width: number, name: number) => fitTreeColumns(two, false, width, name).map((c) => c.column.heading);
    expect(kept(47, 8)).toEqual(["x", "y"]);
    expect(kept(47, 30)).toEqual(["y"]);
    expect(kept(157, 30)).toEqual(["x", "y"]);
    expect(kept(157, 130)).toEqual(["y"]);
  });
});
