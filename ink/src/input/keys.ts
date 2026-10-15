import type { Key as InkKey } from "ink";
import type { Chord, Key } from "../chrome/types.js";

/** An Ink keypress as the view's Chord, or none for a keypress that is not one key (a paste is split by the caller). */
export function chordOf(input: string, key: InkKey): Chord | undefined {
  const mods = { ...(key.ctrl ? { ctrl: true } : {}), ...(key.meta ? { alt: true } : {}) };
  const named: Key | undefined = key.upArrow
    ? "up"
    : key.downArrow
      ? "down"
      : key.leftArrow
        ? "left"
        : key.rightArrow
          ? "right"
          : key.pageUp
            ? "page_up"
            : key.pageDown
              ? "page_down"
              : key.home
                ? "home"
                : key.end
                  ? "end"
                  : key.return
                    ? "enter"
                    : key.escape
                      ? "esc"
                      : key.tab
                        ? key.shift
                          ? "back_tab"
                          : "tab"
                        : key.backspace
                          ? "backspace"
                          : key.delete
                            ? "delete"
                            : undefined;
  if (named) return { key: named, ...mods };
  const chars = [...input];
  if (chars.length !== 1) return undefined;
  const c = chars[0]!;
  // Ink sets shift for capitals; the host's chords carry the capital itself, not a shift flag.
  return { key: { char: c }, ...mods };
}

export function sameChord(a: Chord, b: Chord): boolean {
  const ka = JSON.stringify(a.key);
  const kb = JSON.stringify(b.key);
  return ka === kb && !!a.ctrl === !!b.ctrl && !!a.alt === !!b.alt;
}

/** The chord a menu's `keys` string names (`d`, `⏎`, `space`, `ctrl-r`), as `Chord::parse` reads it. */
export function parseChord(keys: string): Chord | undefined {
  let ctrl = false;
  let k = keys;
  if (k.startsWith("ctrl-")) {
    ctrl = true;
    k = k.slice(5);
  }
  const named: Record<string, Key> = {
    "⏎": "enter", enter: "enter", esc: "esc", tab: "tab", "shift-tab": "back_tab", "←": "left",
    "→": "right", "↑": "up", "↓": "down", pgup: "page_up", pgdn: "page_down", space: { char: " " },
  };
  const hit = named[k];
  if (hit) return { key: hit, ...(ctrl ? { ctrl: true } : {}) };
  const chars = [...k];
  if (chars.length === 1) return { key: { char: chars[0]! }, ...(ctrl ? { ctrl: true } : {}) };
  return undefined;
}
