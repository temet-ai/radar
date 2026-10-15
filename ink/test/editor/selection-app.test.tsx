import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick } from "../app/harness.js";
import { parseAnsi } from "../nodes/ansi.js";
import { bgOf } from "../../src/text/look.js";

// The whole App over the real host's compose editor (T13's editor_compose fixture: lines "first line", "second line",
// the typed line "héllo 日本", caret 2 from the end): a shift-arrow is the renderer's Intent::EditSelect, anything else
// is still a key chord, and a selection the host sends is painted.
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures/diff");
const last = (m: { intents(): any[] }) => m.intents().at(-1)?.intent;
let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

async function open(withSelection?: [number, number]) {
  const view = JSON.parse(fs.readFileSync(path.join(dir, "editor_compose_80x24.json"), "utf8"));
  if (withSelection) {
    const body = view.screen.overlay.body;
    body.data = { ...body.data, selection: withSelection };
    view.overlay.body.data = body.data;
  }
  const m = mount(view.width, view.height);
  live.push(m.stop);
  m.welcome();
  m.snapshot({ chrome: view.chrome, screen: view.screen, section: view.section, focus: view.focus }, 1);
  await tick(60);
  return { m, data: view.screen.overlay.body.data as { line: string; caret: number } };
}

describe("selecting in the compose editor, through the whole App", () => {
  it("sends the selection as an intent for shift+left and shift+right, not as a key", async () => {
    const { m, data } = await open();
    m.type("\x1b[1;2D"); // shift+left
    await tick(40);
    expect(last(m)).toMatchObject({ intent: "edit_select", anchor: data.caret, extent: data.caret - 1 });
    m.type("\x1b[1;2C"); // shift+right
    await tick(40);
    expect(last(m)).toMatchObject({ intent: "edit_select", anchor: data.caret, extent: data.caret + 1 });
    expect(JSON.stringify(m.intents())).not.toContain('"key":"left"');
  });

  it("still sends a plain arrow as a key chord", async () => {
    const { m } = await open();
    m.type("\x1b[D");
    await tick(40);
    expect(last(m)).toMatchObject({ intent: "key", chord: { key: "left" } });
  });

  it("paints the range the host sends on bg.selection", async () => {
    const { m } = await open([0, 3]);
    const rows = parseAnsi(m.frame());
    const text = rows.map((r) => r.map((c) => c.ch).join(""));
    const y = text.findIndex((t) => t.includes("héllo"));
    expect(y).toBeGreaterThanOrEqual(0);
    const x = text[y]!.indexOf("h");
    const sel = bgOf("bg.selection");
    expect(rows[y]!.slice(x, x + 3).every((c) => c.bg === sel)).toBe(true); // h, é, l
    expect(rows[y]![x + 3]!.bg).not.toBe(sel); // the second l
  });
});
