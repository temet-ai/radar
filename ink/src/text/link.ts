// Links the terminal opens (ui::link for Ink): words wrapped in OSC 8, so a ctrl-click on them in Ghostty opens the
// address, while a plain click still does radar's own action. Only where the host said so (Look.hyperlinks), and only
// for an address a terminal may be handed. The frame writer (app/fast-output.ts) carries the link in the cells' style.
import { currentLook } from "./look.js";

/** ui::link::openable: a web or mail address, no control characters, at most 2048 bytes. */
export function openable(url: string): boolean {
  const lower = url.slice(0, 8).toLowerCase();
  return (
    (lower.startsWith("https://") || lower.startsWith("http://") || lower.startsWith("mailto:")) &&
    url.length <= 2048 &&
    !/[\x00-\x1f\x7f-\x9f]/.test(url)
  );
}

/** The address with every character Ink's width and wrapping code (ansi-regex) would not read as part of an OSC 8
 * percent-encoded, so the sequence measures as no cells. */
export function escaped(url: string): string {
  return url.replace(/[^-a-zA-Z\d/#&.:=?%@~_]/gu, (c) =>
    Array.from(new TextEncoder().encode(c), (b) => `%${b.toString(16).toUpperCase().padStart(2, "0")}`).join(""),
  );
}

/** `text` opening `url`, when links are on and the address may be opened; else `text` as it is. */
export function hyperlink(url: string | null | undefined, text: string, on = currentLook().hyperlinks ?? false): string {
  if (!on || !url || !text || !openable(url)) return text;
  return `\x1b]8;;${escaped(url)}\x1b\\${text}\x1b]8;;\x1b\\`;
}
