import React from "react";
import { Box } from "ink";
import { describe, expect, it } from "vitest";
import { RichBlocks } from "../../src/text/RichDoc.js";
import { styledFrameAt } from "../nodes/helpers.js";
import type { RichBlock } from "../../src/nodes/view.js";

// ui::rich::Layout::sep: a blank row between blocks that follow each other (ui::rich's own test expects
// ["▣ image screen shot.png https://x/f/1", "", "▤ file notes.csv"]); none before the first.
const para = (text: string): RichBlock => ({ t: "para", text: [{ text }] }) as unknown as RichBlock;
const media = (name: string, image: boolean, url = ""): RichBlock =>
  ({ t: "media", image, name, url, src: "", size: null }) as unknown as RichBlock;

// helpers.frameAt collapses blank rows (its /\s+$/gm eats newlines), which is the thing under test: use the styled frame.
// eslint-disable-next-line no-control-regex
const rowsOf = (el: React.ReactElement, columns: number) => styledFrameAt(el, columns).replace(/\u001b\[[0-9;?]*[A-Za-z]/g, "").split("\n").map((r) => r.trimEnd());

describe("blocks of a rich document", () => {
  it("are a blank row apart", () => {
    const rows = rowsOf(<Box flexDirection="column"><RichBlocks blocks={[para("hello"), media("screen shot.png", true, "https://x/f/1"), media("notes.csv", false)]} /></Box>, 60);
    expect(rows).toEqual(["hello", "", "▣ image screen shot.png https://x/f/1", "", "▤ file notes.csv"]);
  });
});
