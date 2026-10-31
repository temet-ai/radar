import { describe, expect, it } from "vitest";
import { InputController, type InputView } from "../../src/input/controller.js";
import { UiState } from "../../src/state/ui-state.js";
import type { Hit } from "../../src/chrome/types.js";
import type { MouseEvent } from "../../src/input/mouse.js";
import type { Shape } from "../../src/app/pointer.js";

/** The pointer around a drag (#25): its shape over the dock's edge, and what is under it once a drag is let go. */
describe("the pointer around a border, a column or the dock's edge", () => {
  const hits: Hit[] = [
    { x: 10, y: 5, w: 1, h: 1, target: "layout:edge:state:1" },
    { x: 0, y: 8, w: 40, h: 1, target: "row:a" },
    { x: 0, y: 12, w: 40, h: 1, target: "dock:edge:30" },
  ];
  function rig() {
    const shapes: Shape[] = [];
    const lit: Array<string | null> = [];
    let holding = false;
    let ended = 0;
    const view: InputView = {
      menu: () => null,
      capturing: () => false,
      route: (chord) => ({ intent: "key", chord }),
      hits: () => hits,
      // a drag held from a press on a layout target until a release, or a move with no button down
      layoutMouse: (ev: MouseEvent, hit) => {
        if (holding) {
          if (ev.kind === "release" || (ev.kind === "motion" && ev.button === "none")) holding = false;
          return true;
        }
        holding = ev.kind === "press" && !!hit?.target.startsWith("layout:");
        return holding;
      },
      endDrags: () => void ended++,
    };
    const ctl = new InputController(view, () => {}, new UiState(), () => 0, (hit) => lit.push(hit?.target ?? null), () => {}, (s) => shapes.push(s));
    return { ctl, shapes, lit, ended: () => ended };
  }

  it("the dock's top edge shows a row-resize pointer", () => {
    const { ctl, shapes } = rig();
    ctl.mouse("\x1b[<35;5;13M");
    expect(shapes.at(-1)).toBe("row-resize");
  });

  it("let go over a row, the row is lit and the pointer is a hand at once, with no move needed", () => {
    const { ctl, shapes, lit } = rig();
    ctl.mouse("\x1b[<0;11;6M");
    ctl.mouse("\x1b[<32;11;9M");
    ctl.mouse("\x1b[<0;11;9m");
    expect(lit.at(-1)).toBe("row:a");
    expect(shapes.at(-1)).toBe("pointer");
  });

  it("let go outside the window (heard as a move with no button), nothing there is lit", () => {
    const { ctl, lit, shapes } = rig();
    ctl.mouse("\x1b[<0;11;6M");
    ctl.mouse("\x1b[<35;200;9M");
    expect(lit.at(-1)).toBeNull();
    expect(shapes.at(-1)).toBe("default");
  });

  it("the terminal losing focus drops whatever is held", () => {
    const { ctl, ended } = rig();
    ctl.blur();
    expect(ended()).toBe(1);
  });
});
