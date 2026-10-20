// The view data setup sends, for the four screens whose the reference text is committed in
// crates/radar-core/src/setup/snapshots/. Built from the protocol types the way crates/radar-core/src/setup.rs builds
// them (heading, blank, tick, sheet, the rail's stops), so a field the host adds shows up here as a type error.
import type { ChromeView } from "../../src/protocol/generated/ChromeView.js";
import type { SectionView } from "../../src/protocol/generated/SectionView.js";
import type { ListRow } from "../../src/protocol/generated/ListRow.js";
import type { Block } from "../../src/protocol/generated/Block.js";
import type { Text } from "../../src/protocol/generated/Text.js";
import type { Hint } from "../../src/protocol/generated/Hint.js";

export type Scene = { name: string; width: number; height: number; chrome: ChromeView; section: SectionView };

const heading = (target: string, words: string): ListRow => ({
  target,
  text: [{ text: words.toUpperCase(), ink: "muted", strong: true }],
  heading: true,
});
const blank = (n: number): ListRow => ({ target: `setup:blank:${n}`, text: [], heading: true });
const tick = (on: boolean) =>
  ({ text: on ? "● " : "○ ", ink: on ? "good" : "muted", icon: on ? "done" : "todo", icon_lead: true }) as const;
const lines = (words: string[]): Block[] => words.map((w) => ({ block: "line", text: [{ text: w, ink: "secondary" }] }));
const section = (mark: string, title: string, blocks: Block[]): Block => ({ block: "section", mark, title: title.toUpperCase(), blocks });
const command = (cmd: string): Block => ({
  block: "line",
  text: [
    { text: "$ ", ink: "muted" },
    { text: cmd, ink: "primary" },
  ],
  target: "setup:copy",
});
const sheet = (scope: string, body: SectionView["body"], blocks: Block[], title = "about this step"): SectionView => ({
  version: 1,
  scope,
  body,
  detail_title: title,
  detail: [],
  detail_blocks: blocks,
  actions: [],
});

// Explicit offsets mirror chrome_hint's host-authored metadata; the renderer never chooses a letter.
const hint = (keys: string, label: string, mnemonic_at?: number): Hint => ({ keys, label, keys_of: [], ...(mnemonic_at == null ? {} : { mnemonic_at }) });

function chrome(width: number, height: number, over: Partial<ChromeView>, hints: Hint[], health = false): ChromeView {
  return {
    version: 1,
    width,
    height,
    top_bar: { brand: "radar", tabs: [], mode: { text: "✎ edit", ink: "secondary", target: "chrome:mode" } },
    status: {
      scope: "SETUP",
      hints,
      least_needed: [],
      ...(health ? { health: [{ part: "shelf", label: "shelf talk", state: "ok", ink: "muted", target: "chrome:health:shelf" }] } : {}),
    },
    keys_at: "body",
    ...over,
  } as ChromeView;
}

const scan = (): Scene => {
  const rows: ListRow[] = [
    heading("setup:h:where", "where it looks"),
    { target: "setup:h:env", text: [{ text: "variables ", ink: "muted" }, { text: "SHELF_TOKEN, LEDGER_…", ink: "primary" }], heading: true },
    { target: "setup:h:files", text: [{ text: "sign-ins  ", ink: "muted" }, { text: "shelfctl", ink: "primary" }], heading: true },
    blank(0),
    heading("setup:h:choice", "your choice"),
    { target: "setup:scan:0", text: [tick(true), { text: "yes, look", ink: "primary" }] },
    { target: "setup:scan:1", text: [tick(false), { text: "no thanks", ink: "primary" }] },
  ];
  const blocks = [
    section(
      "search",
      "what it reads",
      lines([
        "The names of the variables listed, and whether the sign-in files listed exist.",
        "It does not read a token's value or a file's contents.",
        "Nothing is sent anywhere, and nothing is saved until you finish a step.",
      ]),
    ),
    section("settings", "if you say no", lines(["Nothing is ticked. Each service's step asks for a token instead."])),
    section("updated", "next time", lines(["radar asks again each time setup runs."])),
  ];
  return {
    name: "scan_80x24",
    width: 80,
    height: 24,
    chrome: chrome(80, 24, {
      setup: {
        title: "look for keys?",
        heading: "May radar look for keys on this machine?",
        lead: "If you have tokens or sign-ins already, radar can use them instead of you pasting them.",
        rail: [],
      },
    }, [
      hint("⏎", "continue"),
      hint("↑↓", "move"),
      hint("esc", "no thanks"),
      hint("y", "copy", 3),
      hint("PgUp/PgDn", "scroll"),
    ]),
    section: sheet("setup.scan", { node: "list", rows, selected: 5 }, blocks),
  };
};

