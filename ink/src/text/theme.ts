import type { InkName, Tone } from "../nodes/view.js";
import { slotStyle, specStyle, type Style } from "./look.js";

export type { Style };

/** view_tui::slot: the theme slot each Ink name stands for (`plain` has none: the terminal's own text). */
const SLOT_OF: Readonly<Record<InkName, string | null>> = {
  plain: null,
  primary: "text.primary",
  secondary: "text.secondary",
  muted: "text.muted",
  emphasis: "text.emphasis",
  accent: "accent.primary",
  accent_muted: "accent.muted",
  accent_secondary: "accent.secondary",
  accent_tertiary: "accent.tertiary",
  good: "status.success",
  warn: "status.warning",
  bad: "status.error",
  info: "status.info",
  number: "data.number",
  added: "diff.added",
  removed: "diff.removed",
};

export function inkStyle(ink: InkName | undefined): Style {
  const slot = SLOT_OF[ink ?? "plain"];
  return slot ? slotStyle(slot) : {};
}

export function toneInk(tone: Tone): InkName {
  return tone === "plain" ? "plain" : tone;
}

/** A part's own look (`tenant.green bold`, `text.muted`) resolved in the host's theme (Theme::spec). */
export function lookStyle(look: string | null | undefined): Style {
  return look ? specStyle(look) : {};
}
