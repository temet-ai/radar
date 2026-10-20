import { cellWidth, clipSegments, clusters, fit } from "./width.js";
import { wordmarkRuns } from "../text/frame.js";
import type { ChromeView, Hint, Hit, Ink, MenuView, SettingsNav, Text } from "./types.js";
import { bgOf, iconOf, slotStyle, specStyle, type Style } from "../text/look.js";
import { inkStyle } from "../text/theme.js";
import { ATTENTION_DOT } from "../text/icons.js";
import { paletteLines, paletteWidth, wrapPalette } from "./palette.js";
import { cutCells, fieldWindow } from "../editor/InputField.js";
import { mnemonicPrefix, mnemonicRange, mnemonicRuns } from "../text/mnemonic.js";
import { cellsOf, paragraphCells } from "../text/cells.js";

/** A run of text on one row. Rows are laid out in cells so a pointer position maps to a target. `style` is the
 * theme's look for it (colours as the host's slots resolve them); `ink` names a slot, `style` wins over it. */
export type Cell = { text: string; ink?: Ink; style?: Style; underline?: boolean; inverse?: boolean; bold?: boolean };
export type Laid = { cells: Cell[]; hits: Hit[] };

class Row {
  cells: Cell[] = [];
  hits: Hit[] = [];
  x = 0;
  /** Under every cell: the shade the row sits on. */
  base: Style | undefined;
  constructor(readonly y: number) {}
  put(text: string, rest: Omit<Cell, "text"> = {}, target?: string): number {
    const w = cellWidth(text);
    if (w === 0) return this.x;
    if (target) this.hits.push({ x: this.x, y: this.y, w, h: 1, target });
    const style = this.base || rest.style ? { ...this.base, ...rest.style } : undefined;
    this.cells.push({ text, ...rest, ...(style ? { style } : {}) });
    this.x += w;
    return this.x;
  }
  gap(n: number) {
    if (n > 0) this.put(" ".repeat(n));
  }
  /** The row, padded with its shade to `width` cells when given. */
  done(width?: number): Laid {
    if (width !== undefined && this.x < width) this.gap(width - this.x);
    return { cells: this.cells, hits: this.hits };
  }
}

export const textOf = (t: Text | undefined): string => (t ?? []).map((s) => s.text ?? "").join("");

/** shell.rs ink_style: the ink's slot (the one a plain ink lacks falls to primary text), bold when strong. */
export function chromeInk(ink: Ink | undefined, strong = false): Style {
  const s = inkStyle(ink);
  const base = s.color || s.backgroundColor ? s : slotStyle("text.primary");
  return strong ? { ...base, bold: true } : base;
}

/** widgets::tone: the style of a tone, which a count is drawn in (the one place tones become
 * slots): accent, warn and bad are bold; an ink that is no tone is plain. */
export function toneStyle(ink: Ink | undefined): Style {
  switch (ink) {
    case "muted":
      return slotStyle("text.muted");
    case "accent":
      return { ...slotStyle("accent.primary"), bold: true };
    case "info":
      return slotStyle("status.info");
    case "good":
      return slotStyle("status.success");
    case "warn":
      return { ...slotStyle("status.warning"), bold: true };
    case "bad":
      return { ...slotStyle("status.error"), bold: true };
    default:
      return slotStyle("text.primary");
  }
}

/**
 * ` radar   Books ●   Work   History   Settings          ⠷ synced 12s   ◉ read only `, row `y`, drawn as
 * shell.rs top_bar does: the row on the overlay shade, each tab padded one cell either side with its icon,
 * its shortcut letter underlined in place, the active tab on the selection shade (or the accent when the bar
 * has the keys), the mode at the right end in its ink. When it does not fit the icons go first, then the
 * names become their keys, then the part's words are cut, then dropped. The mode stays.
 */
