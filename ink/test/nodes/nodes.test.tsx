import React from "react";
import { describe, expect, it } from "vitest";
import { renderNode, intentForChord, invokeIntent, type ActionSpec, type Node, type NodeContext } from "../../src/nodes/index.js";
import { frameAt } from "./helpers.js";

const ctx = (width: number, over: Partial<NodeContext> = {}): NodeContext => ({ width, focused: true, ...over });
const t = (text: string, ink?: string) => [{ text, ...(ink ? { ink } : {}) }] as never;

const draw = (node: Node, width = 80, over: Partial<NodeContext> = {}) => frameAt(renderNode(node, ctx(width, over)), width);

describe("one test per node variant (hand-written minimal nodes)", () => {
  it("list: rows, selection mark, right text, heading", () => {
    const out = draw({
      node: "list",
      selected: 2,
      rows: [
        { target: "h", text: t("Views"), heading: true },
        { target: "a", text: t("Open"), right: t("3") },
        { target: "b", text: t("Mine") },
      ],
    });
    expect(out).toMatch(/Views/);
    expect(out).toMatch(/^ Mine/m);
    expect(out).toMatch(/Open\s+3/);
  });

  it("tree: open groups show children, closed ones do not, columns line up", () => {
    const node: Node = {
      node: "tree",
      columns: [{ heading: "Branch", width: 10, keep: 4 }],
      name_width: 16,
      open: [["g"]],
      selected: ["g", "c1"],
      rows: [
        { id: "g", name: t("group"), children: [{ id: "c1", name: t("clone-1"), cells: [{ heading: "Branch", text: t("main") }] }] },
        { id: "h", name: t("closed"), children: [{ id: "c2", name: t("hidden-child") }] },
      ],
    };
    const out = draw(node);
    expect(out).toMatch(/^group\n/); // top level: no joint, no chevron
    expect(out).toMatch(/^  └─ clone-1\s+main/m); // folding is shown by a tree line, GUIDE 2.12
    expect(out).toMatch(/closed ⋯/); // a folded node shows a dim ⋯
    expect(out).not.toMatch(/hidden-child/);
    expect(out).not.toMatch(/[▸▾]/);
  });

  it("document: one line per text, links underline and plain mode shows the address", () => {
    const node: Node = { node: "document", lines: [t("first"), [{ text: "site", link: "https://x.test" }] as never] };
    expect(draw(node)).toMatch(/^ first\n site/);
    expect(draw(node, 80, { plainText: true })).toMatch(/site <https:\/\/x\.test>/);
  });

  it("rich: lines, wrapped docs with a bar, a meter and a wrapped block", () => {
    const out = draw({
      node: "rich",
      blocks: [
        { block: "line", text: t("Title") },
        {
          block: "doc",
          bar: { text: "│ ", ink: "muted" }, // the bar carries its own gap (view_tui inserts it before each line as it is)
          doc: [
            { t: "heading", level: 2, text: [{ text: "Plan" }] },
            { t: "para", text: [{ text: "Do " }, { text: "this", marks: { bold: true } }, { text: " " }, { text: ":tada:", kind: "emoji" }] },
            { t: "list", items: [{ marker: "bullet", blocks: [{ t: "para", text: [{ text: "one" }] }] }, { marker: { number: 2 }, blocks: [{ t: "para", text: [{ text: "two" }] }] }] },
            { t: "code", lang: "rs", text: "let x = 1;" },
          ],
        },
        { block: "meter", stages: [{ name: "build", mark: "done", look: "" }, { name: "test", mark: "failed", look: "" }] },
        { block: "wrapped", text: "wrapped words", prefix: { text: "> " }, ink: "plain" },
      ],
    });
    expect(out).toMatch(/Title/);
    expect(out).toMatch(/│ Plan/);
    expect(out).toMatch(/Do this 🎉/);
    expect(out).toMatch(/• one/);
    expect(out).toMatch(/2\. two/);
    expect(out).toMatch(/let x = 1;/);
    expect(out).toMatch(/✓ build\s+✗ test/);
    expect(out).toMatch(/> wrapped words/);
  });

  it("table: headings, cells by heading, the selected row, empty text", () => {
    const columns = [{ heading: "Name", width: 12, keep: 4 }, { heading: "State", width: 8, keep: 5 }];
    const out = draw({ node: "table", columns, selected: 0, rows: [{ target: "a", cells: [{ heading: "State", text: t("open") }, { heading: "Name", text: t("alpha") }] }] });
    expect(out).toMatch(/NAME\s+STATE/);
    expect(out).toMatch(/^ alpha\s+open/m);
    expect(draw({ node: "table", columns, rows: [], empty: "No rows." })).toBe(" ∅ No rows.");
  });

  it("entries: before, head and body lines; empty text", () => {
    const out = draw({ node: "entries", empty: "none", selected: 0, rows: [{ target: "e", head: t("Ada commented"), before: [t("── May ──")], body: [t("looks good")] }] });
    expect(out).toMatch(/── May ──\n Ada commented\n\s+looks good/);
    expect(draw({ node: "entries", empty: "none", rows: [] })).toBe(" ∅ none");
  });

  it("log: wrapped rows with the selected one marked and the position when scrolled", () => {
    const out = draw({ node: "log", top: 4, total: 40, empty: "no log", lines: [{ text: t("line a"), row: 4, selected: false }, { text: t("line b"), row: 5, selected: true }] });
    expect(out).toMatch(/line a\nline b/);
    expect(out).toMatch(/5–6 of 40/);
    expect(draw({ node: "log", top: 0, total: 0, empty: "no log", lines: [] })).toBe(" ∅ no log");
  });

  it("diff: numbers, signs, hunk heading, a comment under a line, split layout when wide", () => {
    const node: Node = {
      node: "diff",
      selected: 2,
      top: 0,
      split: false,
      empty: "no changes",
      under: [{ line: 2, lines: [t("Ada: why?"), t("")], input: { text: "because", caret: 3 } }],
      lines: [
        { kind: "Hunk", old: null, new: null, text: "@@ -1,2 +1,2 @@" },
        { kind: "Context", old: 1, new: 1, text: "keep" },
        { kind: "Removed", old: 2, new: null, text: "old line" },
        { kind: "Added", old: null, new: 2, text: "new line" },
      ],
    };
    const out = draw(node);
    expect(out).toMatch(/@@ -1,2 \+1,2 @@/);
    expect(out).toMatch(/1\s+1\s+ keep/);
    expect(out).toMatch(/2\s+- old line\n\s+Ada: why\?\n\s+because/);
    expect(out).toMatch(/2 \+ new line/);
    const wide = frameAt(renderNode({ ...node, split: true } as Node, ctx(160)), 160);
    expect(wide).toMatch(/old line\s+│/);
  });

  it("state: empty, loading and failed in words; plain mode names the state", () => {
    expect(draw({ node: "state", state: "empty", text: "Nothing here." })).toBe(" ∅ Nothing here.");
    expect(draw({ node: "state", state: "loading", text: "Fetching" })).toBe(" ∅ Fetching");
    expect(draw({ node: "state", state: "failed", text: "No network" })).toBe(" ∅ No network");
    expect(draw({ node: "state", state: "failed", text: "No network" }, 80, { plainText: true })).toBe("failed: No network");
  });

  it("an unknown node type is a visible fallback naming the type, not a crash", () => {
    const out = draw({ node: "hologram" } as unknown as Node);
    expect(out).toMatch(/this renderer does not draw node hologram/);
    const rich = draw({ node: "rich", blocks: [{ block: "teleport" }] } as unknown as Node);
    expect(rich).toMatch(/does not draw block teleport/);
  });

  it("a node drawn as a fallback lists the view's actions, and a key or choice sends Invoke by action id", () => {
    const actions: ActionSpec[] = [
      { id: "", label: "Merge request", heading: true },
      { id: "gitlab.mr.approve", label: "Approve", key: { key: { char: "a" } }, change: "edit" },
      { id: "gitlab.mr.open", label: "Open in browser", key: { key: "enter" } },
      { id: "gitlab.mr.refresh", label: "Refresh", key: { key: { char: "r" }, ctrl: true } },
    ];
    const unknown = { node: "hologram" } as unknown as Node;
    const ext: Node = { node: "extension", capability: "terminal", version: 1, data: {}, fallback: { node: "document", lines: [t("log tail")] } };
    for (const node of [unknown, ext]) {
      const out = draw(node, 80, { actions });
      expect(out).toMatch(/Merge request\n\s+a\s+Approve\n\s+⏎\s+Open in browser\n\s+ctrl-r\s+Refresh/);
    }
    expect(intentForChord(actions, { key: { char: "a" } })).toEqual({ intent: "invoke", action: "gitlab.mr.approve" });
    expect(intentForChord(actions, { key: "enter" })).toEqual({ intent: "invoke", action: "gitlab.mr.open" });
    expect(intentForChord(actions, { key: { char: "r" }, ctrl: true })).toEqual({ intent: "invoke", action: "gitlab.mr.refresh" });
    expect(intentForChord(actions, { key: { char: "z" } })).toBeNull();
    expect(invokeIntent(actions[1])).toEqual({ intent: "invoke", action: "gitlab.mr.approve" });
  });

  it("extension: editor capability draws the editor; others draw the fallback and say so", () => {
    const fallback: Node = { node: "document", lines: [t("typed so far")] };
    const editor: Node = { node: "extension", capability: "editor", version: 1, data: { title: "Comment", lines: ["a"], line: "b", caret: 1 }, fallback };
    const out = draw(editor);
    expect(out).toMatch(/Comment\na\nb/);
    expect(draw(editor, 80, { capabilities: [] })).toMatch(/shown simply: this needs editor\n typed so far/);
    const video: Node = { node: "extension", capability: "terminal", version: 1, data: {}, fallback };
    expect(draw(video)).toMatch(/shown simply: this needs terminal\n typed so far/);
  });
});

describe("list rows with counts (survey defect 8: Branches views showed no 27/22/14)", () => {
  it("draws each row's right text at the right edge, at 80 and 160 columns", () => {
    const node: Node = {
      node: "list",
      selected: 0,
      rows: [
        { target: "view:0", text: [{ icon: "views" }, { text: "all" }] as never },
        { target: "view:1", text: [{ icon: "warning" }, { text: "dirty" }] as never, right: [{ ink: "number", text: "27" }] as never },
        { target: "view:2", text: t("ahead of remote"), right: [{ ink: "number", text: "22" }] as never },
        { target: "view:3", text: t("stashes"), right: [{ ink: "number", text: "14" }] as never },
      ],
    };
    for (const w of [24, 80, 160]) {
      const lines = draw(node, w).split("\n");
      expect(lines[1], `dirty at ${w}`).toMatch(/dirty\s+27$/);
      expect(lines[2]).toMatch(/ahead of remote\s+22$/);
      expect(lines[3]).toMatch(/stashes\s+14$/);
      expect(lines[3].length).toBe(lines[1].length);
    }
  });
});
