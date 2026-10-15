import React, { useLayoutEffect, useRef } from "react";
import { Box, Text, type DOMElement } from "ink";
import { cellWidth, fit } from "../chrome/width.js";
import { slotStyle, type Style } from "../text/look.js";
import { textProps } from "../text/TextLine.js";
import type { NodeContext } from "../nodes/view.js";

type Registry = NodeContext["registry"];

/** ui::page::tab_row: `Overview  [Changes · 2]  Threads`: the tab showing in brackets and bold accent (on the
 * accent when the row has the keys, as the top bar's), two spaces between; when the row is too short a window
 * holding the selected tab. Each tab but the one showing is a press target, `tab:N`, with a cell either side. */
export function tabWindow(labels: string[], on: number, width: number) {
  const titles = labels.map((l, i) => (i === on ? `[${l}]` : l));
  let first = on >= 0 && on < labels.length ? on : 0;
  let used = Math.min(cellWidth(titles[first] ?? ""), width);
  while (first > 0) {
    const before = cellWidth(titles[first - 1]!) + 2;
    if (before > width - used) break;
    used += before;
    first -= 1;
  }
  if (titles[on] !== undefined && cellWidth(titles[on]!) > width) titles[on] = width >= 3 ? `[${fit(labels[on]!, width - 2)}]` : "…";
  const shown: Array<{ index: number; text: string; x: number }> = [];
  let x = 0;
  for (let i = first; i < titles.length; i++) {
    if (x >= width) break;
    const text = fit(titles[i]!, width - x);
    shown.push({ index: i, text, x });
    x += cellWidth(text) + 2;
  }
  return shown;
}

export function TabRow({ labels, on, focused, width, registry, base }: { labels: string[]; on: number; focused: boolean; width: number; registry?: Registry; base?: string }) {
  const ref = useRef<DOMElement>(null);
  const shown = tabWindow(labels, on, width);
  useLayoutEffect(() => {
    if (!registry || !ref.current) return;
    let x = 0;
    let y = 0;
    for (let n: DOMElement | undefined = ref.current; n; n = n.parentNode) {
      x += n.yogaNode?.getComputedLeft() ?? 0;
      y += n.yogaNode?.getComputedTop() ?? 0;
    }
    for (const t of shown) {
      if (t.index === on) continue;
      const from = Math.max(0, t.x - 1);
      registry.add({ x: x + from, y, w: Math.min(width, t.x + cellWidth(t.text) + 1) - from, h: 1, target: `tab:${t.index}` });
    }
  });
  const showing = focused
    ? { ...slotStyle("bg.base"), bold: true, backgroundColor: slotStyle("accent.primary").color }
    : { ...slotStyle("accent.primary"), bold: true, backgroundColor: base };
  // The tab under the pointer is lit over the finished frame (app/hover.ts), from the hits registered above.
  const pieces: Array<{ text: string; style: Style }> = [];
  const push = (text: string, style: Style) => void pieces.push({ text, style });
  const secondary: Style = { ...slotStyle("text.secondary"), backgroundColor: base };
  push(" ", { backgroundColor: base });
  shown.forEach((t, i) => {
    if (i > 0) push("  ", secondary);
    push(t.text, t.index === on ? showing : secondary);
  });
  push(" ".repeat(Math.max(0, width + 1 - 1 - shown.reduce((n, t, i) => n + cellWidth(t.text) + (i > 0 ? 2 : 0), 0))), secondary);
  return (
    <Box ref={ref} width={width + 2} flexShrink={0}>
      {/* Ratatui writes no background here, so the page's bg.base shows under every cell of the row (live cells, tmux): the leading space, every title, the gaps; the showing title is on the accent only while the row has the keys */}
      <Text wrap="truncate">
        {pieces.map((p, i) => (
          <Text key={i} {...textProps(p.style)}>{p.text}</Text>
        ))}
      </Text>
    </Box>
  );
}