export function layoutTopBar(chrome: ChromeView, width: number, y = 0): Laid {
  const bar = chrome.top_bar;
  const keysOnBar = (chrome.keys_at ?? "body") === "bar";
  const overlay = bgOf("bg.overlay");
  const onBar = (s: Style): Style => ({ ...s, backgroundColor: s.backgroundColor ?? overlay });
  const rowOf = () => {
    const r = new Row(y);
    r.base = { backgroundColor: overlay };
    return r;
  };
  const inkOf = chromeInk;

  const tabs = (icons: boolean, compact: boolean) => {
    const row = rowOf();
    for (const r of wordmarkRuns(bar.brand, { ...slotStyle("accent.primary"), bold: true }, " ", " ")) row.put(r.text, { style: r.style });
    row.put("  ");
    for (const tab of bar.tabs) {
      const on = !!tab.active;
      let base: Style;
      if (on && keysOnBar) base = { ...slotStyle("bg.base"), bold: true, backgroundColor: slotStyle("accent.primary").color };
      else if (on) base = { ...slotStyle("text.emphasis"), bold: true, backgroundColor: bgOf("bg.selection") };
      else base = slotStyle("text.secondary");
      const bg = base.backgroundColor ?? overlay;
      const mark: Style = { ...base, ...slotStyle("accent.secondary"), bold: true, underline: true, backgroundColor: bg };
      const raw = compact && tab.target.startsWith("chrome:tab:") ? tab.mnemonic : tab.label;
      const name = raw.charAt(0).toUpperCase() + raw.slice(1);
      const start = row.x;
      row.put(" ", { style: { ...base, backgroundColor: bg } });
      const ic = icons ? lead(tab.icon) : undefined;
      if (ic) row.put(ic.text, { style: { ...base, ...ic.style, backgroundColor: bg } });
      const at = tab.mnemonic ? name.toLowerCase().indexOf(tab.mnemonic.toLowerCase()) : -1;
      const st: Style = { ...base, backgroundColor: bg };
      if (at >= 0) {
        row.put(name.slice(0, at), { style: st });
        row.put(name.slice(at, at + tab.mnemonic.length), { style: mark });
        row.put(name.slice(at + tab.mnemonic.length), { style: st });
      } else {
        row.put(name, { style: st });
        row.put(` (${tab.mnemonic})`, { style: mark });
      }
      if (tab.attention) {
        row.put(" ", { style: st });
        row.put(ATTENTION_DOT, { style: { ...slotStyle("status.warning"), backgroundColor: bg } });
      }
      row.put(" ", { style: st });
      // A tab is clicked on its padding and on the gap after it, so there are no dead cells between tabs.
      // shell.rs tabs: the rect is [x - lead, x + w + 1) with lead 2 for the first tab (the gap after `radar` is its too).
      const gapLead = row.hits.length === 0 ? Math.min(2, start) : 0;
      row.hits.push({ x: start - gapLead, y, w: row.x - start + 1 + gapLead, h: 1, target: tab.target });
      row.put(" ");
    }
    return row;
  };

  const modeCell = { text: modeText(bar.mode), style: inkOf(bar.mode.ink, !!bar.mode.strong) };
  const modeW = cellWidth(modeCell.text);
  const throbber = bar.busy ? [`${SPIN[Math.floor(Date.now() / 100) % SPIN.length]} `] : [];
  const words = bar.words ? `${bar.words.text} ` : "";
  const terminal = bar.terminal ? `${bar.terminal.text} ` : "";
  type Part = { text: string; style: Style };
  let right: Part[] = [
    ...throbber.map((t) => ({ text: t, style: slotStyle("status.info") })),
    ...(words ? [{ text: words, style: inkOf(bar.words!.ink) }] : []),
    ...(terminal ? [{ text: terminal, style: inkOf(bar.terminal!.ink) }] : []),
  ];
  const widthOf = (p: Part[]) => p.reduce((n, c) => n + cellWidth(c.text), 0);
  let row = tabs(true, false);
  let partW = widthOf(right);
  const over = () => row.x + partW + modeW > width;
  if (over()) row = tabs(false, false);
  if (over()) row = tabs(false, true);
  if (over()) {
    const available = width - row.x - modeW;
    if (available > 0) {
      const text = right.map((p) => p.text).join("");
      right = [{ text: fit(text, available), style: right[0]?.style ?? {} }];
      partW = widthOf(right);
    }
  }
  if (over()) {
    right = terminal ? [{ text: terminal, style: inkOf(bar.terminal!.ink) }] : [];
    partW = widthOf(right);
  }
  if (over()) row = tabs(false, false);
  if (over()) row = tabs(false, true);
  if (over()) {
    right = [];
    partW = 0;
  }
  const rightW = partW + modeW;
  // The tabs are painted first, then overlays the right-hand words and mode when that side fits.
  // Reserve those cells before appending anything so compact tabs cannot push the mode off screen.
  const showRight = rightW < width;
  const leftWidth = Math.max(0, width - (showRight ? rightW : 0));
  row.cells = clipSegments(row.cells, leftWidth);
  row.x = row.cells.reduce((n, c) => n + cellWidth(c.text), 0);
  row.hits = row.hits
    .map((hit) => ({ ...hit, w: Math.max(0, Math.min(leftWidth, hit.x + hit.w) - hit.x) }))
    .filter((hit) => hit.w > 0);
  row.gap(Math.max(0, leftWidth - row.x));
  if (showRight) {
    for (const p of right) row.put(p.text, { style: p.style });
    const modeX = Math.max(0, row.x - 1); // shell.rs padded mode target includes its leading neighbour
    row.put(modeCell.text, { style: modeCell.style });
    row.hits.push({ x: modeX, y, w: row.x - modeX, h: 1, target: bar.mode.target });
  }
  return row.done(width);
}

/**
 * ` radar  setup / look & terminal          step 1 of 5   read-only `, row `y`, drawn as shell/setup.rs top_bar does:
 * the row on the overlay shade, the brand, `setup / ` and the screen's name; at the right end `step n of m` and the mode in
 * its ink. No tabs, no words, no terminal chip, no throbber while setup shows. The left is cut to what the right leaves.
 */
export function layoutSetupTopBar(chrome: ChromeView, setup: NonNullable<ChromeView["setup"]>, width: number, y = 0): Laid {
  const bar = chrome.top_bar;
  const row = new Row(y);
  row.base = { backgroundColor: bgOf("bg.overlay") };
  const mode = modeText(bar.mode);
  const step = setup.step ? `${setup.step}  ` : "";
  const rightW = Math.min(cellWidth(step) + cellWidth(mode), width);
  const left: Array<[string, Style]> = [
    ...wordmarkRuns(bar.brand, { ...slotStyle("accent.primary"), bold: true }, " ").map(({ text, style }): [string, Style] => [text, style]),
    ["  setup / ", slotStyle("text.muted")],
    [setup.title, slotStyle("text.emphasis")],
  ];
  let room = Math.max(0, width - rightW);
  for (const [text, style] of left) {
    const shown = clip(text, room);
    room -= cellWidth(shown);
    row.put(shown, { style });
  }
  row.gap(Math.max(0, width - rightW - row.x));
  if (step) row.put(step, { style: slotStyle("text.muted") });
  row.put(mode, { style: chromeInk(bar.mode.ink, !!bar.mode.strong) }, bar.mode.target);
  return row.done(width);
}

/** The kit's spinner frames (ui::states::spinner, throbber-widgets-tui's BRAILLE_SIX). */
export const SPIN = ["⠷", "⠯", "⠟", "⠻", "⠽", "⠾"];

/** The mode chip's cell: its icon (by name, drawn in the pack) and its word, one space either side. */
function modeText(mode: { text: string; icon?: string | null }): string {
  return ` ${lead(mode.icon)?.text ?? ""}${mode.text} `;
}

/** An icon by name as its glyph and a space, in the pack's colour for it; none when the pack has none. */
function lead(icon: string | null | undefined): { text: string; style: Style } | undefined {
  const i = iconOf(icon);
  if (!i || !i.glyph) return undefined;
  return { text: `${i.glyph} `, style: specStyle(i.style) };
}


