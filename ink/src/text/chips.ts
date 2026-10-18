import type { Seg } from "../nodes/view.js";
import { ICONS } from "./icons.js";
import { withEmoji } from "./emoji.js";
import { bgOf, hexRgb, mix, rgbHex, slotStyle } from "./look.js";
import { segRunStyle } from "./TextLine.js";
import type { Style } from "./theme.js";

/** One styled run of a chip. */
export interface ChipRun {
  text: string;
  style: Style;
}

/** widgets::chip_ground: a chip's ground is its tone's colour mixed 16% over the overlay shade, so a row of chips reads
 * by their grounds before their words; the flat overlay shade where either is not full colour (256, 16, mono). */
export function chipGround(fg: string | undefined): string | undefined {
  const overlay = bgOf("bg.overlay");
  const o = hexRgb(overlay);
  const t = hexRgb(fg);
  return o && t ? rgbHex(mix(o, t, 160)) : overlay;
}

/** widgets::chip: the tone's style (a plain chip is text.primary, not the terminal's own text) on its tone's tint. */
export function chipWordStyle(seg: Seg): Style {
  const base = segRunStyle(seg);
  const tone = !seg.look && (!seg.ink || seg.ink === "plain") && !seg.strong ? slotStyle("text.primary") : {};
  // widgets::tone: Accent, Warn and Bad are bold, the others are not.
  const bold = !seg.look && (seg.ink === "accent" || seg.ink === "warn" || seg.ink === "bad") ? { bold: true } : {};
  const style = { ...base, ...tone, ...bold };
  return { ...style, backgroundColor: chipGround(style.color) };
}

/** ui::chips::chip_spans: a chip is ` ` + the icon and its gap (the icon's own colour) + `words ` (the tone's
 * colour), all on the chip's ground. With no glyph (icons off, or an unknown icon) it is ` words ` in one run. */
export function chipRuns(seg: Seg, literal = false): ChipRun[] {
  const words = literal ? seg.text ?? "" : withEmoji(seg.text ?? "");
  const glyph = seg.icon ? ICONS[seg.icon] : undefined;
  const wordStyle: Style = chipWordStyle(seg.icon ? { ...seg, icon: null } : seg);
  const bg = wordStyle.backgroundColor;
  if (!glyph) return words ? [{ text: ` ${words} `, style: wordStyle }] : [];
  const lead: Style = { ...segRunStyle(seg), backgroundColor: bg };
  const runs: ChipRun[] = [
    { text: " ", style: wordStyle },
    { text: `${glyph} `, style: lead },
  ];
  if (words) runs.push({ text: `${words} `, style: wordStyle });
  else runs[0] = { text: " ", style: wordStyle };
  return runs;
}

/** The cells' text of a chip, as chipRuns lays it out. */
export function chipText(seg: Seg, literal = false): string {
  let out = "";
  for (const r of chipRuns(seg, literal)) out += r.text;
  return out;
}
