import React, { useRef, useContext, useId, useLayoutEffect } from "react";
import { Box, Text, Transform } from "ink";
import { layerMark } from "../app/scrim.js";
import { inkStyle } from "../text/theme.js";
import { textProps } from "../text/TextLine.js";
import { Rows, useBg } from "../text/cells.js";
import { bgOf, slotStyle, type Style } from "../text/look.js";
import { selectedStyle } from "../nodes/tree.js";
import { ScrollBar } from "../nodes/index.js";
import {
  layoutMenu,
  layoutNav,
  layoutSetupTopBar,
  layoutSources,
  layoutStatus,
  layoutTopBar,
  MENU_BOX,
  type Cell,
  type Laid,
  type MenuLayout,
} from "./layout.js";
import { cellWidth, clipSegments } from "./width.js";
import type { HistoryLayout } from "./history.js";
import type { RailLaid } from "./rail.js";
import { layoutTour, plainTourText } from "./tour.js";
import { TextLine } from "../text/TextLine.js";
import { paneTitle, Framed } from "../app/Panes.js";
import { motion } from "../motion/motion.js";
import { MotionSceneContext } from "../motion/scene.js";
import { settingsTreeMark } from "../motion/node-marks.js";
import type { ChromeView, Hit, MenuView } from "./types.js";
import type { TourCard } from "../protocol/generated/TourCard.js";

export const styleOf = (c: Cell): Style => ({
  ...inkStyle(c.ink),
  ...c.style,
  ...(c.bold ? { bold: true } : {}),
  ...(c.underline ? { underline: true } : {}),
});

export function CellRow({ cells }: { cells: Cell[] }) {
  const bg = useBg();
  // A row with the same cells as last time is the same element (text/cells.tsx CellBlock does the same for the body).
  const last = useRef<{ key: string; el: React.ReactElement } | null>(null);
  const key = `${bg ?? ""}|${JSON.stringify(cells)}`;
  if (last.current?.key === key) return last.current.el;
  const el = rowElement(cells, bg);
  last.current = { key, el };
  return el;
}

function rowElement(cells: Cell[], bg: string | undefined): React.ReactElement {
  const rows = new Rows(false, bg).line();
  for (const c of cells)
    rows.put(c.text, {
      ...styleOf(c),
      ...(c.inverse ? { inverse: true } : {}),
    });
  return rows.element("truncate-end") ?? <Text> </Text>;
}

/** Where the chrome sits on screen: top bar row 0, sources row 1 (if any), status on the last row. */
export type ChromeFrame = {
  width: number;
  height: number;
  top: Laid;
  sources: Laid | null;
  status: Laid;
  bodyTop: number;
  bodyHeight: number;
  navWidth: number;
  menu: MenuLayout | null;
  navRows: ReturnType<typeof layoutNav>["rows"];
  /** The whole tree's row count and the first row shown (a tree taller than its pane scrolls). */
  navTotal: number;
  navAt: number;
  hits: Hit[];
};

export const NAV_WIDTH = 24;

/** A notice stands in the status line as the host's own message does (host.rs chrome_view: Ink::Info, drawn
 * in the status.info slot), not as a warning. */
export function withNotice(
  chrome: ChromeView,
  notice: string | undefined,
): ChromeView {
  if (!notice) return chrome;
  // The bridge sends each host message's words as a notice too (bridge.rs: icon dropped), so when the snapshot still
  // holds that message its icon (✓, ×) stays before the words.
  const held = chrome.status.message;
  const icon = held && held.text === notice ? held.icon : undefined;
  return {
    ...chrome,
    status: {
      ...chrome.status,
      message: { text: notice, ink: "info", ...(icon ? { icon } : {}) },
    },
  };
}

