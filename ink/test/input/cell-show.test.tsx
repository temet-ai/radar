import { describe, expect, it } from "vitest";
import { InputController, type InputView } from "../../src/input/controller.js";
import { UiState } from "../../src/state/ui-state.js";
import type { Intent } from "../../src/chrome/types.js";
import { hitSpecs } from "../../src/nodes/hits.js";
import type { Node } from "../../src/nodes/view.js";

// A contributed cell that shows a subject (another part's cell, such as a ticket on a branch row) is pressed on its own:
// the row is selected and the subject shown (the host's Intent::Show). With no target of its own it still has a hit.
const t = (text: string) => [{ text }] as never;
const ticket = { kind: "jira", id: "WATCH-12", label: "WATCH-12" };

function treeWith(cell: Record<string, unknown>): Node {
  return {
    node: "tree",
    columns: [{ heading: "ticket", width: 10, keep: 1 }],
    name_width: 12,
    rows: [{ id: "b", name: t("feature"), cells: [{ heading: "ticket", text: t("WATCH-12"), ...cell }] }],
  } as unknown as Node;
}

describe("a contributed cell with show and no target", () => {
  it("has a hit on its row's target that carries its subject", () => {
    const specs = hitSpecs(treeWith({ show: ticket }), 40);
    const cell = specs.find((s) => s.show);
    expect(cell).toBeDefined();
    expect(cell!.target).toBe("b");
    expect(cell!.show).toEqual(ticket);
    expect(cell!.dx).toBeGreaterThan(0);
  });

  it("keeps its own target and sends no subject when it has one", () => {
    const specs = hitSpecs(treeWith({ show: ticket, target: "ticket:b" }), 40);
    const cell = specs.find((s) => s.target === "ticket:b");
    expect(cell).toBeDefined();
    expect(cell!.show).toBeUndefined();
  });

  it("has no hit of its own when it has neither a target nor a subject", () => {
    const specs = hitSpecs(treeWith({}), 40);
    expect(specs.filter((s) => s.target !== "b")).toEqual([]);
  });
});

function setup(hit: Record<string, unknown>) {
  const sent: Intent[] = [];
  const ui = new UiState();
  const view: InputView = {
    menu: () => null,
    capturing: () => false,
    route: (chord) => ({ intent: "key", chord }),
    hits: () => [{ x: 0, y: 0, w: 20, h: 1, target: "b", ...hit } as never],
    paneAt: () => "middle",
  };
  const controller = new InputController(view, (i) => sent.push(i), ui, () => 1000);
  return { controller, sent };
}

describe("a press on a contributed cell with show", () => {
  it("sends one show with the row's target and the cell's subject", () => {
    const { controller, sent } = setup({ show: ticket });
    controller.mouse("\x1b[<0;5;1M");
    controller.mouse("\x1b[<0;5;1m");
    expect(sent).toEqual([{ intent: "show", target: "b", subject: ticket }]);
  });

  it("a right press on it sends the row's context, as before", () => {
    const { controller, sent } = setup({ show: ticket });
    controller.mouse("\x1b[<2;5;1M\x1b[<2;5;1m");
    expect(sent).toEqual([{ intent: "context", target: "b" }]);
  });

  it("a plain row, with no subject, still sends a select", () => {
    const { controller, sent } = setup({});
    controller.mouse("\x1b[<0;5;1M");
    controller.mouse("\x1b[<0;5;1m");
    expect(sent).toEqual([{ intent: "select", target: "b" }]);
  });
});
