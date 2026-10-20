import { vi } from "vitest";
const environment = vi.hoisted(() => {
  const forceColor = process.env.FORCE_COLOR;
  process.env.FORCE_COLOR = "3";
  return { forceColor };
});
import { afterAll, afterEach, describe, expect, it } from "vitest";
import { freezeClock, mount, until } from "./harness.js";
import { booksChrome } from "../chrome/fixture.js";
import { parseAnsi, type TCell } from "../nodes/ansi.js";
import { applyLook, currentLook, defaultLook, bgOf } from "../../src/text/look.js";
import { fastOutputInstalled, useFastOutput, type Layer } from "../../src/app/fast-output.js";
import * as scrim from "../../src/app/scrim.js";
import { selected } from "../../src/app/selected.js";
import { motion } from "../../src/motion/motion.js";
import { shellMarks } from "../../src/motion/attach.js";
import type { MenuView } from "../../src/chrome/types.js";

// App/MenuBox/session wiring, not another scrim arithmetic or native parity suite.
// Dense nonselected document cells expose holes and keep real line glyphs under both shadow edges.
// At 80 columns, the pane border and document inset leave 76 content cells without wrapping.
const screen = { version: 1, scope: "watch", actions: [], panes: [{
  pane: "middle", title: "Watch ledger", body: { node: "document",
    lines: Array.from({ length: 20 }, () => [{ text: "┼".repeat(76), ink: "muted", strong: true }]),
  },
}] };
const help: MenuView = {
  id: "keys", title: "Watch keys", question: false, selected: 0,
  items: [
    { value: "watch.read", label: "read reports", hint: "Watch ledger", key: "r", target: "chrome:menu:0" },
    { value: "watch.post", label: "post report", hint: "ask first", key: "p", target: "chrome:menu:1" },
  ],
};
const confirm: MenuView = {
  id: "confirm", title: "Post Watch report?", question: true, selected: 1,
  items: [
    { value: "", label: "Send the report to Sam Vimes.", hint: "", note: true },
    { value: "yes", label: "post it", hint: "", key: "y", target: "chrome:menu:1" },
    { value: "no", label: "drop it", hint: "", key: "n", target: "chrome:menu:2" },
  ],
};
const palette: MenuView = {
  id: "palette", title: "Watch commands", question: false, selected: 0, typed: "wa", caret: 2,
  items: [
    { value: "watch.read", label: "Watch reports", hint: "Vimes", keys: "r", target: "chrome:menu:0" },
    { value: "watch.search", label: "Watch search", hint: "Carrot", keys: "s", target: "chrome:menu:1" },
  ],
  palette: { matched: 2, total: 7, highlights: [[0, 1], [0, 1]] },
};

const originalLook = currentLook();
const originalFast = fastOutputInstalled();
const live: Array<() => void> = [];
afterEach(() => {
  live.splice(0).reverse().forEach((stop) => stop());
  motion.reset(() => performance.now());
  applyLook(originalLook);
  useFastOutput(originalFast);
  vi.restoreAllMocks();
});
afterAll(() => {
  if (environment.forceColor === undefined) delete process.env.FORCE_COLOR;
  else process.env.FORCE_COLOR = environment.forceColor;
});

const text = (rows: TCell[][]) => rows.map((row) => row.map((cell) => cell.ch).join(""));
/** Find the actual visible border, independently of layoutMenu and the hidden layer mark. */
function menuBox(rows: TCell[][], title: string): Layer {
  const lines = text(rows);
  const y = lines.findIndex((line) => line.includes(`╭ ${title}`));
  expect(y, `the rendered ${title} border exists`).toBeGreaterThan(1);
  const x = lines[y]!.indexOf(`╭ ${title}`);
  const right = lines[y]!.indexOf("╮", x);
  const bottom = rows.findIndex((row, at) => at > y && row[x]?.ch === "╰");
  expect(right).toBeGreaterThan(x + 2);
  expect(bottom).toBeGreaterThan(y + 1);
  return { x, y, w: right - x + 1, h: bottom - y + 1 };
}

