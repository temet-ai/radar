import React, {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useReducer,
  useRef,
  useState,
} from "react";
import { Box, measureElement, Text as InkText, type DOMElement } from "ink";
import type { Block, Node, NodeContext, Text } from "./view.js";
import { TextLine, SegRun, segWords, textProps } from "../text/TextLine.js";
import { wrapWords } from "../text/wrap.js";
import { cellWidth } from "../chrome/width.js";
import { RichBlocks, RichBlockView } from "../text/RichDoc.js";
import { DiffView } from "../text/DiffView.js";
import { StateView } from "../text/StateView.js";
import { EditorView, isEditorData } from "../editor/EditorView.js";
import { EntriesNode, ListNode, LogNode, TableNode } from "./lists.js";
import { TreeNode } from "./tree.js";
import { ActionList } from "./actions.js";
import { HitRegion } from "./hits.js";
import {
  Rows,
  useBg,
  CellLine,
  cellsOf,
  clipCells,
  lineCells,
  padCells,
  widthOfCells,
  wrapCells,
  type Cell,
} from "../text/cells.js";
import { slotStyle, specStyle } from "../text/look.js";

/** ui::scroll::Scroll for a Document or Rich pane: how far it is scrolled and how far it can be, as of the last frame.
 * The frontend scrolls these panes itself (view_tui::key); T4's router asks `scrollers()` for the one that has the keys
 * (`focused`) or, for PgUp/PgDn on a screen without a page scroll, the detail Document. */
export interface Scroller {
  kind: "document" | "rich";
  focused: boolean;
  /** The first row shown. */
  at(): number;
  /** The furthest it can scroll, as of the last frame (0: everything fits). */
  max(): number;
  /** Rows visible, as of the last frame. */
  height(): number;
  /** Scroll::key for `down j up k pageDown pageUp home g end G`; false for any other key and for up/k at the top. */
  key(name: string): boolean;
  /** Scroll::by / Scroll::page: whether it moved. */
  by(rows: number): boolean;
  page(down: boolean): boolean;
  /** Whether the screen cell (0-based) is in the rows it draws, as of the last frame: the wheel there scrolls it. */
  under(x: number, y: number): boolean;
}

/** The screen cell of a laid-out element's top-left corner (0-based), from its yoga nodes; null while it has none. */
export function screenAt(el: DOMElement | null): { x: number; y: number } | null {
  if (!el) return null;
  let x = 0;
  let y = 0;
  try {
    for (let n: DOMElement | undefined = el; n; n = n.parentNode) {
      x += n.yogaNode?.getComputedLeft() ?? 0;
      y += n.yogaNode?.getComputedTop() ?? 0;
    }
  } catch {
    return null; // torn down mid-commit
  }
  return { x, y };
}

const mounted = new Set<Scroller>();
/** The Document and Rich panes drawn now, oldest first. */
export const scrollers = (): Scroller[] => [...mounted];
/** The Document or Rich pane drawn at the screen cell (0-based), the newest first. */
export const scrollerAt = (x: number, y: number): Scroller | undefined => [...mounted].reverse().find((s) => s.under(x, y));
/** The scroller of the pane that has the keys, if it is a Document or Rich pane. */
export const focusedScroller = (): Scroller | undefined =>
  [...mounted].reverse().find((s) => s.focused);

/** The thumb of ratatui's vertical-right Scrollbar (begin/end symbols off, viewport length unset) on a track of `len`
 * cells for a content `max + 1` long at `at`: its first cell and length. */
export function thumbOf(
  len: number,
  max: number,
  at: number,
): { start: number; len: number } {
  const maxPos = max; // content_length - 1
  const pos = Math.min(Math.max(0, at), maxPos);
  const span = maxPos + len; // max_viewport_position: viewport_content_length is the track's length when unset
  const start = Math.round((pos * len) / span);
  const end = Math.round(((pos + len) * len) / span);
  return { start, len: Math.max(1, end - start) };
}

