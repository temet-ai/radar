import { vi } from "vitest";
vi.hoisted(() => { process.env.FORCE_COLOR = "3"; });
import React from "react";
import fs from "node:fs";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { render } from "ink";
import { EventEmitter } from "node:events";
import { App } from "../../src/app/App.js";
import { Session, capabilitiesFrom } from "../../src/app/session.js";
import { FakeConnection } from "../../src/transport/fake.js";
import { MouseBus } from "../../src/input/stdin-tap.js";
import { attachMotion } from "../../src/motion/attach.js";
import { motion } from "../../src/motion/motion.js";
import { parseFrame } from "../../src/motion/frame.js";
import { frameOf } from "../../src/chrome/Chrome.js";
import { Out, until, tick } from "../app/harness.js";
import { booksChrome } from "../chrome/fixture.js";
import { notice, historyStatus } from "./fixture.js";
import { parseAnsi } from "../nodes/ansi.js";
import { applyLook, defaultLook } from "../../src/text/look.js";
const plain = (s: string) => s.replace(/\x1b\[[0-9;]*m/g, "");
const stops: Array<() => void> = [];
afterEach(() => { for (const stop of stops.splice(0)) stop(); motion.reset(); applyLook(defaultLook()); });
const slots = { base: "bg.base", surface: "bg.surface", overlay: "bg.overlay", glow: "accent.primary", text: "text.primary" };
function capture(name: string, ansi: string, width: number, height: number) {
  const directory = process.env.RADAR_NOTICE_SHOTS_DIR; if (!directory) return;
  fs.mkdirSync(directory, { recursive: true }); fs.writeFileSync(path.join(directory, `ink-${name}-${width}x${height}.ansi`), ansi);
  fs.writeFileSync(path.join(directory, `ink-${name}-${width}x${height}.json`), JSON.stringify({ source: "actual App + notice output compositor", width, height, cells: parseFrame(ansi, width, height).cells }));
}
function open(width: number, height: number, level = "off") {
  let now = 0, cursor = 0;
  const out = new Out(width, height), conn = new FakeConnection(), mouse = new MouseBus();
  const stdin = Object.assign(new EventEmitter(), { isTTY: false, setEncoding() {}, resume() {}, pause() {}, ref() {}, unref() {}, read: () => null });
  const session = new Session(conn, capabilitiesFrom({}, out), undefined, false, () => now);
  motion.reset(); const detach = attachMotion(session, mouse, out);
  const instance = render(<App session={session} mouse={mouse} />, { stdout: out as any, stdin: stdin as any, stderr: new Out(width, height) as any, debug: true, exitOnCtrlC: false, patchConsole: false });
  stops.push(() => { instance.unmount(); detach(); }); conn.receive({ type: "welcome", protocol: 1, view: 1, cursor: 0 });
  const chrome = booksChrome({ width, height, status: { scope: "WATCH", hints: [
    { keys: "q", label: "quit", keys_of: [{ label: "q", chord: { key: { char: "q" } }, target: "chrome:key:0:0" }] },
    { keys: "x", label: "menu", keys_of: [{ label: "x", chord: { key: { char: "x" } }, target: "chrome:key:1:0" }] },
    { keys: "~", label: "notices", keys_of: [{ label: "~", chord: { key: { char: "~" } }, target: "chrome:key:2:0" }] },
  ] } });
  const screen = { id: "watch", scope: "watch", panes: [{ pane: "middle", title: "City Watch", body: { node: "document", lines: [[{ text: "Underlying report" }]] } }], actions: [] };
  const snapshot = (over: Record<string, unknown> = {}) => conn.receive({ type: "snapshot", cursor: ++cursor, chrome: { ...chrome, notices_visible: true, ...(over.notice_history ? { status: historyStatus } : {}), ...over }, screen, section: null, focus: "middle", motion: { level, slots, cues: [], timings: {} } });
  return { out, conn, session, mouse, snapshot, time: (t: number) => { now = t; }, frame: () => plain(out.last()), intents: () => (conn.sent as any[]).filter((m) => m.type === "intent").map((m) => m.intent) };
}
describe("notices through actual App and output composition", () => {
  it.each([[80, 24], [160, 40]])("paints inside the %dx%d panel and opens a distinct history with its real footer", async (width, height) => {
    const app = open(width, height), first = notice(1, { text: "Watch report saved" }), second = notice(2, { tone: "error", text: "Watch report unavailable", lifetime_ms: 10000 });
    const state = { now_ms: 0, revision: 2, through_id: 2, active: [first, second], hidden: 0 }; app.snapshot({ notices: state }); await until(() => app.frame().includes(second.text));
    capture("active", app.out.last(), width, height);
    const lines = app.frame().split("\n"); expect(lines[height - 4]).toContain(first.text); expect(lines[height - 3]).toContain(second.text); expect(lines[height - 2]).toContain("╰"); expect(lines[height - 1]).toContain("quit");
    const cells = parseAnsi(app.out.last()); expect(cells[height - 4]![width - 2]!.dim).toBe(true); expect(cells[height - 3]![width - 2]!.bold).toBe(true); expect(app.session.noticeHits.map((hit) => hit.target)).toEqual(["chrome:notice:1", "chrome:notice:2"]);
    app.snapshot({ notices: state, notices_visible: false, notice_history: { entries: [first, second], detail: second, omitted: 0, selected: 0, scroll: 0, detail_scroll: 0 } }); await until(() => app.frame().includes("Recent notices")); capture("history", app.out.last(), width, height);
    expect(app.frame()).toContain("This session only · 2 retained"); expect(app.frame()).toContain("Notice details · error"); expect(app.frame()).not.toContain("Underlying report"); expect(app.frame().split("\n")[height - 1]).toMatch(/WATCH.*move.*details.*back/); expect(app.session.noticeHits).toEqual([]);
    app.mouse.emitInput([{ kind: "key", chord: { key: "esc" } }]); expect(app.intents()).toEqual([{ intent: "notice_input", escape: true, through_id: 2 }, { intent: "key", chord: { key: "esc" } }]);
    app.snapshot({ notices: state }); await until(() => app.frame().includes("Underlying report")); expect(app.frame()).not.toContain(second.text); expect(app.session.noticeHits).toEqual([]); expect(motion.active).toBe(false);
  });
  it("shows rich immediate occurrences before snapshots and suppresses them under a modal without resurrection", async () => {
    const app = open(80, 24); app.snapshot(); await tick(); app.conn.receive({ type: "notice", cursor: 1, text: "ignored legacy duplicate", occurrence: notice(1), revision: 1, now_ms: 0 }); await until(() => app.frame().includes("Watch updated")); expect(app.frame()).not.toContain("ignored legacy duplicate");
    app.snapshot({ notices: { now_ms: 0, revision: 1, through_id: 1, active: [notice(1)], hidden: 0 }, menu: { id: "watch", title: "Confirm Watch action", question: true, items: [] }, notices_visible: false }); await until(() => app.frame().includes("Confirm Watch action")); expect(app.session.noticeHits).toEqual([]); expect(app.frame()).not.toContain("Watch updated");
    app.time(5000); app.snapshot({ notices: { now_ms: 5000, revision: 2, through_id: 1, active: [], hidden: 0 } }); await until(() => !app.frame().includes("Confirm Watch action")); expect(app.frame()).not.toContain("Watch updated"); expect(motion.active).toBe(false);
  });
  it("history back click sends the captured seen-ID watermark before the stable close target", async () => {
    const app = open(80, 24), first = notice(1), target = "chrome:notices:close";
    app.snapshot({ notices: { now_ms: 0, revision: 1, through_id: 1, active: [first], hidden: 0 }, notices_visible: false, notice_history: { entries: [first], detail: first, omitted: 0, selected: 0, scroll: 0, detail_scroll: 0 } }); await until(() => app.frame().includes("Recent notices"));
    const hit = frameOf(app.session.state.snapshot!.chrome, null, 80, 24).status.hits.find((h) => h.target === target)!;
    app.mouse.emitInput([{ kind: "mouse", data: `\x1b[<0;${hit.x + 1};${hit.y + 1}M\x1b[<0;${hit.x + 1};${hit.y + 1}m` }]);
    expect(app.intents()).toEqual([{ intent: "notice_input", escape: true, through_id: 1 }, { intent: "activate", target }]);
    app.conn.receive({ type: "notice", cursor: 1, text: "new", occurrence: notice(2), revision: 2, now_ms: 0 }); expect(app.session.noticeStore.active().map((n) => n.id)).toEqual([2]);
  });
  it.each([[80, 24], [160, 40]])("drains the actual %dx%d frame without retaining expired pixels", async (width, height) => {
    const app = open(width, height, "basic"), first = notice(1, { text: "Watch report saved" }), second = notice(2, { tone: "error", text: "Watch report unavailable", lifetime_ms: 10000 });
    const state = { now_ms: 0, revision: 2, through_id: 2, active: [first, second], hidden: 0 }; app.snapshot({ notices: state }); await until(() => app.frame().includes(second.text)); capture("drain-full", app.out.last(), width, height);
    const full = app.out.last(); app.time(2500); app.snapshot({ notices: { ...state, now_ms: 2500 } }); await until(() => app.out.last() !== full); capture("drain-partial", app.out.last(), width, height);
    const grid = parseFrame(app.out.last(), width, height); expect(grid.at(width - 65, height - 4).bg).not.toEqual(grid.at(width - 2, height - 4).bg);
    app.time(10000); app.snapshot({ notices: { ...state, now_ms: 10000, revision: 3, active: [] } }); await until(() => !app.frame().includes(second.text)); capture("drain-expired", app.out.last(), width, height); expect(app.frame()).not.toContain(first.text); expect(app.session.noticeHits).toEqual([]); expect(motion.active).toBe(false);
  });
});
