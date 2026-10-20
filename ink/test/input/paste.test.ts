import { describe, expect, it } from "vitest";
import type { Key as InkKey } from "ink";
import { InputController, type InputView } from "../../src/input/controller.js";
import { UiState } from "../../src/state/ui-state.js";
import { typingPane } from "../../src/input/route.js";
import type { Intent } from "../../src/chrome/types.js";

const none = { upArrow: false, downArrow: false, leftArrow: false, rightArrow: false, pageDown: false, pageUp: false,
  home: false, end: false, return: false, escape: false, ctrl: false, shift: false, tab: false,
  backspace: false, delete: false, meta: false, super: false, hyper: false, capsLock: false, numLock: false } as InkKey;

// GitLab's files-tree filter `/` is a pane field with a caret: the host captures text there, Ink's capturing() does not.
const filtering = { panes: [{ pane: "nav", fields: [{ caret: 0 }] }] };
const idle = { panes: [{ pane: "nav", body: { node: "tree" } }] };

function run(over: { capturing?: boolean; screen?: object; editor?: boolean }, text: string, viaKey = false) {
  const keys: Intent[] = [];
  const pastes: string[] = [];
  const screen = over.screen ?? idle;
  const view: InputView = {
    menu: () => null,
    capturing: () => !!over.capturing,
    partTyping: () => typingPane(screen) !== undefined,
    editor: over.editor ? () => ({ line: "", caret: 0 }) : undefined,
    route: (chord) => ({ intent: "key", chord }),
    hits: () => [],
  };
  const c = new InputController(view, (i) => keys.push(i), new UiState(), () => 0, undefined, undefined, undefined, (t) => pastes.push(t));
  if (viaKey) c.key(text, none);
  else c.paste(text);
  return { keys, pastes };
}

describe("a paste reaches the host as one message", () => {
  it("a part's own field is capturing: the whole text, once, no keys", () => {
    const r = run({ screen: filtering }, "sam\nvimes");
    expect(r).toEqual({ keys: [], pastes: ["sam\nvimes"] });
  });
  it("a host input or menu field: one message, no keys", () => {
    expect(run({ capturing: true }, "my shelf")).toEqual({ keys: [], pastes: ["my shelf"] });
  });
  it("a multi-character read while a part types is a paste, not one key per character", () => {
    expect(run({ screen: filtering }, "héj", true)).toEqual({ keys: [], pastes: ["héj"] });
    expect(run({ capturing: true }, "héj", true)).toEqual({ keys: [], pastes: ["héj"] });
  });
  it("the capturing editor keeps its own path: line breaks one space, one key per character", () => {
    const r = run({ capturing: true, editor: true }, "a\r\n\r\nb");
    expect(r.pastes).toEqual([]);
    expect(r.keys.map((i) => (i as any).chord.key.char).join("|")).toBe("a| |b");
  });
  it("nothing capturing: nothing is sent", () => {
    expect(run({}, "ab")).toEqual({ keys: [], pastes: [] });
    expect(run({ screen: { panes: [{ pane: "nav", fields: [{ caret: null }] }] } }, "ab")).toEqual({ keys: [], pastes: [] });
  });
});
