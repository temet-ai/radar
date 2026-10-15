import { describe, expect, it } from "vitest";
import { InputController, type InputView } from "../../../src/input/controller.js";
import { UiState } from "../../../src/state/ui-state.js";
import type { Intent } from "../../../src/chrome/types.js";

/**
 * The host opens a context menu where the pointer pressed: ToHost::Pointer{column,row} (0-based screen cells) goes
 * just before the Context intent (ipc/bridge.rs). Without it the menu opens centred.
 */
describe("a right-click tells the host its cell before the Context", () => {
  const log: string[] = [];
  const view: InputView = {
    menu: () => null,
    capturing: () => false,
    route: (chord) => ({ intent: "key", chord }),
    hits: () => [{ x: 10, y: 20, w: 30, h: 1, target: "row:a" }],
  };
  const ctl = () =>
    new InputController(
      view,
      (i: Intent) => log.push(`intent:${i.intent}`),
      new UiState(),
      () => 0,
      () => {},
      (column, row) => log.push(`pointer:${column},${row}`),
    );
  it("pointer first, then context, at the press cell", () => {
    log.length = 0;
    ctl().mouse("\x1b[<2;13;21M"); // SGR is 1-based: column 13 row 21 = cell 12,20
    expect(log).toEqual(["pointer:12,20", "intent:context"]);
  });
  it("a left click sends no pointer", () => {
    log.length = 0;
    ctl().mouse("\x1b[<0;13;21M");
    expect(log.filter((l) => l.startsWith("pointer"))).toEqual([]);
  });
});