/** scroll::scrollbar: ║ track in bg.overlay, █ thumb in accent.muted, one cell wide; nothing when everything fits. */
/** Where a scrolled node reports how far it can scroll and where it is (null: it left), for a frame to draw the bar. */
export type ScrollReport = { max: number; at: number } | null;
export const ScrollSink = createContext<((r: ScrollReport) => void) | null>(
  null,
);

/** A node that scrolls its own rows (list, table, log) tells the pane's frame how far it can and where it is, for the bar
 * on the panel's border (scroll::scrollbar); `null` while everything fits. */
export function useScrollReport(report: ScrollReport) {
  const sink = useContext(ScrollSink);
  const max = report?.max ?? 0;
  const at = report?.at ?? 0;
  useLayoutEffect(() => {
    if (!sink || max <= 0) return;
    sink({ max, at });
    return () => sink(null);
  }, [sink, max, at]);
}

export function ScrollBar({
  rows,
  max,
  at,
  plain,
}: {
  rows: number;
  max: number;
  at: number;
  plain?: boolean;
}) {
  const bg = useBg();
  if (max <= 0)
    return (
      <InkText>{Array.from({ length: rows }, () => " ").join("\n")}</InkText>
    );
  const { start, len } = thumbOf(rows, max, at);
  const track = plain ? {} : slotStyle("bg.overlay");
  const thumb = plain ? {} : slotStyle("accent.muted");
  return (
    <Box flexDirection="column" width={1}>
      {(() => {
        const bar = new Rows(plain, bg);
        for (let i = 0; i < rows; i++) {
          const on = i >= start && i < start + len;
          bar.line().put(on ? "█" : "║", on ? thumb : track, 1);
        }
        return bar.element("truncate");
      })()}
    </Box>
  );
}

/** A body taller than its pane: shows `room` rows from the scroll's offset and the scrollbar in the last column.
 * `total` is the content's height when the caller knows it; otherwise it is measured after layout. The offset is kept
 * across frames while the content stays the same (`id`), as Ratatui keeps a panel's Scroll; new content starts at the top. */
