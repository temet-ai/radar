import { cellWidth } from "../chrome/width.js";
import { currentLook, hexRgb, mix, rgbHex, type Rgb, type Style } from "./look.js";

export type FocusRamp = readonly [Rgb, Rgb, Rgb];
export type FrameRun = { text: string; style: Style };

/** Only the host's resolved truecolor slots participate. No palette or RGB fallback is invented. */
export function focusRamp(): FocusRamp | undefined {
  const look = currentLook();
  if (look.tier !== "truecolor") return undefined;
  const a = hexRgb(look.slots["glow.purple"]);
  const b = hexRgb(look.slots["accent.tertiary"]);
  const c = hexRgb(look.slots["glow.cyan"]);
  return a && b && c ? [a, b, c] : undefined;
}

/** Shared with ui::frame: integer phase, then existing half-up per-channel mixing. */
export function rampColor(ramp: FocusRamp, index: number, count: number): string {
  const at = Math.max(0, Math.min(Math.floor(index), count - 1));
  const phase = count <= 1 ? 0 : Math.floor(at * 2000 / (count - 1));
  return rgbHex(phase <= 1000 ? mix(ramp[0], ramp[1], phase) : mix(ramp[1], ramp[2], phase - 1000));
}

/** Clockwise from top-left, every corner once; a collapsed rectangle is a straight line. */
export function perimeterLength(width: number, height: number): number {
  if (width <= 0 || height <= 0) return 0;
  if (height === 1) return width;
  if (width === 1) return height;
  return 2 * width + 2 * height - 4;
}

export function perimeterIndex(width: number, height: number, x: number, y: number): number | undefined {
  if (x < 0 || y < 0 || x >= width || y >= height) return undefined;
  if (height === 1) return x;
  if (width === 1) return y;
  if (y === 0) return x;
  if (x === width - 1) return width + y - 1;
  if (y === height - 1) return width + height - 1 + width - 2 - x;
  if (x === 0) return 2 * width + height - 2 + height - 2 - y;
  return undefined;
}

/** A horizontal border run at its real coordinates. Title gaps never compress the perimeter. */
export function frameRuns(text: string, width: number, height: number, x: number, y: number, base: Style, ramp?: FocusRamp): FrameRun[] {
  if (!ramp) return [{ text, style: base }];
  const count = perimeterLength(width, height);
  const out: FrameRun[] = [];
  for (const ch of text) {
    const index = perimeterIndex(width, height, x, y);
    out.push({ text: ch, style: index === undefined ? base : { ...base, color: rampColor(ramp, index, count) } });
    x += cellWidth(ch);
  }
  return out;
}

const graphemes = new Intl.Segmenter(undefined, { granularity: "grapheme" });

/** Brand alone, left to right by display cells. Padding keeps its old style and is not sampled. */
export function wordmarkRuns(text: string, base: Style, before = "", after = ""): FrameRun[] {
  const ramp = focusRamp();
  if (!ramp) return [{ text: before + text + after, style: base }];
  const out: FrameRun[] = before ? [{ text: before, style: base }] : [];
  const count = cellWidth(text);
  let at = 0;
  for (const { segment } of graphemes.segment(text)) {
    out.push({ text: segment, style: { ...base, color: rampColor(ramp, at, count) } });
    at += cellWidth(segment);
  }
  if (after) out.push({ text: after, style: base });
  return out;
}