export function layoutSources(chrome: ChromeView, width: number, y: number): Laid | null {
  const sources = chrome.sources ?? [];
  if (sources.length === 0) return null;
  // shell.rs source_tabs: the row on the overlay shade, each source `  label  `; the active one on the selection
  // shade in bold emphasis, or the accent when the sources have the keys; the others in secondary text.
  const row = new Row(y);
  row.base = { backgroundColor: bgOf("bg.overlay") };
  const keys = (chrome.keys_at ?? "body") === "sources";
  for (const s of sources) {
    if (row.x >= width) break;
    const style: Style = s.active
      ? keys
        ? { ...slotStyle("bg.base"), bold: true, backgroundColor: slotStyle("accent.primary").color }
        : { ...slotStyle("text.emphasis"), bold: true, backgroundColor: bgOf("bg.selection") }
      : slotStyle("text.secondary");
    const label = `  ${s.label}  `;
    const room = width - row.x;
    row.put(cellWidth(label) > room ? fit(label, room) : label, { style }, s.target);
  }
  return row.done(width);
}

/** Buffer::set_line: the words cut at `max` cells, no ellipsis (shell.rs status_line draws the message so). */
function clip(s: string, max: number): string {
  let out = "";
  let w = 0;
  for (const [ch, cw] of clusters(s)) {
    if (w + cw > max) break;
    out += ch;
    w += cw;
  }
  return out;
}

type RightPart = { text: string; style: Style; target?: string };

/** shell.rs status_line: the right of the status line, the counts ` · ` apart, two cells, then the services' chips
 * a space apart (each its glyph, its name where the pack has none, and the count of failing sources, in its
 * state's ink), then a cell; and which show at `width`: both under half the line, else the chips alone. */
export function statusRight(chrome: ChromeView, width: number) {
  const st = chrome.status;
  const counts: RightPart[] = [];
  (st.counts ?? []).forEach((c, i) => {
    if (i > 0) counts.push({ text: " · ", style: slotStyle("text.muted") });
    counts.push({ text: c.text, style: toneStyle(c.ink) });
  });
  const health: RightPart[] = [];
  (st.health ?? []).forEach((c, i) => {
    if (i > 0) health.push({ text: " ", style: {} });
    const head = c.glyph ? c.glyph : c.label;
    health.push({ text: c.count ? `${head} ${c.count}` : head, style: chromeInk(c.ink), target: c.target });
  });
  const w = (ps: RightPart[]) => ps.reduce((n, p) => n + cellWidth(p.text), 0);
  const countsW = w(counts);
  const healthW = w(health);
  const sepW = counts.length > 0 && healthW > 0 ? 2 : 0;
  const widthOf = (c: boolean, h: boolean) => (c ? countsW : 0) + (h ? healthW : 0) + (c && h ? sepW : 0) + 1;
  const half = Math.floor(width / 2);
  let showCounts = true;
  let showHealth = healthW > 0;
  let rightW: number;
  if (widthOf(true, showHealth) < half) rightW = widthOf(true, showHealth);
  else if (showHealth && widthOf(false, true) < half) {
    showCounts = false;
    rightW = widthOf(false, true);
  } else {
    showHealth = false;
    rightW = 0;
  }
  return { counts, health, sepW, widthOf, showCounts, showHealth, rightW };
}

/** The visible hint form, shared with History so fit, paint and hits use identical words. */
export function hintCells(h: Hint): Cell[] {
  const key = mnemonicPrefix(h.label, h.keys, h);
  const icon = lead(h.icon);
  const keyStyle = { ...slotStyle("accent.secondary"), bold: true };
  const words = slotStyle("text.muted");
  if (key && !icon && !mnemonicRange(h.label, h.keys, h.mnemonic_at)) return [{ text: key, style: keyStyle }, { text: ` ${h.label}`, style: words }];
  const valid = !!mnemonicRange(h.label, h.keys, h.mnemonic_at);
  const prefix = key ? [{ text: key, style: keyStyle }, { text: " ", style: words }] : [];
  return [
    ...(!valid ? prefix : []),
    ...(icon ? [{ text: icon.text, style: icon.style }] : []),
    ...(valid ? prefix : []),
    ...mnemonicRuns(h.label, h.keys, h, words),
  ];
}

/** Paragraph wrapping with styles retained through whole graphemes. */
function wrapStyled(cells: Cell[], width: number): Cell[][] {
  const chars = cells.flatMap((c) => cellsOf(c.text, c.style));
  return paragraphCells(chars, Math.max(1, width)).map((line) => line.map((c) => ({ text: c.ch, style: c.s })));
}