function Scrolled({
  kind,
  id,
  total,
  room,
  width,
  focused,
  plain,
  children,
}: {
  kind: Scroller["kind"];
  id: string;
  total?: number;
  room: number;
  width: number;
  focused: boolean;
  plain?: boolean;
  children: (at: number) => React.ReactNode;
}) {
  const [, bump] = useReducer((n: number) => n + 1, 0);
  const [measured, setMeasured] = useState(0);
  const inner = useRef<DOMElement>(null);
  const outer = useRef<DOMElement>(null);
  const state = useRef({ at: 0, max: 0, id, room, focused, total: 0, rect: null as null | { x: number; y: number; w: number; h: number } });
  const s = state.current;
  if (s.id !== id) {
    s.id = id;
    s.at = 0;
  }
  const all = total ?? measured;
  // Scroll::fit
  s.room = room;
  s.focused = focused;
  s.total = all;
  s.max = Math.max(0, all - room);
  s.at = Math.min(s.at, s.max);
  const handle = useRef<Scroller | null>(null);
  if (!handle.current) {
    const st = state.current;
    const by = (rows: number) => {
      const at = Math.min(Math.max(0, st.at + rows), st.max);
      const moved = at !== st.at;
      st.at = at;
      if (moved) bump();
      return moved;
    };
    const top = () => {
      const moved = st.at !== 0;
      st.at = 0;
      if (moved) bump();
    };
    handle.current = {
      kind,
      get focused() {
        return st.focused;
      },
      at: () => st.at,
      max: () => st.max,
      height: () => st.room,
      by,
      page: (down) => by(Math.max(1, st.room - 1) * (down ? 1 : -1)),
      under: (x, y) => !!st.rect && x >= st.rect.x && x < st.rect.x + st.rect.w && y >= st.rect.y && y < st.rect.y + st.rect.h,
      key(name) {
        switch (name) {
          case "down":
          case "j":
            by(1);
            return true;
          case "up":
          case "k":
            if (st.at === 0) return false;
            by(-1);
            return true;
          case "pageDown":
            return handle.current!.page(true);
          case "pageUp":
            return handle.current!.page(false);
          case "home":
          case "g":
            top();
            return true;
          case "end":
          case "G": {
            const moved = st.at !== st.max;
            st.at = st.max;
            if (moved) bump();
            return true;
          }
          default:
            return false;
        }
      },
    };
  }
  useEffect(() => {
    const h = handle.current!;
    mounted.add(h);
    return () => {
      mounted.delete(h);
    };
  }, []);
  useLayoutEffect(() => {
    // Where it is drawn, for the wheel (scrollerAt).
    const at = screenAt(outer.current);
    state.current.rect = at ? { ...at, w: width, h: room } : null;
    if (total !== undefined || !inner.current) return;
    const h = measureElement(inner.current).height;
    if (h !== measured) setMeasured(h);
  });
  const at = s.at;
  // A frame that takes the report draws the bar on its right border (scroll::scrollbar on the panel); without one
  // the bar is drawn here, in the node's last column.
  const sink = useContext(ScrollSink);
  useLayoutEffect(() => {
    sink?.({ max: s.max, at: s.at });
    return () => sink?.(null);
  }, [sink, s.max, s.at]);
  // Everything fits and a frame owns the bar: no row, no clip, no bar column (the common case draws as it did before).
  if (sink && s.max <= 0 && total !== undefined) {
    return (
      <Box ref={outer} flexDirection="column" width={width}>
        {children(0)}
      </Box>
    );
  }
  return (
    <Box ref={outer} flexDirection="row" width={width} height={room}>
      <Box
        flexDirection="column"
        width={Math.max(1, width - 1)}
        height={room}
        overflow="hidden"
      >
        {total !== undefined ? (
          children(at)
        ) : (
          <Box
            ref={inner}
            flexDirection="column"
            flexShrink={0}
            marginTop={-at}
          >
            {children(at)}
          </Box>
        )}
      </Box>
      <ScrollBar rows={room} max={sink ? 0 : s.max} at={at} plain={plain} />
    </Box>
  );
}

/** widgets::paragraph / ui::card: the lines inside one cell of padding each side, wrapped between words (or cut at the
 * edge when the node says `cut`: a column of facts). In a pane of known height it scrolls (ui::scroll::paragraph). */
function DocumentNode({
  lines,
  cut,
  ctx,
}: {
  lines: Text[];
  cut: boolean;
  ctx: NodeContext;
}) {
  const room = Math.max(1, ctx.width - 2);
  const rows = lines.flatMap((l) => {
    const cells = lineCells(l, ctx.plainText);
    return cut ? [clipCells(cells, room)] : wrapCells(cells, room);
  });
  const draw = (from: number, count: number, width: number) => (
    <Box flexDirection="column">
      {rows.slice(from, from + count).map((r, i) => (
        <CellLine
          key={i}
          cells={padCells([...cellsOf(" "), ...r], width)}
          plain={ctx.plainText}
        />
      ))}
    </Box>
  );
  if (ctx.height === undefined) return draw(0, rows.length, ctx.width);
  const height = Math.max(1, ctx.height);
  // view_tui draws a Document with widgets::paragraph and never fits a Scroll to it: it is cut at the pane height, has
  // no bar, and its Scroll stays at max 0 (Down j g G Home End are used by it, Up k PgUp PgDn go on to the host, whose
  // "nothing to scroll here" answers PgUp/PgDn). The scroller is still registered so the router sees the same answers.
  return (
    <Scrolled
      kind="document"
      id={docId(lines)}
      total={ctx.scrolls ? rows.length : Math.min(rows.length, height)}
      room={height}
      width={ctx.width}
      focused={ctx.focused}
      plain={ctx.plainText}
    >
      {(at) => draw(at, height, ctx.width - 1)}
    </Scrolled>
  );
}

