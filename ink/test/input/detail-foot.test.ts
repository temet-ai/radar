import { describe, expect, it } from "vitest";
import { InputController, type InputView } from "../../src/input/controller.js";
import { UiState } from "../../src/state/ui-state.js";
import type { Intent } from "../../src/chrome/types.js";

function setup(capturing = false) {
  const sent: Intent[] = [], ui = new UiState();
  ui.select("original-row");
  let now = 1000;
  const view: InputView = {
    menu: () => null, capturing: () => capturing, route: (chord) => ({ intent: "key", chord }),
    hits: () => [{ x: 1, y: 4, w: 8, h: 1, target: "detail-foot:7:0" }, { x: 1, y: 1, w: 8, h: 1, target: "row:1" }],
    selectAt: (_x, y) => y === 4 ? { x: 0, y: 0, w: 80, h: 20, id: "pane:detail", kind: "text" } : undefined,
    frameRow: () => [..."copy watch report"],
  };
  const controller = new InputController(view, (i) => sent.push(i), ui, () => now);
  return { controller, sent, ui, advance: () => { now += 100; } };
}

describe("detail-foot physical control presses", () => {
  it("activates once per left press before text selection, never selects a row or double-activates", () => {
    const { controller, sent, ui, advance } = setup();
    for (let i = 0; i < 3; i++) {
      controller.mouse("\x1b[<0;3;5M");
      expect(sent).toHaveLength(i + 1); // acts on press, not delayed until release
      controller.mouse("\x1b[<0;3;5m");
      advance();
    }
    expect(sent).toEqual(Array.from({ length: 3 }, () => ({ intent: "activate", target: "detail-foot:7:0" })));
    expect(ui.focus).toBe("original-row");
    expect(controller.selector.selection).toBeNull();
  });
  it("context and middle presses are consumed and capture cannot activate the background", () => {
    const { controller, sent } = setup();
    controller.mouse("\x1b[<2;3;5M\x1b[<2;3;5m\x1b[<1;3;5M\x1b[<1;3;5m");
    expect(sent).toEqual([]);
    const capture = setup(true);
    capture.controller.mouse("\x1b[<0;3;5M\x1b[<0;3;5m");
    expect(capture.sent).toEqual([]);
  });
  it("a footer press clears a preceding ordinary-row double-click candidate", () => {
    const { controller, sent, advance } = setup();
    controller.mouse("\x1b[<0;3;2M"); advance();
    controller.mouse("\x1b[<0;3;5M"); advance();
    controller.mouse("\x1b[<0;3;2M");
    expect(sent.map((i) => i.intent)).toEqual(["select", "activate", "select"]);
  });
});
