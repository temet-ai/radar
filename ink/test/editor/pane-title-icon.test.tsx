import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { mount, tick } from "../app/harness.js";
import { applyLook, currentLook, defaultLook, iconOf } from "../../src/text/look.js";
import { paneTitle } from "../../src/app/Panes.js";

// ui view_tui draw_pane: a pane whose view data carries `title_icon` shows its title with the icon before it (the diff
// pane: the file's icon, `file_markdown` for docs/README.md in T13's real-host diff view). Found live on a real MR:
// the reference drew `╭ <icon> src/…`, Ink `╭ src/…`.
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures/diff");
const before = currentLook();
beforeAll(() => applyLook(defaultLook("unicode")));
afterAll(() => applyLook(before));
let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

describe("a pane's title icon", () => {
  it("is the glyph and a space before the title, and nothing with icons off", () => {
    const glyph = iconOf("file_markdown")?.glyph;
    expect(glyph).toBeTruthy();
    expect(paneTitle({ title: "docs/README.md", title_icon: "file_markdown" })).toBe(`${glyph} docs/README.md`);
    expect(paneTitle({ title: "files", title_icon: null })).toBe("files");
    applyLook(defaultLook("off"));
    expect(paneTitle({ title: "docs/README.md", title_icon: "file_markdown" })).toBe("docs/README.md");
    applyLook(defaultLook("unicode"));
  });

  it("is drawn in the diff pane's frame over the real host's view", async () => {
    const view = JSON.parse(fs.readFileSync(path.join(dir, "diff_unified_160x40.json"), "utf8"));
    const m = mount(view.width, view.height);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: view.chrome, screen: view.screen, section: view.section, focus: view.focus }, 1);
    await tick(60);
    const glyph = iconOf("file_markdown")!.glyph;
    const row = m.frame().split("\n").find((l) => l.includes("docs/README.md") && l.includes("╭"))!;
    expect(row).toContain(`${glyph} docs/README.md`);
  });
});
