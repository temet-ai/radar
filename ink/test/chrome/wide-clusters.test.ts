// Text is cut, wrapped and placed by grapheme (chrome/width.ts clusters), never by code point: `⚠` alone is one cell,
// `⚠` + U+FE0F two. The corpus is shared with radar-core's ui width tests (crates/radar-core/src/ui/tests), so the
// host and Ink count every case the same.
import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { cellWidth, clipSegments, clusters, fit } from "../../src/chrome/width.js";
import { middleFit } from "../../src/dock/tools.js";
import { noticeWidth, wrapNotice } from "../../src/notices/text.js";
import { wrapDoc, wrapWords } from "../../src/text/wrap.js";

const corpus: Array<{ text: string; cells: number }> = JSON.parse(
  fs.readFileSync(path.resolve(path.dirname(fileURLToPath(import.meta.url)), "fixtures/wide-clusters.json"), "utf8"),
);

describe("wide clusters", () => {
  it("are measured as the terminal draws them", () => {
    for (const { text, cells } of corpus) {
      expect(cellWidth(text), text).toBe(cells);
      expect(clusters(text).reduce((n, [, w]) => n + w, 0), text).toBe(cells);
      expect(clusters(text).map(([g]) => g).join(""), text).toBe(text);
      expect(noticeWidth(text), text).toBe(cells);
    }
  });

  it("are never cut in half or past the room they are cut to", () => {
    for (const { text, cells } of corpus) {
      for (let room = 0; room <= cells + 1; room++) {
        const at = `${JSON.stringify(text)} in ${room}`;
        expect(cellWidth(fit(text, room)), at).toBeLessThanOrEqual(room);
        expect(cellWidth(middleFit(text, room)), at).toBeLessThanOrEqual(room);
        expect(cellWidth(clipSegments([{ text }], room).map((s) => s.text).join("")), at).toBeLessThanOrEqual(room);
        // a two-cell cluster is kept whole when it alone is wider than the room (as 界 is); wrapping starts at two cells
        if (room < 2) continue;
        // ui::wrap_words keeps a word wider than the room whole, on a line of its own, and never wraps under 8 cells
        for (const line of wrapWords(text, room))
          if (/\s/u.test(line)) expect(cellWidth(line), at).toBeLessThanOrEqual(Math.max(8, room));
        for (const line of wrapDoc(text, room)) expect(cellWidth(line), at).toBeLessThanOrEqual(room);
        for (const line of wrapNotice(text, room)) expect(cellWidth(line), at).toBeLessThanOrEqual(room);
      }
    }
  });
});
