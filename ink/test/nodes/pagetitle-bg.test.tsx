import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import React from "react";
import { describe, expect, it } from "vitest";
import { PageTitleLine } from "../../src/nodes/pagetitle.js";
import { styledFrameAt } from "./helpers.js";
import { parseAnsi } from "./ansi.js";

// Live survey (T22): Ratatui's page title line has the page's bg.base (#0a0812) under every cell, the air before the
// title and the title included; Ink left those two without one.
describe("the page title line's background", () => {
  it("is bg.base on every cell of the row", () => {
    const row = parseAnsi(
      styledFrameAt(
        <PageTitleLine title={[{ text: "Tidy the build", strong: true }]} details={[{ text: "Alice" }]} width={40} base="#0a0812" />,
        40,
      ),
    )[0]!;
    expect(row.slice(0, 40).every((c) => c.bg === "#0a0812")).toBe(true);
  });
});
