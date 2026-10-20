import React, { useContext, useLayoutEffect } from "react";
import { Box, Text as InkText } from "ink";
import type { DiffLine, DiffUnder } from "../nodes/view.js";
import { InputField } from "../editor/InputField.js";
import { selectionRange } from "../editor/selection.js";
import {
  CellLine,
  type Cell,
  cellsOf,
  clipCells,
  lineCells,
  padCells,
  patch,
} from "./cells.js";
import { bgOf, slotStyle, type Style } from "./look.js";
import { selectedStyle } from "../nodes/tree.js";
import { ScrollSink, thumbOf } from "../nodes/index.js";

// ui::diff::DiffView as the host lays it out: a mark column (▎ on the cursor's line in accent.primary), the old and new line
// numbers muted, the sign and the text in the kind's slot; added and removed lines sit on a tint (the diff slot blended
// into bg.surface), the words that changed within a replaced pair on a stronger one; hunk headings status.info; the
// cursor line lit with the selection across the row. Side by side: old on the left, new on the right, a removed line
// facing the line that replaced it, a rule between.

/** widgets::blend: `a` `amount` of the way over `b`, only with two RGB colours (else there is nothing in between). */
export function blendHex(
  a: string | undefined,
  b: string | undefined,
  amount: number,
): string | undefined {
  const rgb = (h?: string) =>
    h && /^#[0-9a-f]{6}$/i.test(h)
      ? [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16))
      : null;
  const x = rgb(a);
  const y = rgb(b);
  if (!x || !y) return undefined;
  const mix = (p: number, q: number) => Math.round(q + (p - q) * amount);
  return `#${[0, 1, 2].map((i) => mix(x[i]!, y[i]!).toString(16).padStart(2, "0")).join("")}`;
}

interface Look {
  added?: string;
  removed?: string;
  addedWord: Style;
  removedWord: Style;
}

/** The diff view's look: added and removed rows on their tints, the words that changed on a stronger one (bold and
 * underlined where there is no colour to tint with). */
export function lookOf(): Look {
  const surface = bgOf("bg.surface");
  const tint = (slot: string, amount: number) =>
    blendHex(slotStyle(slot).color, surface, amount);
  const word = (slot: string): Style => {
    const bg = tint(slot, 0.42);
    return bg
      ? { backgroundColor: bg, bold: true }
      : { bold: true, underline: true };
  };
  return {
    added: tint("diff.added", 0.16),
    removed: tint("diff.removed", 0.16),
    addedWord: word("diff.added"),
    removedWord: word("diff.removed"),
  };
}

/** Words, runs of spaces, and each other character on its own (ui::diff::changed_words). */
function tokens(s: string): string[] {
  const out: string[] = [];
  let start = 0;
  let prev: number | null = null;
  const cls = (c: string) =>
    /[\p{L}\p{N}_]/u.test(c) ? 0 : /\s/.test(c) ? 1 : 2;
  const chars = [...s];
  let pos = 0;
  chars.forEach((c, idx) => {
    const k = cls(c);
    if (prev !== null && (prev !== k || k === 2)) {
      out.push(chars.slice(start, idx).join(""));
      start = idx;
    }
    prev = k;
    pos = idx;
  });
  if (chars.length && start <= pos) out.push(chars.slice(start).join(""));
  return out;
}

/** Where a removed line and the line that replaced it differ, word by word, as character ranges of each; nothing when
 * they share too little for the difference to help. */
export function changedWords(
  oldText: string,
  newText: string,
): { o: [number, number][]; n: [number, number][] } {
  const a = tokens(oldText);
  const b = tokens(newText);
  const dp: number[][] = Array.from({ length: a.length + 1 }, () =>
    new Array(b.length + 1).fill(0),
  );
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--)
      dp[i]![j] =
        a[i] === b[j]
          ? dp[i + 1]![j + 1]! + 1
          : Math.max(dp[i + 1]![j]!, dp[i]![j + 1]!);
  const o: [number, number][] = [];
  const n: [number, number][] = [];
  let same = 0;
  let i = 0;
  let j = 0;
  let ao = 0;
  let bo = 0;
  const push = (r: [number, number][], from: number, to: number) => {
    const last = r[r.length - 1];
    if (last && last[1] === from) last[1] = to;
    else r.push([from, to]);
  };
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) {
      if (a[i]!.trim()) same += [...a[i]!].length;
      ao += [...a[i]!].length;
      bo += [...b[j]!].length;
      i++;
      j++;
    } else if (
      j < b.length &&
      (i === a.length || dp[i]![j + 1]! >= dp[i + 1]![j]!)
    ) {
      const len = [...b[j]!].length;
      push(n, bo, bo + len);
      bo += len;
      j++;
    } else {
      const len = [...a[i]!].length;
      push(o, ao, ao + len);
      ao += len;
      i++;
    }
  }
  const longest = Math.max(
    [...oldText.trim()].length,
    [...newText.trim()].length,
    1,
  );
  if (same * 4 < longest) return { o: [], n: [] };
  return { o, n };
}

