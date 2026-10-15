import { cellsOf, clipCells, padCells, wrapCells, type Cell } from "../text/cells.js";
import { currentLook, iconOf, slotStyle, type Style } from "../text/look.js";
import { changedWords, lookOf } from "../text/DiffView.js";
import { docRows } from "../text/richRows.js";
import type { RichBlock } from "../nodes/view.js";
import type { DockActivity, DockItem, DockPicker, DockTab, DockStatus } from "./model.js";
import { cellWidth, fit } from "../chrome/width.js";
import { SPIN } from "../chrome/layout.js";
import { diffCounts, groupWords, KIND_ICON, middleFit, PATH_KINDS, shortPaths, toolKind, toolVerb, transcriptUnits, type ToolItem, type ToolKind, type Unit } from "./tools.js";

/** How far the transcript is scrolled up from its foot now that it has `total` rows: the rows that came in under a
 * scrolled view since it was scrolled (when it had `anchor`) are added, so the view stays on what it showed; at the foot
 * (0) it follows. */
export const dockScroll = (local: { scroll: number; anchor: number }, total: number): number =>
  local.scroll > 0 ? local.scroll + Math.max(0, total - local.anchor) : 0;

/**
 * The dock's rows as cells, built from view data alone (no hooks), so a state is tested by reading its rows.
 * Folding follows the UI rules: no chevrons; tree lines show an open item, a folded one ends in `⋯ n`; keys sit
 * inside their word, underlined.
 */

const style = (slot: string, extra: Style = {}): Style => ({ ...slotStyle(slot), ...extra });
const indent = (n: number) => cellsOf(" ".repeat(n));

const wrap = (cs: Cell[], width: number): Cell[][] => wrapCells(cs, Math.max(1, width));

/** The glyph for an item's lead column. */
/** A pack icon (the pack's glyph, nothing with icons off) in its slot, padded to two cells. */
export const mark = (icon: string, slot: string, w = 2): Cell[] => padCells(cellsOf(iconOf(icon)?.glyph ?? "", style(slot)), w);
const lead = mark;

/** `a̲pprove once`: the key letter underlined inside its word. */
export function keyed(word: string, key: string, slot = "text.primary"): Cell[] {
  const i = word.toLowerCase().indexOf(key.toLowerCase());
  if (i < 0) return cellsOf(word, style(slot));
  return [...cellsOf(word.slice(0, i), style(slot)), ...cellsOf(word[i]!, style(slot, { underline: true, bold: true })), ...cellsOf(word.slice(i + 1), style(slot))];
}

/** What a message sent during a turn waits for, by who holds it (DockItem user `queued`). */
const QUEUED: Record<string, string> = {
  here: "queued: sent when this turn ends",
  provider: "sent: the agent takes it after this turn",
};

const ACTIONS: Record<string, [string, string]> = {
  approve_once: ["approve once", "a"],
  approve_for_session: ["approve for session", "s"],
  deny: ["deny", "d"],
  view_diff: ["view diff", "v"],
};

/** A pointer target inside a row: `x` and `w` in cells from the row's left edge. */
export type RowHit = { x: number; w: number; target: string };

/** Where a request's buttons send a click: `dock:<what>:<request id>[:<question id>[:<choice label>]]`, each part
 * URI-encoded (ids are the providers' own and may hold any character). src/dock/composer.ts dockClick reads them. */
export const hitTarget = (what: string, ...parts: string[]) => `dock:${what}:${parts.map(encodeURIComponent).join(":")}`;

/** Ink-side picks of a question that takes several choices, by request and question (DockLocal.picks). */
export const pickKey = (request: string, question: string) => `${request}\u0000${question}`;

const ACTION_TARGET: Record<string, string> = { approve_once: "approve", approve_for_session: "session", deny: "deny", view_diff: "diff" };

export function itemRows(item: DockItem, width: number, diffs: string[] = [], picks: Record<string, string[]> = {}, opts: LayoutOpts = {}): Cell[][] {
  return itemLayout(item, width, diffs, picks, opts).rows;
}

/** What the transcript's layout knows besides its items: the session's folder (paths are shown relative to it) and the
 * home folder, the groups of calls shown open (Ink-side, DockLocal.groups), and whether the session is working now
 * (its latest reasoning then shows a line of what it thinks). */
export type LayoutOpts = {
  cwd?: string;
  home?: string;
  groups?: string[];
  live?: boolean;
  /** The turn's spinner frame (spinAt of the activity's clock), for a call still running. */
  spin?: string;
};

