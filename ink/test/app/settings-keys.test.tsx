import { afterEach, describe, expect, it } from "vitest";
import { mount, tick, until } from "./harness.js";
import { settingsNav } from "../chrome/fixture.js";
import { booksChrome } from "../chrome/fixture.js";

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

const rows = ["views", "look", "tenancies"].map((t) => ({ target: `row:${t}`, text: [{ text: t }] }));
const section = {
  scope: "projects",
  detail_title: "what this does",
  detail: [],
  actions: [],
  body: { node: "list", selected: 0, rows },
};

async function open(focus: string) {
  const m = mount(160, 40);
  live.push(m.stop);
  m.welcome();
  m.snapshot({ chrome: { ...booksChrome({ width: 160, height: 40 }), settings: settingsNav }, section, screen: null, focus }, 1);
  await tick(60);
  return m;
}

describe("settings: the keys go where the focused pane is", () => {
  it.each(["down", "up", "j", "k"])("with the nav focused, %s is a key for the host (it moves the nav), never a row select", async (name) => {
    const bytes: Record<string, string> = { down: "\x1b[B", up: "\x1b[A", j: "j", k: "k" };
    const m = await open("nav");
    m.type(bytes[name]!);
    await until(() => m.intents().length >= 1);
    expect(m.intents().map((i) => i.intent.intent)).toEqual(["key"]);
  });

  it("with the section focused, down selects the next row", async () => {
    const m = await open("middle");
    m.type("\x1b[B");
    await until(() => m.intents().length >= 1);
    expect(m.intents()[0]!.intent).toEqual({ intent: "select", target: "row:look" });
  });

  it("with the detail focused, down is the host's (it scrolls the detail)", async () => {
    const m = await open("detail");
    m.type("\x1b[B");
    await until(() => m.intents().length >= 1);
    expect(m.intents()[0]!.intent.intent).toBe("key");
  });
});
