import type { Cell } from "./layout.js";
import type { Choice } from "./types.js";
import { cellWidth } from "./width.js";
import { mnemonicRange } from "../text/mnemonic.js";
import { graphemes } from "../editor/graphemes.js";
import { bgOf, iconOf, slotStyle, specStyle, type Style } from "../text/look.js";

/** Palette layout, like shell/menu.rs, keeps a grapheme whole. In particular a
 * joiner sequence, flag, or emoji modifier occupies one two-cell glyph. */
export function paletteWidth(text: string): number {
  return cellWidth(text);
}

/** Host offsets count Unicode scalars. Highlight the containing grapheme so a
 * combining accent or emoji sequence is never split into separate styled runs. */
export function paletteLabel(text: string, highlights: readonly number[], base: Style, matched: Style, mnemonic?: [number, number]): Cell[][] {
  const positions = new Set(highlights);
  let offset = 0;
  return text.split("\n").map((raw) => {
    const line = raw.replace(/\r$/, "");
    const cells = graphemes(line).map((g) => {
      const end = offset + [...g].length;
      let on = false;
      for (let i = offset; i < end; i++) if (positions.has(i)) on = true;
      const underline = mnemonic && offset < mnemonic[1] && end > mnemonic[0];
      offset = end;
      const style = on ? matched : base;
      return { text: g, style: underline ? { ...style, underline: true } : style };
    });
    offset += [...raw].length - [...line].length + 1;
    return cells;
  });
}

/** Explicit grapheme wrapping shared with the Rust palette. No truncation,
 * space normalization, or searching for repeated substrings after wrapping. */
export function wrapPalette(lines: Cell[][], width: number): Cell[][] {
  width = Math.max(2, width);
  const out: Cell[][] = [];
  for (const line of lines) {
    let cells: Cell[] = [];
    let used = 0;
    for (const cell of line) for (const g of graphemes(cell.text)) {
      const w = paletteWidth(g);
      if (used > 0 && used + w > width) {
        out.push(cells);
        cells = [];
        used = 0;
      }
      cells.push({ ...cell, text: g });
      used += w;
    }
    out.push(cells);
  }
  return out;
}

export const paletteLead = (item: Choice): Cell | undefined => {
  const icon = item.icon ? iconOf(item.icon) : undefined;
  return icon?.glyph ? { text: `${icon.glyph} `, style: specStyle(icon.style) } : undefined;
};

export function paletteLines(item: Choice, selected: boolean, highlights: readonly number[], width: number, keyW: number, hintHighlights: readonly number[] = []): Cell[][] {
  const backgroundColor = selected ? bgOf("bg.selection") : undefined;
  const shade = backgroundColor ? { backgroundColor } : {};
  const row: Style = selected ? { ...slotStyle("text.emphasis"), bold: true, ...shade } : slotStyle(item.muted ? "text.muted" : "text.primary");
  const matched: Style = { ...slotStyle("accent.primary"), bold: true, ...shade };
  const keyStyle: Style = { ...slotStyle("accent.secondary"), bold: true, ...shade };
  const keys = item.keys ?? item.key ?? "";
  const mnemonic = mnemonicRange(item.label, keys, item.mnemonic_at);
  const labels = paletteLabel(item.label, highlights, row, matched, mnemonic);
  if (mnemonic && item.mnemonic_shift) labels[0]!.unshift({ text: "shift ", style: keyStyle });
  const icon = paletteLead(item);
  if (icon) labels[0]!.unshift({ ...icon, style: { ...icon.style, ...shade } });
  if (item.sample) labels[0]!.unshift({ text: "██ ", style: { ...specStyle(item.sample), ...shade } });
  if (item.hint) {
    const hints = paletteLabel(item.hint, hintHighlights, { ...slotStyle("text.muted"), ...shade }, matched);
    labels.at(-1)!.push({ text: "   ", style: row }, ...hints[0]!);
    labels.push(...hints.slice(1));
  }
  const inner = Math.max(2, width - 2);
  const sideKeys = keyW > 0 && keyW + 4 <= inner;
  const textW = sideKeys ? inner - keyW - 2 : inner;
  const key = mnemonic ? "" : keys;
  const lines = wrapPalette(labels, textW).map((cells, i) => {
    const used = cells.reduce((n, c) => n + paletteWidth(c.text), 0);
    const shownKey = i === 0 ? key : "";
    return [
      { text: " ", style: row }, ...cells,
      { text: " ".repeat(Math.max(0, textW - used)), style: row },
      ...(sideKeys ? [{ text: `  ${" ".repeat(Math.max(0, keyW - paletteWidth(shownKey)))}${shownKey}`, style: keyStyle }] : []),
      { text: " ", style: row },
    ];
  });
  if (!sideKeys && key) {
    for (const cells of wrapPalette(key.split("\n").map((s) => [{ text: s.replace(/\r$/, ""), style: keyStyle }]), inner)) {
      const used = cells.reduce((n, c) => n + paletteWidth(c.text), 0);
      lines.push([{ text: " ".repeat(Math.max(0, inner - used) + 1), style: row }, ...cells, { text: " ", style: row }]);
    }
  }
  return lines;
}
