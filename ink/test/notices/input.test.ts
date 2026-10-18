import { describe, expect, it } from "vitest";
import { InputController, type InputView } from "../../src/input/controller.js";
import { UiState } from "../../src/state/ui-state.js";
import { resolveKeyOwner } from "../../src/app/key-owner.js";
import type { Hit } from "../../src/chrome/types.js";
import type { MouseEvent } from "../../src/input/mouse.js";
const mouse = (kind: MouseEvent["kind"], over = {}): MouseEvent => ({ kind, button: "left", x: 10, y: 20, ctrl: false, alt: false, shift: false, ...over });
function make(over: Partial<InputView> = {}) {
  const sent: unknown[] = [], dismissed: boolean[] = [], activated: string[] = [];
  let notice: Hit | undefined = { x: 0, y: 20, w: 50, h: 1, target: "chrome:notice:1" };
  const view: InputView = { menu: () => null, capturing: () => false, route: (chord) => ({ intent: "key", chord }), hits: () => [{ x: 0, y: 20, w: 50, h: 1, target: "chrome:dangerous-underneath" }], noticeAt: (x, y) => notice && x >= notice.x && x < notice.x + notice.w && y === notice.y ? notice : undefined, noticeInput: (escape) => dismissed.push(escape), noticeActivate: (target) => activated.push(target), ...over };
  return { controller: new InputController(view, (intent) => sent.push(intent), new UiState()), sent, dismissed, activated, setNotice: (next: Hit | undefined) => { notice = next; } };
}
describe("notice input ownership and stable pointer identities", () => {
  it("Esc dismisses once and reaches its owner once", () => { const m = make(); m.controller.keyChord({ key: "esc" }); expect(m.dismissed).toEqual([true]); expect(m.sent).toEqual([{ intent: "key", chord: { key: "esc" } }]); });
  it("does not steal ~ from a capturing editor or menu", () => {
    const m = make({ capturing: () => true, scroll: () => { throw Error("editor key stolen"); } }); m.controller.keyChord({ key: { char: "~" } }); expect(m.dismissed).toEqual([false]); expect(m.sent).toHaveLength(1);
    const menu = make({ menu: () => ({ id: "watch", title: "Watch", question: false, items: [] }), route: () => { throw Error("menu key stolen"); } }); menu.controller.keyChord({ key: { char: "~" } }); expect(menu.sent).toHaveLength(1);
  });
  it("notifies for local scrolling and leaves dock ~ with its composer", () => {
    const local = make({ scroll: () => "done" }); local.controller.keyChord({ key: "down" }); expect(local.dismissed).toEqual([false]); expect(local.sent).toEqual([]);
    const keys: unknown[] = [], dock = make({ dock: (chord) => { keys.push(chord); return true; } }); dock.controller.keyChord({ key: { char: "~" } }); expect(keys).toEqual([{ key: { char: "~" } }]); expect(dock.sent).toEqual([]); expect(dock.dismissed).toEqual([false]);
  });
  it("intercepts the entire notice rectangle before selection, dock, context and wheel actions", () => {
    const m = make({ dockMouse: () => { throw Error("covered dock action"); }, selectAt: () => { throw Error("covered selection"); } });
    m.controller.mouseEvent(mouse("press")); expect(m.activated).toEqual([]); m.controller.mouseEvent(mouse("release")); expect(m.activated).toEqual(["chrome:notice:1"]);
    m.controller.mouseEvent(mouse("press", { button: "right" })); m.controller.mouseEvent(mouse("release", { button: "right" })); m.controller.mouseEvent(mouse("wheel_down", { button: "none" })); expect(m.sent).toEqual([]); expect(m.dismissed).toEqual([]);
  });
  it("a stale press cannot dismiss a replacement or activate the covered action", () => {
    const m = make(); m.controller.mouseEvent(mouse("press")); m.setNotice({ x: 0, y: 20, w: 50, h: 1, target: "chrome:notice:2" }); m.controller.mouseEvent(mouse("release")); expect(m.activated).toEqual([]); expect(m.sent).toEqual([]);
    m.controller.mouseEvent(mouse("press")); m.setNotice(undefined); m.controller.mouseEvent(mouse("release")); expect(m.activated).toEqual([]); expect(m.sent).toEqual([]);
  });
  it("history owns the body and blocks dock ownership", () => {
    expect(resolveKeyOwner({ snapshot: { chrome: { notice_history: {} }, screen: { panes: [], dock: { visible: true, focused: true } } } })).toEqual({ body: false, dock: false, blocked: true });
  });
});
