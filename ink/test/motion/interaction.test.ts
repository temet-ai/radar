// Owner-local state-machine tests: tiny cell fixtures, a manual clock and a counted
// parser. No React/host render, wall clock, timers, filesystem or ambient settings.
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { parseFrame, writeFrame } from "../../src/motion/frame.js";
import { Grid, copyCell } from "../../src/motion/grid.js";
import { contextBounded, frameMetadataBounded, pathBytes } from "../../src/motion/interaction-bounds.js";
import { MAX_BASELINE_ANSI, perimeter } from "../../src/motion/interaction-effects.js";
import { Interactions, foldArea, type MotionScene, type PaneMark, type SceneMarks, type TreeMark } from "../../src/motion/interaction.js";
import { applyLook, currentLook, type Look } from "../../src/text/look.js";

const W = 18, H = 10;
const TREE = { x: 1, y: 2, w: 4, h: 7 };
const SUFFIX = { x: 1, y: 3, w: 4, h: 6 };
const context: NonNullable<MotionScene["context"]> = { kind: "screen", plugin: "watch", screen: "patrol", page: null, depth: 0, tab: null };
type Interaction = NonNullable<MotionScene["interaction"]>;
type Fold = NonNullable<Interaction["fold"]>;

function look(tier: Look["tier"] = "truecolor"): Look {
  return { tier, icons: {}, iconWidth: 0, slots: {
    "status.error": "#010305", "data.number": "#0b0d0f", "status.warning": "#151719",
    "status.success": "#1f2123", "glow.cyan": "#292b2d", "status.info": "#333537", "glow.purple": "#3d3f41",
  } };
}
let savedLook: Look;
beforeEach(() => { savedLook = currentLook(); applyLook(look()); });
afterEach(() => applyLook(savedLook));

function tree(open: boolean): TreeMark {
  const descendants = ["A", "B", "C", "D"].map((id) => ["watch", id]);
  const tails = ["u", "v", "w", "x", "y", "z"].map((id) => [id]);
  const paths = [["watch"], ...(open ? [...descendants, ...tails.slice(0, 2)] : tails)];
  return { pane: "nav", slot: "body", area: { ...TREE }, rows: paths.map((path, i) => ({ path, y: TREE.y + i })), open: open ? [["watch"]] : [] };
}
function panes(owner = "nav"): PaneMark[] {
  return [
    { pane: "nav", area: { x: 0, y: 0, w: 7, h: H }, owns: owner === "nav", keep: [] },
    { pane: "detail", area: { x: 9, y: 0, w: 9, h: H }, owns: owner === "detail", keep: [] },
  ];
}
function frame(open = false, cursor = 10, owner = "nav", extra: Partial<MotionScene> = {}): SceneMarks {
  return { scene: { cursor, context, blocked: false, local: false, ...extra }, panes: panes(owner), trees: [tree(open)] };
}
function fold(open: boolean, originSeq: number | null = 40, originCursor: number | null = 10): Fold {
  return { context, pane: "nav", slot: "body", path: ["watch"], before_open: !open, after_open: open, origin_seq: originSeq, origin_cursor: originCursor };
}
function ack(seq: number | null, change?: Fold): Interaction {
  return { context, processed_seq: seq, origin_seq: seq, ...(change ? { fold: change } : {}) };
}
function output(open = false, owner = "nav"): string {
  const grid = new Grid(W, H);
  grid.cells.forEach((cell) => Object.assign(cell, { ch: ".", fg: 0x182838, bg: 0x08090a, attr: "1;3" }));
  for (const pane of panes(owner)) for (const [x, y] of perimeter(pane.area))
    grid.at(x, y).fg = pane.owns ? 0x6080a0 : 0x102030;
  const rows = open ? "PABCDuv" : "Puvwxyz";
  [...rows].forEach((ch, i) => {
    for (let x = 0; x < TREE.w; x++) Object.assign(grid.at(TREE.x + x, TREE.y + i), { ch, fg: ch.codePointAt(0)!, bg: 0x303132, attr: "1;3;4" });
  });
  return writeFrame(grid, "", [grid.area]);
}
const suffixLabels = (grid: Grid) => Array.from({ length: SUFFIX.h }, (_, y) => grid.at(SUFFIX.x, SUFFIX.y + y).ch).join("");
function rig() {
  const time = { now: 0 }, parsed: string[] = [];
  const controller = new Interactions(() => time.now, (ansi, width, height) => {
    parsed.push(ansi);
    return parseFrame(ansi, width, height);
  });
  const observe = (marks: SceneMarks, ansi = output(marks.trees[0]?.open.length !== 0, marks.panes.find((pane) => pane.owns)?.pane), enabled = true) =>
    controller.observe(ansi, W, H, marks, enabled);
  const draw = (ansi = output(), phase: "fold" | "focus" | "all" = "all") => {
    const grid = parseFrame(ansi, W, H);
    return { grid, areas: controller.draw(grid, phase) };
  };
  const input = (seq = 40, cursor = 10) => { controller.input(); controller.intent(seq, cursor); };
  return { controller, time, parsed, observe, draw, input };
}

