import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { freezeClock, mount, tick } from "../../app/harness.js";
import { booksChrome } from "../../chrome/fixture.js";
import { parseAnsi } from "../../nodes/ansi.js";

// ui::page::tab_row styles only the cells it writes (`TextSecondary` on an inactive title, bold accent on the showing
// one) but the reference's patch semantics keep the page's bg.base under every cell (its paint takes it first): live, in tmux,
// the whole row carries #0a0812 from the leading space on (T22 survey). The earlier version of this test expected none.
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../fixtures");
let unfreeze = () => {};
beforeAll(() => {
  unfreeze = freezeClock();
});
afterAll(() => unfreeze());
let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

describe.each([80, 160])("an MR page's tab row at %i columns", (cols) => {
  const rows = cols === 80 ? 24 : 40;
  it("sits the inactive tabs' titles and the dividers on bg.base", async () => {
    const view = JSON.parse(fs.readFileSync(path.join(dir, `gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket_${cols}x${rows}.json`), "utf8"));
    view.page.tab = 1; // Changes showing, Overview inactive
    const m = mount(cols, rows);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: booksChrome({ width: cols, height: rows }), screen: view, section: null }, 1);
    await tick(40);
    const cells = parseAnsi(m.frame());
    const row = cells.find((r) => r.map((c) => c.ch).join("").includes("Overview"));
    expect(row, "the tab row").toBeDefined();
    const at = row!.findIndex((c, i) => c.ch === "O" && row!.slice(i, i + 8).map((x) => x.ch).join("") === "Overview");
    expect(at).toBeGreaterThanOrEqual(0);
    for (let i = at; i < at + "Overview".length; i++) {
      expect(row![i]!.fg, "an inactive title is TextSecondary").toBeDefined();
      expect(row![i]!.bg, `cell ${i} of 'Overview'`).toBe("#0a0812");
    }
    // the two cells between Overview and the next title are the Tabs divider, drawn the same way
    for (let i = at + 8; i < at + 10; i++) expect(row![i]!.bg, `divider cell ${i}`).toBe("#0a0812");
  });
});
