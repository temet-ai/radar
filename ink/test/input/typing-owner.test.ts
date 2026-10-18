import { describe, expect, it } from "vitest";
import { screenKey, scrollKey, typingPane } from "../../src/input/route.js";
import type { Chord } from "../../src/chrome/types.js";

// Pure ownership values: no host, global look, clock or frame is needed to pin the same pane order as routing.
describe("the pane whose field owns body keys", () => {
  it("treats caret zero as active and null or missing as inactive", () => {
    expect(typingPane({ panes: [{ pane: "nav", fields: [{ caret: null }] }, { pane: "middle", fields: [{ caret: 0 }] }] })).toBe("middle");
    expect(typingPane({ panes: [{ pane: "middle", fields: [{}] }] })).toBeUndefined();
  });
  it("includes a bottom field", () => {
    expect(typingPane({ panes: [{ pane: "detail", bottom_field: { caret: 3 } }] })).toBe("detail");
  });
  for (const kind of ["list", "tree"]) {
    it(`an inline ${kind} editor in another pane owns movement and fold keys`, () => {
      const editor = { pane: "nav", body: { node: kind, editing: { row: 0, path: ["watch"], at: 0, text: "Watch", caret: 0 } } };
      const focused = { pane: "middle", body: { node: "tree", columns: [], selected: ["watch"], open: [], rows: [
        { id: "watch", name: [{ text: "Watch" }], children: [{ id: "patrol", name: [{ text: "Patrol" }] }] },
      ] } };
      const screen = { panes: [editor, focused] };
      expect(typingPane(screen)).toBe("nav");
      for (const chord of [{ key: { char: "=" } }, { key: "right" }, { key: "down" }, { key: "enter" }] as Chord[])
        expect(screenKey(screen, "middle", chord)).toEqual({ intent: "key", chord });
      expect(scrollKey({ panes: [editor, { pane: "detail", body: { node: "document" } }] }, "detail", { key: "down" }, [
        { kind: "document", focused: true, at: () => 0, max: () => 10, page: () => { throw new Error("editor owns page keys"); }, key: () => { throw new Error("editor owns movement keys"); } },
      ])).toBeUndefined();
      expect(typingPane({ panes: [editor], page: { panes: [focused] } })).toBeUndefined();
      expect(typingPane({ panes: [focused], page: { panes: [editor] } })).toBe("nav");
    });
  }
  it("takes the first active pane in view order", () => {
    expect(typingPane({ panes: [{ pane: "detail", fields: [{ caret: 1 }] }, { pane: "middle", bottom_field: { caret: 0 } }] })).toBe("detail");
  });
  it("uses the current page's panes instead of the underlying screen", () => {
    expect(typingPane({ panes: [{ pane: "nav", fields: [{ caret: 1 }] }], page: { panes: [{ pane: "middle", bottom_field: { caret: 0 } }] } })).toBe("middle");
    expect(typingPane({ panes: [{ pane: "nav", fields: [{ caret: 1 }] }], page: { panes: [] } })).toBeUndefined();
  });
});