const choose = (): Scene => {
  const choice = (i: number, on: boolean, label: string, about: string, found?: string): ListRow => ({
    target: `setup:choose:${i}`,
    text: [tick(on), { text: label.padEnd(17), ink: "primary" }, { text: about, ink: "muted" }],
    ...(found ? { right: [{ text: found, ink: "good" }] as Text } : {}),
  });
  const rows: ListRow[] = [
    heading("setup:h:this machine", "this machine"),
    choice(0, false, "local code", "checkouts, branches"),
    blank(2),
    heading("setup:h:services", "services"),
    choice(1, true, "Shelves", "boxes, labels", "token found"),
    blank(3),
    heading("setup:h:agents", "agents"),
    choice(2, false, "Chat", "conversations"),
    blank(4),
    heading("setup:h:optional", "optional"),
    choice(3, false, "look & terminal", "icons, theme, motion"),
  ];
  const blocks = [
    section("search", "ticked", lines(["radar found a token or a sign-in for each service ticked."])),
    section(
      "navigate",
      "keys",
      lines([
        "⏎ starts at the row you are on. esc in a step skips it, and ← goes back one.",
        "Anything you leave unticked can be set up later in Settings (,).",
      ]),
    ),
  ];
  return {
    name: "choose_160x40",
    width: 160,
    height: 40,
    chrome: chrome(
      160,
      40,
      {
        setup: {
          title: "choose",
          heading: "What would you like to set up?",
          lead: "Tick the ones you use. Each one ticked gets a short step, in this order.",
          rail: [],
        },
      },
      [hint("⏎", "start here"), hint("space", "tick"), hint("a", "all", 0), hint("n", "none", 0), hint("esc", "not now"), hint("←", "back")],
      true,
    ),
    section: sheet("setup.choose", { node: "list", rows, selected: 1 }, blocks),
  };
};

// A part's step (setup.rs part_step): the part's own section, no heading or lead, its setup sheet.
const partStep = (name: string, width: number, height: number): Scene => ({
  name,
  width,
  height,
  chrome: chrome(
    width,
    height,
    { setup: { title: "Shelves", step: "step 2 of 3", heading: "", lead: "", rail: [] } },
    [hint("⏎", "continue"), hint("space", "choose"), hint("esc", "skip this"), hint("←", "back"), hint("f", "finish later", 0)],
    true,
  ),
  section: sheet(
    "connection",
    {
      node: "list",
      rows: [
        { target: "shelves:server", text: [{ text: "server   ", ink: "muted" }, { text: "https://shelves.example", ink: "primary" }] },
        { target: "shelves:token", text: [{ text: "token    ", ink: "muted" }, { text: "env:SHELF_TOKEN", ink: "primary" }] },
      ],
      selected: 0,
    } as SectionView["body"],
    [
      section("search", "a new token", [
        { block: "line", text: [{ text: "1 ", ink: "muted" }, { text: "open the new-token page", ink: "primary" }] },
      ]),
    ],
  ),
});

// The ready screen (setup.rs ready_view): what radar knows, then the tour's question.
const ready = (name: string, width: number, height: number): Scene => {
  const fact = (id: string, label: string, value: string): ListRow => ({
    target: `setup:h:${id}`,
    text: [{ text: label, ink: "muted", strong: true }, { text: value, ink: "primary" }],
    heading: true,
  });
  const rows: ListRow[] = [
    heading("setup:h:setup", "setup"),
    {
      target: "setup:h:local",
      text: [{ text: "local code  ", ink: "primary" }, { text: " 1 of 1 set up", ink: "muted" }],
      heading: true,
    },
    {
      target: "setup:h:step:1",
      text: [{ text: "mode        ", ink: "primary" }, { text: " read only", ink: "secondary" }],
      heading: true,
    },
    blank(0),
    fact("saved", "SAVED IN  ", "config.toml"),
    fact("mode", "MODE      ", "read only"),
    blank(1),
    heading("setup:h:tour", "a quick tour?"),
    { target: "setup:ready:0", text: [tick(true), { text: "yes, show me around", ink: "primary" }] },
    { target: "setup:ready:1", text: [tick(false), { text: "no thanks", ink: "primary" }] },
  ];
  return {
    name,
    width,
    height,
    chrome: chrome(
      width,
      height,
      {
        setup: { title: "ready", heading: "Setup is done.", lead: "This is what radar has now.", rail: [] },
        // The mode step chose read only.
        top_bar: { brand: "radar", tabs: [], mode: { text: "◉ read only", ink: "info", strong: true, target: "chrome:mode" } },
      } as Partial<ChromeView>,
      [hint("⏎", "yes, a tour"), hint("↑↓", "move"), hint("←", "back"), hint("y", "copy", 3), hint("PgUp/PgDn", "scroll")],
    ),
    section: sheet(
      "setup.ready",
      { node: "list", rows, selected: 8 },
      [
        section("settings", "saved", lines([
          "In the config file shown below. A token is saved as the name of the variable that holds it.",
          "radar config path prints where the file is.",
        ])),
        section("updated", "change it later", [
          ...lines(["Settings (,) has every step's fields. To run setup again:"]),
          command("radar setup"),
        ]),
      ],
    ),
  };
};

export const SCENES: Scene[] = [scan(), choose(), partStep("part_step_160x40", 160, 40), ready("ready_80x24", 80, 24)];
