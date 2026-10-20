import { PACK, SLOTS } from "./generated/pack.js";
import type { ColourTier } from "../protocol/generated/ColourTier.js";

/** The look the host resolved for this terminal: the colour of every theme slot (core and the parts' own, as
 * `#rrggbb`) and the icon pack in use (name → glyph and the style the icon is drawn in). Ratatui reads these from
 * `Theme`/`icon::Icons`; Ink draws with exactly them and chooses nothing of its own. */
export interface Look {
  /** The host-resolved capability tier, never inferred from the slot values. */
  tier: ColourTier;
  slots: Readonly<Record<string, string>>;
  icons: Readonly<Record<string, { glyph: string; style: string }>>;
  /** Slots whose own style carries modifiers (`text.emphasis`: bold), in Theme::spec's words. */
  modifiers?: Readonly<Record<string, string>>;
  /** Cells a glyph takes (0 = icons off, 1, or 2 for emoji). */
  iconWidth: number;
  /** Links are written as OSC 8 hyperlinks (the host decides, from `[theme] hyperlinks` and the terminal). */
  hyperlinks?: boolean;
  /** The pointer's shape follows what it is over, by OSC 22 (`[theme] pointer`). */
  pointer?: boolean;
}

/** The host's `look` message (snake_case, ts-rs) as this module holds it. */
export interface HostLook {
  tier: ColourTier;
  slots: Record<string, string>;
  modifiers?: Record<string, string>;
  icons: Record<string, { glyph: string; style: string }>;
  icon_width: number;
  hyperlinks?: boolean;
  pointer?: boolean;
}
export function applyHostLook(l: HostLook) {
  applyLook({
    tier: l.tier,
    slots: l.slots,
    modifiers: l.modifiers,
    icons: l.icons,
    iconWidth: l.icon_width,
    hyperlinks: l.hyperlinks ?? false,
    pointer: l.pointer ?? false,
  });
}

/** The built-in defaults: truecolor, the `unicode` pack. Used until the host sends its own (`applyLook`). */
export function defaultLook(
  pack: "unicode" | "nerd" | "emoji" | "off" = "unicode",
): Look {
  const icons: Record<string, { glyph: string; style: string }> = {};
  for (const [name, i] of Object.entries(PACK))
    icons[name] = { glyph: pack === "off" ? "" : i[pack], style: i.style };
  return {
    tier: "truecolor",
    slots: SLOTS,
    modifiers: { "text.emphasis": "bold" },
    icons,
    iconWidth: pack === "off" ? 0 : pack === "emoji" ? 2 : 1,
  };
}

let current: Look = defaultLook();
export const currentLook = (): Look => current;
/** The host's mono tier (`Tier::Mono`): every slot is "" (the terminal's own colours), so a colour cannot mark
 * anything and the selection, the focus and the hover carry a modifier instead. */
export const monoTier = (): boolean => current.slots["bg.selection"] === "";
export function applyLook(look: Look) {
  current = look;
}

/** What a run of text is drawn in: colours as `#rrggbb` and the modifiers Ratatui's `Style` carries. */
export interface Style {
  color?: string;
  backgroundColor?: string;
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  underline?: boolean;
  inverse?: boolean;
  strike?: boolean;
  /** The address the words open (OSC 8, text/link.ts): written only where the host turned links on. */
  link?: string;
}

/** Theme::style: a slot's own style (the colour, and the modifiers the slot carries); an unknown name is
 * text.primary. An empty colour is "no colour" (the mono tier). */
export function slotStyle(name: string): Style {
  const known = name in current.slots;
  const key = known ? name : "text.primary";
  const hex = current.slots[key];
  const out: Style = hex ? { color: hex } : {};
  for (const w of (current.modifiers?.[key] ?? "").split(/\s+/)) {
    if (w === "bold") out.bold = true;
    else if (w === "italic") out.italic = true;
    else if (w === "underline") out.underline = true;
    else if (w === "dim") out.dim = true;
    else if (w === "reversed") out.inverse = true;
    else if (w === "strike") out.strike = true;
  }
  return out;
}

/** Theme::spec: a slot name then modifiers (`status.error bold`). */
export function specStyle(spec: string | null | undefined): Style {
  const words = (spec ?? "").split(/\s+/).filter(Boolean);
  const out: Style = words.length
    ? slotStyle(words[0]!)
    : slotStyle("text.primary");
  for (const w of words.slice(1)) {
    if (w === "bold") out.bold = true;
    else if (w === "italic") out.italic = true;
    else if (w === "underline") out.underline = true;
    else if (w === "dim") out.dim = true;
    else if (w === "reversed") out.inverse = true;
    else if (w === "strike") out.strike = true;
  }
  return out;
}

export type Rgb = [number, number, number];

/** theme::mix: `a` mixed `perMille` of the way toward `b`, each channel rounded half up, as Ratatui rounds it. */
export const mix = (a: Rgb, b: Rgb, perMille: number): Rgb =>
  a.map((x, i) => Math.floor((x * (1000 - perMille) + b[i]! * perMille + 500) / 1000)) as Rgb;

/** `#rrggbb` as channels; undefined for any other colour (a 256 index, a terminal colour's name, none). */
export const hexRgb = (h: string | undefined): Rgb | undefined => {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(h ?? "");
  return m ? [parseInt(m[1]!, 16), parseInt(m[2]!, 16), parseInt(m[3]!, 16)] : undefined;
};
export const rgbHex = (c: Rgb): string => `#${c.map((v) => v.toString(16).padStart(2, "0")).join("")}`;

/** Theme::bg: a slot's colour as a background. */
export function bgOf(name: string): string | undefined {
  return slotStyle(name).color;
}

export function iconOf(
  name: string | null | undefined,
): { glyph: string; style: string } | undefined {
  return name ? current.icons[name] : undefined;
}
