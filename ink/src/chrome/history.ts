import { cellWidth, clipSegments, fit } from "./width.js";
import { focusRamp, frameRuns } from "../text/frame.js";
import { segWords } from "../text/TextLine.js";
import { hintCells, keyHits, type Cell } from "./layout.js";
import { lineCells, paragraphCells, type Cell as TextCell } from "../text/cells.js";
import type { Hint, Hit, Text } from "./types.js";
import { bgOf, iconOf, slotStyle, specStyle, type Style } from "../text/look.js";
import { inkStyle } from "../text/theme.js";
import { selectedStyle } from "../nodes/tree.js";
import { fitTableColumns } from "../nodes/columns.js";
import { thumbOf } from "../nodes/index.js";
import type { ColumnSpec } from "../nodes/view.js";
import type { HistoryView } from "../protocol/generated/HistoryView.js";
import type { PaneMark } from "../motion/interaction.js";
import { insideOf, type SelectArea } from "../input/selection.js";

/** The history overlay laid out in cells (history.rs `History::paint`): every line is exactly `width` cells. */
export type HistoryLayout = { lines: Cell[][]; hits: Hit[]; selects: SelectArea[]; panes: PaneMark[] };

/** Break `s` into lines of at most `width` cells, at spaces where there are any. */
export function wrap(s: string, width: number): string[] {
  if (width <= 0) return [];
  const out: string[] = [];
  let line = "";
  const flush = () => {
    out.push(line);
    line = "";
  };
  for (const word of s.split(/( +)/)) {
    if (word === "") continue;
    if (cellWidth(line + word) <= width) {
      line += word;
      continue;
    }
    if (line.trim() !== "") flush();
    if (/^ +$/.test(word)) continue;
    let rest = word;
    while (cellWidth(rest) > width) {
      let cut = "";
      for (const ch of rest) {
        if (cellWidth(cut + ch) > width) break;
        cut += ch;
      }
      if (cut === "") break;
      out.push(cut);
      rest = rest.slice(cut.length);
    }
    line = rest;
  }
  if (line !== "" || out.length === 0) flush();
  return out;
}

const pad = (s: string, width: number) => s + " ".repeat(Math.max(0, width - cellWidth(s)));

/** `style` with `bg` under it unless it has its own. */
const on = (style: Style, bg: string | undefined): Style => ({ ...style, backgroundColor: style.backgroundColor ?? bg });

type TargetCell = Cell & { target?: string };
/** Preserve the supplied target through wrapping; the host owns each action phrase. */
function wrapText(t: Text, width: number, plain = false): TargetCell[][] {
  if (width <= 0) return [];
  const targets = new Map<TextCell, string>();
  const chars = t.flatMap((seg) => {
    const cells = lineCells([seg], plain);
    if (seg.target) for (const cell of cells) targets.set(cell, seg.target);
    return cells;
  });
  return paragraphCells(chars, width).map((line) => line.map((c) => ({ text: c.ch, style: c.s, target: targets.get(c) })));
}

type PRow = { cells: TargetCell[]; /** The row's own shade across the panel's width (the selection). */ fill?: Style };

/** widgets::panel: a rounded border (accent.primary when focused, else accent.muted), the title in bold emphasis
 * when focused else secondary, the inside on bg.surface; `rows` fill the inside (w - 4 cells, 1 cell padding). */
