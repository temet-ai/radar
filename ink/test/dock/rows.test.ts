import { describe, expect, it } from "vitest";
import { itemRows, keyed, transcriptRows, widthOf } from "./helpers.js";
import { commandRows, itemLayout, pickKey, transcriptView } from "../../src/dock/rows.js";
import { applyLook, defaultLook, iconOf, slotStyle } from "../../src/text/look.js";
import { STATES } from "./states.js";
import { tabLabel } from "../../src/dock/rows.js";
import type { DockItem } from "../../src/dock/model.js";

const g = (n: string) => (iconOf(n)?.glyph ?? "").padEnd(2);
const text = (cs: { ch: string }[]) => cs.map((c) => c.ch).join("");
const lines = (items: DockItem[], w: number) => transcriptRows(items, w, 100).map(text);
const tool = { item: "tool", id: "t", title: "ls", kind: "shell", state: "done", open: false, result: ["a", "b"] } as const;

describe("a message sent while a turn runs", () => {
  it("is drawn at once, dimmed, with who holds it until the agent takes it", () => {
    const sent = (queued?: "here" | "provider"): DockItem => ({ item: "user", id: "q", text: "and?", ...(queued ? { queued } : {}) });
    expect(lines([sent("here")], 40)).toEqual([`${g("you")}and?`, "  queued: sent when this turn ends"]);
    expect(lines([sent("provider")], 50)[1]).toBe("  sent: the agent takes it after this turn");
    // Once the agent has it, it is the operator's message like any other: emphasised, no line under it.
    expect(lines([sent()], 40)).toEqual([`${g("you")}and?`]);
    const queued = itemRows(sent("here"), 40)[0]!;
    const mine = itemRows(sent(), 40)[0]!;
    expect(queued[2]!.s.bold).toBeFalsy();
    expect(mine[2]!.s.bold).toBe(true);
  });
});

describe("slash menu rows", () => {
  const cmds = [
    { name: "compact", detail: "[instructions]  summarise" },
    { name: "model", detail: "" },
    { name: "openai-templates:artifact-template-investment-committee-memo", detail: "a very long name" },
  ];
  it("puts the names in one column and what each does after it", () => {
    const rows = commandRows(cmds.slice(0, 2), 60).map(text);
    expect(rows[0]).toBe(" /compact  [instructions]  summarise".padEnd(60));
    expect(rows[1]).toBe(" /model".padEnd(60));
    // The column is as wide as the longest name shown, up to 24 cells.
    expect(commandRows([cmds[0]!, { name: "ab", detail: "x" }], 40).map(text)[1]).toBe(" /ab       x".padEnd(40));
    const long = commandRows(cmds, 80).map(text);
    expect(long[0]).toBe(` /compact${" ".repeat(24 - 7 + 2)}[instructions]  summarise`.padEnd(80));
    // A name past the column keeps two cells before its text.
    expect(long[2]).toBe(` /${cmds[2]!.name}  a very long name`.slice(0, 80).padEnd(80));
  });
  it("marks the selected row on the selection background, and no other", () => {
    const bg = (row: ReturnType<typeof commandRows>[number]) => new Set(row.map((c) => c.s.backgroundColor ?? null));
    const rows = commandRows(cmds, 60, 1);
    expect([...bg(rows[1]!)]).toEqual([slotStyle("bg.selection").color]);
    expect(rows[1]![1]!.s.bold).toBe(true);
    expect(bg(rows[0]!).has(slotStyle("bg.selection").color)).toBe(false);
    expect(commandRows(cmds, 60).every((r) => !bg(r).has(slotStyle("bg.selection").color))).toBe(true);
  });
});

