import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // keep the cells' styles: several checks read colours
});
import React from "react";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Box, Text } from "ink";
import { afterEach, describe, expect, it } from "vitest";
import { renderNode, type Node } from "../../src/nodes/index.js";
import { shellRenderNode } from "../../src/app/Body.js";
import { PaneFrame } from "../../src/app/Panes.js";
import { HitRegistry } from "../../src/state/ui-state.js";
import { hitAt, layoutMenu, layoutNav, layoutTopBar } from "../../src/chrome/layout.js";
import { layoutNotices, noticeContent } from "../../src/notices/layout.js";
import { applyLook, bgOf, currentLook, defaultLook, iconOf, slotStyle } from "../../src/text/look.js";
import { frameAt, styledFrameAt } from "../nodes/helpers.js";
import { parseAnsi } from "../nodes/ansi.js";
import { booksChrome, settingsNav } from "../chrome/fixture.js";
import { mount, tick, until } from "../app/harness.js";
import { notice } from "../notices/fixture.js";
import { attachMotion } from "../../src/motion/attach.js";
import { motion } from "../../src/motion/motion.js";
import type { Choice, MenuView } from "../../src/chrome/types.js";

// radar#26, set 2: the checks the part crates' tests lost when they moved from painted Ratatui frames to view data
// (dd14ae6 lists where the others went). Each is proved here on the view data the host sends, drawn by Ink:
//  1. no ghost row targets in a body shorter than its rows        (github 7b14909, jira 61b8a96)
//  2. rows and the status line stay inside tiny panels            (jira 61b8a96)
//  3. a press on a page's status or error line only focuses the pane (jira 61b8a96, gitlab efe5f9e)
//  4. a failed line is drawn in the theme's bad colour            (jira 61b8a96, gitlab efe5f9e)
//  5. the settings navigation list                                (jira 61b8a96, git 5287019, gitlab efe5f9e)
//  6. top-bar words cut with … and tabs shortened at 80 columns   (git 5287019)
//  7. the notice toast and its error tone                         (git 5287019)
//  8. gitlab: the x menu's shift word, the tracker chip and column at 80 columns, a rich table in columns (efe5f9e)
const fixtures = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures");
const load = (name: string) => JSON.parse(fs.readFileSync(path.join(fixtures, name), "utf8"));
const plain = (s: string) => s.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "");
const t = (text: string, ink?: string) => [{ text, ...(ink ? { ink } : {}) }] as never;

const live: Array<() => void> = [];
const original = currentLook();
afterEach(() => {
  live.splice(0).forEach((stop) => stop());
  motion.reset();
  applyLook(original);
});

// ---------------------------------------------------------------------------------------------------------------
// 1. No ghost row targets in a body shorter than its rows
// ---------------------------------------------------------------------------------------------------------------
type Hit = { x: number; y: number; w: number; h: number; target: string };
const OX = 3;
const OY = 2;

/** `node` drawn at `height` rows, `OX` cells in and `OY` lines down, with the hits it registered and its lines. */
function bodyAt(node: Node, width: number, height: number) {
  const hits: Hit[] = [];
  const registry = { add: (h: Hit) => hits.push(h) };
  const el = (
    <Box flexDirection="column">
      <Text>top</Text>
      <Text>top</Text>
      <Box marginLeft={OX}>{renderNode(node, { width, height, focused: true, registry })}</Box>
    </Box>
  );
  const lines = frameAt(el, width + OX, 40).split("\n").slice(OY);
  while (lines.length && lines.at(-1)!.trim() === "") lines.pop();
  return { hits, lines };
}

const rowsOf = (n: number, label: (i: number) => string) => Array.from({ length: n }, (_, i) => ({ target: `row:${i}`, text: t(label(i)) }));
const ROWS = 12;
const wide: Node[] = [
  { node: "list", rows: rowsOf(ROWS, (i) => `list item ${i}`), selected: 0 } as Node,
  { node: "list", rows: rowsOf(ROWS, (i) => `list item ${i}`), selected: ROWS - 1 } as Node,
  {
    node: "table",
    columns: [{ heading: "name", width: 14, keep: 1 }],
    rows: Array.from({ length: ROWS }, (_, i) => ({ target: `row:${i}`, cells: [{ heading: "name", text: t(`job step-${i}`) }] })),
    selected: ROWS - 1,
  } as Node,
  {
    node: "tree",
    header: t("JOBS"),
    columns: [{ heading: "state", width: 10, keep: 1 }],
    name_width: 16,
    open: [["g"]],
    selected: ["g", `c${ROWS - 1}`],
    rows: [
      {
        id: "g",
        name: t("group"),
        children: Array.from({ length: ROWS }, (_, i) => ({ id: `c${i}`, name: t(`child ${i}`), cells: [{ heading: "state", text: t("ok") }] })),
      },
    ],
  } as unknown as Node,
  { node: "entries", empty: "", selected: 0, rows: Array.from({ length: ROWS }, (_, i) => ({ target: `row:${i}`, head: t(`entry ${i}`), body: [t("one line")] })) } as Node,
];

