import React, { useLayoutEffect, useRef, useState } from "react";
import { Box, type DOMElement } from "ink";
import type { HitRegistry } from "../state/ui-state.js";
import { Framed } from "../app/Panes.js";
import { CellBlock, CellLine, padCells, cellsOf, type Cell } from "../text/cells.js";

/** A transcript row padded to the body's width, once per row and width (the rows come from dock/rows.ts's cache). */
const padded = new WeakMap<Cell[], { width: number; row: Cell[] }>();
function paddedRow(row: Cell[], width: number): Cell[] {
  const kept = padded.get(row);
  if (kept && kept.width === width) return kept.row;
  const out = padCells(row, width);
  padded.set(row, { width, row: out });
  return out;
}
import { InputField } from "../editor/InputField.js";
import { slotStyle } from "../text/look.js";
import type { DockView } from "./model.js";
import { activityCells, commandRows, pickerRows, spinAt, statusCells, tabRowCells, tabSpans, transcriptView } from "./rows.js";
import { MENU_ROWS, dockHint, matchingCommands, menuWindow } from "./composer.js";

/** The lines shown of a draft in `rows` rows (the ones around the caret) and the caret within its line, null elsewhere. */
export function composerLines(text: string, caret: number, focused: boolean, rows: number): Array<{ text: string; caret: number | null }> {
  const lines = text.split("\n");
  let before = caret;
  let at = 0;
  while (at < lines.length - 1 && before > Array.from(lines[at]!).length) {
    before -= Array.from(lines[at]!).length + 1;
    at++;
  }
  const first = Math.max(0, Math.min(at - rows + 1, lines.length - rows));
  return lines.slice(first, first + rows).map((t, i) => ({ text: t, caret: focused && first + i === at ? before : null }));
}

/** The line a transcript with nothing in it shows. */
export const emptyWords = (noTabs: boolean, state?: string): string =>
  noTabs ? " no sessions · ctrl-t starts one" : state === "detached" ? " not attached · a message or alt-r reads it back" : state === "resuming" ? " reading the session back…" : " no messages yet · type below";

/** Rows the composer takes: its text's lines, one to three; none on a terminal tab. */
export const composerRows = (draft: string) => Math.min(3, Math.max(1, draft.split("\n").length));

/**
 * The agent dock: a frame holding the tab row, the transcript, the composer and the status line. It draws the
 * host's `DockView` and holds nothing: keys and mouse become dock intents elsewhere (src/dock/keys.ts). A tab with
 * a terminal draws T7's surface through `terminal` instead of transcript and composer.
 */
