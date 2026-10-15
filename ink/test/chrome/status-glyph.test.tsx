import { describe, expect, it } from "vitest";
import { layoutStatus, type Cell } from "../../src/chrome/layout.js";
import { iconOf, slotStyle } from "../../src/text/look.js";
import { booksChrome } from "./fixture.js";

const line = (cells: Cell[]) => cells.map((c) => c.text).join("");

describe.each([80, 160])("status message glyph at %i columns", (w) => {
  const draw = (message: { text: string; icon?: string | null; ink?: any }) => {
    const chrome = booksChrome({ width: w });
    chrome.status.message = { ink: "info", ...message } as any;
    return layoutStatus(chrome, w, 0).cells;
  };

  it("a good message keeps ✓ in the success colour, a space before its words", () => {
    const cells = draw({ text: "configured repositories are readable", icon: "run_passed" });
    expect(line(cells)).toContain("✓ configured repositories are readable");
    const g = cells.find((c) => c.text.includes("✓"));
    expect(g?.style?.color).toBe(slotStyle(iconOf("run_passed")!.style).color);
  });

  it("a bad message keeps ✗ in the error colour", () => {
    const cells = draw({ text: "GitHub token rejected", icon: "run_failed" });
    expect(line(cells)).toContain("✗ GitHub token rejected");
    const g = cells.find((c) => c.text.includes("✗"));
    expect(g?.style?.color).toBe(slotStyle(iconOf("run_failed")!.style).color);
  });

  it("a plain message has no glyph", () => {
    const text = line(draw({ text: "nothing to scroll here" }));
    expect(text).toContain("  nothing to scroll here");
    expect(text).not.toMatch(/[✓✗]/);
  });
});

describe("a notice standing in for the host's message", () => {
  it("keeps the icon of the message the snapshot still holds", async () => {
    const { withNotice } = await import("../../src/app/../chrome/Chrome.js");
    const chrome = booksChrome({ width: 80 });
    chrome.status.message = { text: "configured repositories are readable", icon: "run_passed", ink: "info" };
    const shown = withNotice(chrome, "configured repositories are readable");
    expect(line(layoutStatus(shown, 80, 0).cells)).toContain("✓ configured repositories are readable");
  });
  it("draws a notice the snapshot does not hold without one", async () => {
    const { withNotice } = await import("../../src/chrome/Chrome.js");
    const chrome = booksChrome({ width: 80 });
    chrome.status.message = { text: "other", icon: "run_passed", ink: "info" };
    expect(line(layoutStatus(withNotice(chrome, "copied"), 80, 0).cells)).not.toContain("✓");
  });
});

describe("a message longer than the room", () => {
  it("is cut hard at the room (width - 1), no ellipsis, as Buffer::set_line cuts it", () => {
    const chrome = booksChrome({ width: 80 });
    chrome.status.message = { text: "x".repeat(200), icon: "run_failed", ink: "info" };
    const text = line(layoutStatus(chrome, 20, 0).cells);
    expect(text).not.toContain("…");
    expect(text.lastIndexOf("x") + 1).toBe(19);
  });
});
