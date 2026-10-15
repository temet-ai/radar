import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { transcriptView, type LayoutOpts } from "../../src/dock/rows.js";
import { diffCounts, middleFit, shortPaths, toolKind, transcriptUnits } from "../../src/dock/tools.js";
import { dockClick, dockKey, emptyLocal } from "../../src/dock/composer.js";
import { applyLook, defaultLook, iconOf, slotStyle } from "../../src/text/look.js";
import { cellWidth } from "../../src/chrome/width.js";
import type { DockItem, DockView } from "../../src/dock/model.js";

// The transcript as an agent's turn fills it: answers in markdown, runs of tool calls, edits, reasoning. Every state the
// host recorded (test/fixtures/dock, both sizes) at the widths the dock has, then the cases one by one.
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures/dock");
const files = fs.readdirSync(dir).filter((f) => /_\d+x\d+\.json$/.test(f)).sort();
const view = (f: string): DockView => JSON.parse(fs.readFileSync(path.join(dir, f), "utf8")).dock;
const text = (r: { ch: string }[]) => r.map((c) => c.ch).join("");
const optsOf = (v: DockView, over: LayoutOpts = {}): LayoutOpts => ({ cwd: v.status?.cwd, home: "/home/dev", live: v.tabs?.find((t) => t.id === v.active)?.state === "streaming", ...over });
const rows = (v: DockView, w: number, over: LayoutOpts = {}) => transcriptView(v.transcript ?? [], w, 500, [], 0, {}, undefined, optsOf(v, over));
const lines = (v: DockView, w: number, over: LayoutOpts = {}) => rows(v, w, over).rows.map(text).map((l) => l.trimEnd());
const g = (n: string) => iconOf(n)?.glyph ?? "";