/** One item's layout: its rows, the buttons in each, and what a click on each row does (`dock:fold:…`, `dock:group:…`). */
export type Layout = { rows: Cell[][]; hits: RowHit[][]; targets: Array<string | undefined> };

const foldTarget = (id: string, open: boolean) => `dock:fold:${id}:${open ? 0 : 1}`;

/** ` ⋯ n`: what a folded row hides. */
const folded = (n: number): Cell[] => (n > 0 ? cellsOf(` ⋯ ${n}`, style("text.muted")) : []);

/** A tool call's target, short: the session's folder off its paths, then cut to `room` (a path in the middle, so the
 * file's name stays; anything else at its end). */
function targetCells(t: ToolItem, room: number, o: LayoutOpts): Cell[] {
  const text = shortPaths(t.title, o.cwd, o.home);
  const shown = PATH_KINDS.has(toolKind(t.kind)) ? middleFit(text, room) : fit(text, room);
  return cellsOf(shown, style("text.primary"));
}

/** A running call's mark: the turn's spinner frame (the activity line's), or the pack's running icon where the turn
 * has no clock (a story, a test). */
const running = (spin?: string): Cell[] => (spin ? padCells(cellsOf(spin, style("status.info")), 2) : lead("run_running", "status.info"));

/** The lead of a call: its state while it runs or when it failed, else its kind's icon in the icon's own slot. */
function toolMark(state: ToolItem["state"], kind: ToolKind, spin?: string): Cell[] {
  if (state === "running") return running(spin);
  if (state === "failed" || state === "denied") return lead("run_failed", "status.error");
  const icon = KIND_ICON[kind];
  return lead(icon, iconOf(icon)?.style ?? "text.secondary");
}

/** One tool call on one row: `<mark> <verb> <target> <+a −r> <failed|denied> ⋯ n`; the target gives way first. */
function toolRow(t: ToolItem, width: number, o: LayoutOpts, open: boolean): Cell[] {
  const kind = toolKind(t.kind);
  const diff = t.diff ?? [];
  const hidden = diff.length || (t.result ?? []).length;
  const verb = cellsOf(`${toolVerb(kind, t.kind)} `, style("text.secondary"));
  const tail: Cell[] = [];
  if (diff.length) {
    const { added, removed } = diffCounts(diff);
    tail.push(...cellsOf(` +${added}`, style("diff.added")), ...cellsOf(` −${removed}`, style("diff.removed")));
  }
  if (t.state === "failed" || t.state === "denied") tail.push(...cellsOf(` ${t.state}`, style("status.error")));
  if (!open) tail.push(...folded(hidden));
  const mark = toolMark(t.state, kind, o.spin);
  const room = Math.max(1, width - rowWidth(mark) - rowWidth(verb) - rowWidth(tail));
  return [...mark, ...verb, ...targetCells(t, room, o), ...tail];
}

/** An edit's diff lines in the diff view's look: added and removed on their tints across the row, the words that
 * changed on a stronger tint; `pre` before each, the row filled to `width`. */
function diffRows(lines: string[], pre: (last: boolean) => Cell[], width: number): Cell[][] {
  const look = lookOf();
  const rows = lines.map((l, i) => {
    const kind = l.startsWith("+") ? "added" : l.startsWith("-") ? "removed" : "context";
    const fill: Style = kind === "added" && look.added ? { backgroundColor: look.added } : kind === "removed" && look.removed ? { backgroundColor: look.removed } : {};
    const fg = style(kind === "added" ? "diff.added" : kind === "removed" ? "diff.removed" : "text.secondary", fill);
    return { kind, text: l.slice(1), cells: cellsOf(l, fg), fill, p: pre(i === lines.length - 1) };
  });
  // A removed run faces the added run after it: the words that changed, emphasised (ui::diff::changed_words).
  for (let i = 0; i < rows.length; ) {
    if (rows[i]!.kind !== "removed") {
      i++;
      continue;
    }
    const r0 = i;
    while (i < rows.length && rows[i]!.kind === "removed") i++;
    const a0 = i;
    while (i < rows.length && rows[i]!.kind === "added") i++;
    for (let k = 0; k < Math.min(a0 - r0, i - a0); k++) {
      const { o, n } = changedWords(rows[r0 + k]!.text, rows[a0 + k]!.text);
      const mark = (row: (typeof rows)[number], ranges: [number, number][], s: Style) => {
        // +1: the sign before the text
        row.cells = row.cells.map((c, idx) => (ranges.some(([f, t]) => idx - 1 >= f && idx - 1 < t) ? { ...c, s: { ...c.s, ...s } } : c));
      };
      mark(rows[r0 + k]!, o, look.removedWord);
      mark(rows[a0 + k]!, n, look.addedWord);
    }
  }
  return rows.map((r) => padCells(clipCells([...r.p, ...r.cells], width), width, r.fill));
}

