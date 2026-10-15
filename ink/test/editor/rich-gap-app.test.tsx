import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick } from "../app/harness.js";
import { booksChrome } from "../chrome/fixture.js";

// The whole App over T14's host view data (radar-jira's ABC-13 page, image and file blocks one after another): ui::rich's
// Layout::sep puts a blank row between blocks that follow each other (its test expects image, "", file), so the
// description's media lines are every other row inside the pane.
const file = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures/media/jira_ticket-media_160x40.json");

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

describe("rich blocks through the whole App", () => {
  it("are a blank row apart: the four media lines of ABC-13 sit on every other row", async () => {
    const view = JSON.parse(fs.readFileSync(file, "utf8"));
    const m = mount(160, 40);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: booksChrome({ width: 160, height: 40 }), screen: view, section: null, focus: "detail" }, 1);
    await tick(80);
    // The description pane's text between its borders (the first pane on the row).
    const rows = m.frame().split("\n").map((l) => (/^│(.*?)│/.exec(l)?.[1] ?? "").trimEnd().trim());
    const at = ["▣ image dashboard.png", "▣ image stats design", "▤ file requests.csv", "▤ file trace.log"].map((t) => rows.findIndex((r) => r.startsWith(t)));
    expect(at.every((i) => i >= 0), JSON.stringify(at)).toBe(true);
    for (let k = 1; k < at.length; k++) {
      expect(at[k]! - at[k - 1]!, `block ${k}`).toBe(2);
      expect(rows[at[k]! - 1], `row between blocks ${k - 1} and ${k}`).toBe("");
    }
  });
});