describe("every recorded transcript", () => {
  it("has the new states at both sizes", () => {
    for (const s of ["markdown", "markdown-streaming", "tools", "edit-open", "reasoning-live"]) for (const z of ["80x24", "160x40"]) expect(files).toContain(`${s}_${z}.json`);
  });
  for (const f of files)
    for (const w of [38, 78, 118])
      it(`${f} at ${w} cells: no markdown marks, no chevrons, every row inside the width, an answered request in its call's row`, () => {
        const v = view(f);
        const shown = rows(v, w);
        const all = shown.rows.map(text);
        for (const r of shown.rows) expect(r.reduce((n, c) => n + c.w, 0)).toBeLessThanOrEqual(w);
        expect(all.join("\n")).not.toMatch(/[▶▸▾›⌄]|\[\+\]|\[-\]/);
        // An answer the host read as markdown shows none of its marks.
        const answers = (v.transcript ?? []).filter((i) => i.item === "assistant" && i.doc?.length);
        const owned = (id: string) => all.filter((_, i) => shown.owners[i]!.id === id).join("\n");
        for (const a of answers) expect(owned(a.id)).not.toMatch(/\*\*|`|^\s*#+ |^\s*[-*] |\|---/m);
        // An approval answered for a call in the transcript draws no row of its own.
        const calls = new Set((v.transcript ?? []).filter((i) => i.item === "tool").map((i) => i.id));
        for (const p of v.transcript ?? []) if (p.item === "permission" && p.answered && p.tool && calls.has(p.tool)) expect(shown.owners).not.toContain(p);
        // A folded call is one row.
        for (const t of v.transcript ?? []) if (t.item === "tool" && !t.open) expect(shown.owners.filter((o, i) => o === t && all[i]!.trim() !== "").length).toBeLessThanOrEqual(1);
      });
});

describe("tool calls", () => {
  const tools = view("tools_160x40.json");
  it("are one row each: the kind's icon, what it does, the target relative to the session's folder, how it went", () => {
    expect(lines(tools, 78)).toEqual([
      `${g("you")} review the branch`,
      "",
      `${g("file")} read 5 files ⋯ 5`,
      `${g("search")} search fn dock_view ⋯ 1`,
      `${g("run_failed")} run cargo test -p app-dock dock failed ⋯ 1`,
      `${g("mode_edit")} edit crates/app-dock/src/dock.rs +1 −1 ⋯ 4`,
      `${g("run_running")} run cargo test -p app-dock dock`,
      `${g("warning")} allow shell?`,
      "   cargo test -p app-dock dock",
      "   approve once  approve for session  deny",
    ]);
  });
  it("draw the counts of an edit in the diff slots and a failure in the error slot", () => {
    const v = rows(tools, 78);
    const row = v.rows.find((r) => text(r).includes("+1 −1"))!;
    const at = (s: string) => row[text(row).indexOf(s)]!.s.color;
    expect(at("+1")).toBe(slotStyle("diff.added").color);
    expect(at("−1")).toBe(slotStyle("diff.removed").color);
    const failed = v.rows.find((r) => text(r).includes("failed"))!;
    expect(failed[text(failed).indexOf("failed")]!.s.color).toBe(slotStyle("status.error").color);
  });
  it("cut a long path in the middle, keeping the file's name, by display cells", () => {
    const narrow = lines(tools, 30);
    const edit = narrow.find((l) => l.includes("+1 −1"))!;
    expect(cellWidth(edit)).toBeLessThanOrEqual(30);
    expect(edit).toMatch(/…/);
    expect(edit).toMatch(/dock\.rs \+1 −1 ⋯ 4$/);
    expect(middleFit("crates/app-dock/src/dock.rs", 16)).toBe("crates…c/dock.rs");
    expect(cellWidth(middleFit("界".repeat(20), 9))).toBeLessThanOrEqual(9);
    expect(shortPaths("cat /home/dev/projects/radar/a.rs /home/dev/b", "/home/dev/projects/radar", "/home/dev")).toBe("cat a.rs ~/b");
    expect(shortPaths("/w/x", "~/w", "/home/dev")).toBe("/w/x");
    expect(shortPaths("/home/dev/w/x", "~/w", "/home/dev")).toBe("x");
    // only where a path starts; the folder itself is `.`; a home with its slash
    expect(shortPaths("/mnt/backup/home/dev/projects/radar/a.ts", "/home/dev/projects/radar", "/home/dev")).toBe("/mnt/backup/home/dev/projects/radar/a.ts");
    expect(shortPaths("ls -la /home/dev/projects/radar", "/home/dev/projects/radar", "/home/dev")).toBe("ls -la .");
    expect(shortPaths("cd '/home/dev/projects/radar' && cat x=/home/dev/projects/radar/y", "~/projects/radar", "/home/dev/")).toBe("cd '.' && cat x=y");
    expect(shortPaths("/home/dev and /home/devel/x", undefined, "/home/dev")).toBe("~ and /home/devel/x");
  });
  it("count an edit's lines, a removed `-- comment` among them, file headers not", () => {
    expect(diffCounts(["--- a/x.sql", "+++ b/x.sql", "--- comment", "+-- note", " same"])).toEqual({ added: 1, removed: 1 });
  });
  it("know their kind from each provider's names", () => {
    expect(["Read", "view", "Grep", "Glob", "Bash", "shell", "Edit", "MultiEdit", "apply_patch", "Write", "WebFetch", "mcp__slack__post", "Task", "Mystery"].map(toolKind)).toEqual([
      "read", "read", "search", "search", "shell", "shell", "edit", "edit", "edit", "write", "web", "mcp", "agent", "other",
    ]);
    const mcp: DockItem = { item: "tool", id: "m", title: "{}", kind: "mcp__slack__post_message", state: "done", open: false };
    expect(lines({ ...tools, transcript: [mcp] }, 60)[0]).toBe(`${g("parts")} slack post_message {}`);
  });
  it("fold reads in a row into one row; open, each read on a tree line, opening on its own", () => {
    const open = lines(tools, 78, { groups: ["group:t0"] });
    expect(open.slice(2, 8)).toEqual([
      `${g("file")} read 5 files`,
      "  ├ ink/src/dock/rows.ts ⋯ 1",
      "  ├ ink/src/dock/Dock.tsx ⋯ 1",
      "  ├ ink/src/dock/composer.ts ⋯ 1",
      "  ├ crates/radar-core/src/dock.rs ⋯ 1",
      "  └ crates/app-dock/src/dock.rs ⋯ 1",
    ]);
    const v = rows(tools, 78, { groups: ["group:t0"] });
    expect(v.targets[2]).toBe("dock:group:group%3At0");
    expect(v.targets[3]).toBe("dock:fold:t0:1");
    const t0 = { ...(tools.transcript![1] as Extract<DockItem, { item: "tool" }>), open: true };
    const withOpen = { ...tools, transcript: tools.transcript!.map((i) => (i.id === "t0" ? t0 : i)) };
    expect(lines(withOpen, 78, { groups: ["group:t0"] }).slice(3, 5)).toEqual(["  ├ ink/src/dock/rows.ts", "  │ └ 120 lines"]);
  });
  it("turn with the activity line's spinner while they run", () => {
    const l = lines(tools, 78, { spin: "⠯" });
    expect(l[6]).toBe("⠯ run cargo test -p app-dock dock");
    expect(l[2]).toBe(`${g("file")} read 5 files ⋯ 5`);
  });
  it("show the target of the read still running beside its group", () => {
    const items = tools.transcript!.map((i) => (i.id === "t4" && i.item === "tool" ? { ...i, state: "running" as const } : i));
    expect(lines({ ...tools, transcript: items }, 78)[2]).toBe(`${g("run_running")} read 5 files · crates/app-dock/src/dock.rs ⋯ 5`);
  });
  it("keep a failed read out of its group, and an approval still waiting in view", () => {
    const items = tools.transcript!.map((i) => (i.id === "t2" && i.item === "tool" ? { ...i, state: "failed" as const } : i));
    const u = transcriptUnits(items);
    expect(u.filter((x) => x.u === "group").map((x) => (x.u === "group" ? x.tools.length : 0))).toEqual([2, 2]);
    expect(u.some((x) => x.u === "item" && x.item.item === "permission" && !x.item.answered)).toBe(true);
  });
  it("show an edit's diff, open, in the diff view's look under its row", () => {
    const v = view("edit-open_160x40.json");
    const shown = rows(v, 78);
    const all = shown.rows.map(text).map((l) => l.trimEnd());
    expect(all.slice(2, 7)).toEqual([
      `${g("mode_edit")} edit crates/app-dock/src/dock.rs +1 −1`,
      "├  fn lines(text: &str) -> Vec<String> {",
      "├ -    text.lines().map(str::to_string).collect()",
      "├ +    text.lines().map(String::from).collect()",
      "└  }",
    ]);
    const added = shown.rows[5]!;
    // the row's tint reaches the edge; the changed words stand out on a stronger one
    expect(added.reduce((n, c) => n + c.w, 0)).toBe(78);
    expect(added[77]!.s.backgroundColor).toBeTruthy();
    const word = added[text(added).indexOf("String::from")]!.s;
    expect(word.bold).toBe(true);
    expect(word.backgroundColor).not.toBe(added[77]!.s.backgroundColor);
  });
});

describe("answers in markdown", () => {
  it("are laid out as documents, a row apart from what is next to them", () => {
    const l = lines(view("markdown_160x40.json"), 78);
    expect(l.slice(0, 6)).toEqual([`${g("you")} review the branch`, "", "  Review", "", "  Verdict: Request changes. The branch is behind origin/main by three commits.", ""]);
    expect(l).toContain("    ◦ The tool rows take two lines each; one is enough.");
    expect(l).toContain("  See the plan (https://example.test/agent-dock) for the dock's rules.");
  });
  it("half received, show their open marks closed: no stars, the words bold", () => {
    const v = view("markdown-streaming_160x40.json");
    const shown = rows(v, 78);
    const l = shown.rows.map(text);
    expect(l.join("\n")).not.toMatch(/\*\*|`/);
    const row = shown.rows.find((r) => text(r).includes("Verdict"))!;
    expect(row[text(row).indexOf("Verdict")]!.s.bold).toBe(true);
    expect(l[2]).toMatch(new RegExp(`^${g("run_running")} Review`));
  });
  it("highlight code where the host painted it", () => {
    const v = view("markdown_160x40.json");
    const shown = rows(v, 78);
    const row = shown.rows.find((r) => text(r).startsWith("    fn lines"))!;
    expect(row[4]!.s).toMatchObject({ color: slotStyle("accent.primary").color, bold: true });
  });
});

