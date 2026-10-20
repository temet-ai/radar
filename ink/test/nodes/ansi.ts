import { charWidth } from "../../src/chrome/width.js";

/** A terminal cell read back from Ink's output: the character and its style, as `#rrggbb`/`ansi256(n)`/`ansi16(n)`
 * colours and modifiers (the same model as the reference's `Buffer` cells). */
export interface TCell {
  ch: string;
  fg?: string;
  bg?: string;
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  underline?: boolean;
  inverse?: boolean;
  strike?: boolean;
}

const hex = (n: number) => n.toString(16).padStart(2, "0");

/** Parse an ANSI frame (SGR only) into rows of cells. Wide characters give one cell followed by an empty-`ch` cell. */
export function parseAnsi(frame: string): TCell[][] {
  const rows: TCell[][] = [];
  let style: Omit<TCell, "ch"> = {};
  for (const line of frame.split("\n")) {
    const row: TCell[] = [];
    let i = 0;
    while (i < line.length) {
      if (line[i] === "\u001b" && line[i + 1] === "[") {
        const end = line.slice(i).search(/[A-Za-z]/);
        const code = line.slice(i + 2, i + end);
        if (line[i + end] === "m") style = sgr(style, code);
        i += end + 1;
        continue;
      }
      const ch = String.fromCodePoint(line.codePointAt(i)!);
      i += ch.length;
      const w = charWidth(ch.codePointAt(0)!);
      if (w === 0 && row.length) row[row.length - 1]!.ch += ch;
      else {
        row.push({ ch, ...style });
        if (w === 2) row.push({ ch: "", ...style });
      }
    }
    rows.push(row);
  }
  return rows;
}

function sgr(s: Omit<TCell, "ch">, code: string): Omit<TCell, "ch"> {
  const out = { ...s };
  const p = code === "" ? [0] : code.split(";").map(Number);
  for (let k = 0; k < p.length; k++) {
    const c = p[k]!;
    if (c === 0)
      for (const key of Object.keys(out))
        delete (out as Record<string, unknown>)[key];
    else if (c === 1) out.bold = true;
    else if (c === 2) out.dim = true;
    else if (c === 3) out.italic = true;
    else if (c === 4) out.underline = true;
    else if (c === 7) out.inverse = true;
    else if (c === 9) out.strike = true;
    else if (c === 22) {
      delete out.bold;
      delete out.dim;
    } else if (c === 23) delete out.italic;
    else if (c === 24) delete out.underline;
    else if (c === 27) delete out.inverse;
    else if (c === 29) delete out.strike;
    else if (c === 39) delete out.fg;
    else if (c === 49) delete out.bg;
    else if (c === 38 || c === 48) {
      const key = c === 38 ? "fg" : "bg";
      if (p[k + 1] === 2) {
        out[key] = `#${hex(p[k + 2]!)}${hex(p[k + 3]!)}${hex(p[k + 4]!)}`;
        k += 4;
      } else if (p[k + 1] === 5) {
        out[key] = `ansi256(${p[k + 2]})`;
        k += 2;
      }
    } else if ((c >= 30 && c <= 37) || (c >= 90 && c <= 97))
      out.fg = `ansi16(${c})`;
    else if ((c >= 40 && c <= 47) || (c >= 100 && c <= 107))
      out.bg = `ansi16(${c})`;
  }
  return out;
}