const text = (l: DiffLine) => l.text.replace(/\t/g, "    ");
const numStr = (v: number | null | undefined, w: number) =>
  v == null ? " ".repeat(w + 1) : `${v}`.padStart(w) + " ";
const widthsOf = (lines: DiffLine[]): [number, number] => {
  const w = (f: (l: DiffLine) => number | null | undefined) =>
    Math.max(
      1,
      ...lines.map((l) => String(f(l) ?? "").length).filter((n) => n > 0),
    );
  return [w((l) => l.old), w((l) => l.new)];
};

/** A run of a diff line and what the code in it is (view::PaintRun; the host highlights, Ink has no grammars). */
export interface PaintRun {
  text: string;
  token: string;
}

/** ui::highlight::Token::style: the slot of each token, bold keywords and italic comments. */
export function tokenStyle(token: string): Style {
  switch (token) {
    case "comment":
      return { ...slotStyle("text.muted"), italic: true };
    case "text":
      return slotStyle("accent.tertiary");
    case "number":
    case "constant":
      return slotStyle("data.number");
    case "keyword":
      return { ...slotStyle("accent.primary"), bold: true };
    case "type":
      return slotStyle("accent.secondary");
    case "function":
      return slotStyle("glow.cyan");
    case "mark":
      return slotStyle("glow.pink");
    default:
      return slotStyle("text.primary");
  }
}

/** Line `i`'s text as cells in the kind's slot (or, where the host sent `painted` runs, in the code's token slots),
 * with the words that changed in a removed/added pair emphasised: the host compares them (`words`, ui::diff's
 * changed_lines, in characters) so both renderers mark the same words and Ink compares nothing. Only the lines on
 * screen are painted. */
function paintLine(
  lines: DiffLine[],
  i: number,
  look: Look,
  painted?: PaintRun[][],
  words?: [number, number][][],
): Cell[] {
  const l = lines[i]!;
  if (l.kind === "Hunk" || l.kind === "Note") return [];
  const runs = painted?.[i];
  const cells =
    runs && runs.length > 0
      ? runs.flatMap((r) => cellsOf(r.text, tokenStyle(r.token)))
      : cellsOf(
          text(l),
          slotStyle(
            l.kind === "Added"
              ? "diff.added"
              : l.kind === "Removed"
                ? "diff.removed"
                : "text.primary",
          ),
        );
  const ranges = words?.[i];
  if (!ranges || ranges.length === 0) return cells;
  const s = l.kind === "Removed" ? look.removedWord : look.addedWord;
  return cells.map((c, idx) =>
    ranges.some(([f, t]) => idx >= f && idx < t)
      ? { ...c, s: patch(c.s, s) }
      : c,
  );
}

/** ui::diff::split_rows: a removed run faces the added run after it, pair by pair. */
export function splitRows(lines: DiffLine[]): [number | null, number | null][] {
  const out: [number | null, number | null][] = [];
  let i = 0;
  while (i < lines.length) {
    if (lines[i]!.kind === "Removed" || lines[i]!.kind === "Added") {
      const r0 = i;
      while (i < lines.length && lines[i]!.kind === "Removed") i++;
      const a0 = i;
      while (i < lines.length && lines[i]!.kind === "Added") i++;
      const nr = a0 - r0;
      const na = i - a0;
      for (let k = 0; k < Math.max(nr, na); k++)
        out.push([k < nr ? r0 + k : null, k < na ? a0 + k : null]);
    } else {
      out.push([i, i]);
      i++;
    }
  }
  return out;
}

type Row =
  | { kind: "line"; i: number }
  | { kind: "pair"; k: number }
  | { kind: "under"; i: number; block: DiffUnder; n: number; last: boolean };

export interface DiffProps {
  lines: DiffLine[];
  under: DiffUnder[];
  selected: number;
  top: number;
  split: boolean;
  empty: string;
  width: number;
  height?: number;
  focused: boolean;
  plain?: boolean;
  /** The host's highlighting of each line (Node::Diff.painted), empty when the language is unknown. */
  painted?: PaintRun[][];
  /** The words that changed in each line (Node::Diff.words), in characters; empty when none did. */
  words?: [number, number][][];
}

