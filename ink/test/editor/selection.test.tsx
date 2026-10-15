import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import React from "react";
import { describe, expect, it } from "vitest";
import { selectIntent, selectionRange } from "../../src/editor/selection.js";
import { EditorOverlay } from "../../src/editor/EditorOverlay.js";
import { bgOf } from "../../src/text/look.js";
import { styledFrameAt } from "../nodes/helpers.js";
import { parseAnsi } from "../nodes/ansi.js";

const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures/diff");

describe("selecting with the keys", () => {
  const ed = { line: "héllo 日本", caret: 4 };
  it("grows from where the caret was, a code point a step, with the caret at the extent", () => {
    expect(selectIntent(ed, "right")).toEqual({ intent: "edit_select", anchor: 4, extent: 5 });
    expect(selectIntent({ ...ed, selection: [4, 5] }, "right")).toEqual({ intent: "edit_select", anchor: 4, extent: 6 });
    expect(selectIntent(ed, "left")).toEqual({ intent: "edit_select", anchor: 4, extent: 3 });
  });
  it("shrinks back through the anchor, and to the line's ends for home and end", () => {
    expect(selectIntent({ ...ed, caret: 5, selection: [4, 5] }, "left")).toEqual({ intent: "edit_select", anchor: 4, extent: 4 });
    expect(selectIntent(ed, "home")).toEqual({ intent: "edit_select", anchor: 4, extent: 0 });
    expect(selectIntent(ed, "end")).toEqual({ intent: "edit_select", anchor: 4, extent: 8 });
  });
  it("stops at the line's edges", () => {
    expect(selectIntent({ line: "ab", caret: 0 }, "left")).toEqual({ intent: "edit_select", anchor: 0, extent: 0 });
    expect(selectIntent({ line: "ab", caret: 2 }, "right")).toEqual({ intent: "edit_select", anchor: 2, extent: 2 });
  });
  it("paints an ascending range whichever way it was made, and nothing for an empty one", () => {
    expect(selectionRange([5, 2])).toEqual([2, 5]);
    expect(selectionRange([3, 3])).toBeNull();
    expect(selectionRange(null)).toBeNull();
  });
});

describe("the compose editor with a selection, through the real host's view", () => {
  it("draws the selected characters of the typed line on bg.selection and the rest not", () => {
    const view = JSON.parse(fs.readFileSync(path.join(dir, "editor_compose_80x24.json"), "utf8"));
    const body = view.overlay.body;
    const data = { ...body.data, selection: [0, 3] as [number, number] };
    const rows = parseAnsi(styledFrameAt(<EditorOverlay data={data} width={80} areaHeight={20} />, 80));
    const rowText = rows.map((r) => r.map((c) => c.ch).join(""));
    const at = rowText.findIndex((t) => t.includes("héllo"));
    expect(at).toBeGreaterThanOrEqual(0);
    const row = rows[at]!;
    const x = rowText[at]!.indexOf("h");
    const sel = bgOf("bg.selection");
    expect(row.slice(x, x + 3).every((c) => c.bg === sel)).toBe(true);
    expect(row[x + 4]!.bg).not.toBe(sel);
  });
});
