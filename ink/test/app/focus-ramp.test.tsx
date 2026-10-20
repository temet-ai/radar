import { vi } from "vitest";
vi.hoisted(() => { process.env.FORCE_COLOR = "3"; });
import React from "react";
import { Text } from "ink";
import { afterEach, describe, expect, it } from "vitest";
import { FrameRampOwnership, Framed } from "../../src/app/Panes.js";
import { applyLook, currentLook, defaultLook, slotStyle, type Look } from "../../src/text/look.js";
import { focusRamp, perimeterIndex, perimeterLength, rampColor } from "../../src/text/frame.js";
import { parseAnsi } from "../nodes/ansi.js";
import { styledFrameAt } from "../nodes/helpers.js";
import { mount, until } from "./harness.js";
import { booksChrome } from "../chrome/fixture.js";
import { STATES } from "../dock/states.js";
import { useScrollReport } from "../../src/nodes/index.js";
import { HitRegistry } from "../../src/state/ui-state.js";
import { shellRenderNode } from "../../src/app/Body.js";

const original = currentLook();
const live: Array<() => void> = [];
afterEach(() => {
  live.splice(0).forEach((stop) => stop());
  applyLook(original);
});
const draw = (owned = true, focused = true) => parseAnsi(styledFrameAt(
  <FrameRampOwnership.Provider value={{ body: owned, dock: owned }}>
    <Framed title="界 é" width={20} height={5} focused={focused}><Text>body</Text></Framed>
  </FrameRampOwnership.Provider>, 20, 5,
));

describe("a focused frame's foreground-only ramp", () => {
  it("paints the physical perimeter while keeping its title, inner origin and dimensions", () => {
    applyLook(defaultLook("off"));
    const ramped = draw();
    const flat = draw(false);
    expect(ramped.map((r) => r.map((c) => c.ch).join(""))).toEqual(flat.map((r) => r.map((c) => c.ch).join("")));
    expect(ramped.map((r) => r.map((c) => c.bg))).toEqual(flat.map((r) => r.map((c) => c.bg)));
    expect(ramped[1]![1]!.ch).toBe("b");
    expect(ramped[0]!.slice(1, 7)).toEqual(flat[0]!.slice(1, 7));
    const ramp = focusRamp()!;
    for (const [x, y] of [[0,0],[19,0],[19,1],[19,4],[0,4],[0,1]]) {
      expect(ramped[y!]![x!]!.fg).toBe(rampColor(ramp, perimeterIndex(20, 5, x!, y!)!, perimeterLength(20, 5)));
    }
  });

  it("keeps a nonowning truecolor border quiet while retaining focused title styling", () => {
    applyLook(defaultLook("off"));
    expect(draw(false)[0]![0]!.fg).toBe(slotStyle("accent.muted").color);
    expect(draw(true, false)[0]![0]!.fg).toBe(slotStyle("accent.muted").color);
    expect(draw(false)[0]!.slice(1, 7)).toEqual(draw()[0]!.slice(1, 7));
  });

  it("paints the actual frame's scrollbar after the ramp without changing its cells", () => {
    applyLook(defaultLook("off"));
    const Scrolled = () => { useScrollReport({ max: 20, at: 7 }); return <Text>body</Text>; };
    const render = (body: boolean) => parseAnsi(styledFrameAt(
      <FrameRampOwnership.Provider value={{ body, dock: false }}>
        <Framed title="scroll" width={20} height={8} focused><Scrolled /></Framed>
      </FrameRampOwnership.Provider>, 20, 8,
    ));
    const ramped = render(true);
    const flat = render(false);
    const bars = (grid: ReturnType<typeof parseAnsi>) => grid.flatMap((row, y) => row.flatMap((cell, x) => /[█║]/.test(cell.ch) ? [{ x, y, ...cell }] : []));
    expect(bars(ramped).length).toBeGreaterThan(0);
    expect(bars(ramped)).toEqual(bars(flat));
  });

  it.each([["256", "ansi256(61)"], ["16", "blue"], ["mono", ""]] as const)("does not change lower-tier output at %s", (tier, color) => {
    const base = defaultLook("off");
    const look: Look = { ...base, tier, slots: Object.fromEntries(Object.keys(base.slots).map((slot) => [slot, color])) };
    applyLook(look);
    expect(draw()).toEqual(draw(false));
  });
});