/** `BOOKS   PgUp/PgDn scroll   q quit   x menu            11 open · 3 need you  ▼ ◉`, row `y` (shell.rs status_line). */
export function layoutStatus(chrome: ChromeView, width: number, y: number, plain = false): Laid {
  const st = plain ? { ...chrome.status, hints: chrome.status.hints?.map((h) => ({ ...h, mnemonic_at: undefined, mnemonic_shift: false })) } : chrome.status;
  const overlay = bgOf("bg.overlay");
  const row = new Row(y);
  row.base = { backgroundColor: overlay };
  const chip: Style = { ...slotStyle("bg.base"), bold: true, backgroundColor: slotStyle("accent.primary").color };
  row.put(` ${st.scope} `, { style: chip });
  row.gap(2);
  // shell.rs status_line: an agent session that wants you, out of sight: the key that goes there, then a square chip on
  // its state's ground (bold base text, as the scope's); it never gives way, and its key and the chip are one target.
  const want = st.attention;
  if (want) {
    const x0 = row.x;
    row.put(want.keys, { style: { ...slotStyle("accent.secondary"), bold: true } });
    row.put(" ");
    const g = iconOf(want.icon)?.glyph;
    row.put(` ${g ? `${g} ` : ""}${want.text} `, { style: { ...slotStyle("bg.base"), bold: true, backgroundColor: chromeInk(want.ink).color } });
    row.hits.push({ x: x0, y, w: Math.max(0, Math.min(row.x, width) - x0), h: 1, target: want.target });
    row.gap(2);
  }
  const right = statusRight(chrome, width);
  let { showCounts, showHealth, rightW } = right;
  if (st.message) {
    const lead0 = lead(st.message.icon);
    if (lead0) row.put(lead0.text, { style: lead0.style });
    const room = Math.max(0, width - (rightW + 1) - row.x);
    row.put(clip(st.message.text, room), { style: chromeInk(st.message.ink) });
  } else {
    let hints = [...(st.hints ?? [])];
    const hintW = (h: (typeof hints)[number]) => hintCells(h).reduce((n, c) => n + cellWidth(c.text), 3);
    const widthOf = (hs: typeof hints) => hs.reduce((n, h) => n + hintW(h), 0);
    const fits = (hs: typeof hints, rw: number) => row.x + widthOf(hs) < width - rw;
    if (!fits(hints, rightW)) {
      // The counts give way first, then the keys needed least, in the host's order, then the services' chips.
      showCounts = false;
      rightW = showHealth ? right.widthOf(false, true) : 0;
      for (const drop of st.least_needed ?? []) {
        if (fits(hints, rightW)) break;
        hints = hints.filter((h) => h.label !== drop);
      }
      if (!fits(hints, rightW)) {
        showHealth = false;
        rightW = 0;
      }
    }
    // The host's complete action list remains reachable even when expanded words
    // exhaust the row. Identify its actual input, never the caption's spelling.
    const mandatory = (h: Hint) => h.keys_of?.some(({ chord }) =>
      !chord.ctrl && !chord.alt && typeof chord.key === "object" && chord.key.char === "x",
    ) ?? false;
    if (hints.some(mandatory)) {
      while (!fits(hints, rightW)) {
        let last = hints.length - 1;
        while (last >= 0 && mandatory(hints[last]!)) last--;
        if (last < 0) break;
        hints.splice(last, 1);
      }
    }
    const room = Math.max(0, width - (rightW + 1));
    for (const h of hints) {
      const cells = hintCells(h);
      const w = cells.reduce((n, c) => n + cellWidth(c.text), 3);
      if (row.x + w > room) break;
      const base = row.x;
      for (const c of cells) row.put(c.text, { style: c.style });
      row.hits.push(...keyHits(h, base, row.x - base, y));
      row.put("   ");
    }
  }
  if (rightW > 0) {
    row.gap(Math.max(0, width - rightW - row.x));
    if (showCounts) {
      for (const p of right.counts) row.put(p.text, { style: p.style });
      if (showHealth && right.sepW > 0) row.put("  ");
    }
    if (showHealth)
      for (const p of right.health) {
        // A chip is clicked on its cells and the one after it.
        if (p.target) row.hits.push({ x: row.x, y, w: Math.min(cellWidth(p.text) + 1, width - row.x), h: 1, target: p.target });
        row.put(p.text, { style: p.style });
      }
    row.put(" ");
  }
  row.cells = clipSegments(row.cells, width);
  row.x = row.cells.reduce((n, c) => n + cellWidth(c.text), 0);
  row.hits = row.hits.map((h) => ({ ...h, w: Math.max(0, Math.min(width, h.x + h.w) - h.x) })).filter((h) => h.w > 0);
  return row.done(width);
}

/** Where a click presses each key of a hint (shell.rs key_hits): one key, the whole hint and a cell before it;
 * several (`PgUp/PgDn`, `+/-`, `←→↑↓`), each label, and each word when there is a word per key. */
export function keyHits(h: Hint, base: number, hw: number, y: number): Hit[] {
  const ks = h.keys_of ?? [];
  if (ks.length === 0) return [];
  const wordsEnd = base + hw;
  const box = (from: number, to: number, target: string): Hit => {
    const x = Math.max(0, from);
    return { x, y, w: Math.max(0, to - x), h: 1, target };
  };
  if (ks.length === 1) return [box(base - 1, wordsEnd + 1, ks[0]!.target)];
  const gap = h.keys.includes("/") ? 1 : 0;
  const labels: Array<{ at: number; text: string }> = [];
  let at = 0;
  for (const k of ks) {
    labels.push({ at, text: k.label });
    at += cellWidth(k.label) + gap;
  }
  const out: Hit[] = [];
  labels.forEach((l, i) => {
    const from = base + l.at - (i === 0 ? 1 : 0);
    const to = labels[i + 1] ? base + labels[i + 1]!.at : base + l.at + cellWidth(l.text) + 1;
    out.push(box(from, to, ks[i]!.target));
  });
  const words = h.label.split("/");
  if (words.length === ks.length) {
    let wat = wordsEnd - cellWidth(h.label);
    words.forEach((w, i) => {
      const ww = cellWidth(w);
      out.push(box(wat, wat + ww + (i === ks.length - 1 ? 1 : 0), ks[i]!.target));
      wat += ww + 1;
    });
  }
  return out;
}

export type MenuLayout = { x: number; y: number; w: number; h: number; lines: Cell[][]; hits: Hit[] };

const shownOnly = (c: MenuView["items"][number]) => !!(c.heading || c.note || c.shown_only);
const keyOf = (c: MenuView["items"][number]) => c.keys ?? c.key ?? "";
const separateKey = (c: MenuView["items"][number]) => mnemonicRange(c.label, keyOf(c), c.mnemonic_at) ? "" : keyOf(c);
const shiftWords = (c: MenuView["items"][number]) => mnemonicRange(c.label, keyOf(c), c.mnemonic_at) && c.mnemonic_shift ? "shift " : "";
const choiceRuns = (c: MenuView["items"][number], style: Style): Cell[] => [
  ...(shiftWords(c) ? [{ text: "shift ", style: { ...slotStyle("accent.secondary"), bold: true, ...(style.backgroundColor ? { backgroundColor: style.backgroundColor } : {}) } }] : []),
  ...mnemonicRuns(c.label, keyOf(c), c, style),
];