/** What a press can reach must be inside the body: that is the class checked for every node below. */
function expectInside(node: Node, width: number, height: number, label: string) {
  const { hits, lines } = bodyAt(node, width, height);
  expect(lines.length, `${label}: ${lines.length} lines drawn in a body of ${height}`).toBeLessThanOrEqual(height);
  for (const h of hits) {
    expect(h.y, `${label}: ${h.target} above the body`).toBeGreaterThanOrEqual(OY);
    expect(h.y + h.h, `${label}: ${h.target} below the body (${h.y}+${h.h} > ${OY + height})`).toBeLessThanOrEqual(OY + height);
  }
  expect(hitAt(hits, OX + 1, OY + height), `${label}: a press under the body`).toBeUndefined();
  expect(hitAt(hits, OX + 1, OY + height + 3), `${label}: a press far under the body`).toBeUndefined();
  return { hits, lines };
}

describe("1. no ghost row targets in a body shorter than its rows", () => {
  it.each(wide.map((n, i) => [`${n.node}#${i}`, n] as const))("%s: only the rows that show register a target, at every height from 1 to 6", (name, node) => {
    for (let height = 1; height <= 6; height++) {
      const { hits, lines } = expectInside(node, 40, height, `${name} at ${height}`);
      // a hit stands on a line that shows its row
      for (const h of hits) expect(lines[h.y - OY], `${name} at ${height}: ${h.target}`).toBeTruthy();
    }
  });

  it("a body of no height draws nothing and registers nothing", () => {
    for (const node of wide) {
      const { hits, lines } = bodyAt(node, 40, 0);
      expect({ node: node.node, hits: hits.length, lines: lines.length }).toEqual({ node: node.node, hits: 0, lines: 0 });
    }
  });

  it("a list shorter than its body registers every row, and the one the cursor moved to is a visible one", () => {
    const list = wide[0]! as { node: "list"; rows: unknown[] };
    const { hits } = bodyAt({ ...list, rows: list.rows.slice(0, 3) } as Node, 40, 6);
    expect(hits.map((h) => h.target)).toEqual(["row:0", "row:1", "row:2"]);
    const last = bodyAt(wide[1]!, 40, 4);
    expect(last.hits.map((h) => h.target)).toContain(`row:${ROWS - 1}`);
    expect(last.lines.join("\n")).toContain(`list item ${ROWS - 1}`);
  });

  // The class: every list, table, tree and entries of every recorded fixture, in a body of 1 to 5 rows.
  const dir = fixtures;
  const pointable = (v: unknown, out: Node[] = []): Node[] => {
    if (Array.isArray(v)) v.forEach((x) => pointable(x, out));
    else if (v && typeof v === "object") {
      const o = v as Record<string, unknown>;
      if (["list", "table", "tree", "entries"].includes(String(o.node)) && Array.isArray(o.rows)) out.push(o as unknown as Node);
      else for (const x of Object.values(o)) pointable(x, out);
    }
    return out;
  };
  it("holds for every pointable node of every recorded fixture", () => {
    const cases: Array<[string, Node]> = [];
    for (const f of fs.readdirSync(dir).filter((f) => f.endsWith(".json") && !f.startsWith("_")).sort())
      pointable(load(f)).forEach((node, i) => cases.push([`${f}#${i}`, node]));
    expect(cases.length).toBeGreaterThan(30);
    const bad: string[] = [];
    for (const [name, node] of cases) {
      for (const height of [1, 2, 3, 5]) {
        try {
          expectInside(node, 52, height, `${name} at ${height}`);
        } catch (e) {
          bad.push((e as Error).message.split("\n")[0]!);
        }
      }
    }
    expect(bad).toEqual([]);
  });
});

// ---------------------------------------------------------------------------------------------------------------
// 2. Rows and the status line stay inside tiny panels
// ---------------------------------------------------------------------------------------------------------------
const failedHead = (source: string, why: string) => [{ icon: "error" }, { text: `${source} could not sync: `, ink: "bad" }, { text: why, ink: "bad", strong: true }];
const linkRows = [
  ["parent", "ABC-1", "Dashboard for the stats"],
  ["child", "ABC-13", "Count requests per route"],
  ["blocks", "ABC-15", "Ship the dashboard"],
  ["web link", "web", "Design notes"],
];
const linksBody = {
  node: "table",
  columns: [
    { heading: "relation", width: 10, keep: 1 },
    { heading: "ticket", width: 8, keep: 2 },
    { heading: "title", width: 24, keep: 3 },
  ],
  selected: 0,
  rows: linkRows.map(([relation, ticket, title], i) => ({
    target: `row:${i}`,
    cells: [
      { heading: "relation", text: t(relation!) },
      { heading: "ticket", text: t(ticket!) },
      { heading: "title", text: t(title!) },
    ],
  })),
} as unknown as Node;
const linksPane = (head: unknown[] | null = failedHead("Jira page", "Jira refresh unavailable")) => ({ pane: "middle" as const, title: "links", head, body: linksBody });
const cx = (width: number, height: number, registry?: HitRegistry) => ({ width, height, actions: [], registry });