// App is the ownership boundary: direct Framed tests cannot prove that the provider reaches ordinary panes,
// part-setup special cases and the dock, or that a later overlay suppresses the background ramp.
const PURPLE = "#004488"; // Deliberately differs from the legacy accent-primary border.
const ownershipLook = (tier: Look["tier"] = "truecolor") => {
  const base = defaultLook("off");
  applyLook({ ...base, tier, slots: { ...base.slots, "glow.purple": PURPLE } });
};
const document = { node: "document", lines: [[{ text: "The Watch's notes." }]] };
const screen = { scope: "watch", version: 1, actions: [], panes: ["nav", "middle", "detail"].map((pane) => ({ pane, title: pane, body: document })) };
const menu = { id: "watch", title: "choose", question: false, items: [{ value: "close", label: "close", hint: "", key: "c", target: "chrome:menu:0" }] };
const corner = (grid: ReturnType<typeof parseAnsi>, title: string) => {
  for (const row of grid) {
    const x = row.map((c) => c.ch).join("").indexOf(`╭ ${title}`);
    if (x >= 0) return row[x]!.fg;
  }
  throw new Error(`No panel titled ${title}`);
};
const app = (tier: Look["tier"] = "truecolor") => {
  ownershipLook(tier);
  const m = mount(160, 40);
  live.push(m.stop);
  m.welcome();
  let cursor = 0;
  return async (over: Record<string, unknown> = {}, chrome: Parameters<typeof booksChrome>[0] = {}) => {
    const frames = m.out.frames.length;
    m.snapshot({ screen, focus: "middle", chrome: booksChrome({ width: 160, height: 40, sources: [], ...chrome }), ...over }, ++cursor);
    await until(() => m.out.frames.length > frames);
    expect(m.out.frames.length, "the changed snapshot rendered").toBeGreaterThan(frames);
    return parseAnsi(m.frame());
  };
};

