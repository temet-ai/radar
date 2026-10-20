import React from "react";
import { Box, Text } from "ink";
import type { Capabilities } from "../protocol/generated/Capabilities.js";
import { arrange, columnWidths, NARROW_BELOW } from "./layout.js";
import { Framed, ScreenPanes, SelectFrame, type SelectSink } from "../app/Panes.js";
import { selectKindOf } from "../input/selection.js";
import type { NodeCx, RenderNode as AppRenderNode } from "../app/Body.js";
import { sectionsOf, type ScreenLike, type ViewNode, type WorkSection } from "./model.js";
import { plainText, plainLines } from "./plain.js";
import { SegRun, TextLine } from "../text/TextLine.js";
import type { Seg } from "../nodes/view.js";
import { HitBox, type HitSink } from "./hit.js";

/** A work.* section's own content is its fallback document (the part's words for hosts that do not know the capability). */
const contentOf = (body: ViewNode): ViewNode =>
  body.node === "extension" && String(body.capability).startsWith("work.") && body.fallback ? (body.fallback as ViewNode) : body;

/**
 * The map's document with each item row a pointer target. The part writes one summary line, then one line
 * per entry of `data.items` in order; a press selects that item's target (the one its list row has).
 */
function MapDocument({ lines, items, registry }: { lines: Seg[][]; items: Array<{ target?: string; selected?: boolean }>; registry?: HitSink }) {
  return (
    <Box flexDirection="column" paddingX={1}>
      {lines.map((line, i) => {
        const l = line;
        const item = i >= 1 ? items[i - 1] : undefined;
        const target = item?.target;
        const row = l.length === 0 ? <Text> </Text> : <Text wrap="wrap">{l.map((seg, j) => <SegRun key={j} seg={seg} />)}</Text>;
        return target ? (
          <HitBox key={i} registry={registry} target={target} flexDirection="column">
            {row}
          </HitBox>
        ) : (
          <React.Fragment key={i}>{row}</React.Fragment>
        );
      })}
    </Box>
  );
}

export type RenderNode = (node: ViewNode, ctx: { columns: number; focused: boolean }) => React.ReactNode;

/** Used until T11's `renderNode` is wired in by the app: the node's content as plain lines. */
export const placeholderRenderNode: RenderNode = (node) => (
  <Box flexDirection="column">
    {plainLines(node).map((l, i) => (
      <Text key={i}>{l}</Text>
    ))}
  </Box>
);

export interface WorkAreaProps {
  /** The Work screen's view. */
  screen: ScreenLike;
  capabilities: Pick<Capabilities, "columns" | "rows" | "plain_text">;
  /** The focused section's target (kept by the app across snapshots). */
  focus?: string;
  /** Resized proportions of the wide columns. */
  weights?: number[];
  renderNode?: RenderNode;
  /** Rows the work area may use (the body's height); defaults to the terminal's rows less the bars. */
  height?: number;
  /**
   * Which of the M11 panels (map, timeline, tabs, agent, rail) are drawn beside the three panes. The reference layout draws only
   * nav/middle/detail (view_tui::render_panes), so the app passes "pending": a panel shows only while it holds a
   * question or approval waiting for the user. Default "all" (the layout's own tests).
   */
  panels?: "all" | "pending";
  /** Where pointer targets are registered (the app's), so a press on a map item reaches the host. */
  registry?: HitSink;
  /** Active-surface pane digits come from the host, including the legacy three-pane area. */
  paneJumps?: NodeCx["paneJumps"];
}

/** Rows a section wants inside its frame: its lines (wrapped at the inner width) and the frame's two edges. */
function want(s: WorkSection, inner: number): number {
  const wrapped = (t: string) => Math.max(1, Math.ceil(Math.max(1, t.length) / Math.max(1, inner - 2)));
  const body = plainLines(s.body).reduce((n, l) => n + wrapped(l), 0) + (s.pending ? 1 : 0);
  const around = (s.head.length ? 1 : 0) + s.notes.length + (s.footer.length ? 1 : 0);
  return body + around + 2;
}

function titleOf(s: WorkSection): string {
  return `${s.title}${s.status ? ` (${s.status})` : ""}`;
}

/** The one cell of padding the panels give text of their own (nodes pad themselves). */
const pad = (el: React.ReactNode) => (el ? <Box paddingX={1}>{el}</Box> : null);

