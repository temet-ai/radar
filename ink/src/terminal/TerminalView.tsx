import React, { useEffect, useReducer, useRef } from "react";
import { Box, Text } from "ink";
import type { MouseBus } from "../input/stdin-tap.js";
import { tokenize } from "../input/tokens.js";
import { slotStyle } from "../text/look.js";
import type { Frame, TerminalState } from "./frames.js";
import { translateInput } from "./input.js";
import type { TerminalLink } from "./link.js";
import { TerminalScreen, type Row } from "./screen.js";

export interface TerminalViewProps {
  /** Opens the connection to the program's terminal (the socket's `connectSocket`, or a recording's). Called once per
   * `openKey`; the link is closed when the view goes or the key changes. */
  open: () => TerminalLink;
  /** Names which terminal `open` opens: a different key is a different terminal. */
  openKey: string;
  /** The cells the surface fills; the program's terminal is resized to them. */
  columns: number;
  rows: number;
  /** The dock has the keys and this tab is showing: every key and mouse report goes to the program. */
  focused: boolean;
  /** radar's input tap: while focused the surface takes reads whole (`MouseBus.claimRaw`). */
  bus?: MouseBus;
  /** Where the surface's top-left cell is on the terminal, 1-based, so mouse reports can be moved into it. */
  origin?: { row: number; col: number };
  /** Ctrl+] (`chord`) or a click outside the surface (`click`): the dock takes the keys back. */
  onLeave?: (why: "chord" | "click") => void;
  /** The program ended (or could not start). */
  onExit?: (state: TerminalState) => void;
}

/** The tab's terminal state as the surface holds it: what the host last said, or that the host went away. */
type Held = TerminalState | { state: "lost"; message: string; generation: number; columns: number; rows: number };

/**
 * A program's own screen inside a dock tab. The host runs the program and keeps its output; this attaches to it, parses the bytes (xterm's engine) and draws the cells. Keys and mouse go back untouched but for
 * the translations in `input.ts`. A program that ended shows its last screen, how it ended and `r̲estart`.
 */