describe("App hands the ramp only to the pane that owns keys", () => {
  it("moves between body, bar, sources and page tabs with every nonowner quiet", async () => {
    const show = app();
    expect(corner(await show(), "middle")).toBe(PURPLE);
    for (const keys_at of ["bar", "sources", "page_tabs"] as const) {
      const grid = await show({}, { keys_at });
      expect(["nav", "middle", "detail"].map((p) => corner(grid, p))).toEqual(["#9580ff", "#9580ff", "#9580ff"]);
    }
    expect(corner(await show(), "middle")).toBe(PURPLE);
  });

  it("suppresses only modal ownership, retaining the ramp for an inline editing field", async () => {
    const show = app();
    const inline = { ...screen, panes: screen.panes.map((p) => p.pane === "middle" ? { ...p, fields: [{ mark: "/", text: "watch", caret: 5, hint: [] }] } : p) };
    expect(corner(await show({ screen: inline }), "middle")).toBe(PURPLE);
    for (const open of [menu, { ...menu, typed: "watch", caret: 5 }, { ...menu, question: true }]) {
      const grid = await show({}, { menu: open });
      // The quiet accent-muted border after the existing 550-per-mille scrim.
      expect(corner(grid, "middle")).toBe("#493e7d");
      expect(corner(grid, "choose")).toBe("#e135ff");
    }
    const overlay = { pane: "middle", title: "overlay", body: { node: "state", state: "empty", text: "overlay owns keys" } };
    expect(corner(await show({ screen: { ...screen, overlay } }), "middle")).toBe("#9580ff");
    expect(corner(await show({ screen: inline }), "middle")).toBe(PURPLE);
  });

  it("ramps a caret field's pane even when legacy focus is on another pane", async () => {
    const show = app();
    const field = { mark: "/", text: "watch", caret: 5, hint: [] };
    const inline = { ...screen, panes: screen.panes.map((p) => p.pane === "middle" ? { ...p, fields: [field] } : p) };
    const top = await show({ screen: inline, focus: "nav" });
    expect(corner(top, "nav")).toBe("#9580ff");
    expect(corner(top, "middle")).toBe(PURPLE);
    const bottom = { ...screen, panes: screen.panes.map((p) => p.pane === "detail" ? { ...p, bottom_field: field } : p) };
    const foot = await show({ screen: bottom, focus: "nav" });
    expect(corner(foot, "nav")).toBe("#9580ff");
    expect(corner(foot, "detail")).toBe(PURPLE);
  });

  it.each(["256", "16", "mono"] as const)("caret ownership leaves legacy frame focus unchanged at %s", async (tier) => {
    const show = app(tier);
    const field = { mark: "/", text: "watch", caret: 5, hint: [] };
    const inline = { ...screen, panes: screen.panes.map((p) => p.pane === "middle" ? { ...p, fields: [field] } : p) };
    const grid = await show({ screen: inline, focus: "nav" });
    // Deliberately keep RGB-looking slots at a lower explicit tier: only the wire tier gates the ramp.
    expect(corner(grid, "nav")).toBe("#e135ff");
    expect(corner(grid, "middle")).toBe("#9580ff");
  });

  it("gives the visible focused dock the ramp and takes it away while a menu owns input", async () => {
    const show = app();
    const docked = { ...screen, dock: STATES.empty };
    const shown = await show({ screen: docked });
    expect(corner(shown, "agents")).toBe(PURPLE);
    expect(["nav", "middle", "detail"].map((p) => corner(shown, p))).toEqual(["#9580ff", "#9580ff", "#9580ff"]);
    expect(corner(await show({ screen: docked }, { menu }), "agents")).toBe("#493e7d");
    expect(corner(await show({ screen: { ...docked, dock: { ...STATES.empty, visible: false } } }), "middle")).toBe(PURPLE);
  });

  it("ignores a stale focused dock where item pages and work areas cannot draw it", async () => {
    const show = app();
    const page = { crumbs: [], title: [{ text: "Watch case" }], tab: 0, panes: screen.panes };
    const item = await show({ screen: { ...screen, page, dock: STATES.empty } });
    expect(corner(item, "middle")).toBe(PURPLE);
    const work = { ...screen, dock: STATES.empty, panes: [...screen.panes, {
      pane: "detail", title: "Map", body: { node: "extension", capability: "work.map", version: 1, data: { target: "work:map" }, fallback: document },
    }] };
    expect(corner(await show({ screen: work }), "middle")).toBe(PURPLE);
    // WorkArea's legacy title focus stays on Middle under chrome; its nonowning border must still be quiet.
    expect(corner(await show({ screen: work }, { keys_at: "bar" }), "middle")).toBe("#9580ff");
  });

  it("keeps a modal editor flat and removes the background pane or dock ramp", async () => {
    const show = app();
    const overlay = { pane: "middle", title: "reply", body: { node: "extension", capability: "editor", version: 1,
      data: { title: "reply", lines: ["Watch report"], line: "hello", caret: 5 } } };
    const body = await show({ screen: { ...screen, overlay } });
    expect(corner(body, "middle")).toBe("#9580ff");
    expect(corner(body, "reply")).toBe("#e135ff");
    const dock = await show({ screen: { ...screen, dock: STATES.empty, overlay } });
    expect(corner(dock, "agents")).toBe("#9580ff");
    expect(corner(dock, "reply")).toBe("#e135ff");
  });

  it("respects keyboard ownership on a part's forced-focused setup pane", async () => {
    const show = app();
    const setup = { ...screen, panes: [{ pane: "middle", title: "set up Watch", body: document }] };
    expect(corner(await show({ screen: setup, focus: "nav" }), "set up Watch")).toBe(PURPLE);
    expect(corner(await show({ screen: setup, focus: "nav" }, { keys_at: "bar" }), "set up Watch")).toBe("#9580ff");
    expect(corner(await show({ screen: setup, focus: "nav" }, { menu }), "set up Watch")).toBe("#493e7d");
  });
});

