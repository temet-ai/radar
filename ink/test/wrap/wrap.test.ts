// wrap.ts against what Ratatui's radar-core wraps (ui::rich::wrap for documents, ui::wrap_words for Wrapped blocks).
import { describe, expect, it } from "vitest";
import { textPieces, trimPieces, wrapDoc, wrapPieces, wrapWords } from "../../src/text/wrap.js";

describe("wrapDoc (ui::rich::wrap)", () => {
  it("breaks the GitHub description where Ratatui's snapshot does, with no leading space", () => {
    // tests/snapshots/github_links__integrated_preview_160x40.snap: the DESCRIPTION pane, 44 cells.
    expect(wrapDoc("Responses stay with the repository and commit they were requested for.", 44)).toEqual([
      "Responses stay with the repository and",
      "commit they were requested for.",
    ]);
  });
  it("a continued line never starts with a space, even at the exact edge", () => {
    for (const room of [5, 6, 7, 10, 12, 13]) {
      for (const l of wrapDoc("alpha beta gamma delta epsilon zeta", room)) expect(l.startsWith(" ")).toBe(false);
    }
    expect(wrapDoc("aaaa bbbb", 9)).toEqual(["aaaa bbbb"]);
    expect(wrapDoc("aaaa bbbb", 8)).toEqual(["aaaa", "bbbb"]);
  });
  it("breaks a word wider than the room where it runs out", () => {
    expect(wrapDoc("ab abcdefghijkl cd", 5)).toEqual(["ab", "abcde", "fghij", "kl cd"]);
  });
  it("counts CJK and emoji as two cells", () => {
    expect(wrapDoc("漢字漢字 abc", 6)).toEqual(["漢字漢", "字 abc"]);
    expect(wrapDoc("😀😀 hi", 4)).toEqual(["😀😀", "hi"]);
  });
  it("collapses double spaces and trims the ends", () => {
    expect(wrapDoc("a  b   c", 80)).toEqual(["a b c"]);
    expect(wrapDoc("a  b   c", 3)).toEqual(["a b", "c"]);
    expect(wrapDoc("   hi  ", 80)).toEqual(["hi"]);
  });
  it("keeps adjacent styled runs together as one word and honours a break", () => {
    const p = trimPieces([
      ...textPieces("bold", "b"),
      ...textPieces(",", "p"),
      ...textPieces(" next", "p"),
      { kind: "break" as const },
      ...textPieces("after", "p"),
    ]);
    expect(wrapPieces(p, 5)).toEqual([
      [{ text: "bold", tag: "b" }, { text: ",", tag: "p" }],
      [{ text: "next", tag: "p" }],
      [{ text: "after", tag: "p" }],
    ]);
  });
});

describe("wrapWords (ui::wrap_words)", () => {
  it("joins words with single spaces and breaks before the word that does not fit", () => {
    expect(wrapWords("one  two   three four", 9)).toEqual(["one two", "three", "four"]);
    expect(wrapWords("aaaa bbbb", 8)).toEqual(["aaaa", "bbbb"]);
    expect(wrapWords("aaaa bbb", 8)).toEqual(["aaaa bbb"]);
  });
  it("never goes under 8 cells and subtracts the prefix", () => {
    expect(wrapWords("aaaa bbbb cccc", 3)).toEqual(["aaaa", "bbbb", "cccc"]);
    expect(wrapWords("aaaa bbb ccc", 12, 4)).toEqual(["aaaa bbb", "ccc"]);
  });
  it("leaves a word wider than the room whole, and keeps empty paragraphs", () => {
    expect(wrapWords("abcdefghijkl x", 8)).toEqual(["abcdefghijkl", "x"]);
    expect(wrapWords("a\n\nb\n", 20)).toEqual(["a", "", "b"]);
  });
  it("counts wide characters as two cells", () => {
    expect(wrapWords("漢字漢 漢字漢", 14)).toEqual(["漢字漢 漢字漢"]);
    expect(wrapWords("漢字漢字 漢字漢字", 17)).toEqual(["漢字漢字 漢字漢字"]);
    expect(wrapWords("漢字漢字 漢字漢字", 16)).toEqual(["漢字漢字", "漢字漢字"]);
  });
});

// Cases Ratatui's own wrap produced (tests/wrap_goldens.rs writes the file): wrap.ts must equal them all.
import { readFileSync } from "node:fs";
type Golden = { fn: "rich" | "words"; text: string; room?: number; width?: number; prefix?: number; lines: string[] };
const golden: Golden[] = JSON.parse(readFileSync(new URL("../fixtures/wrap/golden.json", import.meta.url), "utf8"));
describe("Ratatui goldens", () => {
  for (const g of golden) {
    const name = `${g.fn} ${JSON.stringify(g.text)} @ ${g.room ?? g.width}${g.prefix ? ` prefix ${g.prefix}` : ""}`;
    it(name, () => {
      const got = g.fn === "rich" ? wrapDoc(g.text, g.room!) : wrapWords(g.text, g.width!, g.prefix ?? 0);
      expect(got).toEqual(g.lines);
    });
  }
});