/** A tree line before row `last` of a list: `├ ` and, for the last, `└ `. */
const branch = (last: boolean) => cellsOf(last ? "└ " : "├ ", style("text.muted"));

/** A unit's layout: an item's, or a group's (`<icon> read 5 files ⋯ 5`; open, its calls under it on tree lines, each
 * opening on its own). */
export function unitLayout(u: Unit, width: number, diffs: string[] = [], picks: Record<string, string[]> = {}, o: LayoutOpts = {}): Layout {
  if (u.u === "item") return itemLayout(u.item, width, diffs, picks, o);
  const open = (o.groups ?? []).includes(u.id);
  const going = u.tools.filter((t) => t.state === "running");
  const [verb, count] = groupWords(u.kind, u.tools.length);
  const mark = toolMark(going.length ? "running" : "done", u.kind, o.spin);
  const head: Cell[] = [...mark, ...cellsOf(`${verb} `, style("text.secondary")), ...cellsOf(count, style("text.primary"))];
  const tail = open ? [] : folded(u.tools.length);
  const latest = going[going.length - 1];
  if (latest) {
    const room = Math.max(1, width - rowWidth(head) - rowWidth(tail) - 3);
    head.push(...cellsOf(" · ", style("text.muted")), ...targetCells(latest, room, o).map((c) => ({ ...c, s: style("text.muted") })));
  }
  const rows: Cell[][] = [[...head, ...tail]];
  const targets: Array<string | undefined> = [`dock:group:${encodeURIComponent(u.id)}`];
  if (open)
    u.tools.forEach((t, i) => {
      const last = i === u.tools.length - 1;
      const res = t.result ?? [];
      const pre = [...indent(2), ...branch(last)];
      const tl: Cell[] = [...(t.state === "running" ? cellsOf(` ${o.spin ?? iconOf("run_running")?.glyph ?? ""}`, style("status.info")) : []), ...(t.open ? [] : folded(res.length))];
      rows.push([...pre, ...targetCells(t, Math.max(1, width - rowWidth(pre) - rowWidth(tl)), o), ...tl]);
      targets.push(foldTarget(t.id, t.open));
      if (t.open)
        res.forEach((l, j) => {
          rows.push([...indent(2), ...cellsOf(last ? "  " : "│ ", style("text.muted")), ...branch(j === res.length - 1), ...cellsOf(l, style("text.secondary"))]);
          targets.push(foldTarget(t.id, t.open));
        });
    });
  return { rows: rows.map((r) => clipCells(r, width)), hits: rows.map(() => []), targets };
}

/** An assistant message laid out (rows of its document, or of its text where the host sent none), kept while its text,
 * width and look are the same: the transcript is laid out on every draw. */
const DOCS = new Map<string, { text: string; sig: string; look: unknown; rows: Cell[][] }>();
function answerRows(item: Extract<DockItem, { item: "assistant" }>, body: number): Cell[][] | undefined {
  if (!item.doc?.length) return undefined;
  const key = `${item.id}\u0000${body}\u0000${item.saved}`;
  // The host makes doc and painted from the text; what it sent of them is checked all the same.
  const sig = `${item.doc.length}:${(item.painted ?? []).map((p) => p.length).join(",")}`;
  const hit = DOCS.get(key);
  if (hit && hit.text === item.text && hit.sig === sig && hit.look === currentLook()) return hit.rows;
  const rows = docRows(item.doc as RichBlock[], body, item.painted ?? []);
  if (DOCS.size > 256) DOCS.clear();
  DOCS.set(key, { text: item.text, sig, look: currentLook(), rows });
  return rows;
}

/** An item's rows and, row by row, the buttons in them (approve, deny, view diff; a question's choices, submit and
 * decline), so a click does what the key does, and what a click on the row does. */