export function frameOf(
  chrome: ChromeView,
  menu: MenuView | null,
  width: number,
  height: number,
  plain = false,
): ChromeFrame {
  // What the pointer is over is lit over the finished frame (app/hover.ts), not here.
  const top = chrome.setup ? layoutSetupTopBar(chrome, chrome.setup, width, 0) : layoutTopBar(chrome, width, 0);
  const sources = layoutSources(chrome, width, 1);
  const bodyTop = sources ? 2 : 1;
  const statusY = Math.max(bodyTop, height - 1);
  const status = layoutStatus(chrome, width, statusY, plain);
  const bodyHeight = Math.max(0, statusY - bodyTop);
  const hits: Hit[] = [...top.hits, ...(sources?.hits ?? []), ...status.hits];
  const navWidth = chrome.settings
    ? Math.min(NAV_WIDTH, Math.floor(width / 3))
    : 0;
  let navRows: ChromeFrame["navRows"] = [];
  let navTotal = 0;
  let navAt = 0;
  if (chrome.settings) {
    const nav = layoutNav(
      chrome.settings,
      1,
      bodyTop + 1,
      Math.max(0, navWidth - 2),
      Math.max(0, bodyHeight - 2),
    );
    navRows = nav.rows;
    navTotal = nav.total;
    navAt = nav.at;
    hits.push(...nav.hits);
  }
  const laidMenu = menu ? layoutMenu(menu, width, bodyTop, bodyHeight, plain) : null;
  // A menu is over everything: its hits win and the status line stays live (a click on `q quit`
  // presses it with a menu open); the body below is not a target while a menu is open.
  // The menu's whole frame is a hit too, under its rows (host.rs Hit::MenuBox): a press on its field or its border
  // keeps it open, a press anywhere else closes it.
  const finalHits = laidMenu
    ? [
        ...hits.filter((h) => h.y === 0 || h.y === statusY),
        { x: laidMenu.x, y: laidMenu.y, w: laidMenu.w, h: laidMenu.h, target: MENU_BOX },
        ...laidMenu.hits,
      ]
    : hits;
  return {
    width,
    height,
    top,
    sources,
    status,
    bodyTop,
    bodyHeight,
    navWidth,
    menu: laidMenu,
    navRows,
    navTotal,
    navAt,
    hits: finalHits,
  };
}

export function TopBarRow({ frame }: { frame: ChromeFrame }) {
  return <CellRow cells={frame.top.cells} />;
}

export function StatusRow({ frame }: { frame: ChromeFrame }) {
  return <CellRow cells={frame.status.cells} />;
}

/** Setup's step rail (chrome/rail.ts): its two rows, `inset` cells of ground on each side (setup's body keeps one). */
export function RailRows({ rail, inset = 0 }: { rail: RailLaid; inset?: number }) {
  const ground: Cell = { text: " ".repeat(inset), style: { backgroundColor: bgOf("bg.base") } };
  return (
    <Box flexDirection="column" flexShrink={0}>
      {rail.rows.map((cells, i) => (
        <CellRow key={i} cells={inset > 0 ? [ground, ...cells, ground] : cells} />
      ))}
    </Box>
  );
}

export function SourcesRow({ frame }: { frame: ChromeFrame }) {
  return frame.sources ? <CellRow cells={frame.sources.cells} /> : null;
}

export function SettingsNavPane({
  chrome,
  frame,
  focused,
}: {
  chrome: ChromeView;
  frame: ChromeFrame;
  focused: boolean;
}) {
  const nav = chrome.settings;
  const scene = useContext(MotionSceneContext);
  const markId = useId();
  useLayoutEffect(() => {
    if (!scene || !nav || motion.engine.level !== "full") return;
    const area = { x: 1, y: frame.bodyTop + 1, w: Math.max(0, frame.navWidth - 3), h: Math.max(0, frame.bodyHeight - 2) };
    motion.markTree(markId, scene, settingsTreeMark(nav, frame.navRows, area));
    return () => motion.markTree(markId, scene);
  });
  if (!nav) return null;
  // shell.rs settings: `has(Pane::Nav)` is the body having the keys and the host's focus being the nav; the caller says.
  // BandedTree: a tree taller than its pane draws a scrollbar in the pane's last inner column and keeps the selected row in view.
  const room = Math.max(1, frame.bodyHeight - 2);
  const max = Math.max(0, frame.navTotal - room);
  // The band and the heading run to the column before the scrollbar's (navWidth - 3); the selection's highlight covers the
  // whole area, the scrollbar's column too (tui-tree-widget), so without a bar it runs one cell further.
  const inner = Math.max(0, frame.navWidth - 3);
  const rows = frame.navRows.map((r, i) => {
    const shade: Style | undefined = r.selected
      ? selectedStyle(focused)
      : r.heading || r.band
        ? { backgroundColor: bgOf("bg.overlay") }
        : undefined;
    const cells = r.cells.map((c) => ({
      ...c,
      style: r.selected ? { ...c.style, ...shade } : { ...shade, ...c.style },
    }));
    const full = r.selected && max === 0 ? inner + 1 : inner;
    // A name wider than the column is cut one cell short of the band's edge, with no ellipsis (the tree widget's rule:
    // "Tools and defau" at 80 columns), and the rest of the row is the band or the selection.
    const kept = clipSegments(cells, Math.max(0, inner - 1));
    const used = kept.reduce((n, c) => n + cellWidth(c.text), 0);
    return <CellRow key={i} cells={[...kept, { text: " ".repeat(Math.max(0, full - used)), style: shade }]} />;
  });
  return (
    <Framed
      title={paneTitle({ pane: "nav", title: `settings${nav.unsaved ? " · unsaved" : ""}` }, chrome.pane_jumps)}
      pane="nav"
      width={frame.navWidth}
      height={frame.bodyHeight}
      focused={focused}
    >
      {max > 0 ? (
        <Box flexDirection="row">
          <Box flexDirection="column">{rows}</Box>
          {/* tui-tree-widget sizes its scrollbar for a content one shorter than the scroll range */}
          <ScrollBar rows={room} max={Math.max(1, max - 1)} at={frame.navAt} />
        </Box>
      ) : (
        rows
      )}
    </Framed>
  );
}