describe("2. rows and the status line stay inside tiny panels", () => {
  const PAD = 2;
  /** A pane drawn in an absolute box on a ground of X: whatever it writes outside its own box replaces an X. */
  function around(width: number, height: number, pane = linksPane()) {
    const registry = new HitRegistry();
    const el = (
      <Box width={width + 2 * PAD} height={height + 2 * PAD} flexDirection="column">
        {Array.from({ length: height + 2 * PAD }, (_, y) => (
          <Text key={y}>{"X".repeat(width + 2 * PAD)}</Text>
        ))}
        <Box position="absolute" marginLeft={PAD} marginTop={PAD} width={width} height={height}>
          <PaneFrame pane={pane as never} width={width} height={height} focused cx={cx(width, height, registry)} renderNode={shellRenderNode} />
        </Box>
      </Box>
    );
    const lines = frameAt(el, width + 2 * PAD, height + 2 * PAD).split("\n");
    return { lines, hits: registry.all() };
  }

  it.each([0, 1, 2, 3, 4, 80, 160].flatMap((w) => [0, 1, 2, 3, 4, 5].map((h) => [w, h] as const)))("a pane of %i by %i draws nothing outside its box, and its row targets sit inside its borders", (w, h) => {
    const { lines, hits } = around(w, h);
    const W = w + 2 * PAD;
    lines.slice(0, h + 2 * PAD).forEach((line, y) => {
      for (let x = 0; x < W; x++) {
        const inside = x >= PAD && x < PAD + w && y >= PAD && y < PAD + h;
        if (!inside) expect(line[x] ?? "X", `outside the box at ${x},${y}`).toBe("X");
      }
    });
    // the first and last lines under and over the box are untouched too (nothing drawn past a short panel)
    expect(lines.length).toBeLessThanOrEqual(h + 2 * PAD);
    for (const hit of hits.filter((x) => x.target.startsWith("row:"))) {
      expect(hit.y, `${hit.target} over the top border`).toBeGreaterThan(PAD);
      expect(hit.y + hit.h, `${hit.target} over the bottom border`).toBeLessThan(PAD + h);
      expect(hit.x, `${hit.target} left of the left border`).toBeGreaterThan(PAD);
      expect(hit.x + hit.w, `${hit.target} right of the right border`).toBeLessThan(PAD + w);
    }
    // no room for a data row: the frame (2), the status line (1) and the table's heading (1) leave none up to h = 4
    if (h <= 4) expect(hits.filter((x) => x.target.startsWith("row:"))).toEqual([]);
  });

  it("the status line is never a target and a row never draws over it, at any height that shows both", () => {
    for (const h of [5, 6, 7, 12]) {
      const { lines, hits } = around(60, h);
      const status = lines.findIndex((l) => l.includes("could not sync"));
      expect(status, `status line at ${h}`).toBe(PAD + 1);
      expect(hits.filter((x) => x.target.startsWith("row:")).every((x) => x.y > status)).toBe(true);
      expect(hits.filter((x) => !x.target.startsWith("row:") && x.y === status)).toEqual([]);
    }
  });
});

