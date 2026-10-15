import React from "react";
import { describe, expect, it } from "vitest";
import { graphemeAtChar, graphemes, textWidth } from "../../src/editor/graphemes.js";
import { chordOfKey } from "../../src/editor/keys.js";
import { InputField } from "../../src/editor/InputField.js";
import { frameAt } from "../nodes/helpers.js";

const COMBINING = "é"; // é as e + combining acute: one cursor stop
const WIDE = "日本";
const FAMILY = "👨‍👩‍👧"; // one emoji sequence

describe("graphemes and widths", () => {
  it("keeps combining marks and emoji sequences whole, and counts cells", () => {
    expect(graphemes(`a${COMBINING}b`)).toEqual(["a", COMBINING, "b"]);
    expect(graphemes(`x${FAMILY}y`)).toHaveLength(3);
    expect(textWidth(`a${COMBINING}`)).toBe(2);
    expect(textWidth(WIDE)).toBe(4);
    expect(textWidth(FAMILY)).toBe(2);
  });

  it("maps a code-point caret (how the view counts) to a grapheme index", () => {
    // 'e' + combining mark is 2 code points but one grapheme: caret 2 is after it, caret 1 is inside it.
    expect(graphemeAtChar(`${COMBINING}x`, 2)).toBe(1);
    expect(graphemeAtChar(`${COMBINING}x`, 0)).toBe(0);
  });
});

describe("keys go to the host as chords", () => {
  it("maps Ink's key object to view::Chord", () => {
    expect(chordOfKey("a", {})).toEqual({ key: { char: "a" } });
    expect(chordOfKey("r", { ctrl: true })).toEqual({ key: { char: "r" }, ctrl: true });
    expect(chordOfKey("", { return: true })).toEqual({ key: "enter" });
    expect(chordOfKey("", { tab: true, shift: true })).toEqual({ key: "back_tab" });
    expect(chordOfKey("", { pageDown: true })).toEqual({ key: "page_down" });
    expect(chordOfKey("é", {})).toEqual({ key: { char: "é" } });
    expect(chordOfKey("", {})).toBeNull();
  });
});

describe("InputField", () => {
  it("draws the caret on the character it is before, or after the end", () => {
    expect(frameAt(<InputField text={`a${COMBINING}b`} caret={2} />, 40)).toBe(`a${COMBINING}b`);
    expect(frameAt(<InputField text="ab" caret={2} mark="/ " />, 40)).toBe("/ ab");
  });
});