export function HistoryOverlay({ layout }: { layout: HistoryLayout }) {
  const scene = useContext(MotionSceneContext);
  const id = useId();
  useLayoutEffect(() => {
    if (!scene || motion.engine.level !== "full") return;
    layout.panes.forEach((pane, i) => motion.markPane(`${id}:${i}`, scene, pane));
    return () => layout.panes.forEach((_, i) => motion.markPane(`${id}:${i}`, scene));
  });
  return (
    <Box flexDirection="column" flexShrink={0}>
      {layout.lines.map((cells, i) => (
        <CellRow key={i} cells={cells} />
      ))}
    </Box>
  );
}

export function MenuBox({
  menu,
  frame,
}: {
  menu: MenuView;
  frame: ChromeFrame;
}) {
  const m = frame.menu;
  if (!m) return null;
  // Ink composites an absolute box by the cells its children write: every line is the popup's full width
  // (shell/menu.rs clears it), so the list underneath never shows through.
  return (
    <Box
      position="absolute"
      marginLeft={m.x}
      marginTop={m.y - frame.bodyTop}
      width={m.w}
      height={m.h}
      flexDirection="column"
    >
      {/* The layer's mark (app/scrim.ts): the screen around the box dims and the box throws a shadow. Written first,
          under the box's first line. */}
      <Box position="absolute" width={1} height={1}>
        <Transform transform={layerMark(m.w, m.h)}>
          <Text> </Text>
        </Transform>
      </Box>
      {m.lines.map((cells, i) => (
        <Box key={i} width={m.w} height={1}>
          <CellRow cells={cells} />
        </Box>
      ))}
    </Box>
  );
}

/** The tour's card (shell/tour.rs): a panel on bg.surface at the body's lower right, thrown a shadow and nothing else
 * dims (the layer's mark is shadow-only, app/scrim.ts). A menu draws after it and over it. */
export function TourCardBox({ card, width, height, plain = false }: { card: TourCard; width: number; height: number; plain?: boolean }) {
  const t = layoutTour(card, width, height, plain);
  if (!t) return null;
  const inner = Math.max(0, t.w - 4);
  const emphasis = { ...slotStyle("text.emphasis"), bold: true, backgroundColor: bgOf("bg.surface") };
  const muted = { ...slotStyle("text.muted"), backgroundColor: bgOf("bg.surface") };
  return (
    <Box position="absolute" marginLeft={t.x} marginTop={t.y} width={t.w} height={t.h} flexDirection="column">
      <Box position="absolute" width={1} height={1}>
        <Transform transform={layerMark(t.w, t.h, true)}>
          <Text> </Text>
        </Transform>
      </Box>
      <Framed title={t.title} width={t.w} height={t.h} focused rampOwner={false}>
        <Box flexDirection="column" paddingLeft={1} width={t.w - 2} height={t.h - 2} overflow="hidden">
          {t.lines.map((l, i) => (
            <Box key={i} width={inner} height={1} flexShrink={0}>
              {l.kind === "title" || l.kind === "try" ? (
                <Text wrap="truncate" {...textProps(l.kind === "title" ? emphasis : muted)}>
                  {l.words}
                </Text>
              ) : l.text ? (
                plain ? <Text wrap="truncate">{plainTourText(l.text, inner)}</Text> : <TextLine text={l.text} />
              ) : (
                <Text> </Text>
              )}
            </Box>
          ))}
        </Box>
      </Framed>
    </Box>
  );
}

function fitText(text: string, width: number): string {
  let out = "";
  let w = 0;
  for (const ch of [...text]) {
    const cw = cellWidth(ch);
    if (w + cw > width) break;
    out += ch;
    w += cw;
  }
  return out;
}

export { cellWidth };