describe("the actual App's overlay scrim lifecycle", () => {
  it("paints one help, confirm or typed layer, preserves its interior and shadow lines, then restores and reopens cleanly", async () => {
    freezeClock();
    applyLook(defaultLook("off"));
    useFastOutput();
    motion.reset(() => 0);
    // Both spies call through. Keep the real hover/selection paint hook and real MenuBox layerMark installed.
    const shade = vi.spyOn(scrim, "paintScrim");
    const painted = vi.spyOn(selected, "paint");
    const m = mount(80, 24);
    live.push(m.stop);
    m.welcome();
    let cursor = 0;
    const snapshot = (menu?: MenuView) => m.snapshot({ screen,
      chrome: booksChrome({ width: 80, height: 24, sources: [], menu }),
    }, ++cursor);
    const checkpoint = async (act: () => void) => {
      const before = painted.mock.calls.length;
      act();
      await until(() => painted.mock.calls.length > before);
      expect(painted.mock.calls.length, "the session change reached finished-frame paint").toBeGreaterThan(before);
      const last = painted.mock.calls.length - 1;
      const order = painted.mock.invocationCallOrder[last]!;
      const previous = painted.mock.invocationCallOrder[last - 1] ?? 0;
      const layers = shade.mock.calls.flatMap((args, i) =>
        shade.mock.invocationCallOrder[i]! > previous && shade.mock.invocationCallOrder[i]! < order ? [args[3]] : []);
      const [raw, width, output] = painted.mock.calls[last]!;
      expect(width).toBe(80);
      expect(m.frame()).toBe(output);
      return { output, raw: parseAnsi(raw), rows: parseAnsi(output), layers };
    };
    const baseline = await checkpoint(() => snapshot());
    expect(baseline.layers).toEqual([]);
    expect(baseline.rows).toEqual(baseline.raw);
    expect(baseline.rows[3]![2]).toEqual({ ch: "┼", fg: "#6272a4", bg: "#12101a", bold: true });
    expect(shellMarks(m.session, 80, 24).popup).toBeUndefined();

    const active = (frame: Awaited<ReturnType<typeof checkpoint>>, title: string) => {
      const box = menuBox(frame.rows, title);
      expect(frame.layers, "one mark at the actual displayed menu, never accumulated layers").toEqual([box]);
      expect(shellMarks(m.session, 80, 24).popup, "motion uses that same displayed menu").toEqual(box);
      // Documented implementation: fg 55%, bg 43%, then the shadow. The issue's approximate 70%-both
      // wording is a separate acceptance interpretation, not a reason to change production here.
      expect(frame.rows[3]![2]).toEqual({ ch: "┼", fg: "#323854", bg: "#0f0d17" });
      for (const [x, y] of [[box.x + box.w, box.y + 1], [box.x + 1, box.y + box.h], [box.x + box.w, box.y + box.h]]) {
        expect(baseline.rows[y!]![x!]!.ch, `${title} shadow (${x},${y}) over ${JSON.stringify(box)} covers populated body cells: ${text(baseline.rows)[y!]}`).toBe("┼");
        expect(frame.rows[y!]![x!]).toEqual({ ch: "┼", fg: "#202236", bg: "#060509" });
      }
      let coveredSpaces = 0;
      for (let y = box.y; y < box.y + box.h; y++) {
        const row = frame.rows[y]!.slice(box.x, box.x + box.w);
        expect(row, "scrim and shadow leave every menu cell's original glyph and style intact").toEqual(frame.raw[y]!.slice(box.x, box.x + box.w));
        expect(row.some((cell) => cell.ch === "┼"), "the populated body cannot leak through the popup").toBe(false);
        for (let x = box.x + 1; x < box.x + box.w - 1; x++) {
          if (frame.rows[y]![x]!.ch === " " && baseline.rows[y]![x]!.ch === "┼") coveredSpaces++;
        }
      }
      expect(coveredSpaces, "opaque padding really overwrote nonblank body cells").toBeGreaterThan(20);
      expect(frame.rows[box.y]![box.x]!.bg).toBe(bgOf("bg.overlay"));
      return frame.output;
    };
    const closed = async () => {
      const frame = await checkpoint(() => snapshot());
      expect(m.session.state.questions).toEqual([]);
      expect(m.session.state.inputRequest).toBeUndefined();
      expect(frame.layers).toEqual([]);
      expect(shellMarks(m.session, 80, 24).popup).toBeUndefined();
      expect(frame.output, "no stale dimming, shadow or popup remains anywhere").toBe(baseline.output);
    };

    const opened = active(await checkpoint(() => {
      m.conn.receive({ type: "question", id: 1, cursor: cursor + 1, menu: help });
      snapshot(help);
    }), help.title);
    expect(active(await checkpoint(() => snapshot(help)), help.title), "an identical render does not dim again").toBe(opened);

    const replacement = active(await checkpoint(() => {
      m.session.sendIntent({ intent: "activate", target: "chrome:menu:1" });
      m.conn.receive({ type: "question", id: 2, cursor: cursor + 1, menu: confirm });
      snapshot(confirm);
    }), confirm.title);
    expect(m.session.state.questions.map((question) => question.menu.id)).toEqual(["confirm"]);
    expect(text(parseAnsi(replacement)).join("\n")).not.toContain(help.title);
    expect(active(await checkpoint(() => snapshot(confirm)), confirm.title)).toBe(replacement);
    m.type("n");
    await until(() => m.intents().at(-1)?.intent.action === "no");
    expect(m.intents().at(-1)?.intent).toEqual({ intent: "invoke", action: "no" });
    await closed();

    const searched = active(await checkpoint(() => {
      m.conn.receive({ type: "input_request", id: 3, cursor: cursor + 1, title: "Watch search", value: "wa" });
      snapshot(palette);
    }), palette.title);
    expect(m.session.state.inputRequest).toBeDefined();
    expect(text(parseAnsi(searched)).join("\n")).toContain("Watch commands · 2 of 7");
    expect(active(await checkpoint(() => snapshot(palette)), palette.title)).toBe(searched);
    await closed();

    // Before the host snapshot arrives, the session's input_request supplies the displayed typed popup itself.
    const request = () => m.conn.receive({ type: "input_request", id: 4, cursor: cursor + 1, title: "Watch query", value: "Ankh" });
    const typed = active(await checkpoint(request), "Watch query");
    expect(text(parseAnsi(typed)).join("\n")).toContain("Ankh");
    expect(active(await checkpoint(request), "Watch query")).toBe(typed);
    await closed();

    expect(active(await checkpoint(() => {
      m.conn.receive({ type: "question", id: 5, cursor: cursor + 1, menu: help });
      snapshot(help);
    }), help.title), "reopening starts from the same clean background").toBe(opened);
    await closed();
  });
});
