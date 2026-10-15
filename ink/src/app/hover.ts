// ui::hover::paint for Ink: what a click would act on is lit under the pointer, painted over the finished frame as Ratatui
// paints it over its buffer (shell.rs hover), not drawn by the components. A pointer that moves to another target is
// one frame Ink takes from the last one (fast-output reuses a frame whose operations did not change) with the cells of
// one row restyled: no React render, no row drawn again.
import type { Hit } from "../chrome/types.js";
import { bgOf, monoTier, slotStyle } from "../text/look.js";
import { bgParams, restyle, setFramePaint, styleId, styleOf, UNDERLINE } from "./fast-output.js";
import { paintScrim } from "./scrim.js";

export type HoverRect = { x: number; y: number; w: number; h: number };

const same = (a: HoverRect | null, b: HoverRect | null) =>
  a === b || (!!a && !!b && a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h);

/** Ratatui's paint over one area of a frame: every cell takes `bg.hover`, or an underline where the tier has no colour
 * for it (the mono tier, plain text); the cursor's cells (bg.selection) and the focused tab's (the accent) stay as
 * drawn. */
export function paintHover(output: string, width: number, rect: HoverRect, plain = false): string {
  const hover = bgOf("bg.hover");
  const underline = plain || monoTier() || !hover;
  const lit = underline ? "" : bgParams(hover);
  // The cursor's cells, the focused tab's, and a chip on a state's ground (the status line's attention chip, whose bold
  // base text would not read on the hover shade) stay as drawn (ui::hover::paint).
  const grounds = ["accent.primary", "status.warning", "status.success", "status.error"].map((s) => bgParams(slotStyle(s).color));
  const keep = new Set([bgParams(bgOf("bg.selection")), ...grounds].filter(Boolean));
  const memo = new Map<number, number>();
  return restyle(output, width, rect, (id) => {
    let to = memo.get(id);
    if (to === undefined) {
      const s = styleOf(id);
      to = keep.has(s.bg) ? id : underline ? styleId(s.fg, s.bg, s.attr | UNDERLINE) : styleId(s.fg, lit, s.attr);
      memo.set(id, to);
    }
    return to;
  });
}

/** The area under the pointer and how a frame is asked for. One per renderer (the frames of one Ink output). */
export class Hover {
  private rect: HoverRect | null = null;
  private plain = false;
  private redraw: (() => void) | undefined;
  /** While true (a menu is up) nothing is lit, whatever was under the pointer. */
  hidden: () => boolean = () => false;

  /** The hit under the pointer, cut to the pointer's row (host.rs hover_at), or null. A change asks Ink for a frame. */
  set(hit: Pick<Hit, "x" | "y" | "w" | "h"> | null, plain = this.plain): boolean {
    const r = hit ? { x: hit.x, y: hit.y, w: hit.w, h: hit.h } : null;
    if (same(r, this.rect) && plain === this.plain) return false;
    this.rect = r;
    this.plain = plain;
    this.redraw?.();
    return true;
  }
  get area(): HoverRect | null {
    return this.rect;
  }
  /** How to ask Ink for a frame without a React render (the root node's onRender), and what hides the hover. */
  attach(redraw: () => void, hidden: () => boolean) {
    this.redraw = redraw;
    this.hidden = hidden;
  }
  /** The app that attached `redraw` is gone: nothing is lit and no frame is asked for (another app may have attached
   * since, in tests: it keeps its own). */
  detach(redraw: () => void) {
    if (this.redraw !== redraw) return;
    this.redraw = undefined;
    this.hidden = () => false;
    this.rect = null;
  }
  paint = (output: string, width: number): string => {
    const r = this.rect;
    if (!r || this.hidden()) return output;
    return paintHover(output, width, r, this.plain);
  };
}

/** The renderer's hover: fast-output paints it over every frame, over the scrim (app/scrim.ts). */
export const hover = new Hover();
setFramePaint((output, width, height, layer) =>
  hover.paint(layer ? paintScrim(output, width, height, layer) : output, width),
);