export function itemLayout(item: DockItem, width: number, diffs: string[] = [], picks: Record<string, string[]> = {}, o: LayoutOpts = {}): Layout {
  const body = Math.max(1, width - 2);
  const out: Cell[][] = [];
  const hits: RowHit[][] = [];
  const push = (l: Cell[], cs: Cell[]) => out.push([...l, ...cs]);
  /** A row of buttons: `[glyph or keyed word, target?]` two cells apart, after `pad` cells. A button that would run
   * past the width starts the next row, at the same pad. */
  const buttons = (pad: number, parts: Array<[Cell[], string | undefined]>) => {
    let row: Cell[] = [...indent(pad)];
    let hs: RowHit[] = [];
    const flush = () => {
      hits[out.length] = hs;
      out.push(row);
    };
    parts.forEach(([cs, target], i) => {
      if (i > 0 && rowWidth(row) + 2 + rowWidth(cs) > width) {
        flush();
        row = [...indent(pad)];
        hs = [];
      } else if (i > 0) row.push(...cellsOf("  "));
      const x = rowWidth(row);
      row.push(...cs);
      if (target) hs.push({ x, w: rowWidth(cs), target });
    });
    flush();
  };
  /** `⏎ submit`: a glyph key before its word (UI rules), the glyph bold. */
  const glyphKey = (glyph: string, word: string): Cell[] => [...cellsOf(glyph, style("text.primary", { bold: true })), ...cellsOf(` ${word}`, style("text.secondary"))];
  const cont = indent(2);
  const textRows = (text: string, st: Style, first: Cell[]) =>
    text.split("\n").forEach((line, li) => wrap(cellsOf(line, st), body).forEach((r, ri) => push(li === 0 && ri === 0 ? first : cont, r)));
  switch (item.item) {
    case "user":
      if (!item.queued) {
        textRows(item.text, style("text.emphasis", { bold: true }), lead("you", "accent.primary"));
        break;
      }
      // On its way: dimmed, and a line under it says who holds it until the agent takes it.
      textRows(item.text, style("text.secondary"), lead("you", "text.muted"));
      push(cont, cellsOf(QUEUED[item.queued], style("text.muted", { italic: true })));
      break;
    case "assistant": {
      const first = item.saved ? cont : lead("run_running", "text.muted");
      const doc = answerRows(item, body);
      if (doc) doc.forEach((r, i) => push(i === 0 ? first : cont, r));
      else textRows(item.text, style(item.saved ? "text.primary" : "text.secondary"), first);
      break;
    }
    case "reasoning": {
      const lines = item.text.split("\n").filter((l, i, a) => l.trim() !== "" || i < a.length - 1);
      const quiet = style("text.muted", { italic: true });
      if (!item.open && o.live && lines.length > 0) {
        // Thinking now: the latest line of it beside the fold, cut to the row (none kept to itself: the activity line
        // says the agent thinks).
        const now = [...lines].reverse().find((l) => l.trim() !== "")?.trim() ?? "";
        const headCells = [...indent(2), ...cellsOf("thinking", quiet)];
        const tail = folded(lines.length);
        const room = Math.max(0, width - rowWidth(headCells) - rowWidth(tail) - 3);
        out.push([...headCells, ...(now && room > 1 ? [...cellsOf(" · ", style("text.muted")), ...cellsOf(fit(now, room), style("text.secondary", { italic: true }))] : []), ...tail]);
      } else if (!item.open && lines.length > 0) out.push([...indent(2), ...cellsOf("reasoning", quiet), ...folded(lines.length)]);
      // Reasoning the provider kept to itself (empty) says nothing once the agent has done thinking.
      else lines.forEach((l) => wrap(cellsOf(l, quiet), body).forEach((r) => push(cont, r)));
      break;
    }
    case "tool": {
      out.push(toolRow(item, width, o, item.open));
      if (item.open) {
        const diff = item.diff ?? [];
        const res = item.result ?? [];
        if (diff.length) out.push(...diffRows(diff, (last) => branch(last), width));
        else res.forEach((l, i) => push(branch(i === res.length - 1), clipCells(cellsOf(l, style("text.secondary")), body)));
      }
      break;
    }
    case "permission": {
      if (item.answered) {
        // Answered, with no call of its own in the transcript to tell it: one row.
        const no = item.answered === "deny";
        const said = no ? "denied" : item.answered === "approve_for_session" ? "approved for session" : "approved";
        const headCells = [...(no ? lead("run_failed", "status.error") : lead("approved", "status.success")), ...cellsOf(`${item.kind} ${said}  `, style("text.secondary"))];
        const room = Math.max(1, width - rowWidth(headCells));
        out.push([...headCells, ...cellsOf(middleFit(shortPaths(item.target, o.cwd, o.home), room), style("text.primary"))]);
        break;
      }
      push(lead("warning", "status.warning"), cellsOf(`allow ${item.kind}?`, style("status.warning", { bold: true })));
      wrap(cellsOf(item.target, style("text.emphasis")), body - 1).forEach((r) => push(indent(3), r));
      if (diffs.includes(item.id)) out.push(...diffRows((item.diff ?? []).slice(0, 12), () => indent(3), width));
      buttons(
        3,
        item.actions.map((a) => {
          const [word, k] = ACTIONS[a] ?? [a, a[0]!];
          return [keyed(word, k), hitTarget(ACTION_TARGET[a] ?? a, item.id)];
        }),
      );
      break;
    }
    case "question": {
      const waiting = item.state === "waiting";
      // The question answered next: the first one without its answer (a request asks them in order).
      const at = waiting ? item.questions.findIndex((q) => !q.answered) : -1;
      item.questions.forEach((q, qi) => {
        const first = out.length === 0;
        const head = `${q.header ? `${q.header}: ` : ""}${q.text}`;
        if (qi === at) {
          textRows(head, style("text.emphasis", { bold: true }), first ? lead("mention", "status.warning") : cont);
          const picked = picks[pickKey(item.id, q.id)] ?? [];
          (q.choices ?? []).forEach((c, ci) => {
            // Numbered: the number is the key, before its word (a choice's label is the agent's, no letter of it is free).
            const n = ci + 1;
            const cs: Cell[] = [
              ...(n <= 9 ? cellsOf(String(n), style("text.primary", { underline: true, bold: true })) : cellsOf(" ")),
              ...cellsOf(" "),
              ...(q.multiple ? (picked.includes(c.label) ? mark("approved", "status.success") : indent(2)) : []),
              ...cellsOf(c.label, style("text.primary")),
              ...(c.detail ? cellsOf(`  ${c.detail}`, style("text.muted")) : []),
            ];
            buttons(3, [[cs, hitTarget("choose", item.id, q.id, c.label)]]);
          });
          const n = (q.choices ?? []).length;
          const keys: Array<[Cell[], string | undefined]> = [];
          if (n > 0) keys.push([glyphKey(n > 1 ? `1-${Math.min(9, n)}` : "1", q.multiple ? "pick" : "choose"), undefined]);
          if (q.multiple) keys.push([glyphKey("⏎", "submit"), hitTarget("submit", item.id, q.id)]);
          if (q.free_text) keys.push([glyphKey("⏎", n > 0 ? "or type your own answer" : "sends the answer you type"), undefined]);
          keys.push([glyphKey("esc", "decline"), hitTarget("decline", item.id)]);
          buttons(3, keys);
        } else if (q.answered || !waiting) {
          const said = q.answer?.length ? q.answer.join(", ") : item.state === "declined" ? "declined" : "answered";
          push(first ? lead("mention", "text.muted") : cont, clipCells([...cellsOf(head, style("text.muted")), ...cellsOf(` → ${said}`, style("text.secondary"))], body));
        } else {
          push(cont, clipCells(cellsOf(`then: ${head}`, style("text.muted")), body));
        }
      });
      break;
    }
    case "error":
      textRows(item.text, style("status.error"), lead("error", "status.error"));
      break;
    case "notice":
      textRows(item.text, style("text.muted"), lead("info", "text.muted"));
      break;
  }
  const target = item.item === "tool" || item.item === "reasoning" ? foldTarget(item.id, item.open) : undefined;
  return {
    rows: out.map((r) => clipCells(r, width)),
    // A button past the edge is not there to point at; one cut by it is as wide as what shows.
    hits: out.map((_, i) => (hits[i] ?? []).filter((h) => h.x < width).map((h) => ({ ...h, w: Math.min(h.w, width - h.x) }))),
    targets: out.map(() => target),
  };
}

