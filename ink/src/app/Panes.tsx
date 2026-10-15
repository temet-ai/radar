import React, { useCallback, useLayoutEffect, useRef, useState } from "react";
import { Box, Text, type DOMElement } from "ink";
import { ScrollBar, ScrollSink, screenAt, type ScrollReport } from "../nodes/index.js";
import { TextLine, SegRun } from "../text/TextLine.js";
import { textWords, segWords } from "../text/TextLine.js";
import { cellWidth, fit } from "../chrome/width.js";
import type { NodeCx, RenderNode } from "./Body.js";
import { PageTitleLine } from "../nodes/pagetitle.js";
import { bgOf, iconOf, slotStyle } from "../text/look.js";
import { textProps } from "../text/TextLine.js";
import { TabRow } from "./TabRow.js";
import { bottomFieldRow, fieldRows, FIELD_ROWS } from "../chrome/filter.js";
import { splitDock } from "../dock/layout.js";
import { BgBox, CellLine, cellsOf, clipCells, lineCells, padCells, widthOfCells, wrapCells, type Cell } from "../text/cells.js";

/**
 * The body of a screen, drawn like the Ratatui adapter (crates/radar-core/src/view_tui.rs): panes framed
 * `╭ title ───╮`, the nav / middle / detail by width (Wide from 120, Medium from `medium_from` or 84, Narrow
 * below), and an item page as crumbs, title and details, the line under it, the tab row, then its panes in
 * their layout. Every pane has its own columns and is clipped to them.
 */

type Pane = {
  pane: "nav" | "middle" | "detail";
  title: string;
  title_icon?: string | null;
  body: any;
  notes?: any[];
  footer?: any[] | null;
  head?: any[] | null;
  /** The filter boxes above the body (the '/' input) and the one below it (chrome/filter.tsx draws them). */
  fields?: any[];
  bottom_field?: any | null;
  width?: number | null;
  from?: number | null;
};

const WIDE = 120;
const MEDIUM = 84;
export const detailWidth = (total: number) => Math.min(60, Math.max(38, Math.floor((total * 30) / 100)));

/** Share `total` among parts: lengths first, then fills by weight (ratatui's Length and Fill). */
export function share(total: number, parts: Array<{ length?: number; fill?: number }>): number[] {
  const fixed = parts.reduce((n, p) => n + (p.length ?? 0), 0);
  const weights = parts.reduce((n, p) => n + (p.length === undefined ? (p.fill ?? 1) : 0), 0);
  let room = Math.max(0, total - fixed);
  const out = parts.map((p) => (p.length !== undefined ? Math.min(p.length, total) : 0));
  let spent = 0;
  const fills = parts.map((p, i) => (p.length === undefined ? i : -1)).filter((i) => i >= 0);
  fills.forEach((i, k) => {
    const w = parts[i]!.fill ?? 1;
    const part = k === fills.length - 1 ? room - spent : Math.floor((room * w) / weights);
    out[i] = Math.max(0, part);
    spent += out[i]!;
  });
  return out;
}

/** Marks `width` x `height` cells as pane `pane` (HitRegistry.addPane): the wheel over it scrolls it, a press on its empty
 * space focuses it (host.rs on_mouse, Hit::Pane). Draws only its children. */
export function PaneArea({ pane, registry, width, height, children }: { pane?: string; registry?: NodeCx["registry"]; width: number; height: number; children?: React.ReactNode }) {
  const ref = useRef<DOMElement>(null);
  useLayoutEffect(() => {
    const at = pane && registry?.addPane ? screenAt(ref.current) : null;
    if (at) registry!.addPane!({ x: at.x, y: at.y, w: width, h: height, pane: pane! });
  });
  return (
    <Box ref={ref} flexDirection="column" width={width} height={height} flexShrink={0}>
      {children}
    </Box>
  );
}

