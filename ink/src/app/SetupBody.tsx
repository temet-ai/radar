import React, { useCallback, useLayoutEffect, useRef, useState } from "react";
import { Box, type DOMElement } from "ink";
import { ScrollBar, ScrollSink, detailRows, screenAt, type ScrollReport } from "../nodes/index.js";
import { CellRow } from "../chrome/Chrome.js";
import { cellWidth } from "../chrome/width.js";
import { BgBox } from "../text/cells.js";
import { slotStyle, type Style } from "../text/look.js";
import { MotionPaneContext } from "../motion/scene.js";
import { selectKindOf, type SelectKind } from "../input/selection.js";
import type { NodeCx, RenderNode } from "./Body.js";

/**
 * Setup (crates/radar-core/src/shell/setup.rs `frame`, `areas`, `about_rows`, `tabbed`, `body`): at most 140 x 40
 * cells, the body inset a cell each side and the rest centred. Inside it: a blank row for the sheet's tab to stand on,
 * the sheet ("about this step", as high as its text plus a row, at most 12 and a third of the frame, at least 2), a
 * blank row for the card's tab, then the card filling the rest: read first, then choose, at every width. No borders:
 * each is a filled panel under a square tab (` title `, bold, bg.base on the slot's colour). The card's heading and the
 * line under it come first, then a blank row, then the section's rows; the sheet's lines fill its inside, its scrollbar
 * on the panel's right edge. The canvas under all of it is bg.base.
 */

/** The boards' size: setup never grows past it. */
export const MOST_WIDE = 140;
export const MOST_HIGH = 40;
/** The sheet's most rows. */

export type Box4 = { x: number; y: number; w: number; h: number };
export type SetupAreas = { frame: Box4; card: Box4; sheet: Box4 };

/** shell/setup.rs `areas`, relative to the body's top left; `aboutRows` is the rows the sheet's text wraps to at the
 * frame's width less four (`about_rows`). */
/** The rows the card keeps below the about sheet, where the frame has them (shell/setup.rs CARD_LEAST). */
export const CARD_LEAST = 22;

export function setupAreas(width: number, height: number, aboutRows = 0): SetupAreas {
  const insideW = Math.max(0, width - 2);
  const w = Math.min(insideW, MOST_WIDE);
  const h = Math.min(height, MOST_HIGH);
  const frame: Box4 = { x: 1 + Math.floor((insideW - w) / 2), y: Math.floor((height - h) / 2), w, h };
  const about = Math.max(2, Math.min(aboutRows + 1, Math.max(Math.floor(h / 3), Math.max(0, h - CARD_LEAST))));
  const sheetH = Math.max(0, Math.min(about, h - 1));
  const sheetY = frame.y + Math.min(1, h);
  // A blank row between the sheet and the card's tab (shell/setup.rs areas).
  const cardY = sheetY + sheetH + 2;
  return {
    frame,
    sheet: { x: frame.x, y: sheetY, w, h: sheetH },
    card: { x: frame.x, y: Math.min(cardY, frame.y + h), w, h: Math.max(0, frame.y + h - cardY) },
  };
}

/** shell/setup.rs `tabbed`: the inside of a panel, two cells in and a row down. */
export const insideOfPanel = (a: Box4): Box4 => ({ x: a.x + 2, y: a.y + 1, w: Math.max(0, a.w - 4), h: Math.max(0, a.h - 1) });

/** The card's heading and the line under it, each cut to the inside width (set_stringn cuts, never adds `…`). */
function cut(words: string, w: number): string {
  let out = "";
  let used = 0;
  for (const ch of words) {
    const cw = cellWidth(ch);
    if (used + cw > w) break;
    out += ch;
    used += cw;
  }
  return out;
}

/** The tab over a panel: ` title ` cut to the panel's width, bold bg.base on `ground`'s colour. */
function chip(title: string, width: number, ground: string): { text: string; style: Style } {
  return { text: cut(` ${title} `, width), style: { color: slotStyle("bg.base").color, backgroundColor: slotStyle(ground).color, bold: true } };
}

function Panel({ area, fill, pane, kind, registry, bar: barAt, children, selectAt }: {
  area: Box4;
  fill: string;
  pane: "middle" | "detail";
  kind: SelectKind;
  registry?: NodeCx["registry"];
  /** Where the scrollbar runs, relative to the panel: the sheet's on the panel's right edge between the corners
   * (ui::scroll::paragraph), the card's on the cell after its rows (ui::scroll::scrollbar_in). */
  bar: { left: number; top: number; rows: number };
  /** Where the text selected with the mouse lives, relative to the panel. */
  selectAt: Box4;
  children: (sink: ((r: ScrollReport) => void) | null) => React.ReactNode;
}) {
  const [bar, setBar] = useState<ScrollReport>(null);
  const report = useCallback((r: ScrollReport) => setBar((p) => (p === r || (p && r && p.max === r.max && p.at === r.at) ? p : r)), []);
  const ref = useRef<DOMElement>(null);
  useLayoutEffect(() => {
    const at = registry && (registry.addPane || registry.addSelect) ? screenAt(ref.current) : null;
    if (!at || !registry) return;
    registry.addPane?.({ x: at.x, y: at.y, w: area.w, h: area.h, pane });
    registry.addSelect?.({ x: at.x + selectAt.x, y: at.y + selectAt.y, w: selectAt.w, h: selectAt.h, id: `pane:${pane}`, kind });
  });
  const fillColour = slotStyle(fill).color;
  const edge = bar && bar.max > 0 && barAt.rows > 0;
  return (
    <MotionPaneContext.Provider value={pane}>
    <Box ref={ref} flexShrink={0} width={area.w} height={area.h}>
    <BgBox flexDirection="column" flexShrink={0} width={area.w} height={area.h} backgroundColor={fillColour}>
      {children(report)}
      {edge ? (
        <BgBox position="absolute" top={barAt.top} left={barAt.left} backgroundColor={fillColour}>
          <ScrollBar rows={barAt.rows} max={bar!.max} at={bar!.at} />
        </BgBox>
      ) : null}
    </BgBox>
    </Box>
    </MotionPaneContext.Provider>
  );
}

