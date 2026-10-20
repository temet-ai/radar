import { describe, expect, it } from "vitest";
import { InputController, type InputView } from "../../src/input/controller.js";
import { UiState } from "../../src/state/ui-state.js";
import type { Intent } from "../../src/chrome/types.js";

// A chip a part contributes (a Seg.target in the detail pane's text) is pressed from wherever the keys are: Ink sends the
// chip's own target and nothing before it, so the host (host.rs `pane_of_target`) focuses the pane the chip is in.
function setup() {
  const sent: Intent[] = [];
  const ui = new UiState();
  ui.select("row:1");
  let now = 1000;
  const view: InputView = {
    menu: () => null,
    capturing: () => false,
    route: (chord) => ({ intent: "key", chord }),
    hits: () => [
      { x: 40, y: 6, w: 10, h: 1, target: "under:3:0" },
      { x: 0, y: 2, w: 30, h: 1, target: "row:1" },
    ],
    // The chip sits inside the detail pane's text: a press there is held until it is let go.
    selectAt: (x) => (x >= 35 ? { x: 35, y: 3, w: 40, h: 20, id: "pane:detail", kind: "text" } : undefined),
    paneAt: (x) => (x >= 35 ? "detail" : "middle"),
    frameRow: () => [..."copy branch name"],
  };
  const controller = new InputController(view, (i) => sent.push(i), ui, () => now);
  return { controller, sent, advance: () => { now += 1000; } };
}

describe("a press on a chip in the detail pane", () => {
  it("sends one select with the chip's target and no pane or row select before it", () => {
    const { controller, sent } = setup();
    controller.mouse("\x1b[<0;42;7M");
    controller.mouse("\x1b[<0;42;7m");
    expect(sent).toEqual([{ intent: "select", target: "under:3:0" }]);
  });

  it("sends the chip's context, alone, on a right press", () => {
    const { controller, sent } = setup();
    controller.mouse("\x1b[<2;42;7M\x1b[<2;42;7m");
    expect(sent).toEqual([{ intent: "context", target: "under:3:0" }]);
  });
});