export function Framed({ title, width, height, focused, plain, pane, registry, children }: { title: string; width: number; height: number; focused?: boolean; plain?: boolean; pane?: string; registry?: NodeCx["registry"]; children?: React.ReactNode }) {
  const [bar, setBar] = useState<ScrollReport>(null);
  const report = useCallback((r: ScrollReport) => setBar((p) => (p === r || (p && r && p.max === r.max && p.at === r.at) ? p : r)), []);
  if (width < 4 || height < 2) return null;
  if (pane && registry) {
    return (
      <PaneArea pane={pane} registry={registry} width={width} height={height}>
        <Framed title={title} width={width} height={height} focused={focused} plain={plain}>
          {children}
        </Framed>
      </PaneArea>
    );
  }
  // widgets::panel: a rounded border in accent.primary when focused, else accent.muted; the title in bold
  // text.emphasis when focused, else text.secondary; the inside on bg.surface.
  const border = slotStyle(focused ? "accent.primary" : "accent.muted").color;
  const surface = bgOf("bg.surface");
  const label = ` ${fit(title, Math.max(0, width - 6))} `;
  const rest = `${"─".repeat(Math.max(0, width - 2 - cellWidth(label)))}╮`;
  const titleStyle = focused ? { ...slotStyle("text.emphasis"), bold: true } : slotStyle("text.secondary");
  return (
    <Box flexDirection="column" width={width} height={height} flexShrink={0}>
      <Text wrap="truncate">
        <Text {...textProps({ color: border, backgroundColor: surface })}>╭</Text>
        <Text {...textProps({ ...titleStyle, backgroundColor: surface })}>{label}</Text>
        <Text {...textProps({ color: border, backgroundColor: surface })}>{rest}</Text>
      </Text>
      <BgBox borderStyle="round" borderTop={false} borderColor={border} borderBackgroundColor={surface} backgroundColor={surface} width={width} height={height - 1} overflow="hidden">
        <Box flexDirection="column" width={Math.max(0, width - 2)} height={Math.max(0, height - 2)} overflow="hidden">
          {/* Rows never shrink: a squeezed row keeps its text and would overprint its neighbour. */}
          <Box flexDirection="column" flexShrink={0} width={Math.max(0, width - 2)}>
            <ScrollSink.Provider value={report}>{children}</ScrollSink.Provider>
          </Box>
        </Box>
      </BgBox>
      {/* scroll::scrollbar: on the panel's right border between the corners, where a node scrolls (after the border so it paints over it) */}
      {bar && bar.max > 0 ? (
        // on the border's bg.surface, as Ratatui's bar is
        <BgBox position="absolute" top={1} left={width - 1} backgroundColor={surface}>
          <ScrollBar rows={height - 2} max={bar.max} at={bar.at} plain={plain} />
        </BgBox>
      ) : null}
    </Box>
  );
}

/** ui::card::shows: the pane's notes as a rule `─ shows ───` over the notes wrapped between words (one cell of air each
 * side), at most `most` rows with `…` after the last words when more is left; drawn only while the pane keeps three rows
 * above it. Returns the rows (rule first) or none. */
function showsRows(notes: any[][], inner: number, room: number, most: number, plain?: boolean): Cell[][] {
  if (notes.length === 0 || inner < 4) return [];
  const lines = notes.flatMap((n) => wrapCells(lineCells(n, plain), inner - 2));
  const height = Math.min(Math.max(1, Math.min(lines.length, Math.max(1, most))), Math.max(0, room - 4));
  if (height === 0) return [];
  const muted = slotStyle("text.muted");
  const label = cellsOf("─ shows ", muted);
  const rule = [...label, ...cellsOf("─".repeat(Math.max(0, inner - widthOfCells(label))), muted)];
  const shown = lines.slice(0, height).map((l) => padCells([...cellsOf(" "), ...l], inner));
  if (lines.length > height) {
    const row = shown[height - 1]!;
    let end = row.length;
    while (end > 0 && row[end - 1]!.ch.trim() === "") end--;
    row[Math.min(end, inner - 1)] = { ch: "…", w: 1, s: {} };
  }
  return [rule, ...shown];
}