const mix = (a: [number, number, number], b: [number, number, number], amount: number): string =>
  "#" + a.map((x, i) => Math.round(b[i]! + (x - b[i]!) * amount).toString(16).padStart(2, "0")).join("");
const rgbOf = (hex: string | undefined): [number, number, number] | undefined => {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex ?? "");
  return m ? [parseInt(m[1]!, 16), parseInt(m[2]!, 16), parseInt(m[3]!, 16)] : undefined;
};

const padTo = (s: string, w: number) => s + " ".repeat(Math.max(0, w - cellWidth(s)));

/**
 * A menu or question as a rounded popup in the body (shell/menu.rs): the title in the top border, then the
 * key column, the label and what it does, one row each, the selected row on the selection shade; a
 * question's choices as raised buttons. Rows are full width, so the list under it never shows through.
 */
export function layoutMenu(menu: MenuView, width: number, bodyTop: number, bodyHeight: number, plain = false): MenuLayout {
  if (plain) menu = { ...menu, items: menu.items.map((i) => ({ ...i, mnemonic_at: undefined, mnemonic_shift: false })) };
  const measure = menu.palette ? paletteWidth : cellWidth;
  const title = menu.palette ? `${menu.title} · ${menu.palette.matched} of ${menu.palette.total}` : menu.title;
  const overlay = bgOf("bg.overlay");
  const border: Style = { ...slotStyle("accent.primary"), backgroundColor: overlay };
  const maxW = Math.max(0, width - 2);
  const maxH = Math.max(0, bodyHeight - 2);
  if (width < 2 || bodyHeight < 2) return { x: 0, y: bodyTop, w: Math.max(0, width), h: Math.max(0, bodyHeight), lines: Array.from({ length: Math.max(0, bodyHeight) }, () => [{ text: " ".repeat(Math.max(0, width)) }]), hits: [] };
  const items = menu.items;
  const sel = menu.selected ?? 0;
  const titleIcon = menu.question ? "warning" : menu.id === "bulk" ? "bulk" : menu.id === "keys" ? "menu" : menu.id === "quit" ? "quit" : undefined;
  const titleLeads = [titleIcon, menu.icon].map((i) => lead(i)).filter((l): l is NonNullable<typeof l> => !!l);
  // menu.rs: the title is ` `, the icons, then the words and a space, in bold emphasis (the icons in their own
  // style); a title that does not fit the width, or has several lines, is the first lines of the body instead.
  const emphasis: Style = { ...slotStyle("text.emphasis"), bold: true, backgroundColor: overlay };
  const titleCells: Cell[] = [
    { text: " ", style: emphasis },
    ...titleLeads.map((l) => ({ text: l.text, style: { ...l.style, backgroundColor: overlay } })),
    { text: `${title} `, style: emphasis },
  ];
  const titleText = titleCells.map((c) => c.text).join("");
  const longTitle = measure(titleText) > maxW || title.includes("\n");
  const keyW = Math.max(0, ...items.map((i) => measure(separateKey(i))));
  const iconW = (c: (typeof items)[number]) => (lead(c.icon) ? measure(lead(c.icon)!.text) : 0);
  const labelW = Math.max(0, ...items.filter((i) => !i.heading).map((i) => measure(i.label) + measure(shiftWords(i)) + iconW(i)));
  const longest = (text: string) => Math.max(0, ...text.split("\n").map(measure));
  const compactW = (c: (typeof items)[number]) => menu.palette
    ? longest(c.label) + measure(shiftWords(c)) + iconW(c) + (c.sample ? 3 : 0) + (c.hint ? 3 + longest(c.hint) : 0) + (keyW === 0 ? 2 : keyW + 4)
    : (keyW === 0 ? 1 : keyW + 2) + (c.sample ? 3 : 0) + 1 + labelW + 4 + measure(c.hint);
  const question = menu.question;
  const buttons = question ? items.map((c, i) => ({ c, i })).filter(({ c }) => !c.heading && !c.note) : [];
  const buttonWords = (c: (typeof items)[number]) => c.hint || c.label;
  const buttonText = (c: (typeof items)[number]) => {
    const prefix = mnemonicPrefix(buttonWords(c), keyOf(c), c);
    return ` ${prefix ? `${prefix} ` : ""}${lead(c.icon)?.text ?? ""}${buttonWords(c)} `;
  };
  let buttonsW = 0;
  let currentButtonsW = 0;
  for (const { c } of buttons) {
    const bw = Math.min(measure(buttonText(c)), maxW);
    if (currentButtonsW > 0 && currentButtonsW + 2 + bw > maxW) {
      buttonsW = Math.max(buttonsW, currentButtonsW);
      currentButtonsW = 0;
    }
    currentButtonsW += (currentButtonsW ? 2 : 0) + bw;
  }
  buttonsW = Math.max(buttonsW, currentButtonsW);
  const natural = Math.max(
    1,
    measure(titleText),
    menu.typed != null ? 58 : 0,
    ...(question ? [] : items.filter((c) => !c.heading).map(compactW)),
    ...items.filter((c) => c.heading).map((c) => 1 + measure(c.label) + iconW(c)),
    ...items.filter((c) => c.note).map((c) => 1 + Math.min(measure(c.label), 60) + iconW(c)),
  );
  const innerW = Math.min(maxW, Math.max(natural, buttonsW));
  const w = Math.min(width, innerW + 2);
  const countText = menu.palette ? ` ${menu.palette.matched} of ${menu.palette.total} ` : "";
  const narrowCount = menu.palette && longTitle && measure(countText) <= innerW ? [{ text: countText, style: emphasis }] : undefined;

  const columnsFit = items.every((c) => c.heading || (compactW(c) <= innerW && !c.label.includes("\n") && !c.hint.includes("\n")));
  type Line = { cells: Cell[]; target?: string; sel?: boolean };
  const body: Line[] = [];
  const fillTo = (cells: Cell[], bg?: string): Cell[] => {
    const used = cells.reduce((n, c) => n + measure(c.text), 0);
    return [...cells, { text: " ".repeat(Math.max(0, innerW - used)), style: bg ? { backgroundColor: bg } : {} }];
  };
  if (menu.refused) body.push({ cells: [{ text: " " + menu.refused, ink: "warn", style: { backgroundColor: overlay } }] });
  if (longTitle) {
    // Content::push wraps the icon and first title line together; continuations regain its full room.
    const inset = Math.min(1, Math.max(0, innerW - 2));
    const titleLines = (narrowCount ? menu.title : title).split("\n").map((raw, n) => [
      ...(n === 0 ? titleLeads.map((l) => ({ text: l.text, style: l.style })) : []),
      { text: raw.replace(/\r$/, ""), style: emphasis },
    ]);
    const wrapped = menu.palette ? wrapPalette(titleLines, innerW - 1) : titleLines.flatMap((l) => wrapStyled(l, innerW - inset));
    for (const line of wrapped) body.push({ cells: [{ text: " ".repeat(inset) }, ...line] });
  }

  items.forEach((c, i) => {
    if (c.heading || c.note || c.shown_only) {
      // The host marks a note as a heading too (MenuItem::note builds on heading); a note is drawn as written.
      const isHeading = c.heading && !c.note;
      const text = isHeading ? c.label.toUpperCase() : c.label;
      const style: Style = isHeading ? { ...slotStyle("text.muted"), bold: true } : slotStyle("text.primary");
      // menu.rs: the item's icon leads its first line, after the cell of air.
      const ic = lead(c.icon);
      const lines = text.split("\n").map((part, k) => [...(k === 0 && ic ? [{ text: ic.text, style: ic.style }] : []), { text: menu.palette ? part.replace(/\r$/, "") : part, style }]);
      for (const line of menu.palette ? wrapPalette(lines, innerW - 1) : lines.flatMap((l) => wrapStyled(l, innerW - Math.min(1, Math.max(0, innerW - 2))))) body.push({ cells: [{ text: " ".repeat(Math.min(1, Math.max(0, innerW - 2))) }, ...line] });
      return;
    }
    if (question) return;
    const on = i === sel;
    if (menu.palette) {
      for (const cells of paletteLines(c, on, menu.palette.highlights[i] ?? [], innerW, keyW, menu.palette.hint_highlights?.[i] ?? [])) {
        body.push({ cells, target: c.target ?? undefined, sel: on });
      }
      return;
    }
    const rowBg = on ? bgOf("bg.selection") : undefined;
    const row: Style = on ? { ...slotStyle("text.emphasis"), bold: true, backgroundColor: rowBg } : slotStyle(c.muted ? "text.muted" : "text.primary");
    const keyStyle: Style = { ...slotStyle("accent.secondary"), bold: true, ...(rowBg ? { backgroundColor: rowBg } : {}) };
    if (!columnsFit) {
      const prefixW = keyW === 0 ? 2 : keyW + 3;
      const inset = prefixW + 2 <= innerW ? prefixW : 0;
      const prefix = keyW === 0 ? "  " : ` ${padTo(separateKey(c), keyW)}  `;
      if (inset === 0 && separateKey(c)) for (const line of wrapStyled([{ text: separateKey(c), style: keyStyle }], innerW)) body.push({ cells: line, target: c.target ?? undefined, sel: on });
      const labels = c.label.split("\n");
      let offset = 0;
      labels.forEach((raw, n) => {
        const chars = [...raw].length;
        const meta = { ...c, mnemonic_at: c.mnemonic_at == null ? undefined : c.mnemonic_at - offset };
        const ic = n === 0 ? lead(c.icon) : undefined;
        const runs: Cell[] = [
          ...(n === 0 && c.sample ? [{ text: "██ ", style: specStyle(c.sample) }] : []),
          ...(ic ? [{ text: ic.text, style: { ...ic.style, backgroundColor: rowBg } }] : []),
          ...(n === 0 && shiftWords(c) ? [{ text: shiftWords(c), style: keyStyle }] : []),
          ...mnemonicRuns(raw.replace(/\r$/, ""), keyOf(c), meta, row),
        ];
        wrapStyled(runs, innerW - inset).forEach((line, j) => body.push({ cells: fillTo([{ text: n === 0 && j === 0 ? prefix.slice(0, inset) : " ".repeat(inset), style: keyStyle }, ...line], rowBg), target: c.target ?? undefined, sel: on }));
        offset += chars + 1;
      });
      if (c.hint) for (const raw of c.hint.split("\n")) for (const line of wrapStyled([{ text: raw, style: { ...slotStyle("text.muted"), backgroundColor: rowBg } }], innerW - inset)) body.push({ cells: fillTo([{ text: " ".repeat(inset), style: row }, ...line], rowBg), target: c.target ?? undefined, sel: on });
      return;
    }
    const cells: Cell[] = [{ text: keyW === 0 ? " " : ` ${padTo(separateKey(c), keyW)} `, style: keyStyle }];
    if (c.sample) cells.push({ text: " ██", style: { ...specStyle(c.sample), ...(rowBg ? { backgroundColor: rowBg } : {}) } });
    const ic = lead(c.icon);
    if (ic) {
      cells.push({ text: " ", style: row });
      cells.push({ text: ic.text, style: { ...ic.style, ...(rowBg ? { backgroundColor: rowBg } : {}) } });
      cells.push(...choiceRuns(c, row));
      cells.push({ text: " ".repeat(Math.max(0, labelW - measure(ic.text) - measure(c.label) - measure(shiftWords(c)))), style: row });
    } else cells.push({ text: " ", style: row }, ...choiceRuns(c, row), { text: " ".repeat(Math.max(0, labelW - measure(c.label) - measure(shiftWords(c)))), style: row });
    cells.push({ text: `   ${c.hint} `, style: { ...slotStyle("text.muted"), ...(rowBg ? { backgroundColor: rowBg } : {}) } });
    body.push({ cells: fillTo(cells, rowBg), target: c.target ?? undefined, sel: on });
  });

  // A typed field takes the first row of the body.
  const field: Line[] = [];
  if (menu.typed != null) {
    // input::render_field in the row one cell in from each edge: the text on bg.overlay in bold emphasis from its scroll
    // offset, the caret a reversed cell over the character it is before (a space at the end).
    const room = Math.max(0, innerW - 2);
    const caret = Math.min(Math.max(menu.caret ?? [...menu.typed].length, 0), [...menu.typed].length);
    const v = fieldWindow(menu.typed, caret, room);
    const before = cutCells(v.before, room);
    const at = cutCells(v.at, room - measure(before));
    const after = cutCells(v.after, room - measure(before + at));
    const emphasisField: Style = { ...slotStyle("text.emphasis"), bold: true, backgroundColor: overlay };
    const used = measure(before + at + after);
    field.push({
      cells: [
        { text: " " },
        ...(before ? [{ text: before, style: emphasisField }] : []),
        ...(at ? [{ text: at, style: emphasisField, inverse: true }] : []),
        ...(after ? [{ text: after, style: emphasisField }] : []),
        { text: " ".repeat(Math.max(0, room - used)), style: emphasisField },
        { text: " " },
      ],
    });
  }

  // Native choices(): pack right-aligned button rows, wrap long captions, then drop bevels if needed.
  type ButtonHit = { at: number; row: number; w: number; h: number; target?: string };
  const choices = (raised: boolean): { rows: Cell[][]; hits: ButtonHit[] } => {
    type Button = { rows: Cell[][]; w: number; target?: string };
    const groups: Button[][] = [];
    let used = 0;
    for (const { c, i } of buttons) {
      const focused = i === sel;
      const face = focused ? slotStyle("accent.primary").color : bgOf("bg.selection");
      const faceRgb = rgbOf(face);
      const edge = (to: [number, number, number], part: number, or: string): Style =>
        faceRgb ? { color: mix(to, faceRgb, part), backgroundColor: face } : { ...slotStyle(or), backgroundColor: face };
      const on: Style = { ...slotStyle("bg.base"), bold: true, backgroundColor: face };
      const keyS: Style = focused ? on : { ...slotStyle("accent.secondary"), bold: true, backgroundColor: face };
      const wordS: Style = focused ? on : { ...slotStyle(c.muted ? "text.muted" : "text.primary"), backgroundColor: face };
      const prefix = mnemonicPrefix(buttonWords(c), keyOf(c), c);
      const ic = lead(c.icon);
      const valid = !!mnemonicRange(buttonWords(c), keyOf(c), c.mnemonic_at);
      const caption: Cell[] = [
        { text: ` ${!valid && prefix ? `${prefix} ` : ""}`, style: keyS },
        ...(ic ? [{ text: ic.text, style: { ...ic.style, backgroundColor: face } }] : []),
        ...(valid && prefix ? [{ text: `${prefix} `, style: keyS }] : []),
        ...mnemonicRuns(buttonWords(c), keyOf(c), c, wordS),
        { text: " ", style: wordS },
      ];
      const bw = Math.min(measure(buttonText(c)), innerW);
      const rows = wrapStyled(caption, bw).map((line) => [...line, { text: " ".repeat(Math.max(0, bw - line.reduce((n, c) => n + measure(c.text), 0))), style: { backgroundColor: face } }]);
      if (raised) rows.unshift([{ text: "▔".repeat(bw), style: edge([255, 255, 255], 0.45, "text.muted") }]);
      if (raised) rows.push([{ text: "▁".repeat(bw), style: edge([0, 0, 0], 0.55, "bg.base") }]);
      if (groups.length === 0 || used + 2 + bw > innerW) { groups.push([]); used = 0; }
      groups.at(-1)!.push({ rows, w: bw, target: c.target ?? undefined });
      used += (used ? 2 : 0) + bw;
    }
    const rows: Cell[][] = [];
    const hits: ButtonHit[] = [];
    for (const group of groups) {
      const height = Math.max(0, ...group.map((b) => b.rows.length));
      const rowWidth = group.reduce((n, b, i) => n + b.w + (i ? 2 : 0), 0);
      const pad = Math.max(0, innerW - rowWidth);
      const y = rows.length;
      for (let j = 0; j < height; j++) rows.push([{ text: " ".repeat(pad) }]);
      let at = pad;
      group.forEach((b, i) => {
        if (i) { for (let j = 0; j < height; j++) rows[y + j]!.push({ text: "  " }); at += 2; }
        for (let j = 0; j < height; j++) rows[y + j]!.push(...(b.rows[j] ?? [{ text: " ".repeat(b.w) }]));
        hits.push({ at, row: y, w: b.w, h: b.rows.length, target: b.target });
        at += b.w;
      });
    }
    return { rows, hits };
  };
  const minBody = Number(body.length > 0);
  let drawnButtons = choices(true);
  if (drawnButtons.rows.length + field.length + minBody > maxH) drawnButtons = choices(false);
  const buttonRows = drawnButtons.rows;
  const buttonHits = drawnButtons.hits;
  const usable = menu.usable !== false && innerW >= (menu.typed != null ? 4 : 2) && maxH > 0 && maxH >= buttonRows.length + field.length + minBody;
  const gap = Number(body.length > 0 && buttonRows.length > 0 && maxH > buttonRows.length + field.length + minBody);
  const shownBody = Math.max(0, Math.min(body.length, maxH - field.length - buttonRows.length - gap));
  const scroll = Math.max(0, menu.scroll ?? 0);
  const rows = usable ? field.length + shownBody + gap + buttonRows.length : maxH;
  const h = Math.min(bodyHeight, rows + 2);
  // Opened by a context click: at that cell, kept on screen; else centred.
  const x = menu.at ? Math.min(Math.max(0, menu.at.column), Math.max(0, width - w)) : Math.max(0, Math.floor((width - w) / 2));
  const y = menu.at
    ? Math.min(Math.max(bodyTop, menu.at.row), Math.max(bodyTop, bodyTop + bodyHeight - h))
    : bodyTop + Math.max(0, Math.floor((bodyHeight - h) / 2));

  const lines: Cell[][] = [];
  const hits: Hit[] = [];
  const edgeCell = (text: string): Cell => ({ text, style: border });
  const shownTitle = longTitle ? narrowCount ?? [] : titleCells;
  const titleW = shownTitle.reduce((n, c) => n + measure(c.text), 0);
  lines.push([edgeCell("╭"), ...shownTitle, edgeCell(`${"─".repeat(Math.max(0, w - 2 - titleW))}╮`)]);
  const put = (cells: Cell[], inner?: number) => {
    cells = clipSegments(cells, innerW);
    const used = cells.reduce((n, c) => n + measure(c.text), 0);
    const filled = used >= innerW ? cells : [...cells, { text: " ".repeat(innerW - used), style: { backgroundColor: overlay } }];
    // Every cell sits on the overlay shade unless it has its own.
    const shaded = filled.map((c) => ({ ...c, style: { backgroundColor: overlay, ...(c.style ?? {}) } }));
    lines.push([edgeCell("│"), ...shaded, edgeCell("│")]);
    void inner;
  };
  if (!usable) {
    for (const line of wrapStyled([{ text: "Make the terminal larger to read this menu.", style: slotStyle("text.muted") }], innerW).slice(0, maxH)) put(line);
  } else {
    const rowAt = () => y + lines.length;
    for (const f of field) put(f.cells);
    body.slice(scroll, scroll + shownBody).forEach((l) => {
      if (l.target) hits.push({ x: x + 1, y: rowAt(), w: innerW, h: 1, target: l.target });
      put(l.cells);
    });
    if (gap) put([]);
    const firstButtonRow = rowAt();
    buttonRows.forEach((l) => put(l));
    for (const b of buttonHits) if (b.target) hits.push({ x: x + 1 + b.at, y: firstButtonRow + b.row, w: b.w, h: b.h, target: b.target });
    // A scrollbar on the right edge where there is more to read than shows.
    const max = (menu.lines ?? body.length) - (menu.shown ?? shownBody);
    if (max > 0 && shownBody > 0) {
      const first = 1 + field.length;
      const thumb = Math.min(shownBody - 1, Math.round((Math.min(scroll, max) / max) * (shownBody - 1)));
      for (let k = 0; k < shownBody; k++) {
        const line = lines[first + k]!;
        line[line.length - 1] = { text: k === thumb ? "┃" : "│", style: k === thumb ? { ...slotStyle("accent.primary"), backgroundColor: overlay } : border };
      }
    }
  }
  while (lines.length < h - 1) put([]);
  lines.push([edgeCell(`╰${"─".repeat(Math.max(0, w - 2))}╯`)]);
  return { x, y, w, h: lines.length, lines, hits };
}