/** Prose (what the user and the agent say) stands a row apart from what is next to it; calls and requests in a run
 * stay together. */
const prose = (u: Unit) => u.u === "item" && (u.item.item === "user" || u.item.item === "assistant");

/** Every transcript row for `items`, then `height` of them ending `scroll` rows above the foot (the transcript follows
 * its foot at 0); `owners` names the item each shown row belongs to and `hits` the buttons in it, for the pointer.
 * `max` is how far it can scroll. */
export function transcriptView(
  items: DockItem[],
  width: number,
  height: number,
  diffs: string[] = [],
  scroll = 0,
  picks: Record<string, string[]> = {},
  /** The rows the transcript had when it was scrolled `scroll` up (DockLocal.anchor): rows added since keep the view
   * where it was; at the foot it follows them. Unset: `scroll` is from the foot as it is now. */
  anchor?: number,
  opts: LayoutOpts = {},
): { rows: Cell[][]; owners: DockItem[]; hits: RowHit[][]; targets: Array<string | undefined>; max: number; total: number } {
  const all: Cell[][] = [];
  const own: DockItem[] = [];
  const hs: RowHit[][] = [];
  const ts: Array<string | undefined> = [];
  const units = transcriptUnits(items);
  const lastItem = items[items.length - 1];
  let prev: Unit | undefined;
  for (const u of units) {
    const it = u.u === "item" ? u.item : u.tools[0]!;
    // The latest reasoning shows what it thinks while the session works.
    const l = unitLayout(u, width, diffs, picks, { ...opts, live: !!opts.live && it === lastItem });
    if (l.rows.length === 0) continue;
    if (prev && (prose(u) || prose(prev))) (all.push([]), own.push(it), hs.push([]), ts.push(undefined));
    l.rows.forEach((r, i) => (all.push(r), own.push(it), hs.push(l.hits[i] ?? []), ts.push(l.targets[i])));
    prev = u;
  }
  const max = Math.max(0, all.length - height);
  const up = anchor === undefined ? scroll : dockScroll({ scroll, anchor }, all.length);
  const end = all.length - Math.min(max, Math.max(0, up));
  const start = Math.max(0, end - height);
  return { rows: all.slice(start, end), owners: own.slice(start, end), hits: hs.slice(start, end), targets: ts.slice(start, end), max, total: all.length };
}

