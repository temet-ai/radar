import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { freezeClock, mount, tick } from "../app/harness.js";
import { plain } from "../contract/support.js";
import { ATTENTION_DOT } from "../../src/text/icons.js";
import { iconOf } from "../../src/text/look.js";
import { booksChrome } from "./fixture.js";

// view_tui.rs render_page labels every page tab `icon::with(theme, tab.icon, &tab.label)`: the pack's glyph, one
// space, the label (the bare label when the pack has no glyph); Ink handed TabRow the bare labels.
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures");
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

describe.each([80, 160])("an item page's tab row at %i columns", (cols) => {
  const rows = cols === 80 ? 24 : 40;
  it("puts each tab's icon in front of its label, as Ratatui's tab_row is given them", async () => {
    const view = JSON.parse(fs.readFileSync(path.join(dir, `jira_tickets-page1-overview-comments-links_${cols}x${rows}.json`), "utf8"));
    const m = mount(cols, rows);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: booksChrome({ width: cols, height: rows }), screen: view, section: null }, 1);
    await tick(40);
    const tabs: Array<{ icon?: string; label: string; attention?: boolean }> = view.page.tabs;
    const lead = (t: { icon?: string }) => {
      const g = iconOf(t.icon)?.glyph;
      return g ? `${g} ` : "";
    };
    expect(tabs.some((t) => lead(t) !== ""), "the look has an icon for these tabs").toBe(true);
    const row = plain(m.frame())
      .split("\n")
      .find((l) => l.includes("Overview"));
    expect(row, "the tab row").toBeDefined();
    const want = tabs.map((t, i) => {
      const label = `${lead(t)}${t.label}${t.attention ? ` ${ATTENTION_DOT}` : ""}`;
      return i === view.page.tab ? `[${label}]` : label;
    }).join("  ");
    expect(row!.trim()).toBe(want);
  });
});
