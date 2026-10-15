import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import React from "react";
import { describe, expect, it } from "vitest";
import { DiffView, tokenStyle } from "../../src/text/DiffView.js";
import { slotStyle } from "../../src/text/look.js";
import { styledFrameAt } from "../nodes/helpers.js";
import { parseAnsi } from "../nodes/ansi.js";
import type { DiffLine } from "../../src/nodes/view.js";

// Ratatui highlights a diff's code with syntect (ui::diff::paint); Ink has no grammars, so the host sends each line cut
// into runs by token (Node::Diff.painted) and Ink styles them as ui::highlight::Token::style does.
const lines: DiffLine[] = [
  { kind: "Hunk", old: null, new: null, text: "@@ -1,2 +1,2 @@" },
  { kind: "Context", old: 1, new: 1, text: "import a;" },
  { kind: "Added", old: null, new: 2, text: "// done" },
];
const painted = [
  [],
  [
    { text: "import", token: "keyword" },
    { text: " a;", token: "plain" },
  ],
  [{ text: "// done", token: "comment" }],
];
const draw = (over: Partial<React.ComponentProps<typeof DiffView>> = {}) =>
  parseAnsi(
    styledFrameAt(
      <DiffView lines={lines} under={[]} selected={0} top={0} split={false} empty="" width={50} focused height={5} {...over} />,
      50,
    ),
  );

describe("the diff's code colours", () => {
  it("styles each token as Token::style does", () => {
    expect(tokenStyle("keyword")).toMatchObject({ ...slotStyle("accent.primary"), bold: true });
    expect(tokenStyle("comment")).toMatchObject({ ...slotStyle("text.muted"), italic: true });
    expect(tokenStyle("number")).toEqual(slotStyle("data.number"));
    expect(tokenStyle("constant")).toEqual(slotStyle("data.number"));
    expect(tokenStyle("function")).toEqual(slotStyle("glow.cyan"));
    expect(tokenStyle("mark")).toEqual(slotStyle("glow.pink"));
    expect(tokenStyle("type")).toEqual(slotStyle("accent.secondary"));
    expect(tokenStyle("text")).toEqual(slotStyle("accent.tertiary"));
    expect(tokenStyle("plain")).toEqual(slotStyle("text.primary"));
  });

  it("draws the host's runs in their token slots, where without them a line is one colour", () => {
    const col = (rows: ReturnType<typeof draw>, r: number) => rows[r]!.map((c) => c.ch).join("").indexOf("import");
    const withRuns = draw({ painted });
    const c = col(withRuns, 1);
    expect(withRuns[1]![c]).toMatchObject({ ch: "i", fg: slotStyle("accent.primary").color, bold: true });
    expect(withRuns[1]![c + 6]).toMatchObject({ fg: slotStyle("text.primary").color });
    const bare = draw();
    expect(bare[1]![col(bare, 1)]!.fg).toBe(slotStyle("text.primary").color);
    // a comment on an added line keeps the added tint behind its colour
    const cm = withRuns[2]!.map((x) => x.ch).join("").indexOf("//");
    expect(withRuns[2]![cm]).toMatchObject({ fg: slotStyle("text.muted").color, italic: true });
    expect(withRuns[2]![cm]!.bg).toBeDefined();
  });

  it("does not paint a hunk heading", () => {
    expect(draw({ painted, selected: 1 })[0]![1]).toMatchObject({ fg: slotStyle("status.info").color });
  });
});

describe("the diff's scrollbar", () => {
  const many: DiffLine[] = Array.from({ length: 30 }, (_, i) => ({ kind: "Context" as const, old: i + 1, new: i + 1, text: `line ${i}` }));
  const bar = (top: number) =>
    parseAnsi(styledFrameAt(<DiffView lines={many} under={[]} selected={top} top={top} split={false} empty="" width={30} focused height={10} />, 30)).map((r) => r[29]!.ch);
  it("is a track with a thumb that follows the first visible row, and absent when everything fits", () => {
    const atTop = bar(0);
    expect(atTop[0]).toBe("█");
    expect(atTop[9]).toBe("║");
    expect(bar(20).includes("█") && bar(20)[0] === "║").toBe(true);
    const fits = parseAnsi(styledFrameAt(<DiffView lines={many.slice(0, 5)} under={[]} selected={0} top={0} split={false} empty="" width={30} focused height={10} />, 30));
    expect(fits.every((r) => r[29]?.ch !== "█" && r[29]?.ch !== "║")).toBe(true);
  });
});

describe("the diff's scrollbar in a frame", () => {
  it("reports how far it can scroll to the frame, and draws no bar of its own", async () => {
    const { ScrollSink } = await import("../../src/nodes/index.js");
    const many: DiffLine[] = Array.from({ length: 30 }, (_, i) => ({ kind: "Context" as const, old: i + 1, new: i + 1, text: `line ${i}` }));
    const seen: Array<{ max: number; at: number } | null> = [];
    const rows = parseAnsi(
      styledFrameAt(
        <ScrollSink.Provider value={(r) => seen.push(r)}>
          <DiffView lines={many} under={[]} selected={12} top={0} split={false} empty="" width={30} focused height={10} />
        </ScrollSink.Provider>,
        30,
      ),
    );
    const last = seen.filter(Boolean).at(-1)!;
    expect(last.max).toBe(20);
    expect(last.at).toBe(3); // the cursor line 12 is kept in view: first = 12 + 1 - 10
    expect(rows.every((r) => r[29]?.ch !== "█" && r[29]?.ch !== "║")).toBe(true);
  });
});

describe("the inline comment's selection", () => {
  it("is drawn on bg.selection in the field under the diff line, over the real host's inline comment", async () => {
    const { bgOf } = await import("../../src/text/look.js");
    const fs = await import("node:fs");
    const view = JSON.parse(fs.readFileSync("test/fixtures/diff/diff_inline_comment_160x40.json", "utf8"));
    const find = (o: any): any => {
      if (o && typeof o === "object") {
        if (o.node === "diff") return o;
        for (const k of Object.keys(o)) {
          const r = find(o[k]);
          if (r) return r;
        }
      }
    };
    const d = find(view.screen);
    const under = d.under.map((u: any) => (u.input ? { ...u, input: { ...u.input, selection: [1, 4] } } : u));
    const rows = parseAnsi(
      styledFrameAt(<DiffView lines={d.lines} under={under} selected={d.selected} top={d.top} split={false} empty="" width={60} focused height={12} />, 60),
    );
    const text = rows.map((r) => r.map((c) => c.ch).join(""));
    const y = text.findIndex((t) => t.includes("héllo"));
    expect(y).toBeGreaterThanOrEqual(0);
    const x = text[y]!.indexOf("h");
    const sel = bgOf("bg.selection");
    expect(rows[y]!.slice(x + 1, x + 4).every((c) => c.bg === sel)).toBe(true); // é, l, l
    expect(rows[y]![x]!.bg).not.toBe(sel);
  });
});
