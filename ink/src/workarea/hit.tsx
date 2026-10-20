import React, { useLayoutEffect, useRef } from "react";
import { Box, type DOMElement } from "ink";
/** Where hits are registered: the app's registry, or the nodes' context view of it. */
export interface HitSink {
  add(hit: { x: number; y: number; w: number; h: number; target: string }): void;
}

/** The screen rectangle of a laid-out Ink element: its yoga offsets summed up to the root. */
export function screenRect(el: DOMElement): { x: number; y: number; w: number; h: number } | undefined {
  const y0 = el.yogaNode;
  if (!y0) return undefined;
  let x = 0;
  let y = 0;
  for (let n: DOMElement | undefined = el; n; n = n.parentNode) {
    if (!n.yogaNode) break;
    x += n.yogaNode.getComputedLeft();
    y += n.yogaNode.getComputedTop();
  }
  return { x, y, w: y0.getComputedWidth(), h: y0.getComputedHeight() };
}

/**
 * A box that is a pointer target: after Ink has laid the frame out it registers its rectangle with the
 * registry, so a press on it reaches the host as the same `select` intent the view data gives the target.
 * Without a registry (a snapshot test) it is only a box.
 */
export function HitBox({
  registry,
  target,
  children,
  ...box
}: { registry?: HitSink; target: string; children: React.ReactNode } & React.ComponentProps<typeof Box>) {
  const ref = useRef<DOMElement>(null);
  // Layout is computed before layout effects run (Ink does it in resetAfterCommit), so the rectangle is final.
  useLayoutEffect(() => {
    const r = ref.current && registry ? screenRect(ref.current) : undefined;
    if (r && registry && r.w > 0 && r.h > 0) registry.add({ x: r.x, y: r.y, w: r.w, h: r.h, target });
  });
  return (
    <Box ref={ref} {...box}>
      {children}
    </Box>
  );
}