export function Dock({
  data,
  width,
  height,
  plain,
  draft,
  caret,
  registry,
  scroll = 0,
  anchor,
  metrics,
  pick = 0,
  cmd = 0,
  diffs,
  picks,
  refused,
  groups,
  terminal,
}: {
  data: DockView;
  width: number;
  height: number;
  plain?: boolean;
  /** The text being written (Ink's editor state until sent); the host's kept draft when absent. */
  draft?: string;
  /** The open picker's selected choice, and the items whose diff is open (Ink-side, DockLocal). */
  caret?: number;
  /** Where the pointer targets are registered: the top edge (drag to resize), each tab, the body (focus). */
  registry?: HitRegistry;
  /** Rows scrolled up from the foot when the transcript had `anchor` rows (DockLocal); `metrics` is told how far the
   * transcript can scroll and how many rows it has. */
  scroll?: number;
  anchor?: number;
  metrics?: { current: { max: number; total?: number } };
  pick?: number;
  /** The slash menu's selected row (Ink-side, DockLocal). */
  cmd?: number;
  diffs?: string[];
  /** A question's choices picked so far (Ink-side, DockLocal.picks). */
  picks?: Record<string, string[]>;
  /** Why the last ⏎ sent nothing (Ink-side, DockLocal.refused): the status line says it. */
  refused?: string;
  /** The groups of calls shown open (Ink-side, DockLocal.groups). */
  groups?: string[];
  /** Draws the active tab's terminal in `columns` x `rows` cells. */
  terminal?: (columns: number, rows: number, origin: { row: number; col: number }) => React.ReactNode;
}) {
  const ref = useRef<DOMElement>(null);
  // Where the terminal tab's top-left cell is on the screen (1-based), measured after layout.
  const [origin, setOrigin] = useState({ row: 1, col: 1 });
  const inner = Math.max(0, width - 2);
  const rows = Math.max(0, height - 2);
  const tabs = data.tabs ?? [];
  const at = Math.max(0, tabs.findIndex((t) => t.id === data.active));
  const tab = tabs[at];
  const text = draft ?? data.composer?.draft ?? "";
  const composerH = tab?.terminal ? 0 : composerRows(text);
  // The activity line above the composer: what the turn does now, or how the last one ended.
  const activity = tab?.terminal ? null : (data.activity ?? null);
  const activityH = activity ? 1 : 0;
  const bodyH = Math.max(0, rows - 2 - composerH - activityH);
  const items = data.transcript ?? [];
  const off = data.composer?.disabled;
  // The slash menu: the commands matching what is typed, a window of them around the selected one, over the foot of the body.
  const matches = data.picker || off ? [] : matchingCommands(text, data.composer?.commands);
  const sel = Math.min(Math.max(0, cmd), Math.max(0, matches.length - 1));
  const win = menuWindow(matches.length, sel, Math.min(MENU_ROWS, Math.max(1, bodyH - 1)));
  const cmds = matches.slice(win.first, win.first + win.count);
  const menuRows = commandRows(cmds, inner, sel - win.first);
  // Paths show relative to the session's folder (`~` for home); Storybook draws in a browser, with no process.
  const home = typeof process !== "undefined" ? process.env.HOME : undefined;
  // A call still running turns with the activity line's spinner while the turn runs.
  const spinning = !!data.activity && ["thinking", "writing", "running"].includes(data.activity.doing);
  const opts = { cwd: data.status?.cwd || undefined, home, groups, live: tab?.state === "streaming", spin: spinning ? spinAt(data.activity!.elapsed) : undefined };
  const shown = transcriptView(items, inner, Math.max(0, bodyH - cmds.length), diffs, scroll, picks, anchor, opts);
  if (metrics) {
    metrics.current.max = shown.max;
    metrics.current.total = shown.total;
  }
  useLayoutEffect(() => {
    if (!ref.current) return;
    let x = 0;
    let y = 0;
    for (let n: DOMElement | undefined = ref.current; n; n = n.parentNode) {
      x += n.yogaNode?.getComputedLeft() ?? 0;
      y += n.yogaNode?.getComputedTop() ?? 0;
    }
    // Registered in this order, topmost last: the body (a click gives the dock the keys), then the tabs, then the edge,
    // whose target carries the row the dock ends at so a drag knows its height (`dock:edge:<foot>`).
    // inside the frame (1), under the tab row (1)
    if (origin.row !== y + 3 || origin.col !== x + 2) setOrigin({ row: y + 3, col: x + 2 });
    if (!registry) return;
    registry.add({ x, y, w: width, h: height, target: "dock:body" });
    // Text is selected in the transcript (under the tab row, above the activity line and the composer): not the frame,
    // not a terminal tab, whose program has the mouse.
    if (!tab?.terminal && inner > 0 && bodyH > 0) registry.addSelect({ x: x + 1, y: y + 2, w: inner, h: bodyH, id: "dock", kind: "text" });
    // each tab, then its close mark on top of it (a click closes it, as a middle click anywhere on the tab does)
    for (const s of tabSpans(data.tabs ?? [], inner, at)) {
      registry.add({ x: x + 1 + s.x, y: y + 1, w: s.w, h: 1, target: `dock:tab:${s.id}` });
      registry.add({ x: x + 1 + s.closeX, y: y + 1, w: s.closeW, h: 1, target: `dock:close:${s.id}` });
    }
    // a tool call, a group of calls or reasoning folds and opens on a click on any of its rows
    if (!data.picker && !tab?.terminal)
      shown.targets.forEach((target, i) => {
        if (target) registry.add({ x: x + 1, y: y + 2 + i, w: inner, h: 1, target });
      });
    // a request's buttons (approve once, approve for session, deny, view diff; a question's choices, submit, decline)
    // answer on a click
    if (!data.picker && !tab?.terminal)
      shown.hits.forEach((row, i) => row.forEach((h) => registry.add({ x: x + 1 + h.x, y: y + 2 + i, w: h.w, h: 1, target: h.target })));
    // each row of the slash menu runs its command on a click, as ⏎ on it does; the menu sits under what the body shows
    if (!data.picker && !tab?.terminal && cmds.length > 0) {
      const top = items.length === 0 ? Math.min(1, Math.max(0, bodyH - cmds.length)) : shown.rows.length;
      cmds.forEach((c, i) => registry.add({ x: x + 1, y: y + 2 + top + i, w: inner, h: 1, target: `dock:cmd:${encodeURIComponent(c.name)}` }));
    }
    registry.add({ x, y, w: width, h: 1, target: `dock:edge:${y + height}` });
  });
  return (
    <Box ref={ref} flexDirection="column" width={width} height={height} flexShrink={0}>
    <Framed title={tab ? `agents · ${tab.title}` : "agents"} width={width} height={height} focused={data.focused} plain={plain} rampOwner="dock">
      <CellLine cells={tabRowCells(tabs, at, inner)} plain={plain} />
      <Box flexDirection="column" width={inner} height={bodyH} overflow="hidden">
        {tab?.terminal ? (
          // The host has not given the terminal a socket yet (or a story draws it without one): say what runs.
          (tab.terminal.socket && terminal ? terminal(inner, bodyH + composerH, origin) : null) ?? (
            <CellLine cells={padCells(cellsOf(` ${tab.terminal.command} · ${tab.terminal.state.state}`, slotStyle("text.muted")), inner)} plain={plain} />
          )
        ) : data.picker ? (
          pickerRows(data.picker, pick, inner, bodyH).map((r, i) => <CellLine key={i} cells={r} plain={plain} />)
        ) : items.length === 0 ? (
          [padCells(cellsOf(emptyWords(tabs.length === 0, tab?.state), slotStyle("text.muted")), inner)]
            .slice(0, Math.max(0, bodyH - cmds.length))
            .concat(menuRows)
            .map((r, i) => <CellLine key={i} cells={r} plain={plain} />)
        ) : (
          // One block for the rows that show, each padded once: a key typed in the composer draws the same rows, so the
          // block is the element it was and nothing in the transcript renders again.
          <CellBlock lines={[...shown.rows.map((r) => paddedRow(r, inner)), ...menuRows]} plain={plain} />
        )}
      </Box>
      {activity ? <CellLine cells={activityCells(activity, inner)} plain={plain} /> : null}
      {tab?.terminal ? null : (
        <Box width={inner} height={composerH} flexDirection="column">
          {off ? (
            <CellLine cells={padCells(cellsOf(` ${off}`, slotStyle("text.muted")), inner)} plain={plain} />
          ) : (
            composerLines(text, caret ?? Array.from(text).length, data.focused, composerH).map((l, i) => (
              <InputField key={i} text={l.text} caret={l.caret} mark="> " width={inner} />
            ))
          )}
        </Box>
      )}
      <CellLine cells={statusCells(data.status ?? {}, inner, dockHint(data, refused))} plain={plain} />
    </Framed>
    </Box>
  );
}
