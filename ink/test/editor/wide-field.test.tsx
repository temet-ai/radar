import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import React from "react";
import { describe, expect, it } from "vitest";
import { cutCells, InputField } from "../../src/editor/InputField.js";
import { textWidth } from "../../src/editor/graphemes.js";
import { frameAt } from "../nodes/helpers.js";

// Found live (real MR diff, a comment typed as `helé日本`): the field row was padded by characters, so wide characters
// pushed it two cells past its width and Ink cut it with "…". the reference's set_stringn cuts and pads by cells.
describe("a field row with wide characters", () => {
  it("is exactly its width in cells, with no ellipsis, whether the caret is inside, at the end or the text overflows", () => {
    for (const [text, caret] of [
      ["helé日本", 4],
      ["helé日本", 7],
      ["日本語日本語日本語", 2],
      ["日本語日本語日本語", 9],
      ["abc", 1],
    ] as const) {
      const row = frameAt(<InputField text={text} caret={caret} width={12} />, 40);
      expect(row, `${text}@${caret}`).not.toContain("…");
      expect(textWidth(row.replace(/\s+$/, "")), `${text}@${caret}`).toBeLessThanOrEqual(12);
    }
  });

  it("cuts a wide character that would cross the edge out whole", () => {
    expect(cutCells("a日本", 2)).toBe("a");
    expect(cutCells("a日本", 3)).toBe("a日");
    expect(cutCells("abc", 10)).toBe("abc");
  });
});

describe("a selection in the field", () => {
  it("is drawn on bg.selection over the selected characters, the caret keeping its own look", async () => {
    const { styledFrameAt } = await import("../nodes/helpers.js");
    const { parseAnsi } = await import("../nodes/ansi.js");
    const { bgOf } = await import("../../src/text/look.js");
    const rows = parseAnsi(styledFrameAt(<InputField text="hello world" caret={5} selection={[1, 5]} width={20} />, 20));
    const row = rows[0]!;
    const sel = bgOf("bg.selection");
    expect(row.slice(1, 5).every((c) => c.bg === sel)).toBe(true);
    expect(row[0]!.bg).not.toBe(sel);
    expect(row[6]!.bg).not.toBe(sel);
    expect(row[5]!.inverse).toBe(true);
  });
});
