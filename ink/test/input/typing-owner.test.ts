import { describe, expect, it } from "vitest";
import { typingPane } from "../../src/input/route.js";

// Pure ownership values: no host, global look, clock or frame is needed to pin the same pane order as routing.
describe("the pane whose field owns body keys", () => {
  it("treats caret zero as active and null or missing as inactive", () => {
    expect(typingPane({ panes: [{ pane: "nav", fields: [{ caret: null }] }, { pane: "middle", fields: [{ caret: 0 }] }] })).toBe("middle");
    expect(typingPane({ panes: [{ pane: "middle", fields: [{}] }] })).toBeUndefined();
  });
  it("includes a bottom field", () => {
    expect(typingPane({ panes: [{ pane: "detail", bottom_field: { caret: 3 } }] })).toBe("detail");
  });
  it("takes the first active pane in view order", () => {
    expect(typingPane({ panes: [{ pane: "detail", fields: [{ caret: 1 }] }, { pane: "middle", bottom_field: { caret: 0 } }] })).toBe("detail");
  });
  it("uses the current page's panes instead of the underlying screen", () => {
    expect(typingPane({ panes: [{ pane: "nav", fields: [{ caret: 1 }] }], page: { panes: [{ pane: "middle", bottom_field: { caret: 0 } }] } })).toBe("middle");
    expect(typingPane({ panes: [{ pane: "nav", fields: [{ caret: 1 }] }], page: { panes: [] } })).toBeUndefined();
  });
});
