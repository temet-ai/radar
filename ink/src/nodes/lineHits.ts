import { useId, useLayoutEffect, type RefObject } from "react";
import type { DOMElement } from "ink";
import type { Seg, Text } from "./view.js";
import { segRunWords } from "../text/TextLine.js";
import { cellWidth } from "../chrome/width.js";
import { screenAt } from "./index.js";
import type { HitSpec } from "./hits.js";

/** Where a hit is registered (NodeContext.registry, the part of it a line needs). */
export interface LineHitSink {
  add(hit: { x: number; y: number; w: number; h: number; target: string }): void;
  replace?(owner: string, hits: Array<{ x: number; y: number; w: number; h: number; target: string }>): void;
  remove?(owner: string): void;
}

/** The cells of each segment of a one-line text that carries a target (a chip on a page's frame line), as the line draws
 * them: `start` cells in, each segment as wide as it is drawn (a chip counts its padding), cut at `room` cells so a
 * chip the line clips does not take a press beyond the edge. */
export function segHits(text: Text, plain = false, start = 0, room = Number.POSITIVE_INFINITY): HitSpec[] {
  const out: HitSpec[] = [];
  let x = start;
  for (const seg of text as Seg[]) {
    const w = cellWidth(segRunWords(seg, plain));
    if (seg.target && w > 0) {
      const shown = Math.min(w, room - x);
      if (shown > 0) out.push({ dx: x, dy: 0, w: shown, h: 1, target: seg.target });
    }
    x += w;
  }
  return out;
}

/** Registers `hits` (cells relative to the element's top-left corner) at the element's place after each layout, the way
 * the page's tab row and the detail pane's foot do. */
export function useLineHits(ref: RefObject<DOMElement | null>, registry: LineHitSink | undefined, hits: HitSpec[]): void {
  const owner = useId();
  useLayoutEffect(() => {
    if (!registry) return;
    const at = screenAt(ref.current);
    const placed = at ? hits.map((h) => ({ x: at.x + h.dx, y: at.y + h.dy, w: h.w, h: h.h, target: h.target })) : [];
    if (registry.replace) registry.replace(owner, placed);
    else placed.forEach((h) => registry.add(h));
  });
  useLayoutEffect(() => () => registry?.remove?.(owner), [registry, owner]);
}
