import { describe, expect, it } from "vitest";
import { InputController, type InputView } from "../../../src/input/controller.js";
import { tokenize } from "../../../src/input/tokens.js";
import { UiState } from "../../../src/state/ui-state.js";
import type { Intent } from "../../../src/chrome/types.js";

/**
 * Class: shift + a movement key while the editor captures. xterm reports shift as the modifier parameter (CSI 1;2D);
 * the host has no Chord.shift, it takes Intent::EditSelect{anchor,extent} (the host owns the range). Plain movement
 * is still the key.
 */
describe("shift+movement in the capturing editor", () => {
  const editorData = { line: "héllo wörld", caret: 5, selection: null as [number, number] | null };
  const run = (bytes: string, o: { editor: boolean; capturing?: boolean } = { editor: true }) => {
    const sent: Intent[] = [];
    const view: InputView = {
      menu: () => null,
      capturing: () => o.capturing ?? true,
      route: (chord) => ({ intent: "key", chord }),
      hits: () => [],
      editor: () => (o.editor ? editorData : null),
    };
    const ctl = new InputController(view, (i) => sent.push(i), new UiState(), () => 0);
    for (const ev of tokenize(bytes).events) if (ev.kind === "key") ctl.keyChord(ev.chord, ev.shift);
    return sent;
  };

  it("the tokenizer carries shift on the arrows, home and end (a mouse selection extends on shift+↑/↓), and only there", () => {
    const shifted = (b: string) => tokenize(b).events.map((e) => (e.kind === "key" ? !!e.shift : null));
    expect(shifted("\x1b[1;2D\x1b[1;2C\x1b[1;2H\x1b[1;2F\x1b[1;2A\x1b[1;2B")).toEqual([true, true, true, true, true, true]);
    expect(shifted("\x1b[D\x1b[1;5D\x1b[A\x1b[5;2~")).toEqual([false, false, false, false]);
  });
  it("shift+↑/↓ in the capturing editor are the key, as before (only ←→ home end select there)", () => {
    expect(run("\x1b[1;2A")).toEqual([{ intent: "key", chord: { key: "up" } }]);
  });
  it("shift-left/right grow from the caret by one code point, home/end to the line's ends", () => {
    expect(run("\x1b[1;2D")).toEqual([{ intent: "edit_select", anchor: 5, extent: 4 }]);
    expect(run("\x1b[1;2C")).toEqual([{ intent: "edit_select", anchor: 5, extent: 6 }]);
    expect(run("\x1b[1;2H")).toEqual([{ intent: "edit_select", anchor: 5, extent: 0 }]);
    expect(run("\x1b[1;2F")).toEqual([{ intent: "edit_select", anchor: 5, extent: 11 }]);
  });
  it("plain movement, and shift without an open editor, are the key", () => {
    expect(run("\x1b[D")).toEqual([{ intent: "key", chord: { key: "left" } }]);
    expect(run("\x1b[1;2D", { editor: false })).toEqual([{ intent: "key", chord: { key: "left" } }]);
  });
});
