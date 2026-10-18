import React from "react";
import { afterEach, describe, expect, it } from "vitest";
import { decodeDetail, detailFactRows, detailHeaderRows, footActionCells, footActionLayout } from "../../src/nodes/detail.js";
import { detailBodyRows } from "../../src/nodes/detailRows.js";
import { detailRows, renderNode } from "../../src/nodes/index.js";
import { plainNode } from "../../src/nodes/plain.js";
import { plainLines } from "../../src/workarea/plain.js";
import { widthOfCells } from "../../src/text/cells.js";
import { applyLook, bgOf, defaultLook, slotStyle } from "../../src/text/look.js";
import { PaneFrame, paneTitle } from "../../src/app/Panes.js";
import { shellRenderNode } from "../../src/app/Body.js";
import type { DetailContent, FootActions, Node } from "../../src/nodes/view.js";
import { frameAt } from "./helpers.js";
import { scrollKey } from "../../src/input/route.js";

const words = (rows: Array<Array<{ ch: string }>>) => rows.map((r) => r.map((c) => c.ch).join(""));
const content: DetailContent = { header: { path: [{ text: "ankh-morpork/watch" }], title: [{ text: "WATCH-12 investigate" }], branch: [{ text: "watch → main" }], author: [{ text: "Sam Vimes" }], age: [{ text: "3d" }] }, facts: [{ label: "Count", value: [{ text: "0" }] }], sections: [{ block: "section", title: "DESCRIPTION", blocks: [{ block: "doc", doc: [{ t: "para", text: [{ text: "A careful investigation in the city." }] }] }] }] };
const fallback: Node = { node: "rich", blocks: [{ block: "line", text: [{ text: "Complete title, path, facts and actions" }] }, { block: "line", text: [{ text: "approve · unavailable" }], key: "a" }] };
const node: Node = { node: "extension", capability: "detail", version: 1, data: content, fallback };
const foot: FootActions = { actions: [{ action: { id: "copy", label: "copy", key: { key: { char: "c" } }, mnemonic_at: 0 }, target: "detail-foot:1:0" }, { action: { id: "approve", label: "approve", key: { key: { char: "a" } }, mnemonic_at: 0, change: "edit" }, disabled: "waiting", target: "detail-foot:1:1" }], menu: { action: { id: "detail.extra_actions", label: "extra actions", key: { key: { char: "x" } }, mnemonic_at: 1 }, target: "detail-foot:1:menu" } };
afterEach(() => applyLook(defaultLook()));

describe("detail extension decoding and complete fallback", () => {
  it("accepts v1 with omitted default arrays, and normalizes empty identity metadata", () => {
    expect(decodeDetail(node)).toEqual(content);
    expect(decodeDetail({ ...node, data: { header: { path: [], title: [], age: [{ text: " \t" }] } } })).toEqual({ header: { path: [], title: [] } });
  });
  it.each([{ version: 2 }, { capability: "other" }, { data: null }, { data: { ...content, sections: [{ block: "doc", doc: [{ t: "para", text: [{ text: "invalid", kind: { link: null, mention: {} } }] }] }] } }, { data: { ...content, facts: null } }, { data: { ...content, sections: null } }, { data: { header: { path: [{ text: null }], title: [] } } }, { data: { ...content, sections: [{ block: "doc", doc: [{ t: "table", head: null, rows: [] }] }] } }, { data: { ...content, sections: [{ block: "doc", doc: [{ t: "para", text: [{ text: "bad", marks: null }] }] }] } }, { data: { header: { path: "bad", title: [] } } }, { data: { ...content, sections: [{ block: "doc", doc: [{ t: "para", text: null }] }] } }, { data: { ...content, facts: [{ label: 0, value: [] }] } }])("rejects unsupported or malformed payloads: %j", (over) => {
    const broken = { ...node, ...over } as Node;
    expect(decodeDetail(broken)).toBeNull();
    expect(frameAt(renderNode(broken, { width: 70, focused: true }), 70)).toContain("Complete title, path, facts and actions");
  });
  it("plain node and workarea readers keep the complete shared fallback with original keys/reasons", () => {
    expect(plainNode(node)).toEqual(["Complete title, path, facts and actions", "approve · unavailable [a]"]);
    expect(plainLines(node)).toEqual(plainNode(node));
    expect(detailRows(node, 30)).toBe(detailBodyRows(content, 30).rows.length);
  });
});