function PaneContent({ pane, width, height, cx, renderNode }: { pane: Pane; width: number; height: number; cx: NodeCx; renderNode: RenderNode }) {
  const inner = Math.max(0, width - 2);
  const room = Math.max(0, height - 2);
  const most = (pane as any).notes_most ?? 2;
  const shows = showsRows(pane.notes ?? [], inner, room, most, cx.plainText);
  // view_tui: the footer is one row at the foot of what the notes leave, with a blank row above it, while more than two rows are left
  const left = room - shows.length;
  const footerRow = pane.footer && pane.footer.length > 0 && left > 2 ? padCells([...cellsOf(" "), ...clipCells(lineCells(pane.footer, cx.plainText), Math.max(0, inner - 2))], inner) : null;
  const tail = shows.length + (footerRow ? 2 : 0);
  // view_tui draw_pane: the top fields first (FIELD_ROWS each), then the body, the bottom field under it
  let used = 0;
  const tops = (pane.fields ?? []).flatMap((f) => {
    const r = fieldRows(f, inner, room - used, cx.plainText);
    used += Math.min(FIELD_ROWS, room - used);
    return r;
  });
  const bottom = pane.bottom_field && room - used - tail > 1 ? bottomFieldRow(pane.bottom_field, inner) : null;
  const bodyH = Math.max(1, room - used - tail - (bottom ? 1 : 0) - (pane.head && pane.head.length > 0 ? 1 : 0));
  return (
    <>
      {tops.map((r, i) => (
        <CellLine key={`f${i}`} cells={r} plain={cx.plainText} />
      ))}
      {/* view_tui draw_pane: the head sits a cell in from the left, clipped to the width less two */}
      {pane.head && pane.head.length > 0 ? <CellLine cells={padCells([...cellsOf(" "), ...clipCells(lineCells(pane.head, cx.plainText), Math.max(0, inner - 2))], inner)} plain={cx.plainText} /> : null}
      <Box flexDirection="column" flexGrow={1} flexShrink={1} overflow="hidden" width={inner} height={bodyH}>
        {/* PaneView.inset: the rows sit a cell in from each side (widgets::padded) */}
        <Box flexDirection="column" flexShrink={0} width={inner} paddingLeft={(pane as any).inset ? 1 : 0}>
          {renderNode(pane.body, { ...cx, width: (pane as any).inset ? Math.max(0, inner - 2) : inner, height: bodyH, focused: cx.focused })}
        </Box>
      </Box>
      {bottom ? <CellLine cells={bottom} plain={cx.plainText} /> : null}
      {footerRow ? <Text> </Text> : null}
      {footerRow ? <CellLine cells={footerRow} plain={cx.plainText} /> : null}
      {shows.map((r, i) => (
        <CellLine key={`s${i}`} cells={r} plain={cx.plainText} />
      ))}
    </>
  );
}

/** view_tui draw_pane: the title with the pane's icon before it (icon::with: the glyph and a space, nothing with icons off). */
export const paneTitle = (pane: { title: string; title_icon?: string | null }) => {
  const glyph = iconOf(pane.title_icon)?.glyph;
  return glyph ? `${glyph} ${pane.title}` : pane.title;
};

export function PaneFrame({ pane, width, height, focused, cx, renderNode }: { pane: Pane; width: number; height: number; focused: boolean; cx: NodeCx; renderNode: RenderNode }) {
  return (
    <Framed title={paneTitle(pane)} width={width} height={height} focused={focused} plain={cx.plainText} pane={pane.pane} registry={cx.registry}>
      <PaneContent pane={pane} width={width} height={height} cx={{ ...cx, focused }} renderNode={renderNode} />
    </Framed>
  );
}

/** The agent dock under the middle pane (src/dock): drawn by `render` in the room the split leaves it. */
export type DockSlot = { rows?: number | null; full?: boolean; render: (w: number, h: number) => React.ReactNode; onEdge?: (top: number) => void };