function SectionBox({
  section,
  focused,
  width,
  height,
  renderNode,
  registry,
}: {
  section: WorkSection;
  focused: boolean;
  width: number;
  height: number;
  renderNode: RenderNode;
  registry?: HitSink;
}) {
  const inner = Math.max(1, width - 2);
  return (
    <SelectFrame registry={registry as SelectSink | undefined} id={section.target} kind={selectKindOf(contentOf(section.body))} width={width} height={height}>
    <Framed title={titleOf(section)} motionPane={`work:${section.target}`} width={width} height={height} focused={focused}>
      {pad(section.pending ? <Text color="yellow">● waiting for you</Text> : null)}
      {pad(section.head.length ? <TextLine text={section.head as Seg[]} /> : null)}
      {(() => {
        const content = contentOf(section.body);
        if (content.node === "document" && section.kind === "map") {
          const items = (section.body.data as { items?: Array<{ target?: string; selected?: boolean }> } | undefined)?.items ?? [];
          return <MapDocument lines={(content.lines as Seg[][]) ?? []} items={items} registry={registry} />;
        }
        return renderNode(section.body, { columns: inner, focused });
      })()}
      {section.notes.map((n, i) => pad(<TextLine key={`n${i}`} text={n as Seg[]} />))}
      {pad(section.footer.length ? <TextLine text={section.footer as Seg[]} /> : null)}
    </Framed>
    </SelectFrame>
  );
}

/** When rows run short, which panel gives up rows first (a pending question never does). */
/** Which panel a short terminal keeps first when it cannot show them all. */
const MEDIUM_FROM = 84;
const NEED: Record<string, number> = { rail: 0, map: 1, tabs: 2, timeline: 3, other: 4, agent: 5 };
const CUT_FIRST: Record<string, number> = { timeline: 0, tabs: 1, map: 2, rail: 3, other: 4, agent: 5 };

/** Share `room` rows among panels that want `wants` each: what fits as asked, rows cut from the least needed first, the surplus to the last. */
function rowsFor(wants: number[], room: number, ranks: number[] = wants.map((_, i) => -i)): number[] {
  const out = wants.map((w) => Math.max(3, w));
  let total = out.reduce((a, b) => a + b, 0);
  const order = out.map((_, i) => i).sort((x, y) => ranks[x]! - ranks[y]!);
  for (const i of order) {
    if (total <= room) break;
    const cut = Math.min(out[i]! - 3, total - room);
    out[i]! -= cut;
    total -= cut;
  }
  if (total < room && out.length) out[out.length - 1]! += room - total;
  return out;
}

/**
 * the base panels (nav, middle, detail by width) with the M11 sections drawn as panels in the same
 * frames: map, timeline, tabs, agent and action rail from the sections the view carries. Wide: the
 * panels, then a band of the sections side by side with resizable proportions. Under 100 columns:
 * one column, a pending question first. Plain-text mode: labelled lines in reading order. Focus is
 * the section's target, so a snapshot that changes other sections never moves it.
 */
