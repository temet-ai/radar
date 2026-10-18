import type { Block, DetailContent, DetailFact, FootAction, FootActions, Node, Seg, Text } from "./view.js";
import { cellsOf, clipCells, lineCells, padCells, paragraphCells, widthOfCells, type Cell } from "../text/cells.js";
import { bgOf, slotStyle, type Style } from "../text/look.js";
import { displayChord } from "./actions.js";
import { mnemonicRange, mnemonicRuns } from "../text/mnemonic.js";
import { cellWidth } from "../chrome/width.js";

export const DETAIL_CAPABILITY = "detail";
export const DETAIL_VERSION = 1;
export const DETAIL_FOOT_PREFIX = "detail-foot:";

const record = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const optional = (v: unknown, check: (v: unknown) => boolean) => v == null || check(v);
const defaulted = (v: unknown, check: (v: unknown) => boolean) => v === undefined || check(v);
const uint = (v: unknown) => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const tones = new Set(["plain", "muted", "accent", "info", "good", "warn", "bad"]);
const string = (v: unknown): v is string => typeof v === "string";
const boolean = (v: unknown) => typeof v === "boolean";
const inks = new Set(["plain", "primary", "secondary", "muted", "emphasis", "accent", "accent_muted", "accent_secondary", "accent_tertiary", "good", "warn", "bad", "info", "number", "added", "removed"]);
const isSeg = (v: unknown): v is Seg => record(v) && defaulted(v.text, string) && defaulted(v.ink, (x) => string(x) && inks.has(x))
  && ["icon", "link", "target", "look", "key_hint"].every((k) => optional(v[k], string))
  && ["strong", "marked", "icon_lead", "chip"].every((k) => defaulted(v[k], boolean));
const isText = (v: unknown): v is Text => Array.isArray(v) && v.every(isSeg);
function isInlineKind(kind: unknown): boolean {
  if (typeof kind === "string") return ["text", "emoji", "break"].includes(kind);
  if (!record(kind) || Object.keys(kind).length !== 1) return false;
  if ("link" in kind) return record(kind.link) && string(kind.link.url);
  if ("mention" in kind) return record(kind.mention) && defaulted(kind.mention.me, boolean);
  if ("badge" in kind) return record(kind.badge) && string(kind.badge.tone) && tones.has(kind.badge.tone);
  if ("image" in kind) return record(kind.image) && string(kind.image.src);
  return false;
}
function isRich(v: unknown): boolean {
  if (!record(v)) return false;
  const inline = (x: unknown): boolean => record(x) && string(x.text)
    && defaulted(x.marks, (m) => record(m) && ["bold", "italic", "strike", "underline", "code"].every((k) => defaulted(m[k], boolean)))
    && defaulted(x.kind, isInlineKind);
  const inlines = (x: unknown): boolean => Array.isArray(x) && x.every(inline);
  const blocks = (x: unknown): boolean => Array.isArray(x) && x.every(isRich);
  switch (v.t) {
    case "para": return inlines(v.text);
    case "heading": return uint(v.level) && Number(v.level) <= 255 && inlines(v.text);
    case "list": return Array.isArray(v.items) && v.items.every((i) => record(i) && (["bullet", "todo", "done", "decision"].includes(i.marker as string) || record(i.marker) && uint(i.marker.number)) && blocks(i.blocks));
    case "quote": return blocks(v.blocks);
    case "callout": return string(v.tone) && tones.has(v.tone) && defaulted(v.symbol, string) && blocks(v.blocks);
    case "code": return string(v.text) && defaulted(v.lang, string);
    case "table": return defaulted(v.aligns, (a) => Array.isArray(a) && a.every((x) => ["left", "center", "right"].includes(x))) && defaulted(v.head, (h) => Array.isArray(h) && h.every(inlines)) && Array.isArray(v.rows) && v.rows.every((r) => Array.isArray(r) && r.every(inlines));
    case "rule": return true;
    case "details": return inlines(v.summary) && blocks(v.blocks);
    case "media": return defaulted(v.image, boolean) && ["name", "url", "src"].every((k) => defaulted(v[k], string)) && optional(v.size, (size) => Array.isArray(size) && size.length === 2 && size.every((n) => uint(n) && n <= 4294967295));
    default: return false;
  }
}
function isBlock(v: unknown): v is Block {
  if (!record(v)) return false;
  switch (v.block) {
    case "line": return isText(v.text) && optional(v.key, string) && optional(v.target, string) && optional(v.mnemonic_at, (x) => Number.isInteger(x) && Number(x) >= 0) && defaulted(v.mnemonic_shift, boolean);
    case "section": return string(v.title) && optional(v.mark, string) && optional(v.detail, string) && Array.isArray(v.blocks) && v.blocks.every(isBlock);
    case "doc": return Array.isArray(v.doc) && v.doc.every(isRich) && optional(v.bar, isSeg) && optional(v.most, (x) => Number.isInteger(x) && Number(x) >= 0) && optional(v.urls, boolean);
    case "meter": return Array.isArray(v.stages) && v.stages.every((x) => record(x) && string(x.name) && string(x.look) && ["done", "failed", "running", "waiting", "manual", "stopped", "skipped"].includes(x.mark as string));
    case "wrapped": return string(v.text) && isSeg(v.prefix) && string(v.ink) && inks.has(v.ink);
    default: return false;
  }
}

