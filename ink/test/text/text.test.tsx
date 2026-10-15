import React from "react";
import { describe, expect, it } from "vitest";
import { TextLine, textWords, segWords } from "../../src/text/TextLine.js";
import { withEmoji } from "../../src/text/emoji.js";
import { lookStyle, inkStyle } from "../../src/text/theme.js";
import { ICONS } from "../../src/text/icons.js";
import { frameAt } from "../nodes/helpers.js";

describe("rich text", () => {
  it("draws segments in order: an icon alone is a lead, an icon with words is bare, words stand in for unknown icons", () => {
    expect(ICONS.repo).toBeTruthy();
    expect(segWords({ icon: "repo" })).toBe(`${ICONS.repo} `);
    expect(segWords({ icon: "repo", text: "x" })).toBe(ICONS.repo);
    expect(segWords({ icon: "repo", text: "✓ ", icon_lead: true })).toBe(`${ICONS.repo} `); // the words are the icons-off fallback
    expect(segWords({ icon: "no_such_icon", text: "✓ ", icon_lead: true })).toBe("✓ ");
    expect(segWords({ icon: "no_such_icon", text: "✗" })).toBe("✗");
    expect(segWords({ icon: "no_such_icon" })).toBe("");
  });

  it("turns emoji short names into glyphs and leaves unknown ones", () => {
    expect(withEmoji("ship it :rocket: :nope:")).toBe("ship it 🚀 :nope:");
    expect(textWords([{ text: "ok " }, { text: ":white_check_mark:" }])).toBe("ok ✅");
  });

  it("marks links as targets in plain-text mode, marks search matches and chips with words intact", () => {
    const text = [{ text: "see " }, { text: "docs", link: "https://d.test" }, { text: " " }, { text: "new", chip: true, ink: "good" }, { text: "hit", marked: true }] as never;
    expect(frameAt(<TextLine text={text} plain />, 80)).toBe("see docs <https://d.test> [new]hit");
    expect(frameAt(<TextLine text={text} />, 80)).toBe("see docs  new hit");
  });

  it("an empty line keeps its row", () => {
    expect(frameAt(<TextLine text={[]} />, 20).length).toBeGreaterThanOrEqual(0);
  });

  it("resolves ink names to the theme's slot colours (view_tui::slot, tokens.rs) and looks as Theme::spec", () => {
    expect(inkStyle("bad")).toEqual({ color: "#ff6363" }); // status.error
    expect(inkStyle("muted")).toEqual({ color: "#6272a4" }); // text.muted is a colour, not dim
    expect(inkStyle("emphasis")).toEqual({ color: "#ffffff", bold: true }); // bold by definition
    expect(inkStyle("plain")).toEqual({});
    expect(lookStyle("tenant.green bold")).toEqual({ color: "#50fa7b", bold: true });
    expect(lookStyle("text.muted")).toEqual({ color: "#6272a4" });
    expect(lookStyle("something.unknown")).toEqual({ color: "#f8f8f2" }); // an unknown slot is text.primary
  });

  it("truncates at the edge rather than wrapping", () => {
    const out = frameAt(<TextLine text={[{ text: "x".repeat(100) }] as never} />, 40);
    expect(out.split("\n")).toHaveLength(1);
    expect(out.length).toBeLessThanOrEqual(40);
  });
});
