import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { dockClick, dockHint, dockKey, emptyLocal, matchingCommands, pendingAsk, pendingPermission, pendingQuestion, type DockLocal } from "../../src/dock/composer.js";
import { activityCells, itemLayout, itemRows, spinAt, tabRowCells, tabState, transcriptRows } from "../../src/dock/rows.js";
import { emptyWords } from "../../src/dock/Dock.js";
import { cellWidth } from "../../src/chrome/width.js";
import { iconOf, slotStyle } from "../../src/text/look.js";
import { SPIN } from "../../src/chrome/layout.js";
import type { Chord } from "../../src/chrome/types.js";
import type { DockActivity, DockItem, DockTab, DockView } from "../../src/dock/model.js";

// Sessions restored from the last radar run: a request or question that waited when radar closed comes back "lapsed" (the
// process that asked is gone), a session not attached to its provider is "detached" and one being read back is
// "resuming". Recorded by radar-core as test/fixtures/dock/restored_* and detached_*; drawn whole in recorded.test.tsx.

const text = (cs: { ch: string }[]) => cs.map((c) => c.ch).join("");
const g = (n: string) => (iconOf(n)?.glyph ?? "").padEnd(2);
const c = (s: string, mods: Partial<Chord> = {}): Chord => ({ key: { char: s }, ...mods });
const muted = slotStyle("text.muted").color;

const lapsedPerm: DockItem = { item: "permission", id: "perm-1", kind: "shell", target: "git push origin agent-dock", actions: [], lapsed: true };
// A lapsed request that (wrongly) still lists actions and a diff: it must not be pressable or openable either.
const lapsedWithActions: DockItem = { ...lapsedPerm, actions: ["approve_once", "approve_for_session", "always_allow", "deny", "view_diff"], diff: ["+x"] } as DockItem;
const lapsedAsk: DockItem = {
  item: "question",
  id: "ask-1",
  state: "lapsed",
  questions: [{ id: "0", header: "Remote", text: "Which remote?", choices: [{ label: "origin" }], free_text: true, multiple: false, answered: true, answer: [] }],
};
const tab = (t: Partial<DockTab> = {}): DockTab => ({ id: "a", title: "one", state: "idle", ...t });
const view = (over: Partial<DockView> = {}): DockView => ({
  visible: true,
  focused: true,
  tabs: [tab()],
  active: "a",
  composer: { can_send: true, can_stop: false, history: [], commands: [] },
  ...over,
});
const withTab = (state: DockTab["state"]) => view({ tabs: [tab({ state })] });
const withItems = (items: DockItem[]) => view({ transcript: items });
const type = (l: DockLocal, v: DockView, s: string) => [...s].reduce((acc, ch) => dockKey(acc, v, c(ch)).local, l);

describe("a request that lapsed when radar closed", () => {
  it("draws muted with the skipped mark, the heading in words, the target, and no button, diff or key word", () => {
    for (const w of [40, 60, 120]) {
      const l = itemLayout(lapsedWithActions, w, ["perm-1"]);
      const rows = l.rows.map(text);
      expect(rows[0]!.trimEnd(), String(w)).toBe(`${g("run_skipped")}shell: lapsed · radar closed before this was answered`.slice(0, w).trimEnd());
      expect(rows[1]!.trim()).toBe("git push origin agent-dock".slice(0, w - 3));
      expect(rows.join("\n")).not.toMatch(/approve|deny|view diff|allow|\+x/);
      expect(l.hits.flat()).toEqual([]);
      for (const r of l.rows) expect(cellWidth(text(r))).toBeLessThanOrEqual(w);
    }
  });
  it("is muted throughout: no warning mark, no bold warning heading", () => {
    const rows = itemRows(lapsedPerm, 80);
    expect(rows[0]![0]!.s.color).toBe(muted);
    expect(rows[0]!.find((c) => c.ch === "s")!.s.color).toBe(muted);
    expect(rows[0]!.some((c) => c.s.bold)).toBe(false);
    expect(rows.flat().some((c) => c.s.color === slotStyle("status.warning").color)).toBe(false);
    expect(rows[1]!.find((c) => c.ch === "g")!.s.color).toBe(muted);
  });
  it("a request still waiting keeps the warning mark and its buttons", () => {
    const l = itemLayout({ ...lapsedPerm, lapsed: false, actions: ["approve_once", "deny"] } as DockItem, 60);
    expect(text(l.rows[0]!)).toContain("allow shell?");
    expect(l.hits.flat().length).toBe(2);
  });
});