/** Reject unsupported/malformed payloads before any renderer touches nested values. */
export function decodeDetail(node: unknown): DetailContent | null {
  if (!record(node) || node.node !== "extension" || node.capability !== DETAIL_CAPABILITY || node.version !== DETAIL_VERSION) return null;
  const d = node.data;
  if (!record(d) || !record(d.header)) return null;
  const h = d.header;
  if (!isText(h.path) || !isText(h.title) || !["branch", "author", "age"].every((k) => optional(h[k], isText))) return null;
  if (!defaulted(d.facts, (v) => Array.isArray(v) && v.every((f) => record(f) && string(f.label) && isText(f.value)))) return null;
  if (!(d.sections === undefined || Array.isArray(d.sections) && d.sections.every(isBlock)) || !optional(d.pictures, string)) return null;
  const header = { ...h };
  for (const key of ["branch", "author", "age"] as const) if (Array.isArray(header[key]) && !(header[key] as Text).some((s) => (s.text ?? "").trim())) delete header[key];
  return { ...d, header } as unknown as DetailContent;
}

/** Meaningful zero survives; whitespace-only and icon-only values are absent facts. */
export const survivingFacts = (facts: readonly DetailFact[] = []) => facts.filter((f) => f.value.some((s) => (s.text ?? "").trim() !== ""));
const width = (n: number) => Math.max(0, Math.floor(n));
const insetOf = (w: number) => w >= 3 ? 1 : 0;
const styledText = (t: Text | null | undefined, style: Style, plain: boolean) => lineCells(t ?? [], plain).map((c) => ({ ...c, s: plain ? {} : { ...style, ...c.s } }));

/** Native fit_line: clip complete graphemes, add ellipsis in the fill style, pad the rest. */
export function fitDetailCells(cells: Cell[], room: number, style: Style): Cell[] {
  const overflow = widthOfCells(cells) > room;
  const kept = clipCells(cells, Math.max(0, room - (overflow ? 1 : 0))).map((c) => ({ ...c, s: { ...style, ...c.s } }));
  if (overflow && room > 0) kept.push({ ch: "…", w: 1, s: style });
  return padCells(kept, room, style);
}

/** Three complete shaded rows, including their blank cells, using the shared heading ground. */
export function detailHeaderRows(data: DetailContent, columns: number, plain = false): Cell[][] {
  const w = width(columns), inset = insetOf(w), room = Math.max(0, w - 2 * inset);
  if (!w) return [];
  const base: Style = plain ? {} : { backgroundColor: bgOf("bg.overlay") };
  const row = (cells: Cell[]) => padCells([...cellsOf(" ".repeat(inset), base), ...fitDetailCells(cells, room, base)], w, base);
  const h = data.header;
  const branch = styledText(h.branch, slotStyle("text.primary"), plain), author = styledText(h.author, slotStyle("text.secondary"), plain), age = styledText(h.age, slotStyle("text.muted"), plain);
  const ageW = widthOfCells(age), authorW = widthOfCells(author);
  const need = ageW + authorW + (ageW && authorW ? 3 : 0);
  const hasBranch = h.branch != null;
  const rightLimit = hasBranch ? Math.min(need, Math.max(ageW, Math.floor(room / 2)), room) : Math.min(need, room);
  const ageRoom = Math.min(ageW, rightLimit);
  const authorRoom = Math.min(authorW, Math.max(0, rightLimit - ageRoom - (ageRoom ? 3 : 0)));
  const rightW = ageRoom + authorRoom + (ageRoom && authorRoom ? 3 : 0);
  const branchRoom = hasBranch ? Math.max(0, room - rightW - (rightW ? 1 : 0)) : 0;
  const third = [
    ...(hasBranch ? fitDetailCells(branch, branchRoom, base) : []),
    ...cellsOf(" ".repeat(Math.max(0, room - branchRoom - rightW)), base),
    ...(authorRoom ? fitDetailCells(author, authorRoom, base) : []),
    ...(authorRoom && ageRoom ? cellsOf(" · ", plain ? {} : { ...slotStyle("text.muted"), ...base }) : []),
    ...(ageRoom ? fitDetailCells(age, ageRoom, base) : []),
  ];
  return [row(styledText(h.path, slotStyle("text.secondary"), plain)), row(styledText(h.title, { ...slotStyle("text.emphasis"), bold: true }, plain)), row(third)];
}