async function pendingDock(tier: Look["tier"] = "truecolor") {
  ownershipLook(tier);
  const registry = new HitRegistry();
  let renders = 0;
  const m = mount(160, 40, { registry, renderNode: (node, cx) => { renders++; return shellRenderNode(node, cx); } });
  live.push(m.stop);
  m.welcome();
  let cursor = 0;
  const grid = () => parseAnsi(m.frame());
  const show = async (focused: boolean, over: Record<string, unknown> = {}) => {
    const before = renders;
    m.snapshot({ screen: { ...screen, dock: { ...STATES.empty, focused }, ...over }, focus: "middle", chrome: booksChrome({ width: 160, height: 40, sources: [] }) }, ++cursor);
    await until(() => renders > before);
  };
  const click = (x: number, y: number) => m.type(`\x1b[<0;${x + 1};${y + 1}M\x1b[<0;${x + 1};${y + 1}m`);
  const focus = () => {
    const hit = registry.all().find((h) => h.target === "dock:body")!;
    click(hit.x + 2, hit.y + hit.h - 2);
  };
  const blur = () => {
    const rows = grid();
    const y = rows.findIndex((r) => r.map((c) => c.ch).join("").includes("╭ middle"));
    const x = rows[y]!.map((c) => c.ch).join("").indexOf("╭ middle");
    click(x + 2, y + 2);
  };
  const redraw = async (action: () => void) => {
    const before = renders;
    action();
    await until(() => renders > before);
    expect(renders, "a changed optimistic dock focus redraws without waiting for another snapshot or key").toBeGreaterThan(before);
  };
  await show(false);
  return { m, grid, show, focus, blur, redraw };
}

describe("pending dock focus immediately owns the settled ramp", () => {
  it("moves the ramp on local focus and blur, keeps it through typing, and preserves ownership on host echo", async () => {
    const p = await pendingDock();
    expect(corner(p.grid(), "middle")).toBe(PURPLE);
    await p.redraw(p.focus);
    expect(corner(p.grid(), "middle")).toBe("#9580ff");
    expect(corner(p.grid(), "agents")).toBe(PURPLE);
    await p.redraw(() => p.m.type("watch"));
    expect(p.grid().map((r) => r.map((c) => c.ch).join("")).join("\n")).toContain("> watch");
    expect(corner(p.grid(), "middle")).toBe("#9580ff");
    expect(corner(p.grid(), "agents")).toBe(PURPLE);
    const pending = ["middle", "agents"].map((title) => corner(p.grid(), title));
    await p.show(true);
    expect(corner(p.grid(), "agents")).toBe(PURPLE);
    expect(["middle", "agents"].map((title) => corner(p.grid(), title))).toEqual(pending);
    await p.redraw(p.blur);
    expect(corner(p.grid(), "middle")).toBe(PURPLE);
    expect(corner(p.grid(), "agents")).toBe("#9580ff");
    p.m.type("q");
    await until(() => p.m.intents().some((i) => i.intent?.intent === "key" && i.intent.chord?.key?.char === "q"));
    expect(p.m.intents().some((i) => i.intent?.intent === "key" && i.intent.chord?.key?.char === "q")).toBe(true);
    await p.show(false);
    expect(corner(p.grid(), "middle")).toBe(PURPLE);
    expect(corner(p.grid(), "agents")).toBe("#9580ff");
  });

  it("clears a disagreement when reversed locally or replaced by a new snapshot, including a hidden dock", async () => {
    const p = await pendingDock();
    await p.redraw(p.focus);
    await p.redraw(p.blur);
    expect(corner(p.grid(), "middle")).toBe(PURPLE);
    await p.redraw(p.focus);
    await p.show(false); // The newer host snapshot wins even if it did not accept the pending focus.
    expect(corner(p.grid(), "middle")).toBe(PURPLE);
    await p.redraw(p.focus);
    await p.show(true, { dock: { ...STATES.empty, visible: false } });
    expect(corner(p.grid(), "middle")).toBe(PURPLE);
  });

  it.each(["256", "16", "mono"] as const)("preserves every output cell during pending focus and blur at %s", async (tier) => {
    const p = await pendingDock(tier);
    const body = p.grid();
    await p.redraw(p.focus);
    expect(p.grid()).toEqual(body);
    await p.show(true);
    const dock = p.grid();
    await p.redraw(p.blur);
    expect(p.grid()).toEqual(dock);
  });
});
