// The document and Wrapped nodes drawn through renderNode break where the reference's wrap does (no leading space).
import { describe, expect, it } from "vitest";
import { renderNode, type Node, type NodeContext } from "../../src/nodes/index.js";
import { frameAt } from "../nodes/helpers.js";

/** A rich node in a pane `inner` cells wide: it keeps one cell of air each side, so a block has `inner - 2`. The
 * lines come back without that left cell. */
const draw = (node: Node, inner: number) =>
  frameAt(renderNode(node, { width: inner + 2, focused: true } as NodeContext), inner + 2)
    .split("\n")
    .map((l) => l.slice(1));
const para = (text: string) => ({ t: "para" as const, text: [{ text }] });

describe("document text through the node renderer", () => {
  it("the GitHub description at 44 cells breaks as the reference's snapshot does", () => {
    const out = draw({ node: "rich", blocks: [{ block: "doc", doc: [para("Responses stay with the repository and commit they were requested for.")] }] } as never, 44);
    expect(out.map((l) => l.trimEnd())).toEqual(["Responses stay with the repository and", "commit they were requested for."]);
  });
  it("no continued line starts with a space, at any width", () => {
    for (let w = 6; w < 30; w++) {
      const out = draw({ node: "rich", blocks: [{ block: "doc", doc: [para("alpha beta gamma delta epsilon zeta eta theta")] }] } as never, w);
      for (const l of out) expect(l.startsWith(" ")).toBe(false);
    }
  });
  it("a Wrapped block breaks as ui::wrapped does: room is width+1 less the prefix, at least 8", () => {
    const out = draw({ node: "rich", blocks: [{ block: "wrapped", text: "aaaa bbbb cccc dddd", prefix: { text: "│ " }, ink: "plain" }] } as never, 10);
    expect(out.map((l) => l.trimEnd())).toEqual(["│ aaaa bbbb", "│ cccc dddd"]);
  });
});