describe("reasoning", () => {
  const v = view("reasoning-live_160x40.json");
  it("while the session works, shows its latest line beside the fold", () => {
    expect(lines(v, 78)[2]).toBe("  thinking · Its log ends in a snapshot diff, so the look changed and the… ⋯ 2");
  });
  it("once the session stops, folds to a word", () => {
    expect(lines(v, 78, { live: false })[2]).toBe("  reasoning ⋯ 2");
  });
  it("kept to itself by the provider (empty), draws no row: the activity line says the agent thinks", () => {
    const empty: DockItem[] = [v.transcript![0]!, { item: "reasoning", id: "r", text: "", open: false }, { item: "assistant", id: "a", text: "done", saved: true }];
    expect(lines({ ...v, transcript: empty }, 78, { live: false })).toEqual([`${g("you")} why does the build fail?`, "", "  done"]);
    expect(lines({ ...v, transcript: empty.slice(0, 2) }, 78)).toEqual([`${g("you")} why does the build fail?`]);
  });
  it("shows only the reasoning still coming, not an older one", () => {
    const items = [...v.transcript!, { item: "assistant", id: "a", text: "so", saved: false } as DockItem];
    expect(lines({ ...v, transcript: items }, 78)[2]).toBe("  reasoning ⋯ 2");
  });
});

