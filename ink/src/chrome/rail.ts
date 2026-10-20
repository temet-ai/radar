// The step rail of setup (crates/radar-core/src/ui/rail.rs): the steps of a run along one line, two rows tall. Each stop
// sits on a track at its own place: a check when done, `»` when skipped, a hollow dot ahead, and the current stop on the
// accent (a square chip, never a pill). Under each, a line says how it went. The track is lit up to the current stop.
// Labels are cut to their stop's room in display cells.
//
// The host writes into a buffer, each write patching the style of the cells under it; so does this: a grid of cells, and
// the rows read off it as runs of one style.

import { cellWidth, clusters, fit } from "./width.js";
import { bgOf, iconOf, slotStyle, type Style } from "../text/look.js";
import { patch } from "../text/cells.js";
import type { RailStop } from "../protocol/generated/RailStop.js";
import type { Cell } from "./layout.js";
import type { Hit } from "./types.js";

/** The rows a rail takes. */
export const RAIL_HEIGHT = 2;

export type RailLaid = {
  /** The two rows, each exactly `width` cells. */
  rows: Cell[][];
  /** One per stop that has room for its words, relative to the rail's top left. */
  hits: Hit[];
};

type Grid = Array<{ ch: string; s: Style }>;

/** `text` cut to `w` cells with an ellipsis, never padded. */
const cut = (text: string, w: number): string => (cellWidth(text) <= w ? text : fit(text, w).trimEnd());

/** The mark before a stop's label, with its style (rail.rs mark). */
function mark(stop: RailStop): { glyph: string; style: Style } {
  const [icon, fallback, slot]: [string | null | undefined, string, string] =
    stop.state === "done"
      ? ["run_passed", "✓", "status.success"]
      : stop.state === "skipped"
        ? ["run_skipped", "»", "text.muted"]
        : stop.state === "upcoming"
          ? ["todo", "○", "text.muted"]
          : [stop.icon, "", "bg.base"];
  const g = iconOf(icon)?.glyph ?? "";
  const glyph = g ? `${g} ` : fallback ? `${fallback} ` : "";
  return { glyph, style: slotStyle(slot) };
}

/** Buffer::set_string: the characters from `x`, each patching the style of the cell it lands on; a wide one takes the
 * next cell too, and what would cross the right edge is left out. */
function put(row: Grid, ground: Style, x: number, text: string, style: Style) {
  let at = x;
  for (const [ch, w] of clusters(text)) {
    if (w === 0) {
      const prev = row[at - 1];
      if (prev && prev.ch !== "") prev.ch += ch;
      continue;
    }
    if (at < 0 || at + w > row.length) return;
    row[at] = { ch, s: patch(row[at]!.s, style) };
    for (let k = 1; k < w; k++) row[at + k] = { ch: "", s: ground };
    at += w;
  }
}

const same = (a: Style, b: Style) =>
  a.color === b.color &&
  a.backgroundColor === b.backgroundColor &&
  !!a.bold === !!b.bold &&
  !!a.dim === !!b.dim &&
  !!a.italic === !!b.italic &&
  !!a.underline === !!b.underline &&
  !!a.inverse === !!b.inverse &&
  !!a.strike === !!b.strike;

function cellsOf(row: Grid): Cell[] {
  const out: Cell[] = [];
  for (const c of row) {
    if (c.ch === "") continue;
    const last = out[out.length - 1];
    if (last && same(last.style ?? {}, c.s)) last.text += c.ch;
    else out.push({ text: c.ch, style: c.s });
  }
  return out;
}

/** Draw `stops` across `width` cells, two rows (rail.rs render). */
export function layoutRail(stops: readonly RailStop[], width: number): RailLaid {
  const ground: Style = { backgroundColor: bgOf("bg.base") };
  const grid: Grid[] = [0, 1].map(() => Array.from({ length: Math.max(0, width) }, () => ({ ch: " ", s: ground })));
  const hits: Hit[] = [];
  if (stops.length === 0 || width <= 0) return { rows: grid.map(cellsOf), hits };
  const n = stops.length;
  const slot = Math.max(1, Math.floor(width / n));
  const center = (i: number) => i * slot + Math.floor(slot / 2);
  const [top, under] = grid as [Grid, Grid];
  // The track, from the first stop to the last; lit up to the current stop (or the last one done).
  let litTo = -1;
  stops.forEach((s, i) => {
    if (s.state === "current") litTo = i;
  });
  if (litTo < 0) stops.forEach((s, i) => (s.state === "done" || s.state === "skipped" ? (litTo = i) : 0));
  const from = center(0);
  const to = Math.min(center(n - 1), width - 1);
  for (let x = from; x <= to; x++) {
    const lit = litTo >= 0 && x <= center(litTo);
    put(top, ground, x, "─", lit ? slotStyle("status.success") : slotStyle("text.muted"));
  }
  const sat = (a: number) => Math.max(0, a);
  stops.forEach((stop, i) => {
    const x0 = i * slot;
    const room = i + 1 === n ? sat(width - x0) : slot;
    // A cell of ground on each side breaks the track around the label.
    const inner = sat(room - 2);
    if (inner === 0) return;
    const { glyph, style: markStyle } = mark(stop);
    const current = stop.state === "current";
    const words = cut(`${glyph}${stop.label}`, sat(inner - (current ? 2 : 0)));
    const label = current ? ` ${words} ` : words;
    const w = cellWidth(label);
    const c = Math.min(center(i), x0 + sat(room - 1));
    const lx = Math.min(Math.max(sat(c - Math.floor(w / 2)), x0 + 1), x0 + sat(room - (w + 1)));
    put(top, ground, sat(lx - 1), " ", ground);
    put(top, ground, lx + w, " ", ground);
    if (current) {
      const base = slotStyle("bg.base");
      put(top, ground, lx, label, { ...base, bold: true, backgroundColor: slotStyle("accent.primary").color });
    } else {
      put(top, ground, lx, label, stop.state === "done" ? slotStyle("text.primary") : slotStyle("text.muted"));
      const glyphW = Math.min(cellWidth(glyph), cellWidth(label));
      if (glyphW > 0) put(top, ground, lx, [...label].slice(0, [...glyph].length).join(""), markStyle);
    }
    if (stop.result) {
      const note = cut(stop.result, inner);
      const nw = cellWidth(note);
      const nx = Math.min(Math.max(sat(c - Math.floor(nw / 2)), x0 + 1), x0 + sat(room - (nw + 1)));
      put(under, ground, nx, note, current ? slotStyle("text.secondary") : slotStyle("text.muted"));
    }
    hits.push({ x: x0, y: 0, w: room, h: RAIL_HEIGHT, target: stop.target });
  });
  return { rows: grid.map(cellsOf), hits };
}
