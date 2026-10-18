// The host's look for each colour tier the toolbar offers. Ink draws only with the Look the host sends
// (ToRenderer::Look through src/text/look.ts); the page never picks a colour.
//
// stories/looks/<tier>.json is the Look the real host sent at that tier, captured by .storybook/capture-looks.mjs.
// Without it, truecolor is the host's defaults as src/text/generated/pack.ts holds them (radar's SilkCircuit Neon
// theme; the captured truecolor look equals them) and mono is every slot "" (ipc/look.rs colour_word at Tier::Mono);
// 256 and 16 are then not offered.
import { applyLook, defaultLook, type Look } from "../../src/text/look";
import { cssColour } from "./colour";

export type Tier = "truecolor" | "256" | "16" | "mono";
export type Pack = "off" | "unicode" | "nerd" | "emoji";

type Captured = { tier: string; source: string; slots: Record<string, string>; modifiers?: Record<string, string> };
const captured = import.meta.glob<Captured>("../looks/*.json", { eager: true, import: "default" });
const capturedTier = (t: Tier): Captured | undefined => captured[`../looks/${t}.json`];

export const tiers = (): Tier[] => ["truecolor", ...(["256", "16"] as Tier[]).filter((t) => capturedTier(t)), "mono"];

export function lookFor(tier: Tier, pack: Pack): Look {
  const base = { ...defaultLook(pack), tier };
  const c = capturedTier(tier);
  if (c) return { ...base, slots: { ...base.slots, ...c.slots }, modifiers: c.modifiers ?? base.modifiers };
  if (tier === "mono") return { ...base, slots: Object.fromEntries(Object.keys(base.slots).map((k) => [k, ""])) };
  return base;
}

/** Make `tier` and `pack` the look Ink draws in; returns a key that changes with them. */
export function useLookOf(tier: Tier, pack: Pack): string {
  applyLook(lookFor(tier, pack));
  return `${tier}/${pack}`;
}

/** The terminal's own colours under the tier: the canvas Ratatui paints on (text.primary on bg.base); at mono, a
 * plain dark terminal. */
export function canvasOf(tier: Tier): { fg: string; bg: string } {
  const look = lookFor(tier, "off");
  return { fg: cssColour(look.slots["text.primary"]) ?? "#d0d0d0", bg: cssColour(look.slots["bg.base"]) ?? "#101010" };
}

/** Where a tier's look came from, for the docs. */
export const lookSource = (tier: Tier): string => capturedTier(tier)?.source ?? (tier === "truecolor" ? "src/text/generated/pack.ts" : "every slot empty");