// Include four-byte and two-byte characters so code-unit counts cannot pass byte caps.
const utf8 = (bytes: number) => "🦉".repeat(Math.floor(bytes / 4)) + "é".repeat(Math.floor(bytes % 4 / 2)) + (bytes % 2 ? "x" : "");
type MetadataLimit = { name: string; atLimit: (marks: SceneMarks) => void; exceed: (marks: SceneMarks) => void; largeViewport?: boolean };
const metadataLimits: readonly MetadataLimit[] = [
  {
    name: "16,384 context field bytes",
    atLimit: (marks) => {
      const route = { ...context, plugin: utf8(4096), screen: utf8(4096), page: utf8(8192) };
      marks.scene.context = route;
      if (marks.scene.interaction) marks.scene.interaction.context = route;
    },
    exceed: (marks) => { marks.scene.context = { ...context, plugin: utf8(4096), screen: utf8(4096), page: utf8(8193) }; },
  },
  {
    name: "64 screen levels",
    atLimit: (marks) => {
      marks.scene.context = { ...context, depth: 64 };
      if (marks.scene.interaction) marks.scene.interaction.context = marks.scene.context;
    },
    exceed: (marks) => { marks.scene.context = { ...context, depth: 65 }; },
  },
  {
    name: "32 pane marks",
    atLimit: (marks) => { marks.panes = [...marks.panes, ...Array.from({ length: 30 }, (_, i) => ({ pane: `watch-${i}`, area: { x: 1, y: 1, w: 1, h: 1 }, owns: false, keep: [] }))]; },
    exceed: (marks) => { marks.panes = [...marks.panes, { pane: "watch-extra", area: { x: 1, y: 1, w: 1, h: 1 }, owns: false, keep: [] }]; },
  },
  {
    name: "16 tree marks",
    atLimit: (marks) => { marks.trees = [...marks.trees, ...Array.from({ length: 15 }, (_, i) => ({ pane: "nav", slot: `watch-${i}`, area: { x: 1, y: 1, w: 1, h: 1 }, rows: [], open: [] }))]; },
    exceed: (marks) => { marks.trees = [...marks.trees, { pane: "nav", slot: "watch-extra", area: { x: 1, y: 1, w: 1, h: 1 }, rows: [], open: [] }]; },
  },
  {
    name: "64 protected rectangles per pane",
    atLimit: (marks) => { marks.panes.forEach((pane) => { pane.keep = Array.from({ length: 64 }, () => ({ x: pane.area.x + 1, y: 1, w: 1, h: 1 })); }); },
    exceed: (marks) => { const pane = marks.panes[0]!; pane.keep = [...pane.keep, { x: 1, y: 1, w: 1, h: 1 }]; },
  },
  {
    name: "1,024 pane-key bytes",
    atLimit: (marks) => { marks.panes[0]!.pane = utf8(1024); },
    exceed: (marks) => { marks.panes[0]!.pane += "x"; },
  },
  {
    name: "1,024 tree-pane-key bytes",
    atLimit: (marks) => { marks.trees[0]!.pane = utf8(1024); },
    exceed: (marks) => { marks.trees[0]!.pane += "x"; },
  },
  {
    name: "1,024 tree-slot bytes",
    atLimit: (marks) => { marks.trees[0]!.slot = utf8(1024); },
    exceed: (marks) => { marks.trees[0]!.slot += "x"; },
  },
  {
    name: "64 row-path components",
    atLimit: (marks) => { marks.trees[0]!.rows = [{ path: Array.from({ length: 64 }, () => "é"), y: TREE.y }]; marks.trees[0]!.open = []; },
    exceed: (marks) => { marks.trees[0]!.rows = [{ path: [...marks.trees[0]!.rows[0]!.path, ""], y: TREE.y }]; },
  },
  {
    name: "4,096 row-path bytes",
    atLimit: (marks) => { marks.trees[0]!.rows = [{ path: [utf8(4096)], y: TREE.y }]; marks.trees[0]!.open = []; },
    exceed: (marks) => { marks.trees[0]!.rows = [{ path: [utf8(4097)], y: TREE.y }]; },
  },
  {
    name: "64 open-path components",
    atLimit: (marks) => { marks.trees[0]!.open = [Array.from({ length: 64 }, () => "é")]; },
    exceed: (marks) => { marks.trees[0]!.open = [[...marks.trees[0]!.open[0]!, ""]]; },
  },
  {
    name: "4,096 open-path bytes",
    atLimit: (marks) => { marks.trees[0]!.open = [[utf8(4096)]]; },
    exceed: (marks) => { marks.trees[0]!.open = [[utf8(4097)]]; },
  },
  {
    name: "65,536 row and open path bytes across trees",
    atLimit: (marks) => {
      marks.trees = [0, 1].map((index) => {
        const rows = Array.from({ length: 4 }, (_, i) => ({ path: [utf8(4095) + i], y: TREE.y + i }));
        return { pane: "nav", slot: `watch-${index}`, area: { ...TREE, h: 4 }, rows, open: rows.map((row) => row.path) };
      });
    },
    exceed: (marks) => { marks.trees = [...marks.trees, { pane: "nav", slot: "watch-extra", area: { ...TREE, h: 1 }, rows: [{ path: ["x"], y: TREE.y }], open: [] }]; },
  },
  {
    name: "one row per tree-area cell height",
    atLimit: () => {},
    exceed: (marks) => { const mark = marks.trees[0]!; mark.rows = [...mark.rows, { path: ["watch-extra"], y: TREE.y + TREE.h }]; },
  },
  {
    name: "one open path per visible tree row",
    atLimit: (marks) => { marks.trees[0]!.open = marks.trees[0]!.rows.map((row) => row.path); },
    exceed: (marks) => { marks.trees[0]!.open = [...marks.trees[0]!.open, ["watch-extra"]]; },
  },
  {
    name: "unique pane keys",
    atLimit: () => {},
    exceed: (marks) => { marks.panes = [...marks.panes, { ...marks.panes[0]! }]; },
  },
  {
    name: "unique tree pane and slot pairs",
    atLimit: (marks) => { marks.trees = [marks.trees[0]!, { ...marks.trees[0]!, slot: "watch-other" }, { ...marks.trees[0]!, pane: "detail" }]; },
    exceed: (marks) => { marks.trees = [...marks.trees, { ...marks.trees[0]! }]; },
  },
  {
    name: "65,536 aggregate perimeter cells",
    atLimit: (marks) => { marks.panes = marks.panes.map((pane) => ({ ...pane, area: { x: 0, y: 0, w: 32768, h: 1 } })); marks.trees = []; },
    exceed: (marks) => { marks.panes[1]!.area.w++; },
    largeViewport: true,
  },
];