function panel(title: string, w: number, h: number, rows: PRow[], focused: boolean): Cell[][] {
  const surface = bgOf("bg.surface");
  if (w < 4 || h < 2) return Array.from({ length: Math.max(0, h) }, () => [{ text: " ".repeat(Math.max(0, w)) }]);
  const border = on(slotStyle(focused ? "accent.primary" : "accent.muted"), surface);
  const ramp = focused ? focusRamp() : undefined;
  const edge = (text: string, x: number, y: number): Cell[] => frameRuns(text, w, h, x, y, border, ramp);
  const titleStyle = on(focused ? { ...slotStyle("text.emphasis"), bold: true } : slotStyle("text.secondary"), surface);
  const label = ` ${fit(title, Math.max(0, w - 6))} `;
  const out: Cell[][] = [
    [
      ...edge("╭", 0, 0),
      { text: label, style: titleStyle },
      ...edge(`${"─".repeat(Math.max(0, w - 2 - cellWidth(label)))}╮`, 1 + cellWidth(label), 0),
    ],
  ];
  const inner = w - 4;
  const body = on(slotStyle("text.primary"), surface);
  for (let i = 0; i < h - 2; i++) {
    const row = rows[i];
    const fill = row?.fill;
    const shade: Style = fill ?? body;
    let used = 0;
    const kept: Cell[] = [];
    for (const c of row?.cells ?? []) {
      const room = inner - used;
      if (room <= 0) break;
      const text = cellWidth(c.text) > room ? fit(c.text, room) : c.text;
      kept.push({ ...c, text, style: fill ? { ...c.style, ...fill } : on({ ...body, ...c.style }, surface) });
      used += cellWidth(text);
    }
    out.push([
      ...edge("│", 0, i + 1),
      { text: " ", style: shade },
      ...kept,
      { text: " ".repeat(Math.max(0, inner - used)), style: shade },
      { text: " ", style: shade },
      ...edge("│", w - 1, i + 1),
    ]);
  }
  out.push(edge(`╰${"─".repeat(w - 2)}╯`, 0, h - 1));
  return out;
}

/** scroll::paragraph in a panel: `rows` from the scroll's offset (as far as the room allows) and the scrollbar. */
function scrolledPanel(title: string, w: number, h: number, rows: PRow[], scroll: number, focused: boolean): Cell[][] {
  const room = Math.max(0, h - 2);
  const max = Math.max(0, rows.length - room);
  const at = Math.min(scroll, max);
  const framed = panel(title, w, h, rows.slice(at), focused);
  barOn(framed, h, { max, at });
  return framed;
}

/** scroll::scrollbar on a painted panel's right border, between its corners, when `bar.max` is more than 0. */
function barOn(framed: Cell[][], h: number, bar: { max: number; at: number }) {
  if (bar.max <= 0 || h <= 2) return;
  const { start, len } = thumbOf(h - 2, bar.max, bar.at);
  const surface = bgOf("bg.surface");
  for (let k = 0; k < h - 2; k++) {
    const on_ = k >= start && k < start + len;
    framed[1 + k]![framed[1 + k]!.length - 1] = { text: on_ ? "█" : "║", style: on(slotStyle(on_ ? "accent.muted" : "bg.overlay"), surface) };
  }
}

/** Match native History's deterministic whole-hint priorities, always reserving Escape. */
export function historyFooter(hints: Hint[], width: number): Hint[] {
  const available = Math.max(0, width - 1);
  const cost = (hs: Hint[]) => hs.reduce((n, h) => n + hintCells(h).reduce((m, c) => m + cellWidth(c.text), 0), Math.max(0, hs.length - 1) * 3);
  let shown = [...hints];
  for (const label of ["previous page", "next page", "refresh", "page", "details", "events", "move", "scroll"]) {
    if (cost(shown) <= available) break;
    shown = shown.filter((h) => h.label !== label);
  }
  if (cost(shown) > available) shown = shown.filter((h) => h.keys === "esc");
  return cost(shown) <= available ? shown : [];
}

