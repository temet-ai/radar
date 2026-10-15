import { describe, expect, it } from "vitest";
import type { Key as InkKey } from "ink";
import { chordOf, parseChord, sameChord } from "../../src/input/keys.js";
import { isMouseData, parseMouse } from "../../src/input/mouse.js";
import { InputController, type InputView } from "../../src/input/controller.js";
import { UiState } from "../../src/state/ui-state.js";
import type { Hit, Intent, MenuView } from "../../src/chrome/types.js";
import { actionsMenu } from "../chrome/fixture.js";

const none: InkKey = {
  upArrow: false, downArrow: false, leftArrow: false, rightArrow: false, pageDown: false, pageUp: false,
  home: false, end: false, return: false, escape: false, ctrl: false, shift: false, tab: false,
  backspace: false, delete: false, meta: false, super: false, hyper: false, capsLock: false, numLock: false,
} as InkKey;
const k = (over: Partial<InkKey>): InkKey => ({ ...none, ...over });

describe("keys → chords (the mapping table)", () => {
  const table: Array<[string, string, Partial<InkKey>, unknown]> = [
    ["a letter", "d", {}, { key: { char: "d" } }],
    ["a capital", "G", { shift: true }, { key: { char: "G" } }],
    ["space", " ", {}, { key: { char: " " } }],
    ["ctrl-r", "r", { ctrl: true }, { key: { char: "r" }, ctrl: true }],
    ["alt-x", "x", { meta: true }, { key: { char: "x" }, alt: true }],
    ["enter", "", { return: true }, { key: "enter" }],
    ["esc", "", { escape: true }, { key: "esc" }],
    ["tab", "", { tab: true }, { key: "tab" }],
    ["shift-tab", "", { tab: true, shift: true }, { key: "back_tab" }],
    ["arrows", "", { upArrow: true }, { key: "up" }],
    ["down", "", { downArrow: true }, { key: "down" }],
    ["left", "", { leftArrow: true }, { key: "left" }],
    ["right", "", { rightArrow: true }, { key: "right" }],
    ["pgup", "", { pageUp: true }, { key: "page_up" }],
    ["pgdn", "", { pageDown: true }, { key: "page_down" }],
    ["home", "", { home: true }, { key: "home" }],
    ["end", "", { end: true }, { key: "end" }],
    ["backspace", "", { backspace: true }, { key: "backspace" }],
    ["delete", "", { delete: true }, { key: "delete" }],
    ["a wide character", "界", {}, { key: { char: "界" } }],
  ];
  it.each(table)("%s", (_n, input, key, want) => {
    expect(chordOf(input, k(key))).toEqual(want);
  });

  it("a menu's key strings parse as Chord::parse does", () => {
    expect(parseChord("⏎")).toEqual({ key: "enter" });
    expect(parseChord("space")).toEqual({ key: { char: " " } });
    expect(parseChord("ctrl-r")).toEqual({ key: { char: "r" }, ctrl: true });
    expect(parseChord("pgdn")).toEqual({ key: "page_down" });
    expect(parseChord("nonsense")).toBeUndefined();
    expect(sameChord({ key: { char: "d" } }, parseChord("d")!)).toBe(true);
  });
});

describe("SGR mouse", () => {
  it("parses press, release, wheel and right button with 0-based cells", () => {
    const ev = parseMouse("\x1b[<0;10;5M\x1b[<0;10;5m\x1b[<2;3;4M\x1b[<64;1;1M\x1b[<65;1;1M");
    expect(ev.map((e) => [e.kind, e.button, e.x, e.y])).toEqual([
      ["press", "left", 9, 4],
      ["release", "left", 9, 4],
      ["press", "right", 2, 3],
      ["wheel_up", "none", 0, 0],
      ["wheel_down", "none", 0, 0],
    ]);
    expect(isMouseData("\x1b[<0;10;5M")).toBe(true);
    expect(isMouseData("q")).toBe(false);
  });
});

function setup(over: Partial<{ menu: MenuView | null; capturing: boolean }> = {}) {
  const sent: Intent[] = [];
  const hits: Hit[] = [
    { x: 0, y: 0, w: 10, h: 1, target: "chrome:tab:0" },
    { x: 0, y: 3, w: 40, h: 1, target: "row:7" },
  ];
  let t = 1000;
  const view: InputView = {
    menu: () => over.menu ?? null,
    capturing: () => !!over.capturing,
    route: (chord) => (chord.key === "enter" ? { intent: "activate", target: "row:7" } : { intent: "key", chord }),
    hits: () => hits,
  };
  const ui = new UiState();
  const c = new InputController(view, (i) => sent.push(i), ui, () => t);
  return { c, sent, ui, advance: (ms: number) => (t += ms) };
}

