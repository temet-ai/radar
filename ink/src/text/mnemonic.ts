import { graphemes } from "../editor/graphemes.js";
import type { Style } from "./look.js";

export type Mnemonic = { mnemonic_at?: number | null; mnemonic_shift?: boolean };
export type StyledRun = { text: string; style?: Style };

/** Validate only the host's supplied scalar position; never search for another match. */
export function mnemonicRange(words: string, keys: string, at: number | null | undefined): [number, number] | undefined {
  if (at == null || !Number.isInteger(at) || at < 0 || !/^\p{Alphabetic}$/u.test(keys)) return undefined;
  const scalars = [...words];
  if (at >= scalars.length || !scalars.slice(at).join("").toLowerCase().startsWith(keys.toLowerCase())) return undefined;
  let start = 0;
  for (const g of graphemes(words)) {
    const end = start + [...g].length;
    if (start <= at && at < end) return [start, end];
    start = end;
  }
  return undefined;
}

/** A visible separate prefix. Valid Shift metadata names the modifier, never duplicates the letter. */
export function mnemonicPrefix(words: string, keys: string, meta: Mnemonic): string {
  return mnemonicRange(words, keys, meta.mnemonic_at) ? (meta.mnemonic_shift ? "shift" : "") : keys;
}

/** Underline the complete containing grapheme and retain every other style property. */
export function mnemonicRuns(words: string, keys: string, meta: Mnemonic, style: Style = {}): StyledRun[] {
  const range = mnemonicRange(words, keys, meta.mnemonic_at);
  if (!range) return [{ text: words, style }];
  const scalars = [...words];
  return [
    { text: scalars.slice(0, range[0]).join(""), style },
    { text: scalars.slice(...range).join(""), style: { ...style, underline: true } },
    { text: scalars.slice(range[1]).join(""), style },
  ].filter((r) => r.text !== "");
}