describe("detail body geometry", () => {
  it.each([1, 2, 3, 8, 13, 14, 32, 80])("fills all three shaded header rows at %i cells, preserving complete graphemes", (width) => {
    applyLook(defaultLook("off"));
    const rows = detailHeaderRows(content, width);
    expect(rows).toHaveLength(3);
    for (const row of rows) {
      expect(widthOfCells(row)).toBe(width);
      expect(row.every((c) => c.s.backgroundColor === bgOf("bg.overlay"))).toBe(true);
    }
    expect(rows[1]!.filter((c) => c.ch.trim() && c.ch !== "…").every((c) => c.s.bold)).toBe(true);
  });
  it("keeps age, author separator and branch in disjoint budgets", () => {
    expect(words(detailHeaderRows(content, 32))[2]).toBe(" watch → main    Sam Vimes · 3d ");
    expect(words(detailHeaderRows(content, 8))[2]!.trim()).toContain("3d");
    expect(words(detailHeaderRows(content, 8))[2]!.length).toBe(8);
  });
  it("filters absent facts, retains zero, aligns wrapping and explicit multiline continuation", () => {
    const facts = [{ label: "Count", value: [{ text: "0" }] }, { label: "Unused", value: [{ text: "\t " }] }, { label: "Owner", value: [{ text: "Sam\nVimes" }] }];
    expect(words(detailFactRows(facts, 24)).map((r) => r.trimEnd())).toEqual([" Count  0", " Owner  Sam", "        Vimes"]);
    expect(words(detailFactRows(facts, 12)).map((r) => r.trimEnd())).toEqual([" Count", " 0", " Owner", " Sam", " Vimes"]);
    const wide = detailFactRows([{ label: "界e\u0301", value: [{ text: "界界界 a b c d e" }] }], 16);
    expect(wide.every((row) => widthOfCells(row) === 16)).toBe(true);
    expect(words(wide).join(" ")).toContain("e\u0301");
  });
  it("retains bounded documents, file/link text, names and section-body inset", () => {
    const data: DetailContent = { header: { path: [], title: [] }, sections: [{ block: "section", title: "FILES", blocks: [{ block: "doc", most: 2, doc: [{ t: "media", name: "watch report", url: "https://example.test/file" }, { t: "para", text: [{ text: "one two three four five six seven eight nine" }] }] }] }] };
    const laid = detailBodyRows(data, 32);
    const text = words(laid.rows);
    expect(text[4]).toContain(" FILES ─");
    expect(text[5]).toMatch(/^   ▤ file watch report/);
    expect(text.at(-1)).toBe("   …");
    expect(laid.rows.every((row) => widthOfCells(row) <= 32)).toBe(true);
  });
  it("inline targets follow the same wrapped Unicode rows and stay inside visible cells", () => {
    const data: DetailContent = { header: { path: [], title: [] }, sections: [{ block: "line", text: [{ text: "a " }, { text: "界e\u0301 link", target: "link:watch" }] }] };
    const { rows, hits } = detailBodyRows(data, 12);
    expect(hits).not.toHaveLength(0);
    expect(hits.every((hit) => hit.dx >= 1 && hit.dx + hit.w <= 11 && hit.dy >= 4 && hit.dy < rows.length)).toBe(true);
    expect(words(rows).join("\n")).toContain("界e\u0301");
  });
  it("plain fallback keeps link destinations and list/task state", () => {
    const data: Node = { node: "rich", blocks: [
      { block: "line", text: [{ text: "report", link: "https://example.test/report" }] },
      { block: "doc", doc: [{ t: "list", items: [
        { marker: "todo", blocks: [{ t: "para", text: [{ text: "review" }] }] },
        { marker: "done", blocks: [{ t: "para", text: [{ text: "read" }] }] },
        { marker: { number: 3 }, blocks: [{ t: "para", text: [{ text: "send" }] }] },
      ] }] },
    ] };
    expect(plainNode(data)).toEqual(["report <https://example.test/report>", "☐ review", "☑ read", "3. send"]);
  });
  it("plain fallback retains explicit hard breaks, including quote and task continuation", () => {
    const text = [{ text: "first" }, { text: "", kind: "break" as const }, { text: "last" }];
    const data: Node = { node: "rich", blocks: [{ block: "doc", doc: [
      { t: "para", text },
      { t: "quote", blocks: [{ t: "para", text }] },
      { t: "list", items: [{ marker: "todo", blocks: [{ t: "para", text }] }] },
    ] }] };
    expect(plainNode(data)).toEqual(["first", "last", "│ first", "│ last", "☐ first", "  last"]);
    expect(plainLines(data)).toEqual(plainNode(data));
  });
  it("frontend scrolling treats typed detail as Rich and preserves zero-height geometry", () => {
    const calls: string[] = [];
    const sc = { kind: "rich" as const, focused: true, at: () => 1, max: () => 4, key: (k: string) => { calls.push(k); return true; }, page: () => true };
    expect(scrollKey({ panes: [{ pane: "detail", body: node }] }, "detail", { key: "down" }, [sc])).toBe("done");
    expect(calls).toEqual(["down"]);
    expect(scrollKey({ panes: [{ pane: "detail", body: { ...node, version: 99 } }] }, "detail", { key: "down" }, [sc])).toBe("done");
    expect(calls).toEqual(["down", "down"]);
    expect(frameAt(renderNode(node, { width: 20, height: 0, focused: true }), 20)).toBe("");
  });
});