describe("dock rows", () => {
  it("marks streamed text unsaved until it is saved", () => {
    expect(lines([{ item: "assistant", id: "a", text: "hi", saved: false }], 40)[0]).toBe(`${g("run_running")}hi`);
    expect(lines([{ item: "assistant", id: "a", text: "hi", saved: true }], 40)[0]).toBe("  hi");
  });
  it("folds a tool call to one line ending in ⋯ n, and opens it with tree lines", () => {
    expect(lines([tool], 40)).toEqual([`${g("terminal")}run ls ⋯ 2`]);
    expect(lines([{ ...tool, open: true }], 40)).toEqual([`${g("terminal")}run ls`, "├ a", "└ b"]);
  });
  it("marks a failed or denied call", () => {
    expect(lines([{ ...tool, state: "denied" }], 40)[0]).toMatch(new RegExp(`^${iconOf("run_failed")!.glyph}`));
  });
  it("folds reasoning by default", () => {
    expect(lines([{ item: "reasoning", id: "r", text: "a\nb\nc", open: false }], 40)).toEqual(["  reasoning ⋯ 3"]);
  });
  it("puts a button under each of an approval's words, where the word is", () => {
    const l = itemLayout({ item: "permission", id: "p:1", kind: "shell", target: "ls", actions: ["approve_once", "deny", "view_diff"] }, 60);
    const row = text(l.rows[2]!);
    expect(l.hits[2]).toEqual([
      { x: row.indexOf("approve once"), w: 12, target: "dock:approve:p%3A1" },
      { x: row.indexOf("deny"), w: 4, target: "dock:deny:p%3A1" },
      { x: row.indexOf("view diff"), w: 9, target: "dock:diff:p%3A1" },
    ]);
    expect(l.hits[0]).toEqual([]);
    // answered: no buttons
    expect(itemLayout({ item: "permission", id: "p", kind: "shell", target: "ls", actions: ["approve_once", "deny"], answered: "deny" }, 60).hits.flat()).toEqual([]);
  });
  it("approve for session has its key inside its word, wraps to the next row when narrow, and shows once chosen", () => {
    const item = { item: "permission", id: "p", kind: "shell", target: "ls", actions: ["approve_once", "approve_for_session", "deny", "view_diff"] } as const;
    const wide = itemLayout(item, 80);
    const row = text(wide.rows[2]!);
    expect(row).toContain("approve once  approve for session  deny  view diff");
    const at = row.indexOf("approve for session");
    expect(wide.rows[2]!.slice(at, at + 19).filter((c) => c.s.underline).map((c) => c.ch)).toEqual(["s"]);
    expect(wide.rows[2]![at + "approve for ".length]!.s.underline).toBe(true);
    expect(wide.hits[2]).toContainEqual({ x: row.indexOf("approve for session"), w: 19, target: "dock:session:p" });
    // 40 cells: the buttons take two rows, each button still where its word is.
    const narrow = itemLayout(item, 40);
    const rows = narrow.rows.slice(2).map(text);
    expect(rows.every((r) => r.length <= 40)).toBe(true);
    const all = narrow.hits.flat().map((h) => h.target);
    expect(all).toEqual(["dock:approve:p", "dock:session:p", "dock:deny:p", "dock:diff:p"]);
    narrow.hits.forEach((hs, y) => hs.forEach((h) => expect(text(narrow.rows[y]!).slice(h.x, h.x + h.w).length).toBe(h.w)));
    expect(text(itemLayout({ ...item, answered: "approve_for_session" }, 60).rows[0]!)).toBe(`${g("approved")}shell approved for session  ls`);
  });
  it("shows a waiting question with its choices numbered, a button on each, and its keys", () => {
    const q = STATES.question!.transcript![1]!;
    const l = itemLayout(q, 70);
    expect(l.rows.map(text).map((r) => r.trimEnd())).toEqual([
      `${g("mention")}Fix: Which fix do you want?`,
      "   1 revert  undo the last commit",
      "   2 patch",
      "   1-2 choose  ⏎ or type your own answer  esc decline",
    ]);
    // the number is the key, underlined, before its word
    expect(l.rows[1]!.filter((c) => c.s.underline).map((c) => c.ch)).toEqual(["1"]);
    expect(l.hits[1]).toEqual([{ x: 3, w: text(l.rows[1]!).trimEnd().length - 3, target: "dock:choose:q1:0:revert" }]);
    expect(l.hits[3]!.map((h) => h.target)).toEqual(["dock:decline:q1"]);
  });
  it("asks several questions one at a time: answered ones say what, the next is whole, later ones wait", () => {
    const q = STATES["question-several"]!.transcript![0]!;
    const picks = { [pickKey("q2", "Which checks?")]: ["clippy"] };
    const rows = itemLayout(q, 70, [], picks).rows.map(text).map((r) => r.trimEnd());
    expect(rows).toEqual([
      `${g("mention")}Crate: Which crate? → radar-core`,
      "  Checks: Which checks?",
      "   1   tests",
      `   2 ${g("approved")}clippy`,
      "   3   fmt",
      "   1-3 pick  ⏎ submit  ⏎ or type your own answer  esc decline",
      "  then: Commit?",
    ]);
    const done = itemLayout(STATES["question-answered"]!.transcript![1]!, 70);
    expect(done.rows.map(text).map((r) => r.trimEnd())).toEqual([`${g("mention")}Which fix do you want? → patch`]);
    expect(done.hits.flat()).toEqual([]);
  });
  it("shows an approval's exact target with keys inside their words", () => {
    const rows = itemRows({ item: "permission", id: "p", kind: "shell", target: "rm -rf build", actions: ["approve_once", "deny", "view_diff"] }, 40);
    expect(rows.map(text)).toEqual([`${g("warning")}allow shell?`, "   rm -rf build", "   approve once  deny  view diff"]);
    expect(rows[2]!.filter((c) => c.s.underline).map((c) => c.ch)).toEqual(["a", "d", "v"]);
  });
  it("shows an answered approval without its keys", () => {
    const rows = itemRows({ item: "permission", id: "p", kind: "shell", target: "ls", actions: ["approve_once", "deny"], answered: "deny" }, 40);
    expect(rows.map(text)).toEqual([`${g("run_failed")}shell denied  ls`]);
  });
  it("never uses chevrons", () => {
    const all = lines([tool, { item: "reasoning", id: "r", text: "r", open: false }], 40).join("");
    expect(all).not.toMatch(/[▶▸▾›⌄]|\[\+\]|\[-\]/);
  });
  it("keeps every row inside the width", () => {
    const long = "x".repeat(300);
    for (const w of [38, 78]) for (const r of transcriptRows([{ item: "user", id: "u", text: long }, { item: "error", id: "e", text: long }], w, 100)) expect(widthOf(r)).toBeLessThanOrEqual(w);
  });
  it("shows the last rows when the transcript is longer than its room", () => {
    const items: DockItem[] = Array.from({ length: 10 }, (_, i) => ({ item: "notice", id: `n${i}`, text: `n${i}` }));
    expect(transcriptRows(items, 40, 3).map(text)).toEqual([7, 8, 9].map((n) => `${g("info")}n${n}`));
  });
  it("scrolls up from the foot and says how far it can", () => {
    const items: DockItem[] = Array.from({ length: 10 }, (_, i) => ({ item: "notice", id: `n${i}`, text: `n${i}` }));
    const v = transcriptView(items, 40, 3, [], 2);
    expect(v.rows.map(text)).toEqual([5, 6, 7].map((n) => `${g("info")}n${n}`));
    expect(v.owners.map((o) => o.id)).toEqual(["n5", "n6", "n7"]);
    expect(v.max).toBe(7);
    expect(transcriptView(items, 40, 3, [], 99).rows.map(text)).toEqual([0, 1, 2].map((n) => `${g("info")}n${n}`));
  });
  it("keeps a scrolled view on the same rows while new ones come in under it, and follows them at the foot", () => {
    const items = (n: number): DockItem[] => Array.from({ length: n }, (_, i) => ({ item: "notice", id: `n${i}`, text: `n${i}` }));
    const ids = (v: { owners: DockItem[] }) => v.owners.map((o) => o.id);
    // Scrolled 2 up when there were 10 rows.
    expect(ids(transcriptView(items(10), 40, 3, [], 2, {}, 10))).toEqual(["n5", "n6", "n7"]);
    expect(ids(transcriptView(items(14), 40, 3, [], 2, {}, 10))).toEqual(["n5", "n6", "n7"]);
    expect(transcriptView(items(14), 40, 3, [], 2, {}, 10).total).toBe(14);
    // At the foot: the newest rows.
    expect(ids(transcriptView(items(14), 40, 3, [], 0, {}, 10))).toEqual(["n11", "n12", "n13"]);
  });
  it("keyed underlines one letter", () => {
    expect(keyed("bulk", "b").filter((c) => c.s.underline).length).toBe(1);
  });

  // Acceptance: glyphs come from the icon pack, none are hard-coded. Every mark is a pack icon, so with the pack off
  // (or another pack) the rows hold none of the unicode marks and follow the pack's glyph otherwise.
  it("draws every mark through the icon pack", () => {
    const items = Object.values(STATES).flatMap((v) => v.transcript ?? []);
    const tabs = Object.values(STATES).flatMap((v) => v.tabs ?? []);
    const MARKS = /[✓✗•›·■…●ℹ⚠☻☺]/u;
    try {
      applyLook(defaultLook("off"));
      const off = items.flatMap((i) => itemRows(i, 60).map(text)).concat(tabs.map(tabLabel));
      for (const r of off) expect(r.replace(/⋯/g, "")).not.toMatch(MARKS);
      applyLook(defaultLook("nerd"));
      const nerd = items.filter((i) => i.item === "tool" && i.state === "done").flatMap((i) => itemRows(i, 60).map(text));
      expect(nerd[0]).toContain(iconOf("terminal")!.glyph);
      expect(iconOf("terminal")!.glyph).not.toBe(defaultLook("unicode").icons.terminal!.glyph);
    } finally {
      applyLook(defaultLook());
    }
  });
});