// ---------------------------------------------------------------------------------------------------------------
// 3. A press on a page's status or error line only focuses the pane
// ---------------------------------------------------------------------------------------------------------------
describe("3. a press on the status or error line only focuses the pane", () => {
  const jira = load("jira_tickets-page1-overview-comments-links_160x40.json");
  const page = (rows: number) => {
    const body = { ...(linksBody as unknown as Record<string, unknown>), rows: (linksBody as any).rows.slice(0, rows) };
    const [, detail] = jira.page.panes;
    return { ...jira, page: { ...jira.page, tab: 2, panes: [{ pane: "middle", title: "links", head: failedHead("Jira page", "Jira refresh unavailable"), body }, detail] } };
  };
  const click = (m: ReturnType<typeof mount>, x: number, y: number, button = 0) => m.type(`\x1b[<${button};${x + 1};${y + 1}M\x1b[<${button};${x + 1};${y + 1}m`);

  it.each([
    ["a cached links page", 4],
    ["code rows", 2],
  ] as const)("%s: the error line is not a row, a left press on it sends select pane:middle and a row stays selected, a right press sends nothing", async (_name, rows) => {
    const registry = new HitRegistry();
    const m = mount(160, 40, { registry });
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: booksChrome({ width: 160, height: 40 }), screen: page(rows), section: null, focus: "detail" }, 1);
    await until(() => plain(m.frame()).includes("could not sync"));
    const lines = plain(m.frame()).split("\n");
    const y = lines.findIndex((l) => l.includes("could not sync"));
    const x = lines[y]!.indexOf("could not sync") + 3;
    const rowHits = registry.all().filter((h) => h.target.startsWith("row:"));
    expect(rowHits).toHaveLength(rows);
    expect(rowHits.every((h) => h.y > y), "every row is under the error line").toBe(true);
    expect(hitAt(registry.all(), x, y), "the error line is not a target").toBeUndefined();

    const sent = () => m.intents().map((i) => i.intent);
    click(m, x, y);
    await until(() => sent().length > 0);
    await tick(20);
    expect(sent()).toEqual([{ intent: "select", target: "pane:middle" }]);

    // a row is selected by its own target; the line above it still sends only the pane
    const last = rowHits.at(-1)!;
    click(m, last.x + 2, last.y);
    await until(() => sent().length > 1);
    expect(sent().at(-1)).toEqual({ intent: "select", target: last.target });
    click(m, x, y);
    await until(() => sent().length > 2);
    await tick(20);
    expect(sent().slice(2)).toEqual([{ intent: "select", target: "pane:middle" }]);

    // a right press there is not a pane focus and offers no menu
    const before = sent().length;
    click(m, x, y, 2);
    await tick(30);
    expect(sent().length).toBe(before);
  });
});

// ---------------------------------------------------------------------------------------------------------------
// 4. The error tone of a failed line
// ---------------------------------------------------------------------------------------------------------------
describe("4. a failed line is drawn in the theme's bad colour", () => {
  const colours = (head: unknown[], needle: string) => {
    const frame = styledFrameAt(
      <PaneFrame pane={{ ...linksPane(head), body: { node: "document", lines: [] } } as never} width={70} height={6} focused cx={cx(70, 6)} renderNode={shellRenderNode} />,
      70,
      6,
    );
    const rows = parseAnsi(frame);
    const y = rows.findIndex((r) => r.map((c) => c.ch).join("").includes(needle));
    expect(y, `${needle} in the frame`).toBeGreaterThanOrEqual(0);
    const text = rows[y]!.map((c) => c.ch).join("");
    const at = text.indexOf(needle);
    return rows[y]!.slice(at, at + needle.length);
  };

  it.each([
    ["Jira page could not sync:", failedHead("Jira page", "Jira refresh unavailable")],
    ["could not load:", [{ icon: "error" }, { text: "could not load: ", ink: "bad" }, { text: "Jira has no ticket WATCH-12", ink: "bad", strong: true }]],
    ["merge request could not sync:", failedHead("merge request", "GitLab answered 500")],
  ])("%s: its words are the status.error slot of the look, the reason as well", (needle, head) => {
    const bad = slotStyle("status.error").color;
    expect(bad).toBeTruthy();
    const cells = colours(head, needle);
    expect(cells.every((c) => c.fg === bad), `${needle} in ${bad}: ${JSON.stringify(cells.map((c) => c.fg))}`).toBe(true);
    const why = colours(head, String((head[2] as { text: string } | undefined)?.text ?? (head[1] as { text: string }).text).trim());
    expect(why.every((c) => c.fg === bad)).toBe(true);
  });

  it("the colour is the host's: another status.error in the look is the colour drawn, and a line without the bad ink is not in it", () => {
    const base = defaultLook("unicode");
    applyLook({ ...base, slots: { ...base.slots, "status.error": "#123456" } });
    expect(colours(failedHead("Jira page", "no route"), "Jira page could not sync:").every((c) => c.fg === "#123456")).toBe(true);
    const calm = colours([{ text: "Jira page is synced", ink: "muted" }], "Jira page is synced");
    expect(calm.some((c) => c.fg === "#123456")).toBe(false);
  });

  it("a seg whose look is status.error draws in it as well", () => {
    const cells = colours([{ text: "refresh failed: no route", look: "status.error" }], "refresh failed:");
    expect(cells.every((c) => c.fg === slotStyle("status.error").color)).toBe(true);
  });
});