export function transcriptRows(items: DockItem[], width: number, height: number, diffs: string[] = [], scroll = 0): Cell[][] {
  return transcriptView(items, width, height, diffs, scroll).rows;
}

/** The pack icon and its slot that tell a session's state in its tab (idle and seen shows none). */
const STATE: Record<string, [string, string]> = {
  approval: ["warning", "status.warning"],
  error: ["run_failed", "status.error"],
  stopped: ["run_canceled", "text.muted"],
  ended: ["run_skipped", "text.muted"],
};

/** The kit's spinner frame for `seconds` of a running turn (ui::states::spinner, the top bar's throbber). */
export const spinAt = (seconds: number): string => SPIN[Math.abs(Math.floor(seconds)) % SPIN.length]!;

/** A tab's state mark: the spinner while its turn runs, the attention mark while it needs you, the failed mark, the
 * unread mark when a turn ended out of sight, and nothing for an idle tab already seen. */
export function tabState(t: DockTab): { glyph: string; slot: string } | undefined {
  if (t.state === "streaming") return { glyph: spinAt(t.running ?? 0), slot: "status.info" };
  if (t.unread && t.state !== "error" && t.state !== "approval") {
    const g = iconOf("unread")?.glyph;
    return g ? { glyph: g, slot: "accent.primary" } : undefined;
  }
  const s = STATE[t.state];
  const g = s ? iconOf(s[0])?.glyph : undefined;
  return s && g ? { glyph: g, slot: s[1] } : undefined;
}

/** One tab: `<icon> name <state>`; the active one in bold accent, as the page tab row's. */
export function tabLabel(t: DockTab): string {
  const glyph = iconOf(t.icon)?.glyph;
  const st = tabState(t)?.glyph ?? "";
  return `${glyph ? `${glyph} ` : ""}${t.title}${st ? ` ${st}` : ""}`;
}

/** A tab's label in `room` cells: the icon and name cut first, the state mark kept whole in its own slot. */
function tabCells(t: DockTab, room: number, st: Style): Cell[] {
  const glyph = iconOf(t.icon)?.glyph;
  const mark = tabState(t);
  const markW = mark ? 1 + cellWidth(mark.glyph) : 0;
  const head = `${glyph ? `${glyph} ` : ""}${t.title}`;
  const headRoom = Math.max(1, room - markW);
  const fitted = cellWidth(head) > headRoom ? fit(head, headRoom) : head;
  return [...cellsOf(fitted, st), ...(mark ? cellsOf(` ${mark.glyph}`, { ...st, ...slotStyle(mark.slot) }) : [])];
}