/** The screen's own panes in the universal layout (view_tui.rs render_panes). */
export function ScreenPanes({ screen, focus, width, height, cx, renderNode, dock }: { screen: any; focus?: string; width: number; height: number; cx: NodeCx; renderNode: RenderNode; dock?: DockSlot }) {
  const panes: Pane[] = screen.panes ?? [];
  const get = (id: string) => panes.find((p) => p.pane === id);
  const nav = get("nav");
  const mid = get("middle");
  const det = get("detail");
  // parts::setup_page draws its panel focused whoever has the keys; the host sends that page as one pane titled "set up <part>"
  // (view.rs setup_screen), whose id the host's focus need not name.
  const setUp = panes.length === 1 && String(panes[0]!.title).startsWith("set up ");
  const focusId = setUp ? panes[0]!.pane : (focus ?? "middle");
  const row = (list: Array<{ pane: Pane; w: number }>) => {
    // The dock sits under the middle pane (or the one pane a narrow screen shows); below 100 columns it covers it.
    const host = list.find((l) => l.pane.pane === "middle") ?? list.find((l) => l.pane.pane !== "nav");
    const split = dock ? splitDock(height, width, true, dock.rows, dock.full) : null;
    return (
      <Box flexDirection="row" width={width} height={height} overflow="hidden">
        {list.map(({ pane, w }) =>
          dock && split && split.dock > 0 && host?.pane === pane ? (
            <Box key={pane.pane} flexDirection="column" width={w} height={height} flexShrink={0}>
              {split.full ? null : <PaneFrame pane={pane} width={w} height={split.above} focused={focusId === pane.pane} cx={cx} renderNode={renderNode} />}
              {dock.render(w, split.dock)}
            </Box>
          ) : (
            <PaneFrame key={pane.pane} pane={pane} width={w} height={height} focused={focusId === pane.pane} cx={cx} renderNode={renderNode} />
          ),
        )}
      </Box>
    );
  };
  if (width >= WIDE) {
    const navW = nav ? (nav.width ?? 26) : 0;
    const detW = det ? detailWidth(width) : 0;
    const [a, b, c] = share(width, [{ length: navW }, { fill: 1 }, { length: detW }]);
    return row([nav && { pane: nav, w: a! }, mid && { pane: mid, w: b! }, det && { pane: det, w: c! }].filter(Boolean) as any);
  }
  if (width >= (screen.medium_from ?? MEDIUM)) {
    if (!nav && mid && det) {
      const [a, b] = share(width, [{ fill: 58 }, { fill: 42 }]);
      return row([{ pane: mid, w: a! }, { pane: det, w: b! }]);
    }
    const navW = nav ? Math.max(0, (nav.width ?? 26) - 2) : 0;
    const [a, b] = share(width, [{ length: navW }, { fill: 1 }]);
    const right = focusId === "detail" ? (det ?? mid) : mid;
    return row([nav && { pane: nav, w: a! }, right && { pane: right, w: b! }].filter(Boolean) as any);
  }
  const only = get(focusId) ?? mid ?? panes[0];
  return only ? row([{ pane: only, w: width }]) : null;
}

function layoutOf(page: any): any {
  return page.layout ?? null;
}

