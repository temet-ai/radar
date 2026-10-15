import { cellWidth, fit } from "./width.js";
import { segWords } from "../text/TextLine.js";
import type { Cell } from "./layout.js";
import type { Hit, Text } from "./types.js";
import { bgOf, iconOf, slotStyle, specStyle, type Style } from "../text/look.js";
import { inkStyle } from "../text/theme.js";
import { segRunStyle } from "../text/TextLine.js";
import { selectedStyle } from "../nodes/tree.js";
import { fitTableColumns } from "../nodes/columns.js";
import { thumbOf } from "../nodes/index.js";
import type { ColumnSpec } from "../nodes/view.js";
import type { HistoryView } from "../protocol/generated/HistoryView.js";

/** The history overlay laid out in cells (history.rs `History::paint`): every line is exactly `width` cells. */
export type HistoryLayout = { lines: Cell[][]; hits: Hit[] };

const textOf = (t: Text): string => t.map(segWords).join("");

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

/** A view text wrapped to `width`, each run in its own look (view_tui::line): the lines' cells, unpadded. */
function wrapText(t: Text, width: number): Cell[][] {
  const units: Style[] = [];
  let plain = "";
  for (const seg of t) {
    const words = segWords(seg);
    const style = segRunStyle(seg);
    for (let k = 0; k < words.length; k++) units.push(style);
    plain += words;
  }
  let at = 0;
  return wrap(plain, width).map((l) => {
    const start = Math.max(at, plain.indexOf(l, at));
    at = start + l.length;
    const cells: Cell[] = [];
    for (let k = 0; k < l.length; ) {
      const style = units[start + k]!;
      let n = 1;
      while (k + n < l.length && units[start + k + n] === style) n++;
      cells.push({ text: l.slice(k, k + n), style });
      k += n;
    }
    return cells;
  });
}

type PRow = { cells: Cell[]; /** The row's own shade across the panel's width (the selection). */ fill?: Style };

/** widgets::panel: a rounded border (accent.primary when focused, else accent.muted), the title in bold emphasis
 * when focused else secondary, the inside on bg.surface; `rows` fill the inside (w - 4 cells, 1 cell padding). */
function panel(title: string, w: number, h: number, rows: PRow[], focused: boolean): Cell[][] {
  const surface = bgOf("bg.surface");
  if (w < 4 || h < 2) return Array.from({ length: Math.max(0, h) }, () => [{ text: " ".repeat(Math.max(0, w)) }]);
  const border = on(slotStyle(focused ? "accent.primary" : "accent.muted"), surface);
  const titleStyle = on(focused ? { ...slotStyle("text.emphasis"), bold: true } : slotStyle("text.secondary"), surface);
  const label = ` ${fit(title, Math.max(0, w - 6))} `;
  const out: Cell[][] = [
    [
      { text: "╭", style: border },
      { text: label, style: titleStyle },
      { text: `${"─".repeat(Math.max(0, w - 2 - cellWidth(label)))}╮`, style: border },
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
      { text: "│", style: border },
      { text: " ", style: shade },
      ...kept,
      { text: " ".repeat(Math.max(0, inner - used)), style: shade },
      { text: " ", style: shade },
      { text: "│", style: border },
    ]);
  }
  out.push([{ text: `╰${"─".repeat(w - 2)}╯`, style: border }]);
  return out;
}

/** The labelled key as a word of `words` (history.rs key_at): where it starts, if it does. */
const keyAt = (words: string, label: string): number => {
  for (let at = words.indexOf(label); at >= 0; at = words.indexOf(label, at + 1)) {
    const before = at === 0 ? " " : words[at - 1]!;
    const after = words[at + label.length] ?? " ";
    if (!/[A-Za-z0-9]/.test(before) && !/[A-Za-z0-9]/.test(after)) return at;
  }
  return -1;
};

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