export function DiffView(p: DiffProps) {
  if (p.lines.length === 0)
    return (
      <InkText {...(p.plain ? {} : { color: slotStyle("text.muted").color })}>
        {p.empty}
      </InkText>
    );
  return <DiffBody {...p} />;
}

function DiffBody(p: DiffProps) {
  const sink = useContext(ScrollSink);
  const look = lookOf();
  const kept = new Map<number, Cell[]>();
  const paintedOf = (i: number): Cell[] => {
    let cells = kept.get(i);
    if (!cells) {
      cells = paintLine(p.lines, i, look, p.painted, p.words);
      kept.set(i, cells);
    }
    return cells;
  };
  const [ow, nw] = widthsOf(p.lines);
  const pairs = p.split ? splitRows(p.lines) : [];
  const underOf = (i: number) => p.under.filter((u) => u.line === i);
  const rows: Row[] = [];
  const span: [number, number][] = p.lines.map(() => [0, 0]);
  if (p.split) {
    pairs.forEach(([a, b], k) => {
      const start = rows.length;
      rows.push({ kind: "pair", k });
      const held =
        a === b
          ? a === null
            ? []
            : [a]
          : [a, b].filter((x): x is number => x !== null);
      for (const i of held)
        underOf(i).forEach((blk) =>
          blk.lines.forEach((_, n) =>
            rows.push({
              kind: "under",
              i,
              block: blk,
              n,
              last: n === blk.lines.length - 1,
            }),
          ),
        );
      for (const i of held) span[i] = [start, rows.length];
    });
  } else {
    p.lines.forEach((_, i) => {
      const start = rows.length;
      rows.push({ kind: "line", i });
      underOf(i).forEach((blk) =>
        blk.lines.forEach((_, n) =>
          rows.push({
            kind: "under",
            i,
            block: blk,
            n,
            last: n === blk.lines.length - 1,
          }),
        ),
      );
      span[i] = [start, rows.length];
    });
  }
  const h = p.height ?? rows.length;
  const cursor = Math.min(Math.max(0, p.selected), p.lines.length - 1);
  const [start, end] = span[cursor]!;
  let first = p.top;
  if (end > first + h) first = end - h;
  if (start < first) first = start;
  first = Math.max(0, Math.min(first, Math.max(0, rows.length - h)));
  const shown = rows.slice(first, first + h);
  const width = p.width;
  // ui::scroll::scrollbar: ║ track (bg.overlay) and █ thumb (accent.muted) when there is more than shows. The host paints
  // it on the pane's border column; Ink's frame is not this node's, so it takes the (blank) last column inside.
  const max = rows.length - h;
  // A frame that takes the report draws the bar on its border; the last column inside stays blank.
  useLayoutEffect(() => {
    sink?.({ max: Math.max(0, max), at: first });
    return () => sink?.(null);
  }, [sink, max, first]);
  const thumb = max > 0 && !sink ? thumbOf(shown.length, max, first) : null;
  const bar = (cells: Cell[], idx: number): Cell[] => {
    if (!thumb) return cells;
    const on = idx >= thumb.start && idx < thumb.start + thumb.len;
    const s = p.plain ? {} : slotStyle(on ? "accent.muted" : "bg.overlay");
    return [...padCells(clipCells(cells, Math.max(0, width - 1)), Math.max(0, width - 1)), ...cellsOf(on ? "█" : "║", s)];
  };
  const muted = slotStyle("text.muted");
  const lit = selectedStyle(p.focused, p.plain);
  const mark = (on: boolean): Cell[] =>
    cellsOf(on ? "▎" : " ", slotStyle("accent.primary"));
  const kindStyle = (k: DiffLine["kind"]): { sign: string; s: Style } =>
    k === "Hunk"
      ? { sign: "", s: slotStyle("status.info") }
      : k === "Added"
        ? { sign: "+ ", s: slotStyle("diff.added") }
        : k === "Removed"
          ? { sign: "- ", s: slotStyle("diff.removed") }
          : k === "Context"
            ? { sign: "  ", s: slotStyle("text.primary") }
            : { sign: "  ", s: muted };

  const tintOf = (k: DiffLine["kind"]) =>
    k === "Added" ? look.added : k === "Removed" ? look.removed : undefined;
  const indent = p.split ? 2 : 1 + ow + 1 + nw + 1 + 2;

  const unified = (i: number): Cell[] => {
    const l = p.lines[i]!;
    const { sign, s } = kindStyle(l.kind);
    let cells: Cell[] = [...mark(i === cursor)];
    if (l.kind === "Hunk") cells.push(...cellsOf(l.text, s));
    else {
      cells.push(
        ...cellsOf(numStr(l.old, ow), muted),
        ...cellsOf(numStr(l.new, nw), muted),
        ...cellsOf(sign, s),
      );
      const own = paintedOf(i);
      cells.push(...(own.length ? own : cellsOf(text(l), s)));
    }
    cells = padCells(clipCells(cells, Math.max(0, width - 1)), width);
    const tint = tintOf(l.kind);
    if (tint && !p.plain) {
      const x = 1 + ow + 1 + nw + 1;
      cells = cells.map((c, idx) =>
        idx >= x && idx < width - 1
          ? { ...c, s: patch({ backgroundColor: tint }, c.s) }
          : c,
      );
    }
    if (i === cursor)
      cells = cells.map((c, idx) =>
        idx >= 1 ? { ...c, s: patch(c.s, lit) } : c,
      );
    return cells;
  };

  const side = (
    i: number | null,
    old: boolean,
    w: number,
    numw: number,
    isCursor: boolean,
  ): Cell[] => {
    if (i === null) return cellsOf(" ".repeat(w));
    const l = p.lines[i]!;
    const { sign, s } = kindStyle(
      l.kind === "Hunk" || l.kind === "Note" ? "Context" : l.kind,
    );
    let cells: Cell[] = [
      ...mark(isCursor),
      ...cellsOf(numStr(old ? l.old : l.new, numw), muted),
      ...cellsOf(sign, s),
    ];
    const own = paintedOf(i);
    cells.push(...(own.length ? own : cellsOf(text(l), s)));
    cells = padCells(clipCells(cells, w), w);
    const tint = tintOf(l.kind);
    const numbers = 1 + numw + 1;
    if (tint && !p.plain)
      cells = cells.map((c, idx) =>
        idx >= numbers ? { ...c, s: patch({ backgroundColor: tint }, c.s) } : c,
      );
    if (isCursor)
      cells = cells.map((c, idx) =>
        idx >= 1 ? { ...c, s: patch(c.s, lit) } : c,
      );
    return cells;
  };

  const usable = Math.max(0, width - 1);
  const lw = Math.floor(usable / 2);
  const rw = Math.max(0, usable - (lw + 1));
  const pairRow = (k: number): Cell[] => {
    const [a, b] = pairs[k]!;
    if (
      a === b &&
      a !== null &&
      (p.lines[a]!.kind === "Hunk" || p.lines[a]!.kind === "Note")
    ) {
      const l = p.lines[a]!;
      let cells = padCells(
        clipCells(
          [
            ...mark(a === cursor),
            ...cellsOf(
              l.text,
              l.kind === "Hunk" ? slotStyle("status.info") : muted,
            ),
          ],
          usable,
        ),
        usable,
      );
      if (a === cursor)
        cells = cells.map((c, idx) =>
          idx >= 1 ? { ...c, s: patch(c.s, lit) } : c,
        );
      return cells;
    }
    return [
      ...side(a, true, lw, ow, a === cursor),
      ...cellsOf("│", slotStyle("accent.muted")),
      ...side(b, false, rw, nw, b === cursor),
    ];
  };

  return (
    <Box flexDirection="column">
      {shown.map((r, idx) => {
        if (r.kind === "line")
          return <CellLine key={idx} cells={bar(unified(r.i), idx)} plain={p.plain} />;
        if (r.kind === "pair")
          return <CellLine key={idx} cells={bar(pairRow(r.k), idx)} plain={p.plain} />;
        const room = Math.max(0, width - (indent + 1));
        if (r.last && r.block.input) {
          return (
            <Box key={idx}>
              <InkText>{" ".repeat(indent)}</InkText>
              <InputField
                text={r.block.input.text}
                caret={r.block.input.caret}
                selection={selectionRange(r.block.input.selection)}
                width={room}
              />
            </Box>
          );
        }
        const cells = clipCells(lineCells(r.block.lines[r.n]!, p.plain), room);
        return (
          <CellLine
            key={idx}
            cells={bar([...cellsOf(" ".repeat(indent)), ...cells], idx)}
            plain={p.plain}
          />
        );
      })}
    </Box>
  );
}
