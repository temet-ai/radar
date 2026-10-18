import { currentLook } from "./look.js";

/** Icon name → the glyph of the pack in use (the host's, else the built-in `unicode` pack; see look.ts). */
export const ICONS: Readonly<Record<string, string>> = new Proxy(
  {},
  {
    get: (_, name) =>
      typeof name === "string"
        ? currentLook().icons[name]?.glyph || undefined
        : undefined,
    has: (_, name) =>
      typeof name === "string" && Boolean(currentLook().icons[name]?.glyph),
  },
) as Record<string, string>;

/** Semantic attention mark, shared with ui::icon::ATTENTION_DOT; visible even with icon packs off. */
export const ATTENTION_DOT = "●";
