import { describe, expect, it } from "vitest";
import { UiState } from "../../src/state/ui-state.js";

describe("UiState", () => {
  it("focus stays on its target across snapshots while the target exists, and goes when it does not", () => {
    const ui = new UiState();
    ui.select("row:3");
    ui.applySnapshot({ focus: "middle" }, ["row:3", "row:4"]);
    expect(ui.focus).toBe("row:3");
    expect(ui.focusPane).toBe("middle");
    ui.applySnapshot({ focus: "detail" }, ["row:4"]);
    expect(ui.focus).toBeUndefined();
  });

  it("a resize never moves focus and only clamps unreachable scroll", () => {
    const ui = new UiState();
    ui.select("row:3");
    ui.setScroll("middle", 40);
    ui.resize(() => 10);
    expect(ui.focus).toBe("row:3");
    expect(ui.scrollOf("middle")).toBe(10);
  });

  it("round-trips through save/restore (a restart)", () => {
    const a = new UiState();
    a.select("row:9");
    a.setScroll("detail", 5);
    const b = new UiState();
    b.restore(a.save());
    expect(b.focus).toBe("row:9");
    expect(b.scrollOf("detail")).toBe(5);
  });
});
