import React, { useLayoutEffect, useRef } from "react";
import { Box, Text, type DOMElement } from "ink";
import { cellWidth, fit } from "../chrome/width.js";
import { iconOf, slotStyle, type Style } from "../text/look.js";
import { textProps } from "../text/TextLine.js";
import type { NodeContext } from "../nodes/view.js";
import type { TabView } from "../protocol/generated/TabView.js";
import { ATTENTION_DOT } from "../text/icons.js";

type Registry = NodeContext["registry"];
type TabPiece = { text: string; attention?: boolean };

/** ui::fit pads the cell left by a clipped wide glyph before the closing bracket or attention mark. */
function fitCells(text: string, width: number): string {
  const shown = fit(text, width);
  return shown + " ".repeat(Math.max(0, width - cellWidth(shown)));
}

/** ui::page::tab_row: `[Overview]  Changes ●  Threads`: a selected title retains its brackets, and a
 * warning dot is separate from its name. Widths and the selected window include the dot and its gap. */
export function tabWindow(tabs: TabView[], on: number, width: number) {
  const labels = tabs.map((tab) => {
    const glyph = iconOf(tab.icon)?.glyph;
    return `${glyph ? `${glyph} ` : ""}${tab.label}`;
  });
  const titles: TabPiece[][] = tabs.map((tab, i) => [
    { text: `${i === on ? "[" : ""}${labels[i]}` },
    ...(tab.attention ? [{ text: " " }, { text: ATTENTION_DOT, attention: true }] : []),
    ...(i === on ? [{ text: "]" }] : []),
  ]);
  const textOf = (pieces: TabPiece[]) => pieces.map((p) => p.text).join("");
  const widthOf = (pieces: TabPiece[]) => cellWidth(textOf(pieces));
  let first = on >= 0 && on < tabs.length ? on : 0;
  let used = Math.min(widthOf(titles[first] ?? []), width);
  while (first > 0) {
    const before = widthOf(titles[first - 1]!) + 2;
    if (before > width - used) break;
    used += before;
    first -= 1;
  }
  if (titles[on] !== undefined && widthOf(titles[on]!) > width) {
    if (tabs[on]!.attention && width >= 5) {
      titles[on] = [{ text: `[${fitCells(labels[on]!, width - 4)} ` }, { text: ATTENTION_DOT, attention: true }, { text: "]" }];
    } else {
      const label = `${labels[on]}${tabs[on]!.attention ? ` ${ATTENTION_DOT}` : ""}`;
      titles[on] = [{ text: width >= 3 ? `[${fitCells(label, width - 2)}]` : "…" }];
    }
  }
  const shown: Array<{ index: number; text: string; pieces: TabPiece[]; x: number }> = [];
  let x = 0;
  for (let i = first; i < titles.length; i++) {
    if (x >= width) break;
    const all = titles[i]!;
    const room = width - x;
    const pieces = widthOf(all) <= room ? all : [{ text: fitCells(textOf(all), room) }];
    const text = textOf(pieces);
    shown.push({ index: i, text, pieces, x });
    x += cellWidth(text) + 2;
  }
  return shown;
}

export function TabRow({ tabs, on, focused, width, registry, base }: { tabs: TabView[]; on: number; focused: boolean; width: number; registry?: Registry; base?: string }) {
  const ref = useRef<DOMElement>(null);
  const shown = tabWindow(tabs, on, width);
  useLayoutEffect(() => {
    if (!registry || !ref.current) return;
    let x = 0;
    let y = 0;
    for (let n: DOMElement | undefined = ref.current; n; n = n.parentNode) {
      x += n.yogaNode?.getComputedLeft() ?? 0;
      y += n.yogaNode?.getComputedTop() ?? 0;
    }
    for (const t of shown) {
      if (t.index === on || cellWidth(t.text) === 0) continue;
      const titleX = t.x + 1; // the page inset is the leading space drawn below
      const from = Math.max(0, titleX - 1);
      registry.add({ x: x + from, y, w: Math.min(width + 1, titleX + cellWidth(t.text) + 1) - from, h: 1, target: `tab:${t.index}` });
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
    const style = t.index === on ? showing : secondary;
    for (const piece of t.pieces) push(piece.text, piece.attention ? { ...style, ...slotStyle("status.warning") } : style);
  });
  push(" ".repeat(Math.max(0, width + 1 - 1 - shown.reduce((n, t, i) => n + cellWidth(t.text) + (i > 0 ? 2 : 0), 0))), secondary);
  return (
    <Box ref={ref} width={width + 2} flexShrink={0}>
      {/* No background is written here, so the page's bg.base shows under every cell of the row (live cells, tmux): the leading space, every title, the gaps; the showing title is on the accent only while the row has the keys */}
      <Text wrap="truncate">
        {pieces.map((p, i) => (
          <Text key={i} {...textProps(p.style)}>{p.text}</Text>
        ))}
      </Text>
    </Box>
  );
}