const richId = (blocks: Block[]) =>
  JSON.stringify(blocks.slice(0, 2)).slice(0, 200);

/** What a Document is, for keeping its offset while the same text is redrawn and starting at the top for another. */
const docId = (lines: Text[]) =>
  lines
    .slice(0, 3)
    .map((l) => l.map((s) => s.text ?? "").join(""))
    .join("\n");

/** ui::meter::stages: two lines, a bar per stage (▀) and its mark and name under it, in the stage's look. */
function MeterLines({
  stages,
  width,
  plain,
}: {
  stages: { name: string; mark: keyof typeof MARK_SYMBOL; look: string }[];
  width: number;
  plain?: boolean;
}) {
  const n = stages.length;
  if (n === 0) return null;
  const all = Math.max(n, width);
  const bars: ReturnType<typeof cellsOf> = [];
  const names: ReturnType<typeof cellsOf> = [];
  stages.forEach((s, i) => {
    // Layout::horizontal(Fill(1) each).spacing(1): stage i spans [i(W+1)/n, ((i+1)(W+1)-n)/n), its ends rounded
    // (the same as Ratatui's split for every n up to 6 and width up to 60).
    const w = Math.round(((i + 1) * (all + 1) - n) / n) - Math.round((i * (all + 1)) / n);
    const style = plain ? {} : specStyle(s.look);
    bars.push(
      ...cellsOf("▀".repeat(w), style),
      ...cellsOf(i < n - 1 ? " " : ""),
    );
    const name = clipCells(
      cellsOf(`${MARK_SYMBOL[s.mark]} ${s.name}`, style),
      w,
    );
    names.push(...padCells(name, w, style), ...cellsOf(i < n - 1 ? " " : ""));
  });
  return (
    <Box flexDirection="column">
      <CellLine cells={bars} plain={plain} />
      <CellLine cells={names} plain={plain} />
    </Box>
  );
}

export {
  ActionList,
  chordName,
  intentForChord,
  invokeIntent,
} from "./actions.js";

export type { Node, NodeContext } from "./view.js";
export * from "./view.js";
export { visibleRows } from "./tree.js";

/** The capabilities this renderer draws itself; any other Extension shows its fallback and says so. */
export const OWN_CAPABILITIES: readonly string[] = ["editor"];

/** view_tui keyed_line: the line, and its key as ` [k]` in muted words at the right edge of `room` when both fit
 * there (one row, never wrapped); otherwise the line broken between words at `wrapAt`. */
function lineRows(
  block: { text: Text; key?: string | null },
  room: number,
  wrapAt: number,
  plain?: boolean,
): Cell[][] {
  const cells = lineCells(block.text, plain);
  if (block.key) {
    const shortcut = cellsOf(
      ` [${block.key}]`,
      plain ? {} : slotStyle("text.muted"),
    );
    if (widthOfCells(cells) + widthOfCells(shortcut) <= room)
      return [[...padCells(cells, room - widthOfCells(shortcut)), ...shortcut]];
  }
  return wrapCells(cells, Math.max(1, wrapAt));
}

/** ui::card::section: `mark`, the title in muted capitals, `detail` in secondary words when it fits with them, then a
 * muted rule to `room`'s edge; all of it bold (widgets::heading's line style, under every span of the line). */
export function sectionHeading(
  block: { mark?: string | null; title: string; detail?: string | null },
  room: number,
  plain?: boolean,
): Cell[] {
  const head: Text = [
    ...(block.mark ? [{ icon: block.mark }] : []),
    { text: block.title, ink: "muted" },
  ];
  let cells = lineCells(head, plain);
  if (block.detail && widthOfCells(cells) + 2 + cellWidth(block.detail) <= room)
    cells = [...cells, ...lineCells([{ text: `  ${block.detail}`, ink: "secondary" }], plain)];
  const rest = room - widthOfCells(cells) - 1;
  if (rest > 0)
    cells = [...cells, ...lineCells([{ text: ` ${"─".repeat(rest)}`, ink: "muted" }], plain)];
  return plain ? cells : cells.map((c) => ({ ...c, s: { ...c.s, bold: true } }));
}