export function WorkArea({ screen, capabilities, focus, weights, renderNode, registry, height, panels = "all", paneJumps }: WorkAreaProps) {
  const sections = sectionsOf(screen);
  const render = renderNode ?? placeholderRenderNode;
  if (capabilities.plain_text) {
    return (
      <Box flexDirection="column">
        {plainText(sections, focus).map((l, i) => (
          <Text key={i}>{l}</Text>
        ))}
      </Box>
    );
  }
  const W = capabilities.columns;
  const H = Math.max(6, height ?? capabilities.rows - 3);
  const panes = (screen.panes ?? []).filter((_, i) => sections[i]!.kind === "other");
  const extras = sections.filter((s) => s.kind !== "other" && (panels === "all" || s.pending));
  const paneFocus = ["nav", "middle", "detail"].includes(String(focus)) ? String(focus) : undefined;
  // The panes register where text is selected in them (issue #51); their rows' hits come from the nodes themselves.
  const sink = registry as SelectSink | undefined;
  const selects = sink?.addSelect ? { add: () => {}, addSelect: (a: Parameters<NonNullable<SelectSink["addSelect"]>>[0]) => sink.addSelect!(a) } : undefined;
  const appCx: NodeCx = { width: W, height: H, actions: [], plainText: false, focused: true, registry: selects, paneJumps };
  const appRender: AppRenderNode = (node, c) => render(node, { columns: c.width, focused: !!c.focused });
  const panesAt = (h: number) =>
    panes.length ? (
      <ScreenPanes screen={{ ...screen, panes }} focus={paneFocus} width={W} height={h} cx={appCx} renderNode={appRender} />
    ) : null;
  // What the panes need: the biggest body among those that can show, framed (they get the whole body; the rest is blank).
  const paneWant = Math.max(
    3,
    ...sections.filter((s) => s.kind === "other" && s.key.startsWith(`${paneFocus ?? "middle"}:`)).map((s) => want(s, W - 2)),
  );
  const pendingTop = W >= NARROW_BELOW ? extras.filter((s) => s.pending) : [];
  const a = arrange(W >= NARROW_BELOW ? extras.filter((s) => !s.pending) : extras, W, weights);

  if (a.mode === "collapsed") {
    // One column: a pending question first, then the base panel, then the rest in reading order. The question
    // and the panel keep what they need; the other panels take the rest in this order of need, and a panel that
    // cannot get three rows is left out and named on the last line (plain-text mode lists everything).
    const sorted = a.columns[0]!.sections;
    const before = sorted.filter((s) => s.pending);
    const rest = sorted.filter((s) => !s.pending);
    // Every panel kept is drawn whole (never squeezed): the question first, then the others in order of need while
    // they fit with a floor for the base panel; the rest are named on the last line. The base panel takes what is left.
    const floor = panes.length ? Math.min(Math.floor(H / 2), paneWant) : 0;
    let room = H - before.reduce((n, s) => n + want(s, W - 2), 0) - floor;
    const keep = new Set<WorkSection>();
    // The Narrow layout (under 84 columns) shows the focused pane alone: there the only
    // addition is a pending question; the other M11 panels are reached from plain-text mode and the actions menu.
    const need = W < MEDIUM_FROM ? [] : [...rest].sort((x, y) => NEED[x.kind]! - NEED[y.kind]!);
    need.forEach((s, i) => {
      const hint = need.slice(i + 1).length ? 1 : 0;
      if (room - want(s, W - 2) >= hint) {
        keep.add(s);
        room -= want(s, W - 2);
      }
    });
    const left = W < MEDIUM_FROM ? [] : rest.filter((s) => !keep.has(s));
    const shown = rest.filter((s) => keep.has(s));
    const hintRows = left.length ? 1 : 0;
    const items: Array<{ s?: WorkSection; wants: number }> = [
      ...before.map((s) => ({ s, wants: want(s, W - 2) })),
      ...(panes.length ? [{ wants: floor + Math.max(0, room - hintRows) }] : []),
      ...shown.map((s) => ({ s, wants: want(s, W - 2) })),
    ];
    const rows = items.map((i) => i.wants);
    return (
      <Box flexDirection="column" width={W}>
        {items.map((it, i) =>
          it.s ? (
            <SectionBox key={it.s.key} section={it.s} focused={it.s.target === focus} width={W} height={rows[i]!} renderNode={render} registry={registry} />
          ) : (
            <React.Fragment key="panes">{panesAt(rows[i]!)}</React.Fragment>
          ),
        )}
        {left.length ? <Text dimColor wrap="truncate">{`+ ${left.map((s) => s.title).join(", ")} (no room here; plain-text mode lists every panel)`}</Text> : null}
      </Box>
    );
  }
  const widths = columnWidths(a.columns.map((c) => c.weight), W);
  const topRows = rowsFor(pendingTop.map((s) => want(s, W - 2)), pendingTop.reduce((n, s) => n + want(s, W - 2), 0));
  const topH = topRows.reduce((n, r) => n + r, 0);
  const bandH = a.columns.length ? Math.min(Math.max(...a.columns.map((c) => c.sections.reduce((n, s) => n + want(s, 20), 0))), Math.floor(H / 2)) : 0;
  return (
    <Box flexDirection="column" width={W}>
      {pendingTop.map((s, i) => (
        <SectionBox key={s.key} section={s} focused={s.target === focus} width={W} height={topRows[i]!} renderNode={render} registry={registry} />
      ))}
      {panesAt(H - bandH - topH)}
      {bandH > 0 ? (
        <Box flexDirection="row" width={W}>
          {a.columns.map((col, ci) => {
            const rows = rowsFor(col.sections.map((s) => want(s, widths[ci]! - 2)), bandH);
            return (
              <Box key={ci} flexDirection="column" width={widths[ci]}>
                {col.sections.map((s, si) => (
                  <SectionBox key={s.key} section={s} focused={s.target === focus} width={widths[ci]!} height={rows[si]!} renderNode={render} registry={registry} />
                ))}
              </Box>
            );
          })}
        </Box>
      ) : null}
    </Box>
  );
}