export function SetupBody(p: {
  section: any;
  width: number;
  height: number;
  cx: NodeCx;
  renderNode: RenderNode;
  title: string;
  heading?: string;
  lead?: string;
  detailTitle: string;
}) {
  const { section, width, height, cx, renderNode } = p;
  const ground = slotStyle("bg.base").color;
  const detailNode = section.detail_blocks?.length ? { node: "rich", blocks: section.detail_blocks } : { node: "document", lines: section.detail ?? [] };
  // about_rows: the sheet's text wrapped at the frame's width less four, which is the width its node is given.
  const frameW = Math.min(Math.max(0, width - 2), MOST_WIDE);
  const areas = setupAreas(width, height, detailRows(detailNode as any, Math.max(0, frameW - 4)));
  const { frame, card, sheet } = areas;

  const cardIn = insideOfPanel(card);
  const sheetIn = insideOfPanel(sheet);
  const head: Array<[string, Style]> = [];
  if (p.heading) head.push([p.heading, { ...slotStyle("text.emphasis"), bold: true }]);
  if (p.lead) head.push([p.lead, slotStyle("text.muted")]);
  const shown = head.slice(0, cardIn.h);
  const skip = shown.length > 0 ? shown.length + 1 : 0;
  const listH = Math.max(0, cardIn.h - skip);

  const cardChip = chip(p.title, card.w, "accent.primary");
  const sheetChip = chip(p.detailTitle, sheet.w, "glow.cyan");
  // The row a tab stands on: the tab from the panel's left edge, the rest blank.
  const tabRow = (chipOf: { text: string; style: Style }, w: number) => {
    const out: Array<{ text: string; style?: Style }> = [{ text: chipOf.text, style: chipOf.style }];
    const x = cellWidth(chipOf.text);
    if (w > x) out.push({ text: " ".repeat(w - x) });
    return <CellRow cells={out} />;
  };

  const cardPanel = (
    <Panel area={card} fill="bg.surface" pane="middle" kind={selectKindOf(section.body)} registry={cx.registry} bar={{ left: cardIn.x - card.x + cardIn.w, top: cardIn.y - card.y + skip, rows: listH }} selectAt={{ x: cardIn.x - card.x, y: cardIn.y - card.y + skip, w: cardIn.w, h: listH }}>
      {(sink) => (
        <Box marginLeft={2} marginTop={1} width={cardIn.w} height={cardIn.h} flexDirection="column" flexShrink={0} overflow="hidden">
          {skip > 0 ? (
            <Box flexDirection="column" height={skip} flexShrink={0}>
              {shown.map(([words, style], i) => (
                <CellRow key={i} cells={[{ text: cut(words, cardIn.w), style }]} />
              ))}
            </Box>
          ) : null}
          <ScrollSink.Provider value={sink}>
            {listH > 0
              ? renderNode(section.body, { ...cx, focused: true, scrolls: section.body?.node === "document", sheet: false, width: cardIn.w, height: listH })
              : null}
          </ScrollSink.Provider>
        </Box>
      )}
    </Panel>
  );
  const sheetPanel = (
    <Panel area={sheet} fill="bg.overlay" pane="detail" kind={selectKindOf(detailNode)} registry={cx.registry} bar={{ left: sheet.w - 1, top: 1, rows: Math.max(0, sheet.h - 2) }} selectAt={{ x: sheetIn.x - sheet.x, y: sheetIn.y - sheet.y, w: sheetIn.w, h: sheetIn.h }}>
      {(sink) => (
        // The node insets its lines a cell (widgets::paragraph); the sheet's lines start at the inside's edge and are
        // laid for its width (section_detail_lines), so the node starts a cell before the inside.
        <Box marginLeft={1} marginTop={1} width={sheetIn.w} height={sheetIn.h} flexDirection="column" flexShrink={0} overflow="hidden">
          <ScrollSink.Provider value={sink}>
            {renderNode(detailNode, { ...cx, focused: false, scrolls: detailNode.node === "document", sheet: true, width: sheetIn.w, height: Math.max(1, sheetIn.h) })}
          </ScrollSink.Provider>
        </Box>
      )}
    </Panel>
  );

  return (
    <BgBox flexDirection="column" width={width} height={height} overflow="hidden" backgroundColor={ground}>
      <Box marginLeft={frame.x} marginTop={frame.y} width={frame.w} height={frame.h} flexDirection="column" flexShrink={0}>
        {frame.h > 0 ? tabRow(sheetChip, frame.w) : null}
        {sheetPanel}
        {card.y - 2 > sheet.y + sheet.h - 1 && card.y - 2 < frame.y + frame.h ? <Box height={1} flexShrink={0} /> : null}
        {card.y - 1 < frame.y + frame.h ? tabRow(cardChip, frame.w) : null}
        {cardPanel}
      </Box>
    </BgBox>
  );
}
