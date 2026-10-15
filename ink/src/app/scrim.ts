// ui::scrim for Ink (crates/radar-core/src/ui/scrim.rs): while a menu, question or help is up, every cell outside it is
// dimmed and it throws a one-cell shadow on the column right of it and the row below it, painted over the finished frame
// as the hover is (app/hover.ts), never by the components. A shadow dims and never erases: every glyph keeps its place.
//
// In full colour each cell's colours are mixed toward bg.base (the shadow's ground toward black); with fewer colours, or
// none, the cells take SGR dim and the shadow's ground is black. The same numbers as Ratatui's, rounded the same.
import chalk from "chalk";
import { bgOf, hexRgb, mix, monoTier, slotStyle, type Rgb } from "../text/look.js";
import { BOLD, DIM, restyle, styleId, styleOf, type Sgr } from "./fast-output.js";

export type ScrimRect = { x: number; y: number; w: number; h: number };

/** Per mille toward bg.base (the scrim's text and ground, the shadow's text) and toward black (the shadow's ground). */
const SCRIM_FG = 720;
const SCRIM_BG = 430;
const SHADOW_BG = 600;
const SHADOW_FG = 450;


/** A cell's colour as its SGR parameters carry it in full colour (`38;2;r;g;b`); undefined for any other. */
const sgrRgb = (p: string, lead: "38" | "48"): Rgb | undefined => {
  const m = p.split(";");
  return m.length === 5 && m[0] === lead && m[1] === "2" ? [Number(m[2]), Number(m[3]), Number(m[4])] : undefined;
};
const fgOf = (c: Rgb) => `38;2;${c.join(";")}`;
const bgParamsOf = (c: Rgb) => `48;2;${c.join(";")}`;

/** The colours a cell shows, read in full colour: no text colour is text.primary, no ground bg.base. Undefined below
 * full colour, or for a colour the terminal picks. */
function colours(s: Readonly<Sgr>, base: Rgb | undefined, primary: Rgb | undefined): [Rgb, Rgb, Rgb] | undefined {
  if (!base) return undefined;
  const fg = s.fg === "" ? primary : sgrRgb(s.fg, "38");
  const bg = s.bg === "" ? base : sgrRgb(s.bg, "48");
  return fg && bg ? [fg, bg, base] : undefined;
}

/** The scrim and shadow of `layer` over a finished frame. */
export function paintScrim(output: string, width: number, height: number, layer: ScrimRect): string {
  // What chalk writes is what the terminal takes: no colour codes at all, or full colour only where chalk writes it.
  if (chalk.level === 0) return output;
  const full = chalk.level >= 3;
  const base = full ? hexRgb(bgOf("bg.base")) : undefined;
  const primary = hexRgb(slotStyle("text.primary").color);
  const mono = monoTier();
  const memo = (f: (s: Readonly<Sgr>) => number) => {
    const m = new Map<number, number>();
    return (id: number) => {
      let to = m.get(id);
      if (to === undefined) m.set(id, (to = f(styleOf(id))));
      return to;
    };
  };
  const dim = memo((s) => {
    const attr = s.attr & ~BOLD;
    const c = colours(s, base, primary);
    return c ? styleId(fgOf(mix(c[0], c[2], SCRIM_FG)), bgParamsOf(mix(c[1], c[2], SCRIM_BG)), attr) : styleId(s.fg, s.bg, attr | DIM);
  });
  const shade = memo((s) => {
    const c = colours(s, base, primary);
    return c
      ? styleId(fgOf(mix(c[0], c[2], SHADOW_FG)), bgParamsOf(mix(c[1], [0, 0, 0], SHADOW_BG)), s.attr)
      : styleId(s.fg, mono ? s.bg : "40", s.attr | DIM);
  });
  const { x, y, w, h } = layer;
  let out = output;
  // Around the layer: the rows above and below it, then left and right of it.
  for (const r of [
    { x: 0, y: 0, w: width, h: y },
    { x: 0, y: y + h, w: width, h: height - (y + h) },
    { x: 0, y, w: x, h },
    { x: x + w, y, w: width - (x + w), h },
  ])
    if (r.w > 0 && r.h > 0) out = restyle(out, width, r, dim);
  if (w <= 0 || h <= 0) return out;
  // The column right of it from its second row down, and the row below it from its second column across.
  out = restyle(out, width, { x: x + w, y: y + 1, w: 1, h }, shade);
  if (w > 1) out = restyle(out, width, { x: x + 1, y: y + h, w: w - 1, h: 1 }, shade);
  return out;
}

/** A Text transformer that leaves its text as it is and marks a layer `w` by `h` cells whose top-left cell the marked
 * text is written in (its `radarLayer`): the frame paint (fast-output.ts, app/hover.ts) reads it from the frame's writes
 * and lays the scrim and shadow around it. One function per size, so a frame drawn again with the same menu is the same
 * writes (fast-output reuses it). */
const marks = new Map<string, ((s: string) => string) & { radarLayer: { w: number; h: number } }>();
export function layerMark(w: number, h: number): (s: string) => string {
  const key = `${w}x${h}`;
  let fn = marks.get(key);
  if (!fn) {
    if (marks.size > 64) marks.clear();
    fn = Object.assign((s: string) => s, { radarLayer: { w, h } });
    marks.set(key, fn);
  }
  return fn;
}