export function layoutHistory(view: HistoryView, width: number, height: number, top: number, plain = false): HistoryLayout {
  if (plain) view = { ...view, footer: view.footer.map((h) => ({ ...h, mnemonic_at: undefined, mnemonic_shift: false })) };
  if (width <= 0 || height <= 0) return { lines: [], hits: [], selects: [], panes: [] };
  const lines: Cell[][] = [];
  const panes: PaneMark[] = [];
  const paneMark = (pane: string, title: string, x: number, w: number, h: number, owns: boolean, scrollbar: boolean) => {
    if (w < 4 || h < 2) return;
    const y = top + lines.length;
    panes.push({ pane, owns, area: { x, y, w, h }, title: { x: x + 1, y, w: cellWidth(` ${fit(title, Math.max(0, w - 6))} `), h: 1 },
      keep: scrollbar ? [{ x: x + w - 1, y: y + 1, w: 1, h: h - 2 }] : [] });
  };
  const hits: Hit[] = [];
  const base = on(slotStyle("text.primary"), bgOf("bg.base"));
  const full = (cells: Cell[]): Cell[] => {
    cells = clipSegments(cells, width);
    const used = cells.reduce((n, c) => n + cellWidth(c.text), 0);
    return [...cells, { text: " ".repeat(Math.max(0, width - used)), style: base }].map((c) => ({ ...c, style: on({ ...base, ...c.style }, bgOf("bg.base")) }));
  };
  const head: Cell[][] = [full([{ text: fit(` ${view.title} · ${view.boundary}`, width), style: { ...slotStyle("text.emphasis"), bold: true } }])];
  for (const l of wrap(` Scope: ${view.scope}`, width).slice(0, 2)) head.push(full([{ text: l, style: slotStyle("text.secondary") }]));
  for (const note of view.notes) {
    // history.rs lead: one cell in, the gap in the first run's look.
    const spaced: Text = note.length > 0 ? [{ ...note[0]!, text: ` ${segWords(note[0]!)}`, icon: undefined }, ...note.slice(1)] : [{ text: " " }];
    for (const l of wrapText(spaced, width, plain)) head.push(full(l));
  }
  const room = Math.max(0, height - 1);
  const shownHead = head.slice(0, Math.max(0, Math.min(head.length, room)));
  lines.push(...shownHead);
  const bodyH = Math.max(0, height - shownHead.length - 1);

  const events = (w: number, h: number, focused: boolean): Cell[][] => {
    const inner = Math.max(0, w - 4);
    const rows: PRow[] = [];
    const visible = Math.max(0, h - 3);
    let bar = { max: 0, at: 0 };
    if (view.rows.length === 0) {
      const icon = iconOf("empty");
      const lead = icon?.glyph ? `${icon.glyph} ` : "";
      wrap(`${lead}${view.state}`, inner).forEach((l, i) => {
        const cells: Cell[] = [];
        if (i === 0 && lead && l.startsWith(lead)) cells.push({ text: lead, style: specStyle(icon!.style) }, { text: l.slice(lead.length), style: slotStyle("text.muted") });
        else cells.push({ text: l, style: slotStyle("text.muted") });
        rows.push({ cells });
      });
    } else {
      if (view.error) for (const l of wrap(view.state, inner)) rows.push({ cells: [{ text: l, style: slotStyle("status.warning") }] });
      const specs = [
        { heading: "observed UTC", width: 21, keep: 3 },
        { heading: "action", width: 24, keep: 4 },
        { heading: "subject", width: 12, keep: 255, flex: true },
        { heading: "outcome", width: 11, keep: 5 },
      ] as ColumnSpec[];
      // ui::table::Table lays the columns out in the padded area (the panel less its padding): `inner` cells.
      const placed = fitTableColumns(specs, inner);
      const gap = (i: number) => (i < placed.length - 1 ? " " : "");
      const text = (cells: string[]) => placed.map((p) => cells[specs.indexOf(p.column)] ?? "");
      const line = (cells: string[], styles: (Style | undefined)[]): Cell[] =>
        placed.map((p, i) => ({ text: pad(fit(text(cells)[i]!, p.width - 1), p.width - 1) + gap(i), style: styles[specs.indexOf(p.column)] }));
      const muted = { ...slotStyle("text.muted"), bold: true };
      rows.push({ cells: line(view.columns.map((c) => c.toUpperCase()), [muted, muted, muted, muted]) });
      const sel = view.selected ?? 0;
      const avail = Math.max(0, visible - rows.length + 1);
      let first = Math.max(0, Math.min(view.list_scroll, Math.max(0, view.rows.length - avail)));
      if (sel < first) first = sel;
      if (avail > 0 && sel >= first + avail) first = sel - avail + 1;
      bar = { max: Math.max(0, view.rows.length - avail), at: first };
      const y0 = top + lines.length + 1 + rows.length;
      view.rows.slice(first, first + avail).forEach((r, k) => {
        const outcome = r.outcome_ink && r.outcome_ink !== "plain" ? inkStyle(r.outcome_ink) : undefined;
        rows.push({
          cells: line([r.observed, r.action, r.subject, r.outcome], [undefined, undefined, undefined, outcome]),
          fill: first + k === sel ? selectedStyle(focused) : undefined,
        });
        hits.push({ x: 1, y: y0 + k, w: Math.max(0, w - 2), h: 1, target: r.target });
      });
    }
    paneMark("events", view.events_title, 0, w, h, focused, bar.max > 0);
    const framed = panel(view.events_title, w, h, rows, focused);
    // table.rs: scroll::scrollbar on the panel's right border, between the corners, when rows run past the list.
    barOn(framed, h, bar);
    return framed;
  };
  const detail = (x0: number, w: number, h: number, focused: boolean): Cell[][] => {
    const inner = Math.max(0, w - 4);
    const rows: PRow[] = [];
    for (const t of view.detail) for (const l of wrapText(t, inner, plain)) rows.push({ cells: l });
    const scroll = Math.min(Math.max(0, view.detail_scroll), Math.max(0, rows.length - Math.max(0, h - 2)));
    const allowed = new Set((view.detail_keys ?? []).map((k) => k.target));
    rows.slice(scroll, scroll + Math.max(0, h - 2)).forEach((row, y) => {
      let x = 0;
      for (const cell of clipSegments(row.cells, inner)) {
        const w = cellWidth(cell.text);
        if (cell.target && allowed.has(cell.target) && w > 0) {
          const targetY = top + lines.length + 1 + y;
          const last = hits.at(-1);
          if (last?.target === cell.target && last.y === targetY && last.x + last.w === x0 + 2 + x) last.w += w;
          else hits.push({ x: x0 + 2 + x, y: targetY, w, h: 1, target: cell.target });
        }
        x += w;
      }
    });
    paneMark("detail", view.detail_title, x0, w, h, focused, rows.length > Math.max(0, h - 2));
    return scrolledPanel(view.detail_title, w, h, rows, scroll, focused);
  };
  const help = (w: number, h: number): Cell[][] => {
    const inner = Math.max(0, w - 4);
    const rows: PRow[] = [];
    for (const t of view.help_lines) for (const l of wrapText(t, inner, plain)) rows.push({ cells: l });
    return scrolledPanel(view.help_title, w, h, rows, Math.max(0, view.help_scroll), true);
  };

  let panels: Cell[][];
  // Where text is selected with the mouse (history.rs panes, shell.rs): each panel's inside, the events as rows.
  const y0 = top + shownHead.length;
  const selects: SelectArea[] = [];
  const mark = (x: number, w: number, id: string, kind: SelectArea["kind"]) => {
    if (w >= 3 && bodyH >= 3) selects.push({ ...insideOf({ x, y: y0, w, h: bodyH }), id, kind });
  };
  if (view.help) {
    panels = help(width, bodyH);
    mark(0, width, "history:help", "text");
  } else if (view.split) {
    const lw = Math.floor((width * 3) / 5);
    const a = events(lw, bodyH, view.focus === "events");
    const b = detail(lw, width - lw, bodyH, view.focus === "detail");
    panels = a.map((l, i) => [...l, ...(b[i] ?? [])]);
    mark(0, lw, "history:events", "rows");
    mark(lw, width - lw, "history:detail", "text");
  } else if (view.focus === "events") {
    panels = events(width, bodyH, true);
    mark(0, width, "history:events", "rows");
  } else {
    panels = detail(0, width, bodyH, true);
    mark(0, width, "history:detail", "text");
  }
  lines.push(...panels.map(full));

  // Measure the same visible hint form as the status line, including host mnemonics.
  const foot: Cell[] = [{ text: " " }];
  let x = 1;
  for (const [i, h] of historyFooter(view.footer, width).entries()) {
    const cells = hintCells(h);
    const hw = cells.reduce((n, c) => n + cellWidth(c.text), 0);
    const sep = i > 0 ? 3 : 0;
    if (x + sep + hw > width) break;
    if (sep) { foot.push({ text: " · " }); x += sep; }
    const start = x;
    foot.push(...cells);
    x += hw;
    for (const hit of keyHits(h, start, hw, top + height - 1)) {
      const x = Math.max(0, hit.x);
      const w = Math.min(width, hit.x + hit.w) - x;
      if (w > 0) hits.push({ ...hit, x, w });
    }
  }
  while (lines.length < height - 1) lines.push(full([]));
  lines.length = Math.min(lines.length, Math.max(0, height - 1));
  lines.push(full(foot));
  return { lines, hits, selects, panes };
}

