import React, { createContext, useCallback, useContext, useLayoutEffect, useRef, useState, useId } from "react";
import { Box, Text, type DOMElement } from "ink";
import { ScrollBar, ScrollSink, screenAt, type ScrollReport } from "../nodes/index.js";
import { TextLine, SegRun } from "../text/TextLine.js";
import { textWords, segWords } from "../text/TextLine.js";
import { cellWidth, fit } from "../chrome/width.js";
import type { NodeCx, RenderNode } from "./Body.js";
import { PageTitleLine } from "../nodes/pagetitle.js";
import { bgOf, currentLook, iconOf, slotStyle } from "../text/look.js";
import { focusRamp, frameRuns } from "../text/frame.js";
import { motion } from "../motion/motion.js";
import { MotionSceneContext, MotionPaneContext } from "../motion/scene.js";
import { textProps } from "../text/TextLine.js";
import { TabRow } from "./TabRow.js";
import { segHits, useLineHits } from "../nodes/lineHits.js";
import type { TabView } from "../protocol/generated/TabView.js";
import { bottomFieldRow, fieldRows, FIELD_ROWS } from "../chrome/filter.js";
import { splitDock } from "../dock/layout.js";
import { BgBox, CellLine, cellsOf, clipCells, lineCells, padCells, widthOfCells, wrapCells, type Cell } from "../text/cells.js";
import type { FootActions } from "../nodes/view.js";
import { footActionLayout, type FootLayout } from "../nodes/detail.js";
import { insideOf, selectKindOf, type SelectArea, type SelectKind } from "../input/selection.js";

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
  foot_actions?: FootActions | null;
  head?: any[] | null;
  /** The filter boxes above the body (the '/' input) and the one below it (chrome/filter.tsx draws them). */
  fields?: any[];
  bottom_field?: any | null;
  width?: number | null;
  from?: number | null;
  /** Where the part moved the body's scroll (PaneView.scroll). */
  scroll?: number | null;
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
export function PaneArea({ pane, registry, width, height, select, selectRight = 0, children }: { pane?: string; registry?: NodeCx["registry"]; width: number; height: number; select?: SelectKind; selectRight?: number; children?: React.ReactNode }) {
  const ref = useRef<DOMElement>(null);
  useLayoutEffect(() => {
    const at = pane && (registry?.addPane || (select && registry?.addSelect)) ? screenAt(ref.current) : null;
    if (at && registry?.addPane) registry.addPane({ x: at.x, y: at.y, w: width, h: height, pane: pane! });
    // Text is selected inside the frame: not its border, its title row or a scrollbar column (`selectRight`).
    if (at && select && registry?.addSelect) registry.addSelect({ ...insideOf({ x: at.x, y: at.y, w: width, h: height }, selectRight), id: `pane:${pane}`, kind: select });
  });
  return (
    <Box ref={ref} flexDirection="column" width={width} height={height} flexShrink={0}>
      {children}
    </Box>
  );
}

/** Where a selection area is registered (HitRegistry.addSelect). */
export type SelectSink = { addSelect?(area: SelectArea): void };

/** A framed box `width` x `height` whose inside text is selected with the mouse (a work area's section). Draws only its
 * children. */
export function SelectFrame({ registry, id, kind, width, height, children }: { registry?: SelectSink; id: string; kind: SelectKind; width: number; height: number; children?: React.ReactNode }) {
  const ref = useRef<DOMElement>(null);
  useLayoutEffect(() => {
    const at = registry?.addSelect ? screenAt(ref.current) : null;
    if (at) registry!.addSelect!({ ...insideOf({ x: at.x, y: at.y, w: width, h: height }), id, kind });
  });
  return (
    <Box ref={ref} flexDirection="column" width={width} height={height} flexShrink={0}>
      {children}
    </Box>
  );
}

/** Eligibility for the decorative ramp only. Legacy focus/title/selection styling is unchanged. */
export const FrameRampOwnership = createContext<{ body: boolean; dock?: boolean; pane?: string }>({ body: true });
type RampOwner = "body" | "dock" | false;