export function layoutHistory(view: HistoryView, width: number, height: number, top: number): HistoryLayout {
  const lines: Cell[][] = [];
  const hits: Hit[] = [];
  const base = on(slotStyle("text.primary"), bgOf("bg.base"));
  const full = (cells: Cell[]): Cell[] => {
    const used = cells.reduce((n, c) => n + cellWidth(c.text), 0);
    return [...cells, { text: " ".repeat(Math.max(0, width - used)), style: base }].map((c) => ({ ...c, style: on({ ...base, ...c.style }, bgOf("bg.base")) }));
  };
  const head: Cell[][] = [full([{ text: fit(` ${view.title} · ${view.boundary}`, width), style: { ...slotStyle("text.emphasis"), bold: true } }])];
  for (const l of wrap(` Scope: ${view.scope}`, width).slice(0, 2)) head.push(full([{ text: l, style: slotStyle("text.secondary") }]));
  for (const note of view.notes) {
    // history.rs lead: one cell in, the gap in the first run's look.
    const spaced: Text = note.length > 0 ? [{ ...note[0]!, text: ` ${segWords(note[0]!)}`, icon: undefined }, ...note.slice(1)] : [{ text: " " }];
    for (const l of wrapText(spaced, width)) head.push(full(l));
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
    const framed = panel(view.events_title, w, h, rows, focused);
    // table.rs: scroll::scrollbar on the panel's right border, between the corners, when rows run past the list.
    barOn(framed, h, bar);
    return framed;
  };
  const detail = (x0: number, w: number, h: number, focused: boolean): Cell[][] => {
    const inner = Math.max(0, w - 4);
    const rows: PRow[] = [];
    let keysAt: number | undefined;
    view.detail.forEach((t, i) => {
      if (i === view.detail_keys_line) keysAt = rows.length;
      for (const l of wrapText(t, inner)) rows.push({ cells: l });
    });
    const scroll = Math.min(Math.max(0, view.detail_scroll), Math.max(0, rows.length - Math.max(0, h - 2)));
    const n = view.detail_keys_line;
    if (n != null && n < view.detail.length && keysAt !== undefined) {
      const words = textOf(view.detail[n]!);
      const y = keysAt - scroll;
      if (cellWidth(words) <= inner && y >= 0 && y < h - 2) {
        for (const k of view.detail_keys ?? []) {
          const at = keyAt(words, k.label);
          if (at < 0) continue;
          hits.push({ x: x0 + 2 + cellWidth(words.slice(0, at)), y: top + lines.length + 1 + y, w: cellWidth(k.label), h: 1, target: k.target });
        }
      }
    }
    return scrolledPanel(view.detail_title, w, h, rows, scroll, focused);
  };
  const help = (w: number, h: number): Cell[][] => {
    const inner = Math.max(0, w - 4);
    const rows: PRow[] = [];
    for (const t of view.help_lines) for (const l of wrapText(t, inner)) rows.push({ cells: l });
    return scrolledPanel(view.help_title, w, h, rows, Math.max(0, view.help_scroll), true);
  };

  let panels: Cell[][];
  if (view.help) panels = help(width, bodyH);
  else if (view.split) {
    const lw = Math.floor((width * 3) / 5);
    const a = events(lw, bodyH, view.focus === "events");
    const b = detail(lw, width - lw, bodyH, view.focus === "detail");
    panels = a.map((l, i) => [...l, ...(b[i] ?? [])]);
  } else if (view.focus === "events") panels = events(width, bodyH, true);
  else panels = detail(0, width, bodyH, true);
  lines.push(...panels.map(full));

  // The keys along the bottom (widgets::key_hint), each key clickable.
  const keyStyle = { ...slotStyle("accent.secondary"), bold: true };
  const foot: Cell[] = [{ text: " " }];
  let x = 1;
  const put = (text: string, style?: Style) => {
    foot.push({ text, style });
    x += cellWidth(text);
  };
  view.footer.forEach((h, i) => {
    if (i > 0) put(" · ");
    const base0 = x;
    for (const k of h.keys_of ?? []) {
      const at = h.keys.indexOf(k.label);
      if (at >= 0 && base0 + cellWidth(h.keys.slice(0, at)) + cellWidth(k.label) <= width) {
        hits.push({ x: base0 + cellWidth(h.keys.slice(0, at)), y: top + height - 1, w: cellWidth(k.label), h: 1, target: k.target });
      }
    }
    put(h.keys, keyStyle);
    put(` ${h.label}`, slotStyle("text.muted"));
  });
  while (lines.length < height - 1) lines.push(full([]));
  lines.length = Math.min(lines.length, Math.max(0, height - 1));
  lines.push(full(foot));
  return { lines, hits };
}