describe("pinned foot controls and pane labels", () => {
  it.each([0, 1, 2, 3, 4, 12, 30, 80])("reserves exactly two rows only with body space at height %i", (height) => {
    const laid = footActionLayout(foot, 30, height);
    expect(laid.rows.length).toBe(height < 3 ? 0 : 2);
    if (height < 3) expect(laid.hits).toEqual([]);
    for (const hit of laid.hits) expect(hit.dx >= 0 && hit.dx + hit.w <= 30 && hit.dy === 1).toBe(true);
    expect(laid.hits.some((hit) => hit.target.endsWith(":1"))).toBe(false);
  });
  it("reserves the menu, fits only complete action prefixes, leaves separator cells unclickable", () => {
    for (const width of [1, 2, 7, 13, 18, 30]) {
      const laid = footActionLayout(foot, width, 3);
      expect(laid.rows.every((row) => widthOfCells(row) === width)).toBe(true);
      const menu = laid.hits.find((hit) => hit.target.endsWith(":menu"))!;
      expect(menu.dx + menu.w).toBe(width);
      for (const hit of laid.hits.filter((h) => h !== menu)) expect(hit.dx + hit.w + 2).toBeLessThanOrEqual(menu.dx);
      expect(words(laid.rows)[1]).not.toContain("cop…");
    }
    expect(words(footActionLayout(foot, 1, 3).rows)[1]).toBe("x");
    const disabledEmpty = { ...foot, actions: [{ ...foot.actions![0]!, disabled: "" }] };
    expect(footActionLayout(disabledEmpty, 40, 3).hits.map((h) => h.target)).toEqual(["detail-foot:1:menu"]);
  });
  it("uses supplied Unicode/Shift metadata, truthful plain keys and mode muting without guessing", () => {
    const action = { action: { id: "watch", label: "e\u0301dit", key: { key: { char: "E" } }, mnemonic_at: 0, mnemonic_shift: true, change: "edit" as const }, target: "detail-foot:1:0" };
    const cells = footActionCells(action, false, "read_only");
    expect(words([cells])[0]).toBe("shift e\u0301dit");
    expect(cells.find((c) => c.ch === "e\u0301")?.s.underline).toBe(true);
    expect(cells.every((c) => c.s.color === slotStyle("text.muted").color && !c.s.bold)).toBe(true);
    expect(words([footActionCells(action, true)])[0]).toBe("e\u0301dit [E]");
    expect(words([footActionCells({ action: { id: "copy", label: "copy", key: { key: { char: "c" } } } })])[0]).toBe("copy [c]");
  });
  it("host mapping keeps hidden Detail=3, single pages=1, setup/auxiliary panes unnumbered", () => {
    applyLook(defaultLook("off"));
    expect(paneTitle({ pane: "detail", title: "message" }, [{ pane: "detail", key: "3" }])).toBe("3 message");
    expect(paneTitle({ pane: "middle", title: "ticket" }, [{ pane: "middle", key: "1" }])).toBe("1 ticket");
    expect(paneTitle({ pane: "detail", title: "setup" }, [])).toBe("setup");
    expect(paneTitle({ pane: "detail", title: "auxiliary" }, [{ pane: "middle", key: "1" }])).toBe("auxiliary");
  });
  it("an actual three-cell frame keeps its one-cell body and truthful x control", () => {
    const hits: Array<{ x: number; y: number; w: number; h: number; target: string }> = [];
    const text = frameAt(<PaneFrame pane={{ pane: "detail", title: "tiny", body: node, foot_actions: foot }} width={3} height={5} focused cx={{ width: 3, height: 5, actions: [], registry: { add: (hit) => hits.push(hit) } }} renderNode={shellRenderNode} />, 3);
    expect(text.split("\n")[3]).toBe("│x│");
    expect(hits).toEqual([{ x: 1, y: 3, w: 1, h: 1, target: "detail-foot:1:menu" }]);
  });
  it("a tiny frame does not force body rows or register footer hits", () => {
    const hits: unknown[] = [];
    frameAt(<PaneFrame pane={{ pane: "detail", title: "tiny", body: node, foot_actions: foot, head: [{ text: "head" }] }} width={20} height={3} focused cx={{ width: 20, height: 3, actions: [], registry: { add: (hit) => hits.push(hit) } }} renderNode={shellRenderNode} />, 20);
    expect(hits).toEqual([]);
  });
});
