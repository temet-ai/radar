// Word wrapping exactly as Ratatui's radar-core does it, so a document breaks where it does there.
//   wrapWords   ui::wrap_words (ui/mod.rs): a Wrapped block (the GitHub DESCRIPTION, notes).
//   wrapPieces  ui::rich::wrap (ui/rich.rs): a document's paragraphs, headings and list items.
// Both collapse every run of whitespace to one space, never start a continued line with a space, count wide
// characters as two cells, and break a word wider than the room where it runs out (rich) or leave it whole (words).
import { cellWidth, clusters } from "../chrome/width.js";

const isSpace = (ch: string) => /\s/u.test(ch);

/** ui::wrap_words: `text` broken between words to fit `width` after a prefix of `prefixWidth` cells. The room is
 * never under 8. Each paragraph (line of `text`) is split on whitespace and rejoined with single spaces; a word
 * wider than the room stays whole on a line of its own; an empty paragraph is an empty line. */
export function wrapWords(text: string, width: number, prefixWidth = 0): string[] {
  const room = Math.max(8, Math.max(0, width - prefixWidth));
  const out: string[] = [];
  // Rust's str::lines: split on \n, a trailing \r is dropped, a final empty piece after a trailing \n is not a line.
  const paras = text.split("\n");
  if (paras.length > 0 && paras[paras.length - 1] === "") paras.pop();
  for (const para of paras) {
    let line = "";
    for (const word of para.split(/\s+/u)) {
      if (word === "") continue;
      if (line !== "" && cellWidth(line) + 1 + cellWidth(word) > room) {
        out.push(line);
        line = "";
      }
      if (line !== "") line += " ";
      line += word;
    }
    out.push(line);
  }
  return out;
}

/** What a run of inline text is made of before it is wrapped (ui::rich::Piece); `T` is the caller's style tag. */
export type Piece<T> =
  | { kind: "text"; text: string; tag: T }
  | { kind: "space"; tag: T }
  | { kind: "break" };

/** A run of one tag on a wrapped line. */
export type Run<T> = { text: string; tag: T };

/** ui::rich::text_pieces: `text` as words and spaces (a run of whitespace is one space). */
export function textPieces<T>(text: string, tag: T): Piece<T>[] {
  const out: Piece<T>[] = [];
  let word = "";
  let inSpace = false;
  for (const ch of text) {
    if (isSpace(ch)) {
      if (word) {
        out.push({ kind: "text", text: word, tag });
        word = "";
      }
      if (!inSpace) out.push({ kind: "space", tag });
      inSpace = true;
    } else {
      inSpace = false;
      word += ch;
    }
  }
  if (word) out.push({ kind: "text", text: word, tag });
  return out;
}

/** ui::rich::trim_pieces: leading and trailing spaces and breaks dropped. */
export function trimPieces<T>(p: Piece<T>[]): Piece<T>[] {
  let end = p.length;
  while (end > 0 && p[end - 1]!.kind !== "text") end--;
  let start = 0;
  while (start < end && p[start]!.kind !== "text") start++;
  return p.slice(start, end);
}

/** ui::rich::wrap: pieces broken between words to lines at most `room` cells wide. Adjacent pieces with no space
 * between them are one word (a bold word then its comma). A space is written only when another word follows it on
 * the same line, so a continued line never starts with one. */
export function wrapPieces<T>(pieces: Piece<T>[], roomIn: number): Run<T>[][] {
  const room = Math.max(1, roomIn);
  const lines: Run<T>[][] = [];
  let cur: Run<T>[] = [];
  let w = 0;
  const put = (text: string, tag: T, cw: number) => {
    const last = cur[cur.length - 1];
    if (last && last.tag === tag) last.text += text;
    else cur.push({ text, tag });
    w += cw;
  };
  const newline = () => {
    lines.push(cur);
    cur = [];
    w = 0;
  };
  let word: Run<T>[] = [];
  let ww = 0;
  let space: { tag: T } | null = null;
  const place = () => {
    if (word.length === 0) return;
    const sp = w > 0 && space ? 1 : 0;
    if (w + sp + ww <= room || (ww <= room && w === 0)) {
      if (sp && space) put(" ", space.tag, 1);
      for (const r of word) put(r.text, r.tag, cellWidth(r.text));
    } else if (ww <= room) {
      newline();
      for (const r of word) put(r.text, r.tag, cellWidth(r.text));
    } else {
      // A word wider than the room: broken where it runs out.
      if (w > 0) newline();
      for (const r of word) {
        for (const [ch, cw] of clusters(r.text)) {
          if (w > 0 && w + cw > room) newline();
          put(ch, r.tag, cw);
        }
      }
    }
    word = [];
    ww = 0;
    space = null;
  };
  for (const p of pieces) {
    if (p.kind === "text") {
      ww += cellWidth(p.text);
      word.push({ text: p.text, tag: p.tag });
    } else if (p.kind === "space") {
      place();
      space = { tag: p.tag };
    } else {
      place();
      newline();
      space = null;
    }
  }
  place();
  if (cur.length > 0) newline();
  return lines;
}

/** A plain string wrapped as a document paragraph does: `wrapPieces` over one tag, as text lines. */
export function wrapDoc(text: string, room: number): string[] {
  return wrapPieces(trimPieces(textPieces(text, 0)), room).map((l) => l.map((r) => r.text).join(""));
}