export function TerminalView(props: TerminalViewProps) {
  const { columns, rows, focused } = props;
  const [, redraw] = useReducer((n: number) => n + 1, 0);
  const screen = useRef<TerminalScreen | null>(null);
  const link = useRef<TerminalLink | null>(null);
  const held = useRef<Held | undefined>(undefined);
  const latest = useRef(props);
  latest.current = props;
  if (!screen.current) screen.current = new TerminalScreen(columns, rows);

  // One redraw per burst of output, not one per chunk.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    screen.current!.onChange = () => {
      timer ??= setTimeout(() => {
        timer = undefined;
        redraw();
      }, 16);
    };
    return () => {
      if (timer) clearTimeout(timer);
      screen.current!.onChange = () => {};
    };
  }, []);

  // The connection: replay, then live. A change of socket is a different terminal.
  useEffect(() => {
    const l = props.open();
    link.current = l;
    let generation = -1;
    // Frames are applied in order: output waits for the write before it.
    let chain: Promise<void> = Promise.resolve();
    const apply = (f: Frame) => {
      chain = chain.then(async () => {
        if (f.kind === "output") await screen.current!.write(f.bytes);
        else {
          if (f.state.generation !== generation) {
            if (generation !== -1) screen.current!.reset();
            generation = f.state.generation;
          }
          const was = held.current?.state;
          held.current = f.state;
          // The program's terminal follows the surface, not the size it was started at.
          const s = latest.current;
          if (f.state.columns !== s.columns || f.state.rows !== s.rows) l.resize(s.columns, s.rows);
          if (f.state.state !== "running" && was !== f.state.state) latest.current.onExit?.(f.state);
          redraw();
        }
      });
    };
    l.onFrame(apply);
    // After the frames already queued, so the end of the connection is the last thing that happened.
    l.onClose((why) => {
      chain = chain.then(() => {
        held.current = { state: "lost", message: why, generation: Math.max(generation, 0), columns: 0, rows: 0 };
        redraw();
      });
    });
    l.resize(latest.current.columns, latest.current.rows);
    return () => {
      l.close();
      if (link.current === l) link.current = null;
    };
  }, [props.openKey]);

  // The size the dock gives is the program's size.
  useEffect(() => {
    screen.current!.resize(columns, rows);
    link.current?.resize(columns, rows);
  }, [columns, rows]);

  // Focus: while it is ours, every read of the terminal's input comes here whole.
  useEffect(() => {
    if (!focused || !props.bus) return;
    return props.bus.claimRaw((text) => {
      const s = latest.current;
      const sc = screen.current!;
      const l = link.current;
      const h = held.current;
      if (!h || h.state === "running") {
        const t = translateInput(text, { modes: sc.modes, origin: s.origin, columns: s.columns, rows: s.rows });
        if (t.scroll) sc.scroll(t.scroll);
        if (t.typed && sc.scrollOffset > 0) sc.scroll(-sc.scrollOffset);
        if (t.data && l) l.input(t.data);
        if (t.leave) s.onLeave?.("chord");
        else if (t.outside) s.onLeave?.("click");
        // A click outside the surface also does what it does anywhere else in radar (a tab's close mark closes it).
        if (t.outside && t.passOn) props.bus!.emitInput(tokenize(t.passOn).events);
        return;
      }
      // Over: the keys are the surface's own.
      if (text.includes("\x1d")) s.onLeave?.("chord");
      else if (h.state !== "lost" && /(^|[^\x1b])r/.test(text)) l?.restart();
    });
  }, [focused, props.bus]);

  const sc = screen.current!;
  const h = held.current;
  const over = !!h && h.state !== "running";
  let shown: Row[] = sc.runs(focused && !over);
  if (over) {
    // Room for the status row: the last rows the program used.
    const last = sc.lastUsedRow();
    const keep = Math.max(1, rows - 1);
    const from = Math.max(0, last - keep + 1);
    shown = shown.slice(from, from + keep);
  }
  return (
    <Box flexDirection="column" width={columns} height={rows} overflow="hidden">
      {shown.map((row, y) => (
        <Text key={y} wrap="truncate">
          {row.map((r, i) => (
            <Text
              key={i}
              color={r.color}
              backgroundColor={r.backgroundColor}
              bold={r.bold}
              dimColor={r.dim}
              italic={r.italic}
              underline={r.underline}
              inverse={r.inverse}
              strikethrough={r.strike}
            >
              {r.text}
            </Text>
          ))}
        </Text>
      ))}
      {over ? <EndedRow held={h!} columns={columns} /> : null}
    </Box>
  );
}

/** How it ended and the keys that apply, in the dock's words: keys inside their word (r̲estart), glyph keys first. */
export function endedText(h: Held): { head: string; failed: boolean } {
  if (h.state === "lost") return { head: `connection lost (${h.message})`, failed: true };
  if (h.state === "failed") return { head: `could not start: ${h.message ?? "unknown error"}`, failed: true };
  if (h.signal) return { head: `ended by signal ${h.signal}`, failed: true };
  const code = h.code ?? null;
  return { head: code === null ? "exited" : `exited with status ${code}`, failed: code !== null && code !== 0 };
}

function EndedRow({ held, columns }: { held: Held; columns: number }) {
  const { head, failed } = endedText(held);
  const tone = slotStyle(failed ? "status.error" : "text.muted");
  const muted = slotStyle("text.muted");
  const canRestart = held.state !== "lost";
  return (
    <Box width={columns} height={1} overflow="hidden">
      <Box flexShrink={0}>
        <Text wrap="truncate" color={tone.color} bold={tone.bold}>
          {head}
        </Text>
      </Box>
      <Box flexShrink={1}>
        <Text wrap="truncate" color={muted.color}>
          {canRestart ? (
            <>
              {"  "}
              <Text underline>r</Text>estart
            </>
          ) : null}
          {"  "}ctrl-] leave
        </Text>
      </Box>
    </Box>
  );
}