/** An item page: crumbs, title + details, the line under, the tab row, then the panes in the page's layout. */
export function PageBody({ page, focus, width, height, cx, renderNode }: { page: any; focus?: string; width: number; height: number; cx: NodeCx; renderNode: RenderNode }) {
  const crumbs: any[] = page.crumbs ?? [];
  const title: any[] = page.title ?? [];
  const details: any[] = page.details ?? [];
  const tabs: Array<{ label: string; icon?: string | null }> = page.tabs ?? [];
  const head: React.ReactNode[] = [];
  head.push(
    <Box key="crumbs" width={width} flexShrink={0} backgroundColor={bgOf("bg.base")}>
    <Text wrap="truncate">
      {" "}
      {crumbs.map((c, i) => (
        <Text key={i}>
          {i > 0 ? <Text {...textProps(slotStyle("text.muted"))}>{" › "}</Text> : null}
          <SegRun seg={{ ...c, strong: c.strong || i === crumbs.length - 1 }} plain={cx.plainText} />
        </Text>
      ))}
    </Text>
    </Box>,
  );
  head.push(<PageTitleLine key="title" title={title} details={details} width={width} plain={cx.plainText} base={bgOf("bg.base")} />);
  // Ratatui clips this line at the frame's edge with no ellipsis: a wide row inside a box that hides what overflows.
  if (page.under) head.push(<Box key="under" width={Math.max(0, width - 1)} overflow="hidden"><Box width={1000} flexShrink={0}><BgBox flexShrink={0} backgroundColor={bgOf("bg.base")}><Text> </Text><TextLine text={page.under} plain={cx.plainText} /></BgBox></Box></Box>);
  if (tabs.length > 0) {
    head.push(<TabRow key="tabs" base={bgOf("bg.base")} labels={tabs.map((t) => `${iconOf(t.icon)?.glyph ? `${iconOf(t.icon)!.glyph} ` : ""}${t.label}`)} on={page.tab} focused={!!cx.tabsFocused} width={Math.max(0, width - 2)} registry={cx.registry} />);
  }
  if (page.notice) head.push(<BgBox key="notice" backgroundColor={bgOf("bg.base")}><Text> </Text><TextLine text={page.notice} plain={cx.plainText} /></BgBox>);
  const bodyH = Math.max(0, height - head.length);
  const panes: Pane[] = (page.panes ?? []).filter((p: Pane) => p.from == null || width >= p.from);
  const focusId = focus ?? "middle";
  const draw = (index: number, w: number, h: number) => {
    const p = (page.panes ?? [])[index] as Pane | undefined;
    return p ? <PaneFrame key={`${index}`} pane={p} width={w} height={h} focused={focusId === p.pane} cx={cx} renderNode={renderNode} /> : null;
  };
  const place = (layout: any, w: number, h: number): React.ReactNode => {
    if (!layout) return null;
    if (layout.kind === "pane") return draw(layout.index, w, h);
    if (layout.kind === "responsive") return place(w >= layout.min_width ? layout.wide : layout.narrow, w, h);
    if (layout.kind === "split") {
      const horizontal = layout.axis === "horizontal";
      const sizes = share(
        horizontal ? w : h,
        layout.parts.map((p: any) => (p.size.kind === "length" ? { length: p.size.value } : { fill: p.size.value })),
      );
      return (
        <Box flexDirection={horizontal ? "row" : "column"} width={w} height={h} overflow="hidden">
          {layout.parts.map((p: any, i: number) => (
            <Box key={i} width={horizontal ? sizes[i] : w} height={horizontal ? h : sizes[i]} overflow="hidden" flexShrink={0}>
              {place(p.layout, horizontal ? sizes[i]! : w, horizontal ? h : sizes[i]!)}
            </Box>
          ))}
        </Box>
      );
    }
    return null;
  };
  let body: React.ReactNode;
  if (layoutOf(page)) body = place(page.layout, width, bodyH);
  else {
    const sizes = share(width, panes.map((p) => (p.width ? { length: p.width } : { fill: 1 })));
    body = (
      <Box flexDirection="row" width={width} height={bodyH} overflow="hidden">
        {panes.map((p, i) => (
          <PaneFrame key={i} pane={p} width={sizes[i]!} height={bodyH} focused={focusId === p.pane} cx={cx} renderNode={renderNode} />
        ))}
      </Box>
    );
  }
  return (
    <Box flexDirection="column" width={width} height={height} overflow="hidden">
      {/* ui::page::PageFrame draws on the page's bg.base: a cell of these rows no line writes (after the line under the
          title, the tab row's last column, after the notice) shows it, as in Ratatui */}
      <Box flexDirection="column" width={width} flexShrink={0} backgroundColor={bgOf("bg.base")}>
        {head}
      </Box>
      {body}
    </Box>
  );
}

export { segWords };