function BlockView({ block, ctx }: { block: Block; ctx: NodeContext }) {
  switch (block.block) {
    case "line": {
      // view_tui Node::Rich: a line block is a whole Line, wrapped between words to the padded width (ui::scroll::scrolled
      // wraps at the body's width; the scrollbar is on the frame's border). Only a doc block is laid out a column short.
      const rows = lineRows(block, ctx.width, ctx.width, ctx.plainText);
      return (
        <Box flexDirection="column">
          {rows.map((r, i) => (
            <CellLine key={i} cells={r} plain={ctx.plainText} />
          ))}
        </Box>
      );
    }
    case "section": {
      // view_tui section_rows: the heading across the room, then the blocks laid out in the room two cells in; a line
      // breaks at that room, each of its rows set in.
      const inner = { ...ctx, width: Math.max(1, ctx.width - 2) };
      return (
        <Box flexDirection="column">
          <CellLine
            cells={sectionHeading(block, ctx.width, ctx.plainText)}
            plain={ctx.plainText}
          />
          {block.blocks.map((b, i) => (
            <Box key={i} flexDirection="column" paddingLeft={2}>
              {b.block === "line" ? (
                lineRows(b, inner.width, inner.width, ctx.plainText).map(
                  (r, j) => <CellLine key={j} cells={r} plain={ctx.plainText} />,
                )
              ) : (
                <BlockView block={b} ctx={inner} />
              )}
            </Box>
          ))}
        </Box>
      );
    }
    case "doc":
      return (
        <Box>
          {block.bar ? (
            <Box marginRight={1}>
              <SegRun seg={block.bar} plain={ctx.plainText} />
            </Box>
          ) : null}
          <Box flexDirection="column" flexGrow={1}>
            <RichBlocks
              blocks={block.doc}
              plain={ctx.plainText}
              width={Math.max(
                1,
                ctx.width -
                  (block.bar ? cellWidth(segWords(block.bar)) + 1 : 0),
              )}
            />
          </Box>
        </Box>
      );
    case "meter":
      return (
        <MeterLines
          stages={block.stages}
          width={ctx.width}
          plain={ctx.plainText}
        />
      );
    case "wrapped":
      return (
        // ui::wrapped(text, width + 1, prefix): one row per wrapped line, each behind the prefix
        <Box flexDirection="column">
          {wrapWords(
            block.text,
            ctx.width + 1,
            cellWidth(segWords(block.prefix)),
          ).map((line, i) => (
            <Box key={i}>
              <SegRun seg={block.prefix} plain={ctx.plainText} />
              <InkText wrap="truncate">{line}</InkText>
            </Box>
          ))}
        </Box>
      );
    default:
      return (
        <UnknownNode
          type={`block ${(block as { block?: string }).block ?? "?"}`}
        />
      );
  }
}

const MARK_SYMBOL = {
  done: "✓",
  failed: "✗",
  running: "●",
  waiting: "◷",
  manual: "▷",
  stopped: "⊘",
  skipped: "»",
} as const;

/** A node this renderer does not know: visible, names the type. The part's actions stay reachable because they live
 * on the ScreenView (`actions`), outside any node, and the shell lists them. */
export function UnknownNode({
  type,
  ctx,
}: {
  type: string;
  ctx?: NodeContext;
}) {
  return (
    <Box flexDirection="column">
      <Box borderStyle="round" borderColor="yellow" paddingX={1}>
        <InkText>{`shown simply: this renderer does not draw ${type}`}</InkText>
      </Box>
      <ActionList actions={ctx?.actions ?? []} />
    </Box>
  );
}

