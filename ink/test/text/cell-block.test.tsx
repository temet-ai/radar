import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import React from "react";
import { Text as InkText } from "ink";
import { describe, expect, it } from "vitest";
import {
  BgBox,
  CellBlock,
  CellLine,
  Rows,
  cellsOf,
  type Cell,
} from "../../src/text/cells.js";
import { styledFrameAt } from "../nodes/helpers.js";
import { parseAnsi } from "../nodes/ansi.js";

// A list, table, tree or log draws its rows as ONE Ink <Text> of pre-styled runs (T33), not one <Text> per run: the
// cells that come out must be the cells the nested <Text>s drew.
const red = { color: "#ff0000", bold: true };
const onBlue = { backgroundColor: "#0000ff", dim: true, underline: true };
const rows: Cell[][] = [
  [
    ...cellsOf("ab", red),
    ...cellsOf("cd", {}),
    ...cellsOf("é界", onBlue),
    ...cellsOf("  ", { inverse: true, italic: true }),
  ],
  [...cellsOf("plain row", {})],
  [
    ...cellsOf("x", {
      color: "#00ff00",
      backgroundColor: "#101010",
      strike: true,
    }),
    ...cellsOf("      ", {}),
  ],
];

const nested = (cells: Cell[]) => {
  const runs: { s: Cell["s"]; t: string }[] = [];
  for (const c of cells) {
    const l = runs[runs.length - 1];
    if (l && JSON.stringify(l.s) === JSON.stringify(c.s)) l.t += c.ch;
    else runs.push({ s: c.s, t: c.ch });
  }
  return (
    <InkText wrap="truncate">
      {runs.map((r, i) => (
        <InkText
          key={i}
          color={r.s.color}
          backgroundColor={r.s.backgroundColor}
          bold={r.s.bold}
          dimColor={r.s.dim}
          italic={r.s.italic}
          underline={r.s.underline}
          strikethrough={r.s.strike}
          inverse={r.s.inverse}
        >
          {r.t}
        </InkText>
      ))}
    </InkText>
  );
};

describe("CellBlock", () => {
  it("draws the cells nested <Text> runs drew", () => {
    const old = parseAnsi(
      styledFrameAt(
        <>
          {rows.map((r, i) => (
            <React.Fragment key={i}>{nested(r)}</React.Fragment>
          ))}
        </>,
        20,
        5,
      ),
    );
    const now = parseAnsi(styledFrameAt(<CellBlock lines={rows} />, 20, 5));
    expect(now).toEqual(old);
    expect(now[0]![0]).toMatchObject({ ch: "a", fg: "#ff0000", bold: true });
  });
  it("CellLine is a block of one", () => {
    const one = parseAnsi(styledFrameAt(<CellLine cells={rows[0]!} />, 20, 2));
    const blk = parseAnsi(
      styledFrameAt(<CellBlock lines={[rows[0]!]} />, 20, 2),
    );
    expect(one).toEqual(blk);
  });
});

describe("CellBlock under a Box with a background", () => {
  it("keeps the background a cell has no colour of its own for, as nested <Text> runs inherit it", async () => {
    const { Box } = await import("ink");
    const plainRows: Cell[][] = [
      [...cellsOf("ab", { color: "#ff0000" }), ...cellsOf("cd", {})],
    ];
    const old = parseAnsi(
      styledFrameAt(
        <Box backgroundColor="#202020" width={10}>
          {nested(plainRows[0]!)}
        </Box>,
        10,
        2,
      ),
    );
    const now = parseAnsi(
      styledFrameAt(
        <BgBox backgroundColor="#202020" width={10}>
          <CellBlock lines={plainRows} />
        </BgBox>,
        10,
        2,
      ),
    );
    expect(now).toEqual(old);
    expect(now[0]!.slice(0, 4).map((c) => c.bg)).toEqual(
      Array(4).fill("#202020"),
    );
  });
});

describe("CellBlock hands Ink plain ASCII", () => {
  const rows2: Cell[][] = [
    [
      ...cellsOf("│ ", { color: "#ff0000" }),
      ...cellsOf("界é…", { bold: true }),
      ...cellsOf(" end", {}),
    ],
  ];
  it("writes characters that are not printable ASCII as `?` in the Text, and draws them as before", async () => {
    const { frameAt } = await import("../nodes/helpers.js");
    const old = parseAnsi(styledFrameAt(<>{nested(rows2[0]!)}</>, 20, 2));
    const now = parseAnsi(styledFrameAt(<CellBlock lines={rows2} />, 20, 2));
    expect(now).toEqual(old);
    expect(now[0]!.map((c) => c.ch).join("")).toContain("│ 界");
    void frameAt;
  });
  it("the transformer Ink's own get() applies puts the characters back", () => {
    const out = new Rows()
      .line()
      .put("│ ", { color: "#ff0000" })
      .put("界é…", { bold: true })
      .put(" end", {})
      .element() as React.ReactElement<{
      internal_transform: (l: string, i: number) => string;
      children: string;
    }>;
    const text = out.props.children;
    expect(text).toBe("? ???? end"); // │ … é one cell each, 界 two: all written as `?` of their width
    expect(
      out.props.internal_transform(text, 0).replace(/\x1b\[[0-9;]*m/g, ""),
    ).toBe("│ 界é… end");
  });
});
