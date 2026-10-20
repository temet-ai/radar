import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick } from "../app/harness.js";
import { booksChrome } from "./fixture.js";
import { parseAnsi } from "../nodes/ansi.js";

// The line under an item page's title is on bg.base under its words and after them to the row's end, as the reference draws
// the page frame. (T22 read the row as ending at its last word: `tmux capture-pane -e` leaves trailing blanks out, but
// the reference's row writes no `49` before them, and `capture-pane -e -N` shows them on bg.base.)
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures");
let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});
describe("the line under an item page's title", () => {
  it("is on bg.base under its words and after them to the right edge", async () => {
    const view = JSON.parse(fs.readFileSync(path.join(dir, "gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket_160x40.json"), "utf8"));
    const m = mount(160, 40);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: booksChrome({ width: 160, height: 40 }), screen: view, section: null }, 1);
    await tick(60);
    const rows = parseAnsi(m.frame());
    const row = rows.find((r) => r.map((c) => c.ch).join("").includes("checkout"))!;
    expect(row.length).toBe(160);
    expect(row.every((c) => c.bg === "#0a0812")).toBe(true);
  });
});