/** An Extension this renderer cannot draw: says so, then draws the fallback the part gave for frontends like it. */
function ExtensionFallback({
  capability,
  fallback,
  ctx,
}: {
  capability: string;
  fallback: Node;
  ctx: NodeContext;
}) {
  return (
    <Box flexDirection="column">
      <InkText
        {...(ctx.plainText ? {} : textProps(slotStyle("text.muted")))}
      >{`shown simply: this needs ${capability}`}</InkText>
      <NodeView node={fallback} ctx={ctx} />
      <ActionList actions={ctx.actions ?? []} />
    </Box>
  );
}

export function NodeView({
  node,
  ctx,
}: {
  node: Node;
  ctx: NodeContext;
}): React.ReactElement {
  switch (node.node) {
    case "list":
      return (
        <ListNode
          rows={node.rows}
          selected={node.selected}
          editing={node.editing}
          ctx={ctx}
        />
      );
    case "tree":
      return <TreeNode tree={node} ctx={ctx} />;
    case "document":
      return (
        <DocumentNode
          lines={node.lines}
          cut={Boolean((node as { cut?: boolean }).cut)}
          ctx={ctx}
        />
      );
    case "rich": {
      const body = (
        <Box flexDirection="column" paddingLeft={1}>
          {node.blocks.map((b, i) => (
            <BlockView
              key={i}
              block={b}
              ctx={{ ...ctx, width: Math.max(1, ctx.width - 2) }}
            />
          ))}
        </Box>
      );
      if (ctx.height === undefined) return body;
      return (
        <Scrolled
          kind="rich"
          id={richId(node.blocks)}
          room={Math.max(1, ctx.height)}
          width={ctx.width}
          focused={ctx.focused}
          plain={ctx.plainText}
        >
          {() => body}
        </Scrolled>
      );
    }
    case "table":
      return (
        <TableNode
          columns={node.columns}
          rows={node.rows}
          selected={node.selected}
          empty={node.empty}
          ctx={ctx}
        />
      );
    case "entries":
      return (
        <EntriesNode
          rows={node.rows}
          selected={node.selected}
          empty={node.empty}
          ctx={ctx}
        />
      );
    case "log":
      return (
        <LogNode
          lines={node.lines}
          top={node.top}
          total={node.total}
          empty={node.empty}
          ctx={ctx}
        />
      );
    case "diff":
      return (
        <DiffView
          lines={node.lines}
          under={node.under}
          selected={node.selected}
          top={node.top}
          split={node.split}
          painted={node.painted}
          empty={node.empty}
          width={ctx.width}
          height={ctx.height}
          focused={ctx.focused}
          plain={ctx.plainText}
        />
      );
    case "state":
      return (
        <StateView
          state={node.state}
          text={node.text}
          plain={ctx.plainText}
          width={ctx.width}
        />
      );
    case "extension": {
      const own = (ctx.capabilities ?? OWN_CAPABILITIES).includes(
        node.capability,
      );
      if (
        own &&
        node.capability === "editor" &&
        node.version === 1 &&
        isEditorData(node.data)
      ) {
        return <EditorView data={node.data} focused={ctx.focused} />;
      }
      return (
        <ExtensionFallback
          capability={node.capability}
          fallback={node.fallback}
          ctx={ctx}
        />
      );
    }
    default:
      return (
        <UnknownNode
          type={`node ${(node as { node?: string }).node ?? "?"}`}
          ctx={ctx}
        />
      );
  }
}

/** The entry T10 imports: one element for any node. */
export function renderNode(node: Node, ctx: NodeContext): React.ReactElement {
  // The node never draws outside the room it was given, whatever its columns want: a pane beside it stays whole.
  return (
    <HitRegion node={node} ctx={ctx}>
      <NodeView node={node} ctx={ctx} />
    </HitRegion>
  );
}
