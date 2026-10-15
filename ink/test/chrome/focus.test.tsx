import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick } from "../app/harness.js";
import { booksChrome, settingsNav } from "./fixture.js";
import { parseAnsi } from "../nodes/ansi.js";

// Live survey (T22): after `,` Ratatui draws the section pane focused (shell.rs settings: has(pane) = the body has the
// keys and host.focus is the pane) and Ink drew the nav focused; with the top bar holding the keys Ratatui draws no
// pane focused (Cx::focus_of: !bar && !tabs && focus == pane).
const FOCUS = "#e135ff"; // accent.primary
const MUTED = "#9580ff"; // accent.muted

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

const section = {
  scope: "projects",
  detail_title: "what this does",
  detail: [],
  actions: [],
  body: { node: "list", selected: 0, rows: [{ target: "row:a", text: [{ text: "a" }] }] },
};

async function draw(focus: string, keysAt: "body" | "bar") {
  const m = mount(160, 40);
  live.push(m.stop);
  m.welcome();
  m.snapshot({ chrome: { ...booksChrome({ width: 160, height: 40 }), settings: settingsNav, keys_at: keysAt }, section, screen: null, focus }, 1);
  await tick(80);
  return parseAnsi(m.frame());
}
/** The colour of each pane's top-left corner `╭` on the first body row that has one. */
const corners = (rows: ReturnType<typeof parseAnsi>) => {
  const y = rows.findIndex((r) => r.some((c) => c.ch === "╭"));
  return rows[y]!.filter((c) => c.ch === "╭").map((c) => c.fg);
};

describe("settings panes: the focused one carries the focus border", () => {
  it.each([
    ["nav", [FOCUS, MUTED, MUTED]],
    ["middle", [MUTED, FOCUS, MUTED]],
    ["detail", [MUTED, MUTED, FOCUS]],
  ] as const)("focus %s", async (focus, want) => {
    expect(corners(await draw(focus, "body"))).toEqual(want);
  });
  it("no pane is focused while the top bar has the keys", async () => {
    expect(corners(await draw("middle", "bar"))).toEqual([MUTED, MUTED, MUTED]);
  });
});

describe("a screen's panes", () => {
  const screen = JSON.parse(fs.readFileSync(path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures/gitlab_repos_160x40.json"), "utf8"));
  const drawScreen = async (keysAt: "body" | "bar") => {
    const m = mount(160, 40);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: { ...booksChrome({ width: 160, height: 40 }), keys_at: keysAt }, screen, section: null, focus: "middle" }, 1);
    await tick(80);
    return corners(parseAnsi(m.frame()));
  };
  it("have the middle pane focused while the body has the keys", async () => {
    const c = await drawScreen("body");
    expect(c.filter((fg) => fg === FOCUS)).toHaveLength(1);
  });
  it("have none focused while the top bar has the keys", async () => {
    expect((await drawScreen("bar")).filter((fg) => fg === FOCUS)).toHaveLength(0);
  });
});

describe("the settings navigation rows (BandedTree)", () => {
  const OVERLAY = "#1a162a";
  const SELECTION = "#44475a";
  async function nav(selected: string) {
    const m = mount(160, 40);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: { ...booksChrome({ width: 160, height: 40 }), settings: { ...settingsNav, selected } }, section, screen: null, focus: "nav" }, 1);
    await tick(80);
    const rows = parseAnsi(m.frame());
    const y = rows.findIndex((r) => r.some((c) => c.ch === "╭"));
    const at = (name: string) => rows.slice(y + 1).find((r) => r.map((c) => c.ch).join("").includes(name))!;
    return { at, width: rows[y]!.findIndex((c, i) => i > 0 && c.ch === "╭") };
  }
  it("puts every other section of a group on bg.overlay up to the column before the scrollbar's", async () => {
    const { at, width } = await nav("chrome:group:gitlab");
    const projects = at("projects");
    expect(projects.slice(1, width - 2).every((c) => c.bg === OVERLAY)).toBe(true);
    expect(at("connection").slice(1, width - 2).some((c) => c.bg === OVERLAY)).toBe(false);
  });
  it("runs the selection's highlight over the whole inner width", async () => {
    const { at, width } = await nav("chrome:section:0");
    expect(at("connection").slice(1, width - 1).every((c) => c.bg === SELECTION)).toBe(true);
  });
});

describe("the set-up page of a part", () => {
  // view.rs setup_screen: one pane titled "set up <part>"; parts::setup_page draws its panel focused, whoever has the keys.
  const screen = { scope: "github", panes: [{ pane: "middle", title: "set up GitHub", body: { node: "document", lines: [[{ text: "GitHub is not set up yet." }]] } }], actions: [], version: 1 };
  it.each([
    ["nav", "body"],
    ["nav", "bar"],
  ] as const)("is focused with the host's focus on %s and the keys at the %s", async (focus, keysAt) => {
    const m = mount(160, 40);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: { ...booksChrome({ width: 160, height: 40 }), keys_at: keysAt }, screen, section: null, focus }, 1);
    await tick(80);
    expect(corners(parseAnsi(m.frame()))).toEqual([FOCUS]);
  });
});