describe("a question that lapsed when radar closed", () => {
  it("draws each question as a muted row ending in → lapsed, with no choices, keys or 'then:'", () => {
    for (const w of [40, 80]) {
      const rows = itemRows(lapsedAsk, w);
      expect(rows.map(text).map((r) => r.trimEnd())).toEqual([`${g("mention")}Remote: Which remote? → lapsed`.slice(0, w)]);
      expect(rows[0]!.every((c) => c.ch === " " || c.s.color === muted)).toBe(true);
    }
    const second = { id: "1", text: "And which branch?", free_text: true, multiple: false, answered: false };
    const two = { ...lapsedAsk, questions: [(lapsedAsk as Extract<DockItem, { item: "question" }>).questions[0]!, second] } as DockItem;
    expect(itemRows(two, 80).map((r) => text(r).trimEnd())).toEqual([`${g("mention")}Remote: Which remote? → lapsed`, "  And which branch? → lapsed"]);
    expect(itemLayout(two, 80).hits.flat()).toEqual([]);
  });
  it("a declined one still says declined, an answered one its answer", () => {
    expect(text(itemRows({ ...lapsedAsk, state: "declined" } as DockItem, 80)[0]!)).toContain("→ declined");
    const q = (lapsedAsk as Extract<DockItem, { item: "question" }>).questions[0]!;
    expect(text(itemRows({ ...lapsedAsk, state: "answered", questions: [{ ...q, answer: ["origin"] }] } as DockItem, 80)[0]!)).toContain("→ origin");
  });
});

describe("what a lapsed ask leaves to the keys and the mouse", () => {
  it("is never picked as what waits", () => {
    for (const v of [withItems([lapsedPerm]), withItems([lapsedWithActions]), withItems([lapsedAsk]), withItems([lapsedWithActions, lapsedAsk])]) {
      expect(pendingAsk(v)).toBeUndefined();
      expect(pendingPermission(v)).toBeUndefined();
      expect(pendingQuestion(v)).toBeUndefined();
    }
    // A live request is picked whichever side of a lapsed one it sits.
    const live: DockItem = { item: "permission", id: "p2", kind: "shell", target: "ls", actions: ["approve_once", "deny"] };
    expect(pendingPermission(withItems([lapsedWithActions, live]))?.id).toBe("p2");
    expect(pendingPermission(withItems([live, lapsedWithActions]))?.id).toBe("p2");
  });
  it("a, s, d, v and the number keys are the composer's, not an answer; esc declines nothing", () => {
    for (const v of [withItems([lapsedWithActions]), withItems([lapsedAsk])]) {
      for (const ch of ["a", "s", "l", "d", "v", "1"]) {
        const r = dockKey(emptyLocal(), v, c(ch));
        expect(r.intents, ch).toEqual([]);
        expect(r.local.text, ch).toBe(ch);
        expect(r.local.diffs, ch).toEqual([]);
      }
      expect(dockKey(emptyLocal(), v, { key: "esc" }).intents).toEqual([{ dock: "blur" }]);
      expect(dockHint(v)).not.toContain("esc decline");
    }
  });
  it("a click on any of their targets does nothing", () => {
    const v = withItems([lapsedWithActions, lapsedAsk]);
    for (const t of ["dock:approve:perm-1", "dock:session:perm-1", "dock:always:perm-1", "dock:deny:perm-1", "dock:diff:perm-1", "dock:choose:ask-1:0:origin", "dock:submit:ask-1:0", "dock:decline:ask-1"])
      expect(dockClick(emptyLocal(), v, t), t).toBeUndefined();
  });
});