export type NavRow = { text: string; ink?: Ink; selected: boolean; target: string; indent: number; heading: boolean; band?: boolean; cells: Cell[] };

/** Settings navigation rows, with tree lines and no chevrons (docs/GUIDE.md 2.12): a group's name, its sections under
 * it as `├─ ` and `└─ `, a folded group with a dim `⋯`. */
export function layoutNav(nav: SettingsNav, x0: number, y0: number, w: number, maxRows: number): { rows: NavRow[]; hits: Hit[]; total: number; at: number } {
  const rows: NavRow[] = [];
  // shell.rs settings: a group is its name in bold emphasis (folded: a muted ` ⋯`); a section is the tree line in
  // muted text, its icon in the pack's style, and its name in primary text.
  for (const g of nav.groups) {
    const folded: Cell[] = g.open ? [] : [{ text: " ⋯", style: slotStyle("text.muted") }];
    rows.push({
      text: g.open ? g.name : `${g.name} ⋯`,
      selected: nav.selected === g.target,
      target: g.target,
      indent: 0,
      heading: true,
      cells: [{ text: g.name, style: { ...slotStyle("text.emphasis"), bold: true } }, ...folded],
    });
    if (g.open) {
      g.sections.forEach((s, i) => {
        const joint = i === g.sections.length - 1 ? "└─ " : "├─ ";
        const icon = lead(s.icon);
        rows.push({
          text: `${joint}${s.name}`,
          selected: nav.selected === s.target,
          target: s.target,
          indent: 2,
          heading: false,
          // BandedTree: every other section of a group sits on bg.overlay (the odd ones, as the parts' trees band)
          band: i % 2 === 1,
          cells: [
            { text: "  " },
            { text: joint, style: slotStyle("text.muted") },
            ...(icon ? [{ text: icon.text, style: icon.style }] : []),
            { text: s.name, style: slotStyle("text.primary") },
          ],
        });
      });
    }
  }
  // BandedTree keeps the selected row in view with the least scrolling.
  const sel = rows.findIndex((r) => r.selected);
  const at = maxRows > 0 && sel >= maxRows ? Math.min(Math.max(0, rows.length - maxRows), sel - maxRows + 1) : 0;
  const shown = rows.slice(at, at + maxRows);
  const hits = shown.map((r, i) => ({ x: x0, y: y0 + i, w, h: 1, target: r.target }));
  return { rows: shown, hits, total: rows.length, at };
}

/** The target of an open menu's whole frame (host.rs Hit::MenuBox): pressed, it does nothing. */
export const MENU_BOX = "chrome:menu:box";

export function hitAt(hits: Hit[], x: number, y: number): Hit | undefined {
  for (let i = hits.length - 1; i >= 0; i--) {
    const h = hits[i]!;
    if (x >= h.x && x < h.x + h.w && y >= h.y && y < h.y + h.h) return h;
  }
  return undefined;
}
