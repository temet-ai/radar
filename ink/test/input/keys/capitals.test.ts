import { describe, expect, it } from "vitest";
import { InputController, type InputView } from "../../../src/input/controller.js";
import { UiState } from "../../../src/state/ui-state.js";
import type { Chord, Intent } from "../../../src/chrome/types.js";

/**
 * Class: a key the host's `on_key` takes before the body's movement (host.rs). An ASCII capital is a screen or
 * source key there (G is GitHub in the Repositories nav pane); only when no screen has it does it reach the part, where
 * `G` is "last row". Ratatui sends the terminal key to the host, which decides; Ink must not answer `G` with its own
 * Select, or the host never sees it (the walk Repositories → G → GitHub stopped at the last row).
 */
function ctl(sent: Intent[], screenKeys?: string[]) {
  const view: InputView = {
    menu: () => null,
    capturing: () => false,
    // The body's movement: every key is a Select, so a key that reaches it is visible in `sent`.
    route: () => ({ intent: "select", target: "body-row" }),
    hits: () => [],
    screenKeys: () => screenKeys,
  };
  return new InputController(view, (i) => sent.push(i), new UiState(), () => 0);
}

const char = (c: string, over: Partial<Chord> = {}): Chord => ({ key: { char: c }, ...over });

describe("capitals are the host's first", () => {
  it.each(["G", "R", "J", "S", "B", "W", "H", "Z"])("%s goes to the host as a key", (c) => {
    const sent: Intent[] = [];
    ctl(sent).keyChord(char(c));
    expect(sent).toEqual([{ intent: "key", chord: char(c) }]);
  });

  it.each(["g", "j", "k", "x", "1", " ", "+", "-"])("%s still goes through the body's movement", (c) => {
    const sent: Intent[] = [];
    ctl(sent).keyChord(char(c));
    expect(sent).toEqual([{ intent: "select", target: "body-row" }]);
  });

  it("named keys and modified capitals take the body's route as before", () => {
    const sent: Intent[] = [];
    const c = ctl(sent);
    c.keyChord({ key: "down" });
    c.keyChord(char("G", { ctrl: true }));
    expect(sent.map((i) => i.intent)).toEqual(["select", "select"]);
  });
});

describe("with the host's list of claimed capitals (ChromeView.screen_keys)", () => {
  it("a claimed capital goes to the host, an unclaimed one is the body's (G: last row / end of a document)", () => {
    const sent: Intent[] = [];
    const c = ctl(sent, ["R", "J", "G"]);
    c.keyChord(char("G"));
    c.keyChord(char("K"));
    expect(sent).toEqual([{ intent: "key", chord: char("G") }, { intent: "select", target: "body-row" }]);
  });
  it("an empty list (no part claims a capital) leaves G the body's: it scrolls a document to its end", () => {
    const sent: Intent[] = [];
    ctl(sent, []).keyChord(char("G"));
    expect(sent).toEqual([{ intent: "select", target: "body-row" }]);
  });
  it("without G in the list G is the body's", () => {
    const sent: Intent[] = [];
    ctl(sent, ["R", "J"]).keyChord(char("G"));
    expect(sent).toEqual([{ intent: "select", target: "body-row" }]);
  });
});