describe("keys and clicks", () => {
  const tools = view("tools_160x40.json");
  it("a click on a group opens and folds it", () => {
    const r = dockClick(emptyLocal(), tools, "dock:group:group%3At0")!;
    expect(r.local.groups).toEqual(["group:t0"]);
    expect(dockClick(r.local, tools, "dock:group:group%3At0")!.local.groups).toEqual([]);
    expect(dockClick(emptyLocal(), tools, "dock:group:nothing")).toBeUndefined();
  });
  it("alt-o opens the latest group of calls when it is the latest that folds", () => {
    const reads = { ...tools, transcript: tools.transcript!.slice(0, 11) };
    const r = dockKey(emptyLocal(), reads, { key: { char: "o" }, alt: true, ctrl: false, shift: false } as any);
    expect(r.local.groups).toEqual(["group:t0"]);
    expect(r.intents).toEqual([]);
  });
  it("the groups stay open when a message is sent", () => {
    const local = { ...emptyLocal("next"), groups: ["group:t0"] };
    const r = dockKey(local, { ...tools, composer: { can_send: true, can_stop: false } }, { key: "enter", alt: false, ctrl: false, shift: false } as any);
    expect(r.intents).toEqual([{ dock: "send", text: "next" }]);
    expect(r.local.groups).toEqual(["group:t0"]);
  });
});

describe("with the icon pack off", () => {
  it("still tells each call by its words", () => {
    try {
      applyLook(defaultLook("off"));
      const l = lines(view("tools_160x40.json"), 78);
      expect(l[2]).toBe("  read 5 files ⋯ 5");
      expect(l[4]).toBe("  run cargo test -p app-dock dock failed ⋯ 1");
    } finally {
      applyLook(defaultLook());
    }
  });
});