/** The mark that closes a tab, from the pack (`close`); a plain `×` where the pack draws nothing, because this one is a control. */
export const closeGlyph = (): string => iconOf("close")?.glyph || "×";

/** What a tab shows after its label: a space and the close mark. */
const closeCells = (on: boolean): Cell[] => cellsOf(` ${closeGlyph()}`, on ? style("accent.primary", { bold: true }) : style("text.muted"));
const closeWidth = () => 1 + cellWidth(closeGlyph());

/** The room one tab's label has in the row: its share of the width less the gaps, the brackets and the close mark. */
const labelRoom = (n: number, width: number) => Math.max(1, Math.floor(width / Math.max(1, n)) - 4 - closeWidth());

/** Where each tab sits in the tab row (cells from the row's left edge), for the pointer: the tab as a whole (`x`, `w`)
 * and its close mark (`closeX`, `closeW`, the space before the mark included). The row is the page tab row's
 * (app/TabRow.tsx): a cell in, the tab showing in brackets and bold accent, two spaces between. */
export function tabSpans(tabs: DockTab[], width: number, active = 0): Array<{ id: string; x: number; w: number; closeX: number; closeW: number }> {
  const room = labelRoom(tabs.length, width);
  let x = 1;
  return tabs.map((t, i) => {
    const on = i === active;
    const closeW = closeWidth();
    const w = rowWidth(tabCells(t, room, {})) + closeW + (on ? 2 : 0);
    const span = { id: t.id, x, w, closeX: x + w - closeW - (on ? 1 : 0), closeW };
    x += w + 2;
    return span;
  });
}

export function tabRowCells(tabs: DockTab[], active: number, width: number): Cell[] {
  const row: Cell[] = [...cellsOf(" ")];
  const room = labelRoom(tabs.length, width);
  tabs.forEach((t, i) => {
    const on = i === active;
    const st = on ? style("accent.primary", { bold: true }) : style("text.secondary");
    if (on) row.push(...cellsOf("[", st));
    row.push(...tabCells(t, room, st), ...closeCells(on));
    if (on) row.push(...cellsOf("]", st));
    if (i < tabs.length - 1) row.push(...cellsOf("  "));
  });
  return padCells(clipCells(row, width), width);
}