describe("interaction metadata budgets", () => {
  test("context limits sum UTF-8 fields for every route kind", () => {
    const pairs: Array<readonly [NonNullable<MotionScene["context"]>, NonNullable<MotionScene["context"]>]> = [
      [{ ...context, plugin: utf8(4096), screen: utf8(4096), page: utf8(8192), depth: 64 }, { ...context, plugin: utf8(4096), screen: utf8(4096), page: utf8(8193), depth: 64 }],
      [{ kind: "settings", plugin: utf8(8192), section: utf8(8192) }, { kind: "settings", plugin: utf8(8192), section: utf8(8193) }],
      [{ kind: "setup", step: utf8(4096), plugin: utf8(4096), section: utf8(8192) }, { kind: "setup", step: utf8(4096), plugin: utf8(4096), section: utf8(8193) }],
    ];
    for (const [atLimit, overLimit] of pairs) {
      expect(contextBounded(atLimit)).toBe(true);
      expect(contextBounded(overLimit)).toBe(false);
    }
    expect(contextBounded({ kind: "history" })).toBe(true);
    expect(contextBounded({ kind: "settings", plugin: null, section: utf8(16384) })).toBe(true);
    expect(contextBounded({ kind: "setup", step: utf8(16384), plugin: null, section: null })).toBe(true);
    expect(contextBounded({ ...context, depth: 64 })).toBe(true);
    expect(contextBounded({ ...context, depth: 65 })).toBe(false);
  });

  test("path limits count UTF-8 bytes across components and include empty components", () => {
    const atLimit = Array.from({ length: 64 }, () => utf8(64));
    expect(pathBytes([])).toBe(0);
    expect(pathBytes([utf8(4096)])).toBe(4096);
    expect(pathBytes(atLimit)).toBe(4096);
    expect(pathBytes([...atLimit, ""])).toBeUndefined();
    expect(pathBytes([...atLimit.slice(0, -1), utf8(65)])).toBeUndefined();
    expect(pathBytes([utf8(4097)])).toBeUndefined();
  });

  test("acknowledgement and fold metadata accept their exact UTF-8 field limits", () => {
    const marks = frame(false, 11, "detail", { interaction: ack(40, fold(true)) });
    marks.scene.interaction!.context = { kind: "setup", step: utf8(16384), plugin: null, section: null };
    const change = marks.scene.interaction!.fold!;
    change.context = { kind: "settings", plugin: utf8(8192), section: utf8(8192) };
    change.slot = utf8(1024);
    change.path = Array.from({ length: 64 }, () => utf8(64));
    expect(frameMetadataBounded(marks)).toBe(true);
  });

  test.each(metadataLimits)("accepts exactly $name and rejects the next value", ({ atLimit, exceed }) => {
    const marks = frame();
    atLimit(marks);
    expect(frameMetadataBounded(marks)).toBe(true);
    exceed(marks);
    expect(frameMetadataBounded(marks)).toBe(false);
  });

  test.each(metadataLimits.filter((limit) => !limit.largeViewport))("$name still permits a real focus transfer", ({ atLimit }) => {
    const r = rig(), before = frame(), after = frame(false, 11, "detail", { interaction: ack(40) });
    atLimit(before); atLimit(after);
    r.observe(before, output(false, "nav")); r.input();
    r.observe(after, output(false, "detail"));
    expect(r.parsed).toEqual([output(false, "nav")]);
    expect(r.draw(output(false, "detail")).areas).toHaveLength(2);
  });
});