/** Facts align after one outer inset; narrow cells stack labels and values. */
export function detailFactRows(facts: readonly DetailFact[] | undefined, columns: number, plain = false): Cell[][] {
  const w = width(columns), inset = insetOf(w), room = Math.max(0, w - 2 * inset);
  if (!room) return [];
  const kept = survivingFacts(facts), labelW = Math.min(16, Math.floor(room / 3), Math.max(0, ...kept.map((f) => cellWidth(f.label))));
  const row = (cells: Cell[]) => padCells([...cellsOf(" ".repeat(inset)), ...clipCells(cells, room)], w);
  const out: Cell[][] = [];
  for (const f of kept) {
    const label = cellsOf(f.label, plain ? {} : slotStyle("text.muted"));
    const lines: Text[] = [[]];
    for (const seg of f.value) {
      const parts = (seg.text ?? "").split("\n");
      parts.forEach((part, i) => {
        if (i) lines.push([]);
        lines[lines.length - 1]!.push({ ...seg, text: part.replace(/\r$/, ""), ...(i ? { icon: undefined, icon_lead: false } : {}) });
      });
    }
    const valueRows = (cells: number) => lines.flatMap((line) => paragraphCells(lineCells(line, plain), cells));
    if (room < 12) {
      out.push(row(fitDetailCells(label, room, plain ? {} : slotStyle("text.muted"))));
      out.push(...valueRows(room).map(row));
    } else {
      const prefix = [...fitDetailCells(label, labelW, plain ? {} : slotStyle("text.muted")), ...cellsOf("  ")];
      const values = valueRows(Math.max(1, room - labelW - 2));
      out.push(...values.map((v, i) => row([...(i === 0 ? prefix : cellsOf(" ".repeat(labelW + 2))), ...v])));
    }
  }
  return out;
}

export interface FootLayout { rows: Cell[][]; hits: Array<{ dx: number; dy: number; w: number; h: number; target: string }> }
export type { WriteMode } from "../protocol/generated/WriteMode.js";
import type { WriteMode } from "../protocol/generated/WriteMode.js";
const blocked = (a: FootAction, mode: WriteMode) => a.disabled != null || !!a.action.change && (mode === "read_only" || a.action.change === "edit" && mode !== "edit");
export function footActionCells(foot: FootAction, plain = false, mode: WriteMode = "read_only"): Cell[] {
  const a = foot.action, key = a.key ? displayChord(a.key) : "";
  const style = plain ? {} : slotStyle(blocked(foot, mode) ? "text.muted" : "text.secondary");
  const icon = a.icon ? lineCells([{ icon: a.icon }], plain).map((c) => blocked(foot, mode) ? { ...c, s: { ...c.s, ...style, bold: false } } : c) : [];
  const valid = !!mnemonicRange(a.label, key, a.mnemonic_at);
  const letter = /^\p{Alphabetic}$/u.test(key);
  const prefix = key && !letter ? key : valid && !plain && a.mnemonic_shift ? "shift" : "";
  const leading = prefix ? cellsOf(`${prefix} `, plain ? {} : { ...slotStyle(blocked(foot, mode) ? "text.muted" : "accent.secondary"), bold: !blocked(foot, mode) }) : [];
  const suffix = key && letter && (!valid || plain) ? cellsOf(` [${key}]`, plain ? {} : slotStyle("text.muted")) : [];
  return [...leading, ...icon, ...mnemonicRuns(a.label, key, plain ? {} : a, style).flatMap((r) => cellsOf(r.text, r.style)), ...suffix];
}

/** One rule and one pinned control row, never partial actions or padded hit targets. */
export function footActionLayout(foot: FootActions | null | undefined, columns: number, rows: number, plain = false, mode: WriteMode = "read_only"): FootLayout {
  const w = width(columns);
  if (!foot || rows < 3 || w === 0) return { rows: [], hits: [] };
  const menuFull = footActionCells(foot.menu, plain, mode);
  const menu = widthOfCells(menuFull) <= w ? menuFull : cellsOf(foot.menu.action.key ? displayChord(foot.menu.action.key) : "x", plain ? {} : { ...slotStyle("accent.secondary"), bold: true });
  const mw = widthOfCells(menu);
  if (mw > w) return { rows: [], hits: [] };
  const menuX = w - mw;
  const shown: Cell[] = [], hits: FootLayout["hits"] = [];
  let x = 0;
  for (const a of foot.actions ?? []) {
    if (a.action.heading || !a.action.id.trim()) continue;
    const cells = footActionCells(a, plain, mode), n = widthOfCells(cells);
    const gap = x ? 2 : 0;
    if (n === 0 || x + gap + n + 2 > menuX) break;
    shown.push(...cellsOf(" ".repeat(gap)), ...cells);
    x += gap;
    if (a.disabled == null && a.target?.startsWith(DETAIL_FOOT_PREFIX)) hits.push({ dx: x, dy: 1, w: n, h: 1, target: a.target });
    x += n;
  }
  if (foot.menu.disabled == null && foot.menu.target?.startsWith(DETAIL_FOOT_PREFIX)) hits.push({ dx: menuX, dy: 1, w: mw, h: 1, target: foot.menu.target });
  return { rows: [cellsOf("─".repeat(w), plain ? {} : slotStyle("text.muted")), [...padCells(shown, menuX), ...menu]], hits };
}

/** Plain readers use the complete, shared fallback so keys/reasons are never lost to clipping. */
export const detailFallback = (node: Node): Node => node.node === "extension" ? detailFallback(node.fallback) : node;
