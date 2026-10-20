import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick } from "../app/harness.js";
import { booksChrome } from "../chrome/fixture.js";

/**
 * Media over view data the real host made on demo data (radar-jira's ABC-13 page holds an image, an external image
 * and two files; written by T14 to ink/test/fixtures/media/jira_ticket-media_<w>x<h>.json). The whole App draws it
 * in a basic terminal (no image protocol); each Media block must be the line ui::rich draws without graphics:
 * `▣ image <name> <url>` / `▤ file <name>` (crates/radar-core/src/ui/rich.rs, media_without_graphics_...).
 */
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures/media");
const sizes = ["80x24", "160x40"] as const;

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

const mediaOf = (node: any): any[] => {
  if (Array.isArray(node)) return node.flatMap(mediaOf);
  if (node && typeof node === "object") return (node.t === "media" ? [node] : []).concat(Object.values(node).flatMap(mediaOf));
  return [];
};
const line = (m: any) => [m.image ? "▣ image" : "▤ file", m.name, m.url].filter(Boolean).join(" ");

describe("media blocks from the host's demo data", () => {
  for (const size of sizes) {
    it(`${size}: the fixture holds the image and file blocks the demo page declares`, () => {
      const view = JSON.parse(fs.readFileSync(path.join(dir, `jira_ticket-media_${size}.json`), "utf8"));
      const media = mediaOf(view);
      expect(media.map((m) => m.name)).toEqual(["dashboard.png", "stats design", "requests.csv", "trace.log"]);
      expect(media.filter((m) => m.image)).toHaveLength(2);
    });

    it(`${size}: a basic terminal draws each block as the reference's no-graphics line`, async () => {
      const [cols, rows] = size.split("x").map(Number) as [number, number];
      const view = JSON.parse(fs.readFileSync(path.join(dir, `jira_ticket-media_${size}.json`), "utf8"));
      const m = mount(cols, rows);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ chrome: booksChrome({ width: cols, height: rows }), screen: view, section: null, focus: "detail" }, 1);
      await tick(40);
      const frame = m.frame();
      const drawn = frame.split("\n").map((l) => l.replace(/[│┃]/g, " ").replace(/\s+/g, " "));
      if (cols < 100) {
        // The description is cut at the pane's edge: the host pages it (the screen has a page, so PgDn is the host's).
        m.type("\x1b[6~");
        await tick(40);
        expect(JSON.stringify(m.intents()), "PgDn reaches the host").toContain("page_down");
        return;
      }
      // Every one of the four blocks is on screen and is the reference's line, whole on one row (not wrapped).
      const all = mediaOf(view);
      expect(all).toHaveLength(4);
      for (const b of all) expect(drawn.some((l) => l.includes(line(b))), `${line(b)} in\n${frame}`).toBe(true);
      expect(frame).not.toContain("\x1b_G");
    });
  }
});