describe("fold suffix identity", () => {
  test("opening and closing select only the rows below the unchanged full-path parent", () => {
    expect(foldArea(tree(false), tree(true), fold(true))).toEqual({ area: SUFFIX, descendants: 4 });
    expect(foldArea(tree(true), tree(false), fold(false))).toEqual({ area: SUFFIX, descendants: 4 });
  });

  test("same labels or counts cannot replace full paths, route slots, open state or row geometry", () => {
    const before = tree(false), now = tree(true), change = fold(true);
    const wrongTail = tree(true); wrongTail.rows = wrongTail.rows.map((row, i) => i === 5 ? { ...row, path: ["v"] } : row);
    const wrapped = tree(true); wrapped.rows = wrapped.rows.map((row, i) => i === 2 ? { ...row, y: row.y + 1 } : row);
    const foreignParent = tree(true); foreignParent.rows = foreignParent.rows.map((row, i) => i === 0 ? { ...row, path: ["other", "watch"] } : row);
    for (const invalid of [
      { ...now, area: { ...now.area, w: 3 } }, { ...now, pane: "detail" },
      { ...now, slot: "settings_nav" }, { ...now, open: [] }, wrongTail, wrapped, foreignParent,
    ]) expect(foldArea(before, invalid, change)).toBeUndefined();
    expect(foldArea(before, now, { ...change, before_open: true })).toBeUndefined();
    expect(foldArea(before, now, { ...change, path: ["other", "watch"] })).toBeUndefined();
    expect(foldArea(before, before, change), "a changed flag without actual descendants is insufficient").toBeUndefined();
  });
});