export function Framed({ title, width, height, focused, plain, pane, motionPane, registry, select, rampOwner = "body", children }: { title: string; width: number; height: number; focused?: boolean; plain?: boolean; pane?: string; motionPane?: string; registry?: NodeCx["registry"]; select?: SelectKind; rampOwner?: RampOwner; children?: React.ReactNode }) {
  const owners = useContext(FrameRampOwnership);
  const owned = rampOwner === "dock" ? (owners.dock ?? !!focused) : rampOwner === "body" && owners.body && (owners.pane !== undefined && pane !== undefined ? pane === owners.pane : !!focused);
  const ramp = owned ? focusRamp() : undefined;
  const scene = useContext(MotionSceneContext);
  const markId = useId();
  const frameRef = useRef<DOMElement>(null);
  const framePane = rampOwner === "dock" ? "dock" : motionPane ?? pane;
  const [bar, setBar] = useState<ScrollReport>(null);
  const report = useCallback((r: ScrollReport) => setBar((p) => (p === r || (p && r && p.max === r.max && p.at === r.at && p.bodyTop === r.bodyTop && p.bodyHeight === r.bodyHeight) ? p : r)), []);
  const frameY = screenAt(frameRef.current)?.y;
  // card::above keeps the original frame top but ends its track at the body bottom.
  const trackRows = bar?.bodyTop != null && bar.bodyHeight != null && frameY != null
    ? Math.max(0, Math.min(height - 2, bar.bodyTop + bar.bodyHeight - frameY - 1))
    : Math.max(0, height - 2);
  useLayoutEffect(() => {
    if (!scene || registry || !framePane || rampOwner === false || width < 4 || height < 2 || motion.engine.level !== "full") return;
    const at = screenAt(frameRef.current);
    if (!at) return;
    const labelWidth = cellWidth(` ${fit(title, Math.max(0, width - 6))} `);
    motion.markPane(markId, scene, {
      pane: framePane, owns: !!owned, area: { x: at.x, y: at.y, w: width, h: height },
      title: { x: at.x + 1, y: at.y, w: labelWidth, h: 1 },
      keep: bar && bar.max > 0 ? [{ x: at.x + width - 1, y: at.y + 1, w: 1, h: trackRows }] : [],
    });
  });
  useLayoutEffect(() => () => { if (scene) motion.markPane(markId, scene); }, [markId, scene]);
  if (width < 3 || height < 2) return null;
  if (pane && registry) {
    return (
      <PaneArea pane={pane} registry={registry} width={width} height={height} select={select}>
        <Framed title={title} width={width} height={height} focused={focused} plain={plain} pane={pane} motionPane={motionPane} rampOwner={rampOwner}>
          {children}
        </Framed>
      </PaneArea>
    );
  }
  // widgets::panel: a rounded border in accent.primary when focused, else accent.muted; the title in bold
  // text.emphasis when focused, else text.secondary; the inside on bg.surface.
  const borderFocus = currentLook().tier === "truecolor" && rampOwner !== false ? owned : focused;
  const border = slotStyle(borderFocus ? "accent.primary" : "accent.muted").color;
  const surface = bgOf("bg.surface");
  const label = width === 3 ? " " : ` ${fit(title, Math.max(0, width - 6))} `;
  const rest = `${"─".repeat(Math.max(0, width - 2 - cellWidth(label)))}╮`;
  const titleStyle = focused ? { ...slotStyle("text.emphasis"), bold: true } : slotStyle("text.secondary");
  const edge = (text: string, x: number, y: number) => frameRuns(text, width, height, x, y, { color: border, backgroundColor: surface }, ramp).map((r, i) => <Text key={i} {...textProps(r.style)}>{r.text}</Text>);
  return (
    <MotionPaneContext.Provider value={framePane}>
    <Box ref={frameRef} flexDirection="column" width={width} height={height} flexShrink={0}>
      <Text wrap="truncate">
        {edge("╭", 0, 0)}
        <Text {...textProps({ ...titleStyle, backgroundColor: surface })}>{label}</Text>
        {edge(rest, 1 + cellWidth(label), 0)}
      </Text>
      <BgBox borderStyle="round" borderTop={false} borderColor={border} borderBackgroundColor={surface} backgroundColor={surface} width={width} height={height - 1} overflow="hidden">
        <Box flexDirection="column" width={Math.max(0, width - 2)} height={Math.max(0, height - 2)} overflow="hidden">
          {/* Rows never shrink: a squeezed row keeps its text and would overprint its neighbour. */}
          <Box flexDirection="column" flexShrink={0} width={Math.max(0, width - 2)}>
            <ScrollSink.Provider value={report}>{children}</ScrollSink.Provider>
          </Box>
        </Box>
      </BgBox>
      {/* Repaint only existing border cells. Absolute overlays preserve Yoga content geometry and all hit boxes.
          The scrollbar is painted afterwards, so its original track/thumb colours remain authoritative. */}
      {ramp ? <>
        {Array.from({ length: Math.max(0, height - 2) }, (_, i) => (
          <React.Fragment key={i}>
            <Box position="absolute" top={i + 1} left={0} width={1} height={1}><Text>{edge("│", 0, i + 1)}</Text></Box>
            <Box position="absolute" top={i + 1} left={width - 1} width={1} height={1}><Text>{edge("│", width - 1, i + 1)}</Text></Box>
          </React.Fragment>
        ))}
        <Box position="absolute" top={height - 1} left={0} width={width} height={1}>
          <Text wrap="truncate">{edge(`╰${"─".repeat(width - 2)}╯`, 0, height - 1)}</Text>
        </Box>
      </> : null}
      {/* scroll::scrollbar: on the panel's right border between the corners, where a node scrolls (after the border so it paints over it) */}
      {bar && bar.max > 0 ? (
        // on the border's bg.surface, as Ratatui's bar is
        <BgBox position="absolute" top={1} left={width - 1} backgroundColor={surface}>
          <ScrollBar rows={trackRows} max={bar.max} at={bar.at} plain={plain} />
        </BgBox>
      ) : null}
    </Box>
    </MotionPaneContext.Provider>
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
  const available = Math.max(0, room - used - tail - (bottom ? 1 : 0) - (pane.head && pane.head.length > 0 ? 1 : 0));
  const foot = footActionLayout(pane.foot_actions, inner, available, cx.plainText, cx.writeMode);
  const bodyH = available - foot.rows.length;
  const bodyRef = useRef<DOMElement>(null);
  useLayoutEffect(() => {
    const at = screenAt(bodyRef.current);
    if (pane.foot_actions && at && bodyH > 0 && inner > 0) cx.registry?.addSelect?.({ ...at, w: inner, h: bodyH, id: `pane:${pane.pane}:body`, kind: selectKindOf(pane.body) });
  });
  return (
    <>
      {tops.map((r, i) => (
        <CellLine key={`f${i}`} cells={r} plain={cx.plainText} />
      ))}
      {/* view_tui draw_pane: the head sits a cell in from the left, clipped to the width less two */}
      {pane.head && pane.head.length > 0 ? <CellLine cells={padCells([...cellsOf(" "), ...clipCells(lineCells(pane.head, cx.plainText), Math.max(0, inner - 2))], inner)} plain={cx.plainText} /> : null}
      <Box ref={bodyRef} flexDirection="column" flexGrow={1} flexShrink={1} overflow="hidden" width={inner} height={bodyH}>
        {/* PaneView.inset: the rows sit a cell in from each side (widgets::padded) */}
        <Box flexDirection="column" flexShrink={0} width={inner} paddingLeft={(pane as any).inset ? 1 : 0}>
          {bodyH > 0 ? renderNode(pane.body, { ...cx, width: (pane as any).inset ? Math.max(0, inner - 2) : inner, height: bodyH, focused: cx.focused, bodyTrack: !!pane.foot_actions }) : null}
        </Box>
      </Box>
      <DetailFoot layout={foot} width={inner} cx={cx} />
      {bottom ? <CellLine cells={bottom} plain={cx.plainText} /> : null}
      {footerRow ? <Text> </Text> : null}
      {footerRow ? <CellLine cells={footerRow} plain={cx.plainText} /> : null}
      {shows.map((r, i) => (
        <CellLine key={`s${i}`} cells={r} plain={cx.plainText} />
      ))}
    </>
  );
}

/** Registers only complete control words, after the body has reserved exactly these rows. */
function DetailFoot({ layout, width, cx }: { layout: FootLayout; width: number; cx: NodeCx }) {
  const ref = useRef<DOMElement>(null);
  const owner = useId();
  useLayoutEffect(() => {
    const at = screenAt(ref.current);
    if (!cx.registry) return;
    const hits = at ? layout.hits.map((hit) => ({ x: at.x + hit.dx, y: at.y + hit.dy, w: hit.w, h: hit.h, target: hit.target })) : [];
    if (cx.registry.replace) cx.registry.replace(owner, hits);
    else hits.forEach((hit) => cx.registry!.add(hit));
  });
  useLayoutEffect(() => () => cx.registry?.remove?.(owner), [cx.registry, owner]);
  if (!layout.rows.length) return null;
  return <Box ref={ref} flexDirection="column" width={width} height={layout.rows.length} flexShrink={0}>
    {layout.rows.map((cells, i) => <CellLine key={i} cells={cells} plain={cx.plainText} />)}
  </Box>;
}

/** Numbers come solely from the host's actual routing map; absence never guesses an ordinal. */
export const paneTitle = (pane: { pane?: string; title: string; title_icon?: string | null }, jumps: NodeCx["paneJumps"] = []) => {
  const glyph = iconOf(pane.title_icon)?.glyph;
  const title = glyph ? `${glyph} ${pane.title}` : pane.title;
  const key = jumps.find((j) => j.pane === pane.pane)?.key;
  return key ? `${key} ${title}` : title;
};

export function PaneFrame({ pane, width, height, focused, cx, renderNode }: { pane: Pane; width: number; height: number; focused: boolean; cx: NodeCx; renderNode: RenderNode }) {
  return (
    <Framed title={paneTitle(pane, cx.paneJumps)} width={width} height={height} focused={focused} plain={cx.plainText} pane={pane.pane} registry={cx.registry} select={pane.foot_actions ? undefined : selectKindOf(pane.body)}>
      <PaneContent pane={pane} width={width} height={height} cx={{ ...cx, focused }} renderNode={renderNode} />
    </Framed>
  );
}

/** The agent dock under the middle pane (src/dock): drawn by `render` in the room the split leaves it. */
export type DockSlot = { rows?: number | null; full?: boolean; render: (w: number, h: number) => React.ReactNode; onEdge?: (top: number) => void };

/** Existing selected row target plus scope, independent of colliding item captions. */
export function detailScrollIdentity(screen: any): string {
  const body = screen.panes?.find((pane: any) => pane.pane === "middle")?.body;
  const selected = body?.node === "tree" ? (body.selected ?? []).join("/")
    : ["list", "table", "entries"].includes(body?.node) ? body.rows?.[body.selected]?.target ?? "" : "";
  return JSON.stringify([screen.scope ?? "", selected]);
}

/** The screen's own panes in the universal layout (view_tui.rs render_panes). */
export function ScreenPanes({ screen, focus, width, height, cx, renderNode, dock }: { screen: any; focus?: string; width: number; height: number; cx: NodeCx; renderNode: RenderNode; dock?: DockSlot }) {
  const panes: Pane[] = screen.panes ?? [];
  // A screen pane's body reports what it laid out for its pane (ToHost::Rows) and goes where the part moved it.
  const paneCx = (pane: Pane): NodeCx => ({
    ...cx,
    pane: pane.pane,
    scrollTo: pane.scroll ?? undefined,
    ...(pane.pane === "detail" ? { scrollIdentity: detailScrollIdentity(screen) } : {}),
  });
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
              {split.full ? null : <PaneFrame pane={pane} width={w} height={split.above} focused={focusId === pane.pane} cx={paneCx(pane)} renderNode={renderNode} />}
              {dock.render(w, split.dock)}
            </Box>
          ) : (
            <PaneFrame key={pane.pane} pane={pane} width={w} height={height} focused={focusId === pane.pane} cx={paneCx(pane)} renderNode={renderNode} />
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

/** The page's breadcrumbs: the last one strong, a chip among them a hit (the separators and the leading space are not). */
function CrumbsLine({ crumbs, width, cx }: { crumbs: any[]; width: number; cx: NodeCx }) {
  const ref = useRef<DOMElement>(null);
  const flat = crumbs.flatMap((c, i) => (i > 0 ? [{ text: " / " }, c] : [c]));
  useLineHits(ref, cx.registry, segHits(flat, cx.plainText, 1, width));
  return (
    <Box ref={ref} width={width} flexShrink={0} backgroundColor={bgOf("bg.base")}>
      <Text wrap="truncate">
        {" "}
        {crumbs.map((c, i) => (
          <Text key={i}>
            {i > 0 ? <Text {...textProps(slotStyle("text.muted"))}>{" / "}</Text> : null}
            <SegRun seg={{ ...c, strong: c.strong || i === crumbs.length - 1 }} plain={cx.plainText} />
          </Text>
        ))}
      </Text>
    </Box>
  );
}

/** Ratatui clips this line at the frame's edge with no ellipsis: a wide row inside a box that hides what overflows. */
function UnderLine({ text, width, cx }: { text: any[]; width: number; cx: NodeCx }) {
  const ref = useRef<DOMElement>(null);
  const room = Math.max(0, width - 1);
  useLineHits(ref, cx.registry, segHits(text, cx.plainText, 1, room));
  return (
    <Box width={room} overflow="hidden">
      <Box width={1000} flexShrink={0}>
        <BgBox ref={ref} flexShrink={0} backgroundColor={bgOf("bg.base")}>
          <Text> </Text>
          <TextLine text={text} plain={cx.plainText} />
        </BgBox>
      </Box>
    </Box>
  );
}

function NoticeLine({ text, width, cx }: { text: any[]; width: number; cx: NodeCx }) {
  const ref = useRef<DOMElement>(null);
  useLineHits(ref, cx.registry, segHits(text, cx.plainText, 1, width));
  return (
    <BgBox ref={ref} backgroundColor={bgOf("bg.base")}>
      <Text> </Text>
      <TextLine text={text} plain={cx.plainText} />
    </BgBox>
  );
}

/** An item page: crumbs, title + details, the line under, the tab row, then the panes in the page's layout. */
export function PageBody({ page, focus, width, height, cx, renderNode }: { page: any; focus?: string; width: number; height: number; cx: NodeCx; renderNode: RenderNode }) {
  const crumbs: any[] = page.crumbs ?? [];
  const title: any[] = page.title ?? [];
  const details: any[] = page.details ?? [];
  const tabs: TabView[] = page.tabs ?? [];
  const head: React.ReactNode[] = [];
  head.push(<CrumbsLine key="crumbs" crumbs={crumbs} width={width} cx={cx} />);
  head.push(<PageTitleLine key="title" title={title} details={details} width={width} plain={cx.plainText} base={bgOf("bg.base")} registry={cx.registry} />);
  // Ratatui clips this line at the frame's edge with no ellipsis: a wide row inside a box that hides what overflows.
  if (page.under) head.push(<UnderLine key="under" text={page.under} width={width} cx={cx} />);
  if (tabs.length > 0) {
    head.push(<TabRow key="tabs" base={bgOf("bg.base")} tabs={tabs} on={page.tab} focused={!!cx.tabsFocused} width={Math.max(0, width - 2)} registry={cx.registry} />);
  }
  if (page.notice) head.push(<NoticeLine key="notice" text={page.notice} width={width} cx={cx} />);
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