describe("alt-r", () => {
  it("on a detached tab sends resume and nothing else, keeping the draft", () => {
    const l = emptyLocal("half a message");
    expect(dockKey(l, withTab("detached"), c("r", { alt: true }))).toEqual({ local: l, intents: [{ dock: "resume" }], handled: true });
  });
  it("⏎ in an empty composer on a detached tab resumes it too; on an attached tab it sends nothing", () => {
    expect(dockKey(emptyLocal(), withTab("detached"), { key: "enter" }).intents).toEqual([{ dock: "resume" }]);
    expect(dockKey(emptyLocal(), withTab("idle"), { key: "enter" }).intents).toEqual([]);
    expect(dockKey(emptyLocal("hello"), withTab("detached"), { key: "enter" }).intents).toEqual([{ dock: "send", text: "hello" }]);
  });
  it("on an errored tab sends resume too (a resume that failed; the host ignores it when attached)", () => {
    expect(dockKey(emptyLocal(), withTab("error"), c("r", { alt: true })).intents).toEqual([{ dock: "resume" }]);
  });
  it("on any other tab falls through, unhandled", () => {
    for (const state of ["idle", "streaming", "approval", "stopped", "ended", "resuming"] as const) {
      const r = dockKey(emptyLocal(), withTab(state), c("r", { alt: true }));
      expect(r.intents, state).toEqual([]);
      expect(r.handled, state).toBe(false);
    }
  });
  it("is the chord alone: a plain r types, ctrl-r and ctrl-alt-r are not it", () => {
    const d = withTab("detached");
    expect(dockKey(emptyLocal(), d, c("r")).local.text).toBe("r");
    expect(dockKey(emptyLocal(), d, c("r", { ctrl: true })).intents).toEqual([]);
    expect(dockKey(emptyLocal(), d, c("r", { alt: true, ctrl: true })).intents).toEqual([]);
  });
  it("/resume typed is completed and sent as a message: the host handles it", () => {
    const v = view({
      tabs: [tab({ state: "detached" })],
      composer: { can_send: true, can_stop: false, commands: [{ name: "resume", detail: "attach to the provider again and read the session back (alt-r)" }] },
    });
    let l = type(emptyLocal(), v, "/res");
    expect(matchingCommands(l.text, v.composer?.commands).map((x) => x.name)).toEqual(["resume"]);
    l = dockKey(l, v, { key: "enter" }).local;
    expect(l.text).toBe("/resume ");
    expect(dockKey(l, v, { key: "enter" }).intents).toEqual([{ dock: "send", text: "/resume" }]);
  });
});

describe("the status line's keys", () => {
  it("leads with how to resume a detached tab, or that one is being read back, and keeps alt-f and alt-w", () => {
    expect(dockHint(withTab("detached"))).toBe("⏎ or alt-r resume · alt-f full · alt-w close");
    expect(dockHint(withTab("resuming"))).toBe("resuming… · alt-f full · alt-w close");
    expect(dockHint({ ...withTab("detached"), full: true })).toBe("⏎ or alt-r resume · alt-f split · alt-w close");
    expect(dockHint(withTab("idle"))).toBe("alt-f full · alt-w close");
    expect(dockHint({ ...withTab("detached"), focused: false })).toBe("⏎ or alt-r resume");
  });
});

describe("tab marks", () => {
  it("a detached tab shows the connection mark, muted", () => {
    expect(tabState(tab({ state: "detached" }))).toEqual({ glyph: iconOf("connection")!.glyph, slot: "text.muted" });
  });
  it("a resuming tab spins in the info colour, frame 0 when no seconds run", () => {
    expect(tabState(tab({ state: "resuming" }))).toEqual({ glyph: spinAt(0), slot: "status.info" });
    expect(SPIN).toContain(tabState(tab({ state: "resuming", running: 4 }))!.glyph);
    expect(tabState(tab({ state: "resuming", running: 4 }))!.glyph).not.toBe(tabState(tab({ state: "resuming", running: 5 }))!.glyph);
  });
  it("draw in the tab row in their own slot, whole, at every width, the name cut before the mark", () => {
    const tabs = [tab({ id: "a", title: "a very long session name indeed", state: "detached" }), tab({ id: "b", title: "codex", state: "resuming" })];
    for (const w of [40, 78, 158]) {
      const row = tabRowCells(tabs, 0, w);
      expect(cellWidth(text(row))).toBe(w);
      expect(row.find((c) => c.ch === iconOf("connection")!.glyph)!.s.color).toBe(muted);
      expect(row.find((c) => c.ch === spinAt(0))!.s.color).toBe(slotStyle("status.info").color);
    }
  });
});