describe("bounded interaction controller", () => {
  test("idle snapshots, labels and background ownership changes never parse ANSI", () => {
    const r = rig();
    r.observe(frame(), "not even ANSI");
    r.observe(frame(false, 11), "new labels");
    r.observe(frame(false, 12, "detail"), "background owner transfer");
    expect(r.parsed).toEqual([]);
    expect(r.controller.running).toBe(false);
    expect(r.draw().areas).toEqual([]);
  });

  test("one accepted fold parses the two clean sources once and retains real old cells", () => {
    const r = rig(), expanded = output(true), compact = output(false);
    r.observe(frame(true), expanded);
    r.input();
    r.observe(frame(false, 11, "nav", { interaction: ack(40, fold(false)) }), compact);
    expect(r.parsed).toEqual([expanded, compact]);
    for (const [ms, expected] of [[0, "ABCDuv"], [40, "ABCuvw"], [80, "ABuvwx"], [120, "Auvwxy"], [159, "Auvwxy"]] as const) {
      r.time.now = ms;
      const result = r.draw(compact);
      expect(result.areas).toEqual([SUFFIX]);
      expect(suffixLabels(result.grid)).toBe(expected);
      expect(result.grid.at(1, 3)).toEqual(parseFrame(expanded, W, H).at(1, 3));
    }
    expect(r.parsed).toHaveLength(2);
    r.time.now = 160;
    const endpoint = r.draw(compact);
    expect(endpoint.areas).toEqual([]);
    expect(endpoint.grid.cells).toEqual(parseFrame(compact, W, H).cells);
    expect(r.controller.running, "one final clean frame is requested").toBe(true);
    expect(r.draw(compact).areas).toEqual([]);
    expect(r.controller.running).toBe(false);
  });

  test.each([39, 43, null])("fold acknowledgement %s outside the current input range is ignored", (seq) => {
    const r = rig(); r.observe(frame()); r.input(40); r.controller.intent(42, 10);
    r.observe(frame(true, 11, "nav", { interaction: ack(seq, fold(true, seq)) }));
    expect(r.parsed).toEqual([]);
    expect(r.controller.running).toBe(false);
  });

  test.each([40, 41, 42])("fold acknowledgement %s inside a coalesced current-input range is accepted", (seq) => {
    const r = rig(); r.observe(frame()); r.input(40); r.controller.intent(42, 10);
    r.observe(frame(true, 11, "nav", { interaction: ack(seq, fold(true, seq)) }));
    expect(r.parsed).toHaveLength(2);
    expect(r.controller.running).toBe(true);
  });

  test("idle input invalidates a late response even when no effect was running", () => {
    const r = rig(); r.observe(frame()); r.input(40);
    expect(r.controller.running).toBe(false);
    r.controller.input();
    expect(r.controller.inputGeneration).toBe(2);
    r.observe(frame(true, 11, "detail", { interaction: ack(40, fold(true)) }));
    expect(r.parsed).toEqual([]);
    expect(r.controller.running).toBe(false);
  });

  test("a newer input rejects old responses and can still accept its own subsequent fold", () => {
    const r = rig(); r.observe(frame()); r.input(40); r.input(50);
    r.observe(frame(false, 10, "nav", { interaction: ack(40) }));
    expect(r.parsed).toEqual([]);
    r.observe(frame(true, 11, "nav", { interaction: ack(50, fold(true, 50, 10)) }));
    expect(r.parsed).toHaveLength(2);
    expect(suffixLabels(r.draw(output(true)).grid)).toBe("uvwxyz");
  });

  test("wrong context, causal cursor and missing semantic evidence never start a fold", () => {
    for (const change of [fold(true, 40, 9), { ...fold(true), context: { ...context, screen: "elsewhere" } }, { ...fold(true), path: ["other", "watch"] }]) {
      const r = rig(); r.observe(frame()); r.input();
      r.observe(frame(true, 11, "nav", { interaction: ack(40, change) }));
      expect(r.parsed).toEqual([]);
      expect(r.controller.running).toBe(false);
    }
    const r = rig(); r.observe(frame()); r.input();
    r.observe(frame(true, 11, "nav", { interaction: ack(40) }));
    expect(r.parsed).toEqual([]);
    expect(r.controller.running).toBe(false);
  });

  test("the same acknowledged fold is consumed once and clean redraws do not restart its clock", () => {
    const r = rig(), accepted = frame(true, 11, "nav", { interaction: ack(40, fold(true)) });
    r.observe(frame()); r.input(); r.observe(accepted);
    r.time.now = 120; r.observe(accepted);
    expect(suffixLabels(r.draw(output(true)).grid)).toBe("ABCuvw");
    expect(r.parsed).toHaveLength(2);
    r.time.now = 160; r.draw(output(true)); r.draw(output(true));
    r.observe(accepted);
    expect(r.controller.running).toBe(false);
    expect(r.parsed).toHaveLength(2);
  });

  test("input cancels active cells immediately and requests just one clean settling frame", () => {
    const r = rig(); r.observe(frame()); r.input();
    r.observe(frame(true, 11, "nav", { interaction: ack(40, fold(true)) }));
    r.time.now = 80;
    expect(suffixLabels(r.draw(output(true)).grid)).toBe("ABuvwx");
    r.controller.input();
    expect(r.controller.running).toBe(true);
    const next = r.draw(output(true));
    expect(next.areas).toEqual([]);
    expect(next.grid.cells).toEqual(parseFrame(output(true), W, H).cells);
    expect(r.controller.running).toBe(false);
    expect(r.parsed).toHaveLength(2);
  });

  test("rapid reversals use the newest clean baseline rather than a partially animated frame", () => {
    const r = rig(); r.observe(frame()); r.input();
    r.observe(frame(true, 11, "nav", { interaction: ack(40, fold(true)) }));
    r.time.now = 80;
    expect(suffixLabels(r.draw(output(true)).grid)).toBe("ABuvwx");
    r.input(50, 11);
    r.observe(frame(false, 12, "nav", { interaction: ack(50, fold(false, 50, 11)) }));
    expect(suffixLabels(r.draw(output(false)).grid)).toBe("ABCDuv");
    r.time.now = 120;
    expect(suffixLabels(r.draw(output(false)).grid)).toBe("ABCuvw");
    r.input(60, 12);
    r.observe(frame(true, 13, "nav", { interaction: ack(60, fold(true, 60, 12)) }));
    expect(suffixLabels(r.draw(output(true)).grid)).toBe("uvwxyz");
    expect(r.parsed).toEqual([output(false), output(true), output(true), output(false), output(false), output(true)]);
  });

  test("focus transfer parses only the prior source and changes border foregrounds alone", () => {
    const r = rig(), old = output(false, "nav"), next = output(false, "detail");
    r.observe(frame(), old); r.input();
    r.observe(frame(false, 11, "detail", { interaction: ack(40) }), next);
    expect(r.parsed).toEqual([old]);
    const first = r.draw(next), before = parseFrame(next, W, H).cells;
    expect(first.areas).toEqual(panes("detail").map((pane) => pane.area));
    expect(first.grid.at(0, 0).fg, "old owner's ramp starts at its prior colour").toBe(0x6080a0);
    expect(first.grid.at(9, 0).fg, "new owner starts at the first custom status stop").toBe(0x010305);
    expect(first.grid.at(10, 0).fg, "unvisited incoming cells use the old quiet border").toBe(0x102030);
    first.grid.cells.forEach((cell, i) => expect({ ...cell, fg: before[i]!.fg }).toEqual(before[i]));
    expect(first.grid.copyOf(TREE)).toEqual(parseFrame(next, W, H).copyOf(TREE));
    r.time.now = 300;
    expect(r.draw(next).grid.cells).toEqual(before);
    r.draw(next);
    expect(r.controller.running).toBe(false);
    expect(r.parsed).toHaveLength(1);
  });

  test.each(["before", "after"] as const)("zero or multiple owners in the %s frame reject the entire focus pair before parsing", (side) => {
    for (const owns of [false, true]) {
      const r = rig(), before = frame(), after = frame(false, 11, "detail", { interaction: ack(40) });
      const invalid = side === "before" ? before : after;
      invalid.panes = invalid.panes.map((pane) => ({ ...pane, owns }));
      r.observe(before, output()); r.input(); r.observe(after, output(false, "detail"));
      expect(r.parsed).toEqual([]);
      expect(r.controller.running).toBe(false);
      expect(r.draw(output(false, "detail")).areas).toEqual([]);
    }
  });

  test.each(["before", "after"] as const)("a missing counterpart in the %s frame cannot start a one-sided focus effect", (side) => {
    const r = rig(), before = frame(), after = frame(false, 11, "detail", { interaction: ack(40) });
    if (side === "before") before.panes = before.panes.filter((pane) => pane.pane !== "detail");
    else after.panes = after.panes.filter((pane) => pane.pane !== "nav");
    r.observe(before, output()); r.input(); r.observe(after, output(false, "detail"));
    expect(r.parsed).toEqual([]);
    expect(r.controller.running).toBe(false);
    expect(r.draw(output(false, "detail")).areas).toEqual([]);
  });

  test.each(["nav", "detail"])("changing only the %s border rejects both halves before parsing", (paneId) => {
    for (const changed of ["area", "title", "keep"] as const) {
      const r = rig(), before = frame(), after = frame(false, 11, "detail", { interaction: ack(40) });
      const pane = after.panes.find((mark) => mark.pane === paneId)!;
      if (changed === "area") pane.area = { ...pane.area, w: pane.area.w - 1 };
      if (changed === "title") pane.title = { x: pane.area.x + 1, y: pane.area.y, w: 1, h: 1 };
      if (changed === "keep") pane.keep = [{ x: pane.area.x, y: pane.area.y + 1, w: 1, h: 1 }];
      r.observe(before, output()); r.input(); r.observe(after, output(false, "detail"));
      expect(r.parsed, changed).toEqual([]);
      expect(r.controller.running, changed).toBe(false);
      expect(r.draw(output(false, "detail")).areas, changed).toEqual([]);
    }
  });

  test("local owner transfer is accepted once and its host echo cannot replay it", () => {
    const r = rig(); r.observe(frame()); r.input();
    r.observe(frame(false, 10, "detail", { local: true }));
    expect(r.parsed).toHaveLength(1);
    expect(r.controller.running).toBe(true);
    r.time.now = 120;
    r.observe(frame(false, 11, "detail", { interaction: ack(40) }));
    expect(r.parsed).toHaveLength(1);
    r.time.now = 300; r.draw(output(false, "detail")); r.draw(output(false, "detail"));
    expect(r.controller.running).toBe(false);
    r.observe(frame(false, 12, "detail", { interaction: ack(40) }));
    expect(r.parsed).toHaveLength(1);
  });

  test("a local owner transfer without current input cannot claim a response", () => {
    const r = rig(); r.observe(frame());
    r.observe(frame(false, 10, "detail", { local: true }));
    expect(r.parsed).toEqual([]);
    expect(r.controller.running).toBe(false);
  });

  test("a durable processed sequence alone cannot authorize a background owner change", () => {
    const r = rig(); r.observe(frame()); r.input();
    r.observe(frame(false, 11, "nav", { interaction: { context, processed_seq: 40 } }));
    r.observe(frame(false, 12, "detail", { interaction: { context, processed_seq: 40 } }));
    expect(r.parsed).toEqual([]);
    expect(r.controller.running).toBe(false);
  });

  test("combined fold and focus share the prior parse and have independent paint phases", () => {
    const r = rig(); r.observe(frame()); r.input();
    r.observe(frame(true, 11, "detail", { interaction: ack(40, fold(true)) }));
    expect(r.parsed).toHaveLength(2);
    const composed = parseFrame(output(true, "detail"), W, H);
    expect(r.controller.draw(composed, "fold")).toEqual([SUFFIX]);
    expect(suffixLabels(composed)).toBe("uvwxyz");
    expect(r.controller.draw(composed, "focus")).toEqual(panes("detail").map((pane) => pane.area));
    r.time.now = 160;
    expect(r.draw(output(true, "detail"), "fold").areas).toEqual([]);
    expect(r.draw(output(true, "detail"), "focus").areas).toHaveLength(2);
    r.time.now = 300; r.draw(output(true, "detail")); r.draw(output(true, "detail"));
    expect(r.controller.running).toBe(false);
    expect(r.parsed).toHaveLength(2);
  });

  test.each(["256", "16", "mono"] as const)("%s capability keeps no source for a later truecolor response", (tier) => {
    const r = rig(); applyLook(look(tier)); r.observe(frame()); r.input();
    r.observe(frame(true, 11, "detail", { interaction: ack(40, fold(true)) }));
    expect(r.parsed).toEqual([]);
    applyLook(look());
    r.observe(frame(false, 12, "nav", { interaction: ack(40, fold(false, 40, 11)) }));
    expect(r.parsed).toEqual([]);
    expect(r.controller.running).toBe(false);
  });

  test("full-to-off clears retained sources, pending intent and active effects", () => {
    const r = rig(); r.observe(frame()); r.input();
    const accepted = frame(true, 11, "nav", { interaction: ack(40, fold(true)) });
    r.observe(accepted);
    r.observe(accepted, output(true), false);
    expect(r.draw(output(true)).areas).toEqual([]);
    expect(r.controller.running).toBe(false);
    r.observe(frame(false, 12, "nav", { interaction: ack(40, fold(false, 40, 11)) }));
    expect(r.parsed).toHaveLength(2);
    expect(r.controller.running).toBe(false);
  });

  test("context, theme identity and terminal resize invalidate an active retained baseline", () => {
    for (const change of ["context", "theme", "resize"] as const) {
      const r = rig(); r.observe(frame()); r.input();
      r.observe(frame(true, 11, "nav", { interaction: ack(40, fold(true)) }));
      const next = frame(true, 11);
      if (change === "context") next.scene.context = { ...context, screen: "elsewhere" };
      if (change === "theme") applyLook(look());
      if (change === "resize") r.controller.observe(output(true), W + 1, H, next, true);
      else r.observe(next);
      expect(r.draw(output(true)).areas, change).toEqual([]);
      expect(r.controller.running, change).toBe(false);
      expect(r.parsed, change).toHaveLength(2);
    }
  });

  test("tree identity or protected-border geometry changes cancel before painting", () => {
    for (const change of ["tree", "title", "scrollbar", "cursor"] as const) {
      const r = rig(); r.observe(frame()); r.input();
      r.observe(frame(true, 11, "detail", { interaction: ack(40, fold(true)) }));
      const next = frame(true, 11, "detail");
      if (change === "tree") next.trees = [{ ...tree(true), open: [["watch"], ["watch", "A"]] }];
      if (change === "title") next.panes = next.panes.map((pane) => ({ ...pane, title: { x: pane.area.x + 1, y: 0, w: 2, h: 1 } }));
      if (change === "scrollbar") next.panes = next.panes.map((pane) => ({ ...pane, keep: [{ x: pane.area.x + pane.area.w - 1, y: 1, w: 1, h: 2 }] }));
      if (change === "cursor") next.scene.cursor = 12;
      r.observe(next);
      expect(r.draw(output(true, "detail")).areas, change).toEqual([]);
      expect(r.controller.running, change).toBe(false);
      expect(r.parsed, change).toHaveLength(2);
    }
  });

  test("blocked, unknown, missing and offscreen scenes clear sources without a parse", () => {
    const invalid: Array<SceneMarks | undefined> = [
      undefined, frame(false, 10, "nav", { blocked: true }), frame(false, 10, "nav", { context: null }),
      { ...frame(), panes: [{ ...panes()[0]!, area: { x: -1, y: 0, w: 7, h: H } }] },
      { ...frame(), trees: [{ ...tree(false), area: { ...TREE, y: H } }] },
      { ...frame(), panes: [], trees: [] },
    ];
    for (const marks of invalid) {
      const r = rig(); r.observe(frame()); r.input();
      r.controller.observe(output(), W, H, marks, true);
      r.observe(frame(true, 11, "detail", { interaction: ack(40, fold(true)) }));
      expect(r.parsed).toEqual([]);
      expect(r.controller.running).toBe(false);
    }
  });

  test("ANSI and viewport caps reject retention before invoking the parser", () => {
    expect(MAX_BASELINE_ANSI).toBe(1_048_576);
    for (const [ansi, width, height] of [["x".repeat(MAX_BASELINE_ANSI + 1), W, H], [output(), 257, 256], [output(), 0, H], [output(), W, -1]] as const) {
      const r = rig(); r.observe(frame()); r.input();
      r.controller.observe(ansi, width, height, frame(true, 11, "detail", { interaction: ack(40, fold(true)) }), true);
      r.observe(frame(false, 12, "nav", { interaction: ack(40, fold(false, 40, 11)) }));
      expect(r.parsed).toEqual([]);
      expect(r.controller.running).toBe(false);
    }
  });

  const invalidMetadata: Array<{ name: string; invalidate: (marks: SceneMarks) => void }> = [
    ...metadataLimits.filter((limit) => !limit.largeViewport).map(({ name, atLimit, exceed }) => ({ name, invalidate: (marks: SceneMarks) => { atLimit(marks); exceed(marks); } })),
    { name: "oversize acknowledgement context", invalidate: (marks) => { marks.scene.interaction!.context = { kind: "setup", step: utf8(16385), plugin: null, section: null }; } },
    { name: "overdeep acknowledgement context", invalidate: (marks) => { marks.scene.interaction!.context = { ...context, depth: 65 }; } },
    { name: "oversize fold context", invalidate: (marks) => { marks.scene.interaction!.fold!.context = { kind: "settings", plugin: null, section: utf8(16385) }; } },
    { name: "overdeep fold context", invalidate: (marks) => { marks.scene.interaction!.fold!.context = { ...context, depth: 65 }; } },
    { name: "oversize fold slot", invalidate: (marks) => { marks.scene.interaction!.fold!.slot = utf8(1025); } },
    { name: "oversize fold pane key", invalidate: (marks) => { marks.scene.interaction!.fold!.pane = utf8(1025) as Fold["pane"]; } },
    { name: "oversize fold path", invalidate: (marks) => { marks.scene.interaction!.fold!.path = [utf8(4097)]; } },
    { name: "overdeep fold path", invalidate: (marks) => { marks.scene.interaction!.fold!.path = Array.from({ length: 65 }, () => ""); } },
  ];

  test.each(invalidMetadata)("$name is rejected before serialization and cannot prime later focus or fold", ({ invalidate }) => {
    for (const position of ["baseline", "response"] as const) {
      const r = rig();
      const invalid = position === "baseline"
        ? frame(false, 10, "nav", { interaction: ack(39, fold(false, 39, 9)) })
        : frame(true, 11, "detail", { interaction: ack(40, fold(true)) });
      invalidate(invalid);
      expect(frameMetadataBounded(invalid), position).toBe(false);
      if (position === "response") { r.observe(frame()); r.input(); }
      const ansi = output(position === "response", position === "response" ? "detail" : "nav");
      const serialization = vi.spyOn(JSON, "stringify");
      let serialized: number;
      try {
        r.observe(invalid, ansi);
        serialized = serialization.mock.calls.length;
      } finally { serialization.mockRestore(); }
      expect(serialized, position).toBe(0);
      expect(r.parsed, position).toEqual([]);
      expect(r.controller.running, position).toBe(false);

      if (position === "baseline") {
        r.input();
        r.observe(frame(true, 11, "detail", { interaction: ack(40, fold(true)) }));
      } else {
        r.input(50, 11);
        r.observe(frame(false, 12, "nav", { interaction: ack(50, fold(false, 50, 11)) }));
      }
      expect(r.parsed, position).toEqual([]);
      expect(r.controller.running, position).toBe(false);
      expect(r.draw().areas, position).toEqual([]);
    }
  });

  test("a 4,097-byte group ID in a tiny viewport never retains or parses an otherwise matching fold", () => {
    const r = rig(), id = utf8(4097), before = frame(), after = frame(true, 11, "detail", { interaction: ack(40, fold(true)) });
    for (const marks of [before, after]) {
      const mark = marks.trees[0]!;
      mark.rows = mark.rows.map((row) => ({ ...row, path: row.path.map((part) => part === "watch" ? id : part) }));
      mark.open = mark.open.map((path) => path.map((part) => part === "watch" ? id : part));
    }
    after.scene.interaction!.fold!.path = [id];
    r.observe(before, output()); r.input(); r.observe(after, output(true, "detail"));
    expect(r.parsed).toEqual([]);
    expect(r.controller.running).toBe(false);
    r.input(50, 11);
    r.observe(frame(false, 12, "nav", { interaction: ack(50, fold(false, 50, 11)) }));
    expect(r.parsed).toEqual([]);
    expect(r.draw().areas).toEqual([]);
  });

  test("fractional and nonfinite viewports never become a baseline for the next response", () => {
    for (const [width, height] of [[W + 0.5, H], [W, H + 0.5], [NaN, H], [W, Infinity]]) {
      const r = rig();
      r.controller.observe(output(), width!, height!, frame(), true);
      r.input();
      r.controller.observe(output(true, "detail"), width!, height!, frame(true, 11, "detail", { interaction: ack(40, fold(true)) }), true);
      expect(r.parsed).toEqual([]);
      expect(r.controller.running).toBe(false);
    }
  });

  test("strict interaction parsing clips excess ANSI rows without changing legacy parsing", () => {
    const ansi = "a\nb\nc\nd";
    const clipped = parseFrame(ansi, 1, 2, true);
    expect(clipped.height).toBe(2);
    expect(clipped.cells.map((cell) => cell.ch)).toEqual(["a", "b"]);
    expect(parseFrame(ansi, 1, 2).cells.map((cell) => cell.ch)).toEqual(["a", "b", "c", "d"]);
    const padded = parseFrame("a", 1, 2, true);
    expect(padded.height).toBe(2);
    expect(padded.cells.map((cell) => cell.ch)).toEqual(["a", " "]);
  });

  test("an invalid custom status stop prevents a focus fallback and costs no ANSI parse", () => {
    const custom = look(); applyLook({ ...custom, slots: { ...custom.slots, "status.error": "" } });
    const r = rig(); r.observe(frame()); r.input();
    r.observe(frame(false, 11, "detail", { interaction: ack(40) }));
    expect(r.parsed).toEqual([]);
    expect(r.controller.running).toBe(false);
  });

  test("reset drops active effects, settling, baseline and input generation", () => {
    const r = rig(); r.observe(frame()); r.input();
    r.observe(frame(true, 11, "nav", { interaction: ack(40, fold(true)) }));
    r.controller.reset();
    expect(r.controller.inputGeneration).toBe(0);
    expect(r.controller.running).toBe(false);
    const clean = parseFrame(output(true), W, H), before = clean.cells.map(copyCell);
    expect(r.controller.draw(clean)).toEqual([]);
    expect(clean.cells).toEqual(before);
    r.observe(frame(false, 12, "detail", { interaction: ack(40, fold(false, 40, 11)) }));
    expect(r.parsed).toHaveLength(2);
    expect(r.controller.running).toBe(false);
  });
});
