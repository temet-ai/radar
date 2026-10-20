import type { DockItem } from "./model.js";
import { cellWidth, clusters } from "../chrome/width.js";

/**
 * What the dock makes of tool calls before it draws them, from view data alone: the kind of each call (from the tool's
 * name as the provider gave it), its target made short (paths relative to the session's folder, cut in the middle so
 * the file's name stays), and the transcript as units: an approval already answered goes into its call's row, and
 * reads (or searches) in a row fold into one row that ends in `⋯ n`.
 */

export type ToolItem = Extract<DockItem, { item: "tool" }>;
export type PermissionItem = Extract<DockItem, { item: "permission" }>;
export type ToolKind = "read" | "search" | "shell" | "edit" | "write" | "web" | "mcp" | "agent" | "todo" | "other";

/** Tool names of Claude Code, Codex, Copilot and the fake, by kind (lower case). */
const NAMES: Record<string, ToolKind> = {
  read: "read", view: "read", read_file: "read", notebookread: "read", cat: "read",
  grep: "search", glob: "search", ls: "search", list: "search", search: "search", find: "search", list_dir: "search", list_directory: "search", codebase_search: "search", file_search: "search",
  bash: "shell", shell: "shell", commandexecution: "shell", powershell: "shell", bashoutput: "shell", killshell: "shell", exec: "shell", run_command: "shell", local_shell: "shell",
  edit: "edit", multiedit: "edit", notebookedit: "edit", str_replace: "edit", str_replace_editor: "edit", apply_patch: "edit", filechange: "edit", edit_file: "edit",
  write: "write", create: "write", write_file: "write",
  webfetch: "web", websearch: "web", web_fetch: "web", web_search: "web", fetch: "web", url: "web",
  task: "agent", agent: "agent",
  todowrite: "todo", update_todo: "todo", todo: "todo",
};

export function toolKind(name: string): ToolKind {
  const n = name.toLowerCase();
  if (n.startsWith("mcp__") || n === "mcp") return "mcp";
  return NAMES[n] ?? "other";
}

/** The pack icon of each kind. */
export const KIND_ICON: Record<ToolKind, string> = {
  read: "file",
  search: "search",
  shell: "terminal",
  edit: "mode_edit",
  write: "mode_edit",
  web: "link",
  mcp: "parts",
  agent: "bot",
  todo: "task",
  other: "action",
};

/** The word before a call's target: what it does. An MCP tool is named by its server and tool. */
export function toolVerb(kind: ToolKind, name: string): string {
  const n = name.toLowerCase();
  switch (kind) {
    case "web":
      return n.includes("search") ? "search web" : "fetch";
    case "shell":
      return "run";
    case "mcp": {
      const [, server, ...tool] = name.split("__");
      return [server, tool.join("__")].filter(Boolean).join(" ") || name;
    }
    case "other":
      return name;
    default:
      return kind;
  }
}

/** The kinds whose target is a path (cut in the middle, so the file's name stays). */
export const PATH_KINDS: ReadonlySet<ToolKind> = new Set(["read", "edit", "write"]);

const noSlash = (p: string) => p.replace(/\/+$/, "");
/** `~/x` as the absolute path it names (`home` without its trailing slash). */
const expand = (dir: string, home?: string) => noSlash(home && (dir === "~" || dir.startsWith("~/")) ? home + dir.slice(1) : dir);
const escaped = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
/** Where a path may start in a command: the text's start, or after a space, a quote, `=` or `(`. */
const LEAD = `(^|[\\s"'=(])`;

/** `text` with the session's folder taken off the paths in it (relative to it; the folder itself is `.`) and the home
 * folder written `~`. Only a path's start is matched, never a folder that ends another path. */
export function shortPaths(text: string, cwd?: string, home?: string): string {
  const h = home ? noSlash(home) : "";
  const dir = cwd ? expand(cwd, h) : "";
  let out = text;
  if (dir && dir !== "/") {
    out = out.replace(new RegExp(`${LEAD}${escaped(dir)}/`, "g"), "$1").replace(new RegExp(`${LEAD}${escaped(dir)}(?=$|[\\s"')])`, "g"), "$1.");
  }
  if (h && h !== "/") out = out.replace(new RegExp(`${LEAD}${escaped(h)}(?=/|$|[\\s"')])`, "g"), "$1~");
  return out;
}

/** `s` in `room` cells, cut in the middle with `…` (the end, where a file's name is, keeps the larger share). */
export function middleFit(s: string, room: number): string {
  if (cellWidth(s) <= room) return s;
  if (room <= 1) return room === 1 ? "…" : "";
  const chars = clusters(s);
  const tailRoom = Math.ceil((room - 1) * 0.6);
  const headRoom = room - 1 - tailRoom;
  let head = "";
  let w = 0;
  for (const [ch, cw] of chars) {
    if (w + cw > headRoom) break;
    head += ch;
    w += cw;
  }
  let tail = "";
  w = 0;
  for (let i = chars.length - 1; i >= 0; i--) {
    const [ch, cw] = chars[i]!;
    if (w + cw > tailRoom) break;
    tail = ch + tail;
    w += cw;
  }
  return `${head}…${tail}`;
}

/** A unified diff's file header (`--- a/x`, `+++ b/x`, `/dev/null`), not a changed line: a removed `-- comment` is
 * `--- comment` and counts. */
const HEADER = /^(---|\+\+\+) (a\/|b\/|\/dev\/null)/;

/** An edit's added and removed lines (file headers not counted). */
export function diffCounts(diff: string[]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const l of diff) {
    if (HEADER.test(l)) continue;
    if (l.startsWith("+")) added++;
    else if (l.startsWith("-")) removed++;
  }
  return { added, removed };
}

/** A row-sized piece of the transcript: one item, or calls of one kind in a row folded together. */
export type Unit = { u: "item"; item: DockItem } | { u: "group"; id: string; kind: ToolKind; tools: ToolItem[] };

/** The kinds whose calls in a row fold into one row. */
const GROUPED: ReadonlySet<ToolKind> = new Set(["read", "search"]);

/** The transcript as units: an answered approval whose call is in the transcript is left out (its call's row tells
 * how it went), and two or more reads (or searches) in a row that did not fail become one group. */
export function transcriptUnits(items: DockItem[]): Unit[] {
  const calls = new Set(items.filter((i): i is ToolItem => i.item === "tool").map((t) => t.id));
  const out: Unit[] = [];
  let run: ToolItem[] = [];
  const flush = () => {
    if (run.length >= 2) out.push({ u: "group", id: `group:${run[0]!.id}`, kind: toolKind(run[0]!.kind), tools: run });
    else run.forEach((t) => out.push({ u: "item", item: t }));
    run = [];
  };
  for (const it of items) {
    if (it.item === "permission" && it.answered && it.tool && calls.has(it.tool)) continue;
    const k = it.item === "tool" ? toolKind(it.kind) : undefined;
    if (it.item === "tool" && k && GROUPED.has(k) && it.state !== "failed" && it.state !== "denied") {
      if (run.length && toolKind(run[0]!.kind) !== k) flush();
      run.push(it);
      continue;
    }
    flush();
    out.push({ u: "item", item: it });
  }
  flush();
  return out;
}

/** What a group of `n` calls of `kind` says it did. */
export const groupWords = (kind: ToolKind, n: number): [string, string] => (kind === "read" ? ["read", `${n} files`] : ["search", `${n} times`]);
