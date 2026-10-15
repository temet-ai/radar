import React from "react";
import { Text as InkText } from "ink";
import type { Seg, Text } from "../nodes/view.js";
import { ICONS } from "./icons.js";
import { inkStyle, lookStyle, toneInk, type Style } from "./theme.js";
import { bgOf, iconOf, slotStyle, specStyle } from "./look.js";
import { withEmoji } from "./emoji.js";
import { Rows, useBg } from "./cells.js";
import { chipRuns, chipText, chipWordStyle } from "./chips.js";

/** The words a segment draws: its icon's glyph and/or its text. An icon with words is drawn bare (the words stand in
 * where there is no glyph); an icon alone is a lead with the pack's one-cell gap. */
export function segWords(seg: Seg): string {
  const text = withEmoji(seg.text ?? "");
  const glyph = seg.icon ? ICONS[seg.icon] : undefined;
  // view::Seg: an icon alone is a lead (the glyph and the pack's gap). An icon with words is drawn bare, the words
  // standing in only where there is no glyph. With `icon_lead` the words are the fallback for icons-off and already
  // carry their own gap, so a glyph replaces them with the glyph and its gap.
  if (seg.icon && !text) return glyph ? `${glyph} ` : "";
  if (seg.icon && glyph) return seg.icon_lead ? `${glyph} ` : glyph;
  return text;
}

/** A line of view text as one plain string (icons as glyphs): widths, plain-text mode, tests. */
export function textWords(text: Text): string {
  return text.map(segWords).join("");
}

/** view_tui::words: a part's `look` (plus bold when strong), else the ink's slot (bold when strong), else plain text
 * (bold primary when strong). An icon is drawn in the icon's own style (icon::span). */
export function segRunStyle(seg: Seg): Style {
  const glyph = seg.icon ? iconOf(seg.icon) : undefined;
  if (glyph?.glyph) return specStyle(glyph.style);
  let base: Style;
  if (seg.look) base = lookStyle(seg.look);
  else if (seg.ink && seg.ink !== "plain") base = inkStyle(seg.ink);
  else base = {};
  if (seg.strong)
    base = {
      ...(seg.look || (seg.ink && seg.ink !== "plain")
        ? base
        : slotStyle("text.primary")),
      bold: true,
    };
  return base;
}

/** The words one run draws and the chip's padding: what view_tui::spans puts in the buffer for a Seg. */
export function segRunWords(seg: Seg, plain?: boolean): string {
  if (seg.chip && seg.icon && !plain) return chipText(seg);
  const words = seg.chip && seg.icon && seg.text ? seg.text : segWords(seg);
  if (!words) return "";
  if (plain) {
    const shown = seg.chip ? `[${words}]` : words;
    return seg.link ? `${shown} <${seg.link}>` : shown;
  }
  return seg.chip ? ` ${words} ` : words;
}

/** A Ratatui-style run as Ink <Text> props. */
export function textProps(s: Style, over: { inverse?: boolean } = {}) {
  return {
    color: s.color,
    backgroundColor: s.backgroundColor,
    bold: s.bold,
    dimColor: s.dim,
    italic: s.italic,
    underline: s.underline,
    strikethrough: s.strike,
    inverse: s.inverse || over.inverse,
  };
}

export function SegRun({ seg, plain }: { seg: Seg; plain?: boolean }) {
  if (seg.chip && seg.icon && !plain)
    return (
      <>
        {chipRuns(seg).map((r, i) => (
          <InkText key={i} {...textProps(r.style)}>
            {r.text}
          </InkText>
        ))}
      </>
    );
  const words = segWords(seg);
  if (!words) return null;
  if (plain) {
    const shown = seg.chip ? `[${words}]` : words;
    return <InkText>{seg.link ? `${shown} <${seg.link}>` : shown}</InkText>;
  }
  const style = segRunStyle(seg);
  if (seg.chip) {
    // widgets::chip: ` words ` in the tone's colour on the overlay shade.
    return <InkText {...textProps(chipWordStyle(seg))}>{` ${words} `}</InkText>;
  }
  // A link travels as an OSC 8 hyperlink in Ratatui, not as underlined text.
  return (
    <InkText {...textProps(style, { inverse: seg.marked })}>{words}</InkText>
  );
}

/** Adds the line's segments to `rows`, each in the style its <SegRun> would draw. */
function fillLine(rows: Rows, text: Text, plain?: boolean) {
  for (const seg of text) {
    if (seg.chip && seg.icon && !plain) {
      for (const r of chipRuns(seg)) rows.put(r.text, r.style);
      continue;
    }
    const words = segWords(seg);
    if (!words) continue;
    if (plain) {
      const shown = seg.chip ? `[${words}]` : words;
      rows.put(seg.link ? `${shown} <${seg.link}>` : shown, {});
      continue;
    }
    let style = segRunStyle(seg);
    if (seg.chip) {
      style = chipWordStyle(seg);
      rows.put(` ${words} `, style);
    } else {
      if (seg.marked) style = { ...style, inverse: true };
      rows.put(words, style);
    }
  }
}

/** The line's characters without styles (tests, widths). */
export function textLineString(text: Text, plain?: boolean): string {
  const rows = new Rows(plain).line();
  fillLine(rows, text, plain);
  return rows.realText();
}

/** One line of rich text, as one Ink <Text> of plain characters with style runs (no node per segment). Wraps nothing: the
 * caller gives it a Box of the width it may use. */
export function TextLine({ text, plain }: { text: Text; plain?: boolean }) {
  const bg = useBg();
  if (text.length === 0) return <InkText> </InkText>;
  const rows = new Rows(plain, bg).line();
  fillLine(rows, text, plain);
  return rows.element("truncate-end") ?? <InkText> </InkText>;
}

export { toneInk };
