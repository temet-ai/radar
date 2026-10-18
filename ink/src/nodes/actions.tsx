import React from "react";
import { Box, Text as InkText } from "ink";
import type { ActionSpec, ChordName } from "./view.js";
import { mnemonicPrefix, mnemonicRuns } from "../text/mnemonic.js";
import { textProps } from "../text/TextLine.js";

/** A chord as the actions menu writes it (`d`, `⏎`, `ctrl-r`), the same names view::Chord::name gives. */
export function chordName(chord: ChordName): string {
  const key = chord.key;
  const base =
    typeof key === "object"
      ? key.char === " " ? "space" : key.char
      : ({ enter: "⏎", esc: "esc", tab: "tab", back_tab: "shift-tab", backspace: "backspace", delete: "delete", up: "↑", down: "↓", left: "←", right: "→", home: "home", end: "end", page_up: "pgup", page_down: "pgdn" } as const)[key];
  return chord.ctrl ? `ctrl-${base}` : base;
}

/** Presentation keeps every supplied modifier; dispatch's legacy comparison stays separate. */
export function displayChord(chord: ChordName): string {
  const base = chordName({ ...chord, ctrl: false, alt: false });
  return `${chord.ctrl ? "ctrl-" : ""}${chord.alt ? "alt-" : ""}${base}`;
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
export function ActionList({ actions, plain }: { actions: readonly ActionSpec[]; plain?: boolean }) {
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
            {(() => {
              const key = a.key ? displayChord(a.key) : "";
              const prefix = plain ? key : mnemonicPrefix(a.label, key, a);
              return <>{prefix ? `${prefix}  ` : ""}{mnemonicRuns(a.label, key, plain ? {} : a).map((run, j) => <InkText key={j} {...textProps(run.style ?? {})}>{run.text}</InkText>)}</>;
            })()}
          </InkText>
        ),
      )}
    </Box>
  );
}
