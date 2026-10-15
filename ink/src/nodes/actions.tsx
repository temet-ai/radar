import React from "react";
import { Box, Text as InkText } from "ink";
import type { ActionSpec, ChordName } from "./view.js";

/** A chord as the actions menu writes it (`d`, `⏎`, `ctrl-r`), the same names view::Chord::name gives. */
export function chordName(chord: ChordName): string {
  const key = chord.key;
  const base =
    typeof key === "object"
      ? key.char === " " ? "space" : key.char
      : ({ enter: "⏎", esc: "esc", tab: "tab", back_tab: "shift-tab", backspace: "backspace", delete: "delete", up: "↑", down: "↓", left: "←", right: "→", home: "home", end: "end", page_up: "pgup", page_down: "pgdn" } as const)[key];
  return chord.ctrl ? `ctrl-${base}` : base;
}

/** The intent that runs an action by its stable id (Intent::Invoke), never by its key. */
export function invokeIntent(action: ActionSpec): { intent: "invoke"; action: string } {
  return { intent: "invoke", action: action.id };
}

/** The action a key runs, as an Invoke intent: the key a view declares for an action wins over sending the raw key,
 * so the host journals the action id. Headings and actions with no id are not runnable. */
export function intentForChord(actions: readonly ActionSpec[], chord: ChordName): { intent: "invoke"; action: string } | null {
  const name = chordName(chord);
  const hit = actions.find((a) => !a.heading && a.id && a.key && chordName(a.key) === name);
  return hit ? invokeIntent(hit) : null;
}

/** The part's actions listed where a node the renderer cannot draw sits, so nothing the part offers is lost with it. */
export function ActionList({ actions }: { actions: readonly ActionSpec[] }) {
  if (actions.length === 0) return null;
  return (
    <Box flexDirection="column">
      {actions.map((a, i) =>
        a.heading ? (
          <InkText key={i} bold>
            {a.label}
          </InkText>
        ) : (
          <InkText key={a.id || i}>
            {"  "}
            {a.key ? `${chordName(a.key)}  ` : "   "}
            {a.label}
          </InkText>
        ),
      )}
    </Box>
  );
}