// ---------------------------------------------------------------------------------------------------------------
// 5. The settings navigation list
// ---------------------------------------------------------------------------------------------------------------
describe("5. the settings navigation list", () => {
  const section = (id: string, n: number) => ({ id, name: id, target: `chrome:section:${n}` });
  const general = ["parts", "mode", "appearance", "colours", "icons"];
  const jira = ["connection", "projects", "views", "look"];
  const nav = (selected: string) => ({
    ...settingsNav,
    groups: [
      { id: "general", name: "General", open: true, target: "chrome:group:general", sections: general.map((s, i) => section(s, i)) },
      { id: "jira", name: "Jira", open: true, target: "chrome:group:jira", sections: jira.map((s, i) => section(s, general.length + i)) },
    ],
    selected,
  });
  const everySection = [...general, ...jira];

  it("layoutNav has a row for every group and section, joined by tree lines, the selected one marked and each row a target", () => {
    const laid = layoutNav(nav("chrome:section:6") as never, 1, 3, 20, 30);
    expect(laid.rows.map((r) => r.text)).toEqual([
      "General",
      "├─ parts",
      "├─ mode",
      "├─ appearance",
      "├─ colours",
      "└─ icons",
      "Jira",
      "├─ connection",
      "├─ projects",
      "├─ views",
      "└─ look",
    ]);
    expect(laid.rows.filter((r) => r.selected).map((r) => r.text)).toEqual(["├─ projects"]);
    expect(laid.hits.map((h) => h.target)).toEqual(["chrome:group:general", ...general.map((_, i) => `chrome:section:${i}`), "chrome:group:jira", ...jira.map((_, i) => `chrome:section:${general.length + i}`)]);
    expect(laid.rows.map((r) => r.text).join("")).not.toMatch(/[▶▸▾›]|\[[+-]\]/);
  });

  it.each([[80, 24], [160, 40]] as const)("at %ix%i the frame draws every section, only the selected row is on the selection shade, and a press on each sends its target", async (cols, rows) => {
    const m = mount(cols, rows);
    live.push(m.stop);
    m.welcome();
    const body = { node: "list", rows: [{ target: "p:1", text: t("acme/api") }], selected: 0 };
    m.snapshot(
      { chrome: { ...booksChrome({ width: cols, height: rows }), settings: nav("chrome:section:6") }, section: { version: 1, scope: "projects", body, detail_title: "detail", detail: [[{ text: "what this does" }]] }, screen: null, focus: "nav" },
      1,
    );
    await until(() => plain(m.frame()).includes("projects"));
    const grid = parseAnsi(m.frame());
    const text = grid.map((r) => r.map((c) => c.ch).join(""));
    const rowOf = (name: string) => text.findIndex((l) => l.includes(`─ ${name}`) || l.slice(1, 12).includes(name));
    for (const name of everySection) expect(rowOf(name), `${name} is drawn`).toBeGreaterThan(1);
    expect(text.some((l) => /General/.test(l)) && text.some((l) => /Jira/.test(l))).toBe(true);
    expect(text.join("\n")).not.toMatch(/[▶▸▾]/);
    // the selected section is on the selection shade across its row, no other section is
    const selection = bgOf("bg.selection");
    const shaded = (name: string) => grid[rowOf(name)]!.slice(2, 14).every((c) => c.bg === selection);
    expect(shaded("projects")).toBe(true);
    for (const name of everySection.filter((n) => n !== "projects")) expect(shaded(name), `${name} is not selected`).toBe(false);
    // a press on a section's name sends its target (the chrome acts on one press)
    for (const [i, name] of everySection.entries()) {
      const before = m.intents().length;
      const y = rowOf(name);
      m.type(`\x1b[<0;${text[y]!.indexOf(name) + 1};${y + 1}M\x1b[<0;${text[y]!.indexOf(name) + 1};${y + 1}m`);
      await until(() => m.intents().length > before, 500);
      expect(m.intents().slice(before).map((x) => x.intent), name).toEqual([{ intent: "activate", target: `chrome:section:${i}` }]);
    }
  });
});