/** `42s`, `1m 12s`, `1h 05m`: how long a turn has run or took. */
export function duration(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(s / 3600)}h ${String(Math.floor((s % 3600) / 60)).padStart(2, "0")}m`;
}

/** `640 tokens`, `3.4k tokens`, `12k tokens`, `1.2M tokens`. */
export function tokenWords(n: number): string {
  const one = (x: number) => (x < 10 ? x.toFixed(1).replace(/\.0$/, "") : String(Math.round(x)));
  if (n < 1000) return `${n} tokens`;
  if (Math.round(n / 1000) < 1000) return `${one(n / 1000)}k tokens`;
  return `${one(n / 1_000_000)}M tokens`;
}

/**
 * The activity line above the composer: while a turn runs, the spinner and what the agent does now (thinking, writing,
 * running a command, waiting for your approval or your answer), how long the turn has run and the tokens it used when
 * the provider says; once it ended, how it ended and how long it took, muted, so the dock does not jump when the next
 * turn starts. Always one row; the words are cut, never the time.
 */
export function activityCells(a: DockActivity, width: number): Cell[] {
  const tail = [duration(a.elapsed), a.tokens != null ? tokenWords(a.tokens) : "", a.queued ? `${a.queued} queued` : ""].filter(Boolean);
  const live = ["thinking", "writing", "running", "approval", "question"].includes(a.doing);
  const ended: Record<string, [string, string, string]> = {
    done: ["run_passed", "status.success", `done in ${tail.join(" · ")}`],
    stopped: ["run_canceled", "text.muted", `stopped after ${tail.join(" · ")}`],
    failed: ["run_failed", "status.error", `failed after ${tail.join(" · ")}`],
  };
  if (!live) {
    const [icon, slot, words] = ended[a.doing] ?? ended.done!;
    return padCells(clipCells([...cellsOf(" "), ...mark(icon, slot), ...cellsOf(words, style("text.muted"))], width), width);
  }
  const waits = a.doing === "approval" || a.doing === "question";
  const lead = waits ? mark("warning", "status.warning") : padCells(cellsOf(spinAt(a.elapsed), style("status.info")), 2);
  const doingWords: Record<string, string> = { thinking: "thinking", writing: "writing", running: "running", approval: "waiting for your approval", question: "waiting for your answer" };
  const what = doingWords[a.doing] ?? a.doing;
  const words = cellsOf(what, waits ? style("status.warning", { bold: true }) : style("text.secondary"));
  const after = cellsOf(` · ${tail.join(" · ")}`, style("text.muted"));
  // The detail (the command, the target) takes what room is left and is cut first.
  // `running git diff`, `waiting for your approval · git push`.
  const sep = waits ? " · " : " ";
  const room = Math.max(0, width - 1 - 2 - rowWidth(words) - rowWidth(after) - cellWidth(sep));
  const detail = a.detail && room > 1 ? [...cellsOf(sep, style("text.muted")), ...cellsOf(fit(a.detail, room), style(waits ? "text.emphasis" : "text.primary"))] : [];
  return padCells(clipCells([...cellsOf(" "), ...lead, ...words, ...detail, ...after], width), width);
}

/** The dock's status line: what runs (provider, agent, model, folder, branch, usage) on the left, cut when the room is
 * short, and the keys that matter now (`hint`: "esc stop · alt-w close") at the right edge, which is never cut. */
export function statusCells(s: DockStatus, width: number, hint?: string): Cell[] {
  const parts = [s.provider, s.agent ? `as ${s.agent}` : "", s.model, s.cwd, s.branch, s.usage].filter((p): p is string => !!p);
  const keys = hint ? ` ${hint} ` : "";
  const room = Math.max(0, width - cellWidth(keys));
  const left = padCells(clipCells(cellsOf(fit(` ${parts.join(" · ")}`, room), style("text.muted")), room), room);
  return padCells(clipCells([...left, ...cellsOf(keys, style("text.muted"))], width), width);
}

export const rowWidth = (cs: Cell[]) => cs.reduce((n, c) => n + c.w, 0);
export { cellWidth };

/** A picker over the transcript: its choices one a row, the selected on bg.selection, the current one marked. */
export function pickerRows(p: DockPicker, at: number, width: number, height: number): Cell[][] {
  const what = { model: "model", session: "session", agent: "agent for new sessions", skill: "skill to ask for" }[p.kind] ?? p.kind;
  // Until the CLI answers the host says it is reading; after that the CLI's own agent is always the first choice.
  const empty = p.reading ? " · reading what the CLI offers…" : p.choices.length === 0 ? " · none here" : p.kind === "agent" && p.choices.length === 1 ? " · the CLI offers no other agent here" : "";
  const title = padCells(cellsOf(` ${what} · ⏎ choose · esc close${empty}`, style("text.muted")), width);
  const first = Math.max(0, Math.min(at - (height - 2), p.choices.length - (height - 1)));
  const rows = p.choices.slice(first, first + Math.max(0, height - 1)).map((c, i) => {
    const sel = first + i === at;
    const st = sel ? style("text.emphasis", { backgroundColor: slotStyle("bg.selection").color, bold: true }) : style("text.primary");
    const label = ` ${c.current ? "• " : "  "}${c.label}${c.detail ? `  ${c.detail}` : ""}`;
    return padCells(clipCells(cellsOf(label, st), width), width, sel ? { backgroundColor: slotStyle("bg.selection").color } : {});
  });
  return [title, ...rows];
}

/** The slash commands matching what is typed, one row each, shown over the foot of the transcript: the names in one
 * column (as wide as the longest shown, up to 24 cells), what each does after it, the selected row on bg.selection. */
export function commandRows(cmds: Array<{ name: string; detail: string }>, width: number, sel = -1): Cell[][] {
  const col = Math.min(24, Math.max(0, ...cmds.map((c) => cellWidth(c.name))));
  return cmds.map((c, i) => {
    const on = i === sel;
    const fill = on ? { backgroundColor: slotStyle("bg.selection").color } : {};
    const name = on ? style("text.emphasis", { ...fill, bold: true }) : style("accent.primary");
    const detail = style(on ? "text.secondary" : "text.muted", fill);
    const gap = " ".repeat(Math.max(2, col - cellWidth(c.name) + 2));
    return padCells(clipCells([...cellsOf(` /${c.name}`, name), ...cellsOf(c.detail ? `${gap}${c.detail}` : "", detail)], width), width, fill);
  });
}
