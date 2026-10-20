import React from "react";
import { describe, expect, it } from "vitest";
import { TextLine, textLineString } from "../../src/text/TextLine.js";
import { lineCells } from "../../src/text/cells.js";
import { styledFrameAt } from "../nodes/helpers.js";

// Service text can carry terminal controls; the host strips them from view data (radar-core view::clean). This is the
// second line of defence: whatever reaches a Text, the drawn cells and the frame hold none of them.
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/;
const SGR = /\u001b\[[0-9;]*m/g;

const PAYLOAD = "name\u001b[31m\u0007\u0085\u009b31m\u007fend";
const NASTY = "name\u001b]52;c;eA==\u0007\u001b[2J\u0085\u009b31m\u007fend";

describe("terminal controls in a Text", () => {
  it("never become a cell", () => {
    const cells = lineCells([{ text: PAYLOAD }, { text: NASTY, ink: "bad" }] as never);
    for (const cell of cells) expect(cell.ch, JSON.stringify(cell.ch)).not.toMatch(CONTROL);
    expect(textLineString([{ text: PAYLOAD }] as never)).not.toMatch(CONTROL);
  });

  it("never reach the frame Ink writes, in words, chips, marks or links", () => {
    const text = [
      { text: NASTY },
      { text: NASTY, chip: true, ink: "good" },
      { text: NASTY, marked: true },
      { text: NASTY, link: "https://link.test/\u001b]52;c;x\u0007" },
    ] as never;
    const frame = styledFrameAt(<TextLine text={text} />, 200);
    expect(frame).toContain("name");
    // What is left once the styles Ink itself writes (SGR) and its own hyperlink wrapper are taken away.
    // eslint-disable-next-line no-control-regex
    const left = frame.replace(SGR, "").replace(/\u001b\]8;;[^\u0007\u001b]*(?:\u0007|\u001b\\)/g, "");
    expect(left).not.toMatch(CONTROL);
  });
});
