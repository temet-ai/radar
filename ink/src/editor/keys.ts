// A terminal key as the view contract's Chord (view::Chord): what the Ratatui adapter's `chord_of` produces. While an
// editor captures, every key goes to the host as `{intent:"key", chord}`; the shell does not interpret it.
export type KeyName =
  | { char: string }
  | "enter" | "esc" | "tab" | "back_tab" | "backspace" | "delete" | "up" | "down" | "left" | "right"
  | "home" | "end" | "page_up" | "page_down";

export interface Chord {
  key: KeyName;
  ctrl?: boolean;
  alt?: boolean;
}

/** The shape of Ink's `useInput` key object (only what is read here). */
export interface KeyInput {
  upArrow?: boolean; downArrow?: boolean; leftArrow?: boolean; rightArrow?: boolean;
  pageUp?: boolean; pageDown?: boolean; home?: boolean; end?: boolean;
  return?: boolean; escape?: boolean; tab?: boolean; backspace?: boolean; delete?: boolean;
  ctrl?: boolean; meta?: boolean; shift?: boolean;
}

export function chordOfKey(input: string, key: KeyInput): Chord | null {
  const mods = { ...(key.ctrl ? { ctrl: true } : {}), ...(key.meta ? { alt: true } : {}) };
  const named: [boolean | undefined, KeyName][] = [
    [key.return, "enter"], [key.escape, "esc"], [key.tab && key.shift, "back_tab"], [key.tab, "tab"],
    [key.backspace, "backspace"], [key.delete, "delete"], [key.upArrow, "up"], [key.downArrow, "down"],
    [key.leftArrow, "left"], [key.rightArrow, "right"], [key.home, "home"], [key.end, "end"],
    [key.pageUp, "page_up"], [key.pageDown, "page_down"],
  ];
  for (const [on, name] of named) if (on) return { key: name, ...mods };
  if (input.length > 0) return { key: { char: Array.from(input)[0] }, ...mods };
  return null;
}
