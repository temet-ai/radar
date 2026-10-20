import React from "react";
import { describe, expect, it } from "vitest";
import { render } from "ink-testing-library";
import { MediaBlock, mediaLine } from "../../src/media/index.js";

// The strings are those of ui::rich's test media_without_graphics_is_a_line_naming_it_and_its_link.
const cases = [
  [{ image: true, name: "screen shot.png", url: "https://x/f/1", src: "https://files/1", size: [800, 600] as [number, number] }, "▣ image screen shot.png https://x/f/1"],
  [{ name: "notes.csv" }, "▤ file notes.csv"],
  [{ image: true }, "▣ image"],
] as const;

describe("a document's media block, as the reference draws it without graphics", () => {
  for (const [m, line] of cases) {
    it(line, () => {
      expect(mediaLine(m)).toBe(line);
      const r = render(<MediaBlock media={m} plain />);
      expect(r.lastFrame()).toBe(line);
      r.unmount();
    });
  }
});

describe("the block is one row when it fits", () => {
  it("an image with a long url stays whole on one row when the width holds it (ink-testing-library: 100 columns)", () => {
    const m = { image: true, name: "dashboard.png", url: "https://example.com/design/dashboard.png" };
    const r = render(<MediaBlock media={m} plain />);
    expect(r.lastFrame()).toBe(mediaLine(m));
    r.unmount();
  });
});