// ---------------------------------------------------------------------------------------------------------------
// 6. Top-bar words cut with … and tabs shortened at 80 columns
// ---------------------------------------------------------------------------------------------------------------
describe("6. the top bar at 80 columns", () => {
  const gitBar = (words: string) => {
    const chrome = booksChrome({ width: 80, height: 24 });
    chrome.top_bar = {
      ...chrome.top_bar,
      tabs: [
        { label: "branches", mnemonic: "B", active: true, target: "chrome:tab:0" },
        { label: "history", mnemonic: "h", target: "chrome:history" },
        { label: "settings", mnemonic: ",", target: "chrome:settings" },
      ],
      words: { text: words, ink: "warn" },
      mode: { text: "✎ edit", ink: "secondary", target: "chrome:mode" },
    };
    return chrome;
  };
  const row = (words: string, width: number) => layoutTopBar(gitBar(words), width).cells.map((c) => c.text).join("");

  it("160 columns hold the names and the whole words", () => {
    expect(row("read incomplete · last complete 1m ago", 160)).toMatch(/^ radar {4}Branches {3}History {3}Settings \(,\) +read incomplete · last complete 1m ago {2}✎ edit $/);
  });

  it("80 columns shorten the screen tab to its key and cut the words with …, as Ratatui draws them", () => {
    expect(row("read incomplete · last complete 1m ago", 80)).toBe(" radar    B   History   Settings (,)  read incomplete · last complete 1… ✎ edit ");
    expect(row("read incomplete · no complete sync yet", 80)).toBe(" radar    B   History   Settings (,)  read incomplete · no complete syn… ✎ edit ");
  });

  it.each([
    ["read incomplete · last complete 1m ago"],
    ["read incomplete · no complete sync yet"],
  ])("the whole bar is 80 cells and the mode keeps its place however long the words (%s)", (words) => {
    const laid = layoutTopBar(gitBar(words), 80);
    const text = laid.cells.map((c) => c.text).join("");
    expect([...text].length).toBe(80);
    expect(text.endsWith("✎ edit ")).toBe(true);
    expect(laid.hits.map((h) => h.target)).toEqual(expect.arrayContaining(["chrome:tab:0", "chrome:history", "chrome:settings", "chrome:mode"]));
  });

  it("the git screen under that chrome, drawn by the whole app at 80x24, has the same first line", async () => {
    const m = mount(80, 24);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: gitBar("read incomplete · last complete 1m ago"), screen: load("git_branch-screen_80x24.json"), section: null, focus: "middle" }, 1);
    await until(() => plain(m.frame()).includes("radar"));
    expect(plain(m.frame()).split("\n")[0]).toBe(" radar    B   History   Settings (,)  read incomplete · last complete 1… ✎ edit ");
  });
});

// ---------------------------------------------------------------------------------------------------------------
// 7. The notice toast and its error tone
// ---------------------------------------------------------------------------------------------------------------
describe("7. the notice toast", () => {
  const WORDS = "worktree changed: press d to review removal again";
  // The toasts are painted by the output compositor over the finished frame (notices/paint.ts), so the app is mounted
  // with the motion attached, as notices/app.test.tsx does.
  const slots = { base: "bg.base", surface: "bg.surface", overlay: "bg.overlay", glow: "accent.primary", text: "text.primary" };
  const open = async (cols: number, rows: number, tone: "error" | "info") => {
    motion.reset();
    const m = mount(cols, rows, {}, {}, () => 0, ({ session, mouse, out }) => live.push(attachMotion(session, mouse, out as never)));
    live.push(m.stop);
    m.welcome();
    const screen = load(cols === 80 ? "git_branch-screen_80x24.json" : "git_branch-screen_160x40.json");
    const n = notice(1, { tone, text: WORDS, lifetime_ms: 60000 });
    m.snapshot({ chrome: { ...booksChrome({ width: cols, height: rows }), notices_visible: true, notices: { now_ms: 0, revision: 1, through_id: 1, active: [n], hidden: 0 } }, screen, section: null, focus: "middle", motion: { level: "off", slots, cues: [], timings: {} } }, 1);
    await until(() => plain(m.frame()).includes(WORDS));
    return m;
  };

  it.each([[80, 24], [160, 40]] as const)("%ix%i: the words stand whole on the panel's last lines, inside its border, with the error icon first", async (cols, rows) => {
    const m = await open(cols, rows, "error");
    const lines = plain(m.frame()).split("\n");
    const y = lines.findIndex((l) => l.includes(WORDS));
    expect(y).toBe(rows - 3); // one toast: the last line inside the panel, over its bottom border
    expect(lines[y]).toContain(`${iconOf("error")!.glyph} ${WORDS}`);
    expect(lines[y]!.trimEnd().endsWith("│")).toBe(true); // the panel's right border closes the row
    expect(lines[rows - 2]).toContain("╰");
    expect(lines.length).toBeLessThanOrEqual(rows);
  });

  it.each([[80, 24], [160, 40]] as const)("%ix%i: the error toast is on the error tint and bold, the info one is not", async (cols, rows) => {
    const cellsOfToast = async (tone: "error" | "info") => {
      const m = await open(cols, rows, tone);
      const grid = parseAnsi(m.frame());
      const y = rows - 3;
      const text = grid[y]!.map((c) => c.ch).join("");
      const at = text.indexOf(WORDS);
      return { words: grid[y]!.slice(at, at + WORDS.length), icon: grid[y]![text.indexOf(iconOf(tone)!.glyph)]! };
    };
    const error = await cellsOfToast("error");
    const info = await cellsOfToast("info");
    const ground = bgOf("bg.overlay");
    expect(error.words.every((c) => c.bold)).toBe(true);
    expect(info.words.some((c) => c.bold)).toBe(false);
    expect(error.words[0]!.bg).toBeTruthy();
    expect(error.words[0]!.bg).not.toBe(ground);
    expect(error.words[0]!.bg).not.toBe(info.words[0]!.bg);
    expect(error.icon.fg).toBe(slotStyle("status.error").color);
    expect(info.icon.fg).not.toBe(slotStyle("status.error").color);
  });

  it("with icons off the word `error` leads in the error colour", () => {
    applyLook(defaultLook("off"));
    const row = layoutNotices([notice(1, { tone: "error", text: WORDS })], 0, noticeContent(160, 40, true), 200, true)[0]!;
    expect(row.cells.slice(1, 7).map((c) => c.ch).join("")).toBe("error ");
    expect(row.cells.slice(1, 6).every((c) => c.s.color === slotStyle("status.error").color)).toBe(true);
    expect(row.cells.map((c) => c.ch).join("")).toContain(WORDS);
  });
});

