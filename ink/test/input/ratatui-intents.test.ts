import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { screenKey, sectionKey } from "../../src/input/route.js";
import { InputController, type InputView } from "../../src/input/controller.js";
import { UiState } from "../../src/state/ui-state.js";
import type { Intent } from "../../src/chrome/types.js";

/**
 * The pairs in ratatui-intents.json are written by a Rust test that drives the Ratatui adapter
 * (radar-core view_tui::key, section_key, click) over recorded views and fails when they drift
 * (RADAR_WRITE_INK_FIXTURES=1 rewrites them). Ink must send the same Intent for the same chord and view.
 */
const file = path.join(path.dirname(fileURLToPath(import.meta.url)), "ratatui-intents.json");
const fixture = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : undefined;

describe("Ink input equals the Ratatui adapter's", () => {
  it("has cases to check (fails when the Rust writer's fixture is missing)", () => {
    expect(fixture, "ink/test/input/ratatui-intents.json is written by radar-core view_tui tests (RADAR_WRITE_INK_FIXTURES=1)").toBeDefined();
    expect(fixture.screens.length).toBeGreaterThan(0);
    expect(fixture.sections.length).toBeGreaterThan(0);
    expect(fixture.clicks.length).toBeGreaterThan(0);
  });

  for (const s of fixture?.screens ?? []) {
    it.each(s.cases)(`screen ${s.name}: %j`, (c: { chord: any; intent: Intent }) => {
      expect(screenKey(s.view, s.focus, c.chord)).toEqual(c.intent);
    });
  }

  for (const s of fixture?.sections ?? []) {
    it.each(s.cases)(`section ${s.name}: %j`, (c: { chord: any; intent: Intent }) => {
      expect(sectionKey(s.view, c.chord)).toEqual(c.intent);
    });
  }

  it.each(fixture?.clicks ?? [])("click %j", (c: { id: string; context: boolean; intent: Intent }) => {
    const sent: Intent[] = [];
    const view: InputView = {
      menu: () => null,
      capturing: () => false,
      route: (chord) => ({ intent: "key", chord }),
      hits: () => [{ x: 0, y: 5, w: 40, h: 1, target: c.id }],
    };
    const ctl = new InputController(view, (i) => sent.push(i), new UiState(), () => 0);
    ctl.mouse(`\x1b[<${c.context ? 2 : 0};3;6M`);
    expect(sent[0]).toEqual(c.intent);
  });
});