describe("controller", () => {
  it("a key on the body goes through the view's route (the Ratatui adapter's rules, pinned in ratatui-intents.test.ts)", () => {
    const { c, sent } = setup();
    c.key("o", none);
    c.key("", k({ return: true }));
    expect(sent).toEqual([
      { intent: "key", chord: { key: { char: "o" } } },
      { intent: "activate", target: "row:7" },
    ]);
  });

  it("a click on a row selects; two within 400 ms activate; chrome activates on one", () => {
    const { c, sent, advance } = setup();
    c.mouse("\x1b[<0;5;4M");
    advance(100);
    c.mouse("\x1b[<0;5;4M");
    advance(1000);
    c.mouse("\x1b[<0;5;1M");
    expect(sent).toEqual([
      { intent: "select", target: "row:7" },
      { intent: "select", target: "row:7" },
      { intent: "activate", target: "row:7" },
      { intent: "activate", target: "chrome:tab:0" },
    ]);
  });

  it("a slow second click selects again; right click is a context; nothing under the pointer sends nothing", () => {
    const { c, sent, advance, ui } = setup();
    c.mouse("\x1b[<0;5;4M");
    advance(900);
    c.mouse("\x1b[<0;5;4M");
    c.mouse("\x1b[<2;5;4M");
    c.mouse("\x1b[<0;50;20M");
    expect(sent.map((i) => i.intent)).toEqual(["select", "select", "context"]);
    expect(ui.focus).toBe("row:7");
  });

  it("the wheel is k/j on the body (host.rs on_mouse) and ↑/↓ in a menu; a press away from a menu closes it", () => {
    const { c, sent } = setup();
    c.mouse("\x1b[<64;1;1M\x1b[<65;1;1M");
    expect(sent).toEqual([
      { intent: "key", chord: { key: { char: "k" } } },
      { intent: "key", chord: { key: { char: "j" } } },
    ]);
    const m = setup({ menu: actionsMenu });
    m.c.mouse("\x1b[<65;1;1M\x1b[<0;70;20M");
    expect(m.sent).toEqual([
      { intent: "key", chord: { key: "down" } },
      { intent: "key", chord: { key: "esc" } },
    ]);
  });

  it("with a menu open a choice's key invokes its value, ⏎ the selected row, movement goes to the host", () => {
    const { c, sent } = setup({ menu: actionsMenu });
    c.key("m", none);
    c.key("", k({ return: true }));
    c.key("", k({ downArrow: true }));
    c.key("", k({ escape: true }));
    expect(sent).toEqual([
      { intent: "invoke", action: "gitlab.mr.merge" },
      { intent: "invoke", action: "gitlab.mr.open" },
      { intent: "key", chord: { key: "down" } },
      { intent: "key", chord: { key: "esc" } },
    ]);
  });

  it("arrows and ⏎ that arrive together in a menu act on the row they reach, not the one the old snapshot showed", () => {
    const menu = { ...actionsMenu, items: [...actionsMenu.items, { value: "gitlab.mr.close", label: "close", hint: "", key: "c", target: "chrome:menu:3" }], selected: 1 };
    const { c, sent } = setup({ menu });
    c.key("", k({ downArrow: true }));
    c.key("", k({ downArrow: true }));
    c.key("", k({ return: true }));
    expect(sent).toEqual([
      { intent: "key", chord: { key: "down" } },
      { intent: "key", chord: { key: "down" } },
      { intent: "invoke", action: "gitlab.mr.close" },
    ]);
  });

  it("while the host captures text every key is Intent::Key, even declared ones", () => {
    const { c, sent } = setup({ capturing: true });
    c.key("o", none);
    expect(sent).toEqual([{ intent: "key", chord: { key: { char: "o" } } }]);
  });

  it("a paste is one key per character", () => {
    const { c, sent } = setup({ capturing: true });
    c.key("héj", none);
    expect(sent.map((i) => (i as any).chord.key.char)).toEqual(["h", "é", "j"]);
  });
});

describe("a paste while the host captures text (TextInput::paste)", () => {
  const run = (capturing: boolean, text: string) => {
    const sent: Intent[] = [];
    const view: InputView = { menu: () => null, capturing: () => capturing, route: (chord) => ({ intent: "key", chord }), hits: () => [] };
    new InputController(view, (i) => sent.push(i), new UiState(), () => 0).paste(text);
    return sent.map((i) => (i as any).chord.key.char).join("|");
  };
  it("line breaks become one space, leading and trailing ones are dropped", () => {
    expect(run(true, "ab\ncd")).toBe("a|b| |c|d");
    expect(run(true, "\n\nx\r\n")).toBe("x");
    expect(run(true, "a\r\n\r\nb")).toBe("a| |b");
  });
  it("is dropped when nothing captures text", () => {
    expect(run(false, "ab")).toBe("");
  });
});

describe("pointer motion over an open menu", () => {
  const menu: MenuView = {
    id: "m", title: "t", question: false, selected: 0,
    items: [
      { value: "a", label: "a", hint: "", key: "a", target: "chrome:menu:0" },
      { value: "h", label: "head", hint: "", heading: true },
      { value: "b", label: "b", hint: "", key: "b", target: "chrome:menu:2" },
    ],
  } as MenuView;
  const hits: Hit[] = [
    { x: 0, y: 1, w: 20, h: 1, target: "chrome:menu:0" },
    { x: 0, y: 3, w: 20, h: 1, target: "chrome:menu:2" },
  ];
  const run = () => {
    const sent: Intent[] = [];
    const lit: Array<string | null> = [];
    const view: InputView = { menu: () => menu, capturing: () => false, route: (chord) => ({ intent: "key", chord }), hits: () => hits };
    const ctl = new InputController(view, (i) => sent.push(i), new UiState(), () => 0, (t) => lit.push(t));
    return { ctl, sent, lit };
  };
  it("moves the highlight to the item under the pointer once, lights nothing", () => {
    const { ctl, sent, lit } = run();
    ctl.mouse("\x1b[<35;3;4M"); // over item 2
    ctl.mouse("\x1b[<35;5;4M"); // still on it: nothing more
    expect(sent).toEqual([{ intent: "select", target: "chrome:menu:2" }]);
    ctl.mouse("\x1b[<35;3;2M"); // back to item 0
    expect(sent.at(-1)).toEqual({ intent: "select", target: "chrome:menu:0" });
    expect(lit.every((t) => t === null)).toBe(true);
  });
  it("sends nothing over the item that is already highlighted or over empty space", () => {
    const { ctl, sent } = run();
    ctl.mouse("\x1b[<35;3;2M");
    ctl.mouse("\x1b[<35;30;10M");
    expect(sent).toEqual([]);
  });
});