// ---------------------------------------------------------------------------------------------------------------
// 8. GitLab: the x menu's shift key word, the tracker chip and column at 80 columns, a rich table in columns
// ---------------------------------------------------------------------------------------------------------------
describe("8. gitlab", () => {
  const item = (label: string, keys: string, over: Partial<Choice> = {}, n = 0): Choice => ({ value: label, label, hint: "", keys, target: `chrome:menu:${n}`, ...over });
  // The row actions of a merge request (crates/radar-gitlab agent_context.rs, list_actions): the agent rows are keyed
  // by a capital, so the host sends them with mnemonic_shift.
  const xMenu: MenuView = {
    id: "actions",
    title: "api !410",
    question: false,
    selected: 0,
    items: [
      item("open its page", "⏎", {}, 0),
      item("refresh now", "r", { mnemonic_at: 0 }, 1),
      { value: "", label: "with an agent", hint: "", heading: true, shown_only: true },
      item("ask an agent about it…", "A", { mnemonic_at: 0, mnemonic_shift: true }, 3),
      item("review it locally…", "i", { mnemonic_at: 3 }, 4),
      item("go to a tenancy, repo or merge request…", ":", {}, 5),
    ] as Choice[],
  };

  it("the x menu draws `shift` before the words of a capital-keyed row, underlines its letter and has no key column for it", () => {
    applyLook(defaultLook("off"));
    const laid = layoutMenu(xMenu, 160, 1, 38);
    const text = laid.lines.map((l) => l.map((c) => c.text).join(""));
    const ask = laid.lines.find((l) => l.map((c) => c.text).join("").includes("ask an agent"))!;
    const row = ask.map((c) => c.text).join("");
    expect(row).toContain("shift ask an agent about it…");
    expect(row).not.toMatch(/\bA\b {2}/); // the letter is not repeated in the key column
    const shift = ask.find((c) => c.text === "shift ")!;
    expect(shift.style).toMatchObject({ ...slotStyle("accent.secondary"), bold: true });
    expect(ask.filter((c) => c.style?.underline).map((c) => c.text).join("")).toBe("a");
    // a lowercase row keeps its word without `shift`, with the letter underlined where it stands; a named key keeps its column
    expect(text.find((l) => l.includes("review it locally"))).not.toContain("shift");
    const review = laid.lines.find((l) => l.map((c) => c.text).join("").includes("review it locally"))!;
    expect(review.filter((c) => c.style?.underline).map((c) => c.text).join("")).toBe("i");
    expect(text.find((l) => l.includes("open its page"))).toMatch(/⏎ +open its page/);
    expect(text.find((l) => l.includes("go to a tenancy"))).toMatch(/: +go to a tenancy/);
    // the heading is a heading: no target
    expect(laid.hits.map((h) => h.target)).not.toContain(undefined);
    expect(laid.hits).toHaveLength(5);
  });

  describe("the tracker's column and chip", () => {
    const state = (text: string) => ({ heading: "state", text: t(text) });
    const title = "WATCH-12: Add a stats endpoint for the dashboard";
    const tree = (): Node =>
      ({
        node: "tree",
        header: t("OPENED BY everyone"),
        columns: [
          { heading: "state", width: 20, keep: 6 },
          { heading: "ticket", width: 19, keep: 3, extra: true },
          { heading: "updated", width: 8, keep: 4 },
        ],
        name_width: 30,
        open: [["t:watch"], ["t:watch", "r:ankh-morpork/watch/api"]],
        rows: [
          {
            id: "t:watch",
            name: t("ankh-morpork/watch"),
            children: [
              {
                id: "r:ankh-morpork/watch/api",
                name: t("api"),
                children: [
                  {
                    id: "m:263",
                    name: [{ text: "!263 ", ink: "accent_secondary" }, { text: title, ink: "primary" }, { text: "  ", ink: "muted" }, { text: "Sam V.", ink: "muted" }],
                    chips: { gives: 1, text: [{ text: " " }, { text: "tracked", chip: true, ink: "info" }] },
                    cells: [state("◷ needs 1 approval"), { heading: "ticket", text: t("WATCH-12 · Review"), target: "ticket:WATCH-12" }, { heading: "updated", text: t("16m") }],
                  },
                ],
              },
            ],
          },
        ],
      }) as unknown as Node;
    const draw = (width: number) => frameAt(renderNode(tree(), { width, height: 12, focused: true }), width, 12);

    it("wide: the ticket has its own column, the title is cut with … and the tracker's chip follows the name", () => {
      const out = draw(108);
      expect(out.split("\n")[0]).toContain("TICKET");
      const row = out.split("\n").find((l) => l.includes("!263"))!;
      expect(row).toContain("WATCH-12 · Review");
      const cut = /!263 (WATCH-12: [^…]*…) +Sam V\. +tracked +◷ needs/.exec(row);
      expect(cut, row).not.toBeNull();
      expect(title.startsWith(cut![1]!.slice(0, -1))).toBe(true);
      // with the room for it the title is whole and the chip is still there
      expect(draw(150)).toMatch(/!263 WATCH-12: Add a stats endpoint for the dashboard +Sam V\. +tracked/);
    });

    it("at the width of Ratatui's 160 columns the chip gives way, the title keeps its room and the column stays", () => {
      const out = draw(84);
      const row = out.split("\n").find((l) => l.includes("!263"))!;
      expect(out.split("\n")[0]).toContain("TICKET");
      expect(row).toContain("WATCH-12 · Re");
      expect(row).not.toContain("tracked");
      expect(row).toMatch(/!263 WATCH-12: Add a st/);
    });

    it("at the width of 80 columns the ticket column goes whole, never half a word, and the chip stays away", () => {
      const out = draw(58);
      expect(out).not.toContain("TICKET");
      expect(out).not.toMatch(/WATCH-12 ·/);
      expect(out).not.toContain("tracked");
      expect(out).toContain("!263");
      expect(out.split("\n")[0]).toContain("STATE");
      for (const line of out.split("\n")) expect([...line].length).toBeLessThanOrEqual(58);
    });

    it("the ticket cell is a target of its own (a press shows the ticket), the row another", () => {
      const hits: Hit[] = [];
      frameAt(<Box>{renderNode(tree(), { width: 150, height: 12, focused: true, registry: { add: (h: Hit) => hits.push(h) } })}</Box>, 150, 12);
      expect(hits.map((h) => h.target)).toEqual(expect.arrayContaining(["ticket:WATCH-12"]));
      const cell = hits.find((h) => h.target === "ticket:WATCH-12")!;
      expect(frameAt(<Box>{renderNode(tree(), { width: 150, height: 12, focused: true })}</Box>, 150, 12).split("\n")[cell.y]!.slice(cell.x, cell.x + 7)).toBe("WATCH-1");
    });
  });

  describe("a rich table is in columns", () => {
    const T = (s: string) => ({ text: s });
    const code = (s: string) => ({ text: s, marks: { code: true } });
    const node = {
      node: "rich",
      blocks: [
        {
          block: "doc",
          urls: false,
          doc: [
            { t: "para", text: [T("This merge request updates one dependency.")] },
            {
              t: "table",
              head: [[T("Package")], [T("Change")], [T("Age")], [T("Confidence")]],
              rows: [
                [[{ text: "watch-lib", kind: { link: { url: "https://example.test/watch-lib" } } }], [code("1.2.2"), T(" -> "), code("1.2.3")], [T("age")], [T("confidence")]],
                [[T("lamp-lib")], [code("0.9.0"), T(" -> "), code("1.0.0")], [T("2d")], [T("high")]],
              ],
            },
          ],
        },
      ],
    } as unknown as Node;

    it.each([160, 80])("at %i columns the heading and every row start each cell at the same column, with no raw markdown", (width) => {
      const lines = frameAt(renderNode(node, { width, height: 12, focused: true }), width, 12).split("\n");
      const head = lines.find((l) => l.includes("Package"))!;
      const first = lines.find((l) => l.includes("watch-lib"))!;
      const second = lines.find((l) => l.includes("lamp-lib"))!;
      expect(first).toContain("watch-lib  1.2.2 -> 1.2.3"); // Ratatui's `acme/widget-lib  1.2.2 -> 1.2.3`: the words, two cells, the next column
      for (const word of ["Change", "Age", "Confidence"]) {
        const at = head.indexOf(word);
        expect(first.slice(at, at + 3), word).not.toMatch(/^\s/);
        expect(second.slice(at, at + 2), word).not.toMatch(/^\s/);
        expect(first[at - 1], `${word} column separator`).toBe(" ");
      }
      expect(head.indexOf("Change")).toBe(first.indexOf("1.2.2"));
      expect(head.indexOf("Change")).toBe(second.indexOf("0.9.0"));
      expect(head.indexOf("Age")).toBe(first.indexOf("age"));
      expect(head.indexOf("Confidence")).toBe(first.indexOf("confidence"));
      const out = lines.join("\n");
      for (const never of ["|---", "](http", "![", " | "]) expect(out).not.toContain(never);
    });
  });
});