describe("the activity line of a restored session", () => {
  const at = (a: Partial<DockActivity>): DockActivity => ({ doing: "resuming", elapsed: 0, detail: "copilot", ...a });
  const line = (a: Partial<DockActivity>, w = 120) => text(activityCells(at(a), w)).trimEnd();
  it("resuming: the spinner, resuming <provider>… and how long it has taken", () => {
    expect(line({ elapsed: 0 })).toBe(` ${spinAt(0)} resuming copilot… · 0s`);
    expect(line({ elapsed: 72 })).toBe(` ${spinAt(72)} resuming copilot… · 1m 12s`);
    expect(line({ elapsed: 5, detail: null })).toBe(` ${spinAt(5)} resuming… · 5s`);
  });
  it("detached: the connection mark and the words, muted, no time", () => {
    expect(line({ doing: "detached" })).toBe(` ${iconOf("connection")!.glyph} not attached to copilot · a message or alt-r resumes it`);
    expect(line({ doing: "detached", detail: null })).toBe(` ${iconOf("connection")!.glyph} not attached · a message or alt-r resumes it`);
    const cells = activityCells(at({ doing: "detached" }), 120);
    expect(cells.find((c) => c.ch === "n")!.s.color).toBe(muted);
    expect(cells.find((c) => c.ch === iconOf("connection")!.glyph)!.s.color).toBe(muted);
    expect(line({ doing: "detached", elapsed: 90 })).not.toMatch(/\d/);
  });
  it.each([40, 78, 158])("is one row of %i cells; the words are cut, never the time", (w) => {
    for (const doing of ["resuming", "detached"] as const) {
      const row = activityCells(at({ doing, elapsed: 72, detail: "a-provider-with-a-rather-long-name" }), w);
      expect(cellWidth(text(row)), doing).toBe(w);
    }
    expect(text(activityCells(at({ elapsed: 72, detail: "a-provider-with-a-rather-long-name" }), w))).toContain("1m 12s");
    expect(text(activityCells(at({ elapsed: 72 }), 20))).toContain("1m 12s");
  });
  it("resuming spins in the info colour and the words are secondary; the elapsed time is muted", () => {
    const cells = activityCells(at({ elapsed: 3 }), 80);
    expect(cells.find((c) => c.ch === spinAt(3))!.s.color).toBe(slotStyle("status.info").color);
    expect(cells.find((c) => c.ch === "r")!.s.color).toBe(slotStyle("text.secondary").color);
    expect(cells.find((c) => c.ch === "s" && cells.indexOf(c) > 20)!.s.color).toBe(muted);
  });
});

describe("an empty transcript of a restored session", () => {
  it("says it is not attached, or that it is read back, in place of 'no messages yet'", () => {
    expect(emptyWords(false, "detached")).toBe(" not attached · a message or alt-r reads it back");
    expect(emptyWords(false, "resuming")).toBe(" reading the session back…");
    expect(emptyWords(false, "idle")).toBe(" no messages yet · type below");
    expect(emptyWords(true, "detached")).toBe(" no sessions · ctrl-t starts one");
  });
});

describe("the recorded restored states", () => {
  const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures/dock");
  const read = (f: string): DockView => JSON.parse(fs.readFileSync(path.join(dir, f), "utf8")).dock;
  it.each(["80x24", "160x40"])("restored_%s: lapsed asks draw muted and press nothing, the notice follows", (z) => {
    const v = read(`restored_${z}.json`);
    const rows = transcriptRows(v.transcript ?? [], 74, 100).map((r) => text(r).trimEnd());
    expect(rows.join("\n")).toContain("shell: lapsed · radar closed before this was answered");
    expect(rows.join("\n")).toContain("→ lapsed");
    expect(rows.join("\n")).not.toMatch(/approve|deny|allow/);
    expect(pendingAsk(v)).toBeUndefined();
    expect(v.tabs?.map((t) => t.state)).toEqual(["idle", "detached", "resuming"]);
    expect(dockKey(emptyLocal(), v, c("a")).intents).toEqual([]);
  });
  it.each(["80x24", "160x40"])("detached_%s: alt-r resumes, the activity says not attached, /resume is listed", (z) => {
    const v = read(`detached_${z}.json`);
    expect(v.activity?.doing).toBe("detached");
    expect(text(activityCells(v.activity!, 120))).toContain("not attached to copilot");
    expect(dockKey(emptyLocal(), v, c("r", { alt: true })).intents).toEqual([{ dock: "resume" }]);
    expect(matchingCommands("/res", v.composer?.commands).map((x) => x.name)).toEqual(["resume"]);
    expect(dockHint(v)).toMatch(/^⏎ or alt-r resume/);
    expect(v.tabs?.map((t) => t.state)).toEqual(["detached", "resuming"]);
  });
});
