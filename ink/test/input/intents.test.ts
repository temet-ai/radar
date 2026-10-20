import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { screenKey, sectionKey } from "../../src/input/route.js";
import { InputController, type InputView } from "../../src/input/controller.js";
import { UiState } from "../../src/state/ui-state.js";
import type { Intent } from "../../src/chrome/types.js";
import { sortedJson, updatingGoldens } from "../support/goldens.js";

/**
 * intents.json pairs a chord (or a click) on a small recorded view with the Intent it comes to. The pairs were first
 * written from the old Rust adapter; the file is Ink's golden now, frozen and committed, and Ink must send the same
 * Intent for the same chord and view. INK_UPDATE_GOLDENS=1 rewrites each pair's `intent` from Ink's own routing.
 */
const file = path.join(path.dirname(fileURLToPath(import.meta.url)), "intents.json");
const fixture = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : undefined;

type Click = { id: string; context: boolean; intent: Intent };

/** What a left or right click on a row of the given target comes to, through the whole controller. */
function clickIntent(c: Pick<Click, "id" | "context">): Intent | undefined {
  const sent: Intent[] = [];
  const view: InputView = {
    menu: () => null,
    capturing: () => false,
    route: (chord) => ({ intent: "key", chord }),
    hits: () => [{ x: 0, y: 5, w: 40, h: 1, target: c.id }],
  };
  const ctl = new InputController(view, (i) => sent.push(i), new UiState(), () => 0);
  ctl.mouse(`\x1b[<${c.context ? 2 : 0};3;6M`);
  return sent[0];
}

if (fixture && updatingGoldens()) {
  for (const s of fixture.screens) for (const c of s.cases) c.intent = screenKey(s.view, s.focus, c.chord);
  for (const s of fixture.sections) for (const c of s.cases) c.intent = sectionKey(s.view, c.chord);
  for (const c of fixture.clicks) c.intent = clickIntent(c);
  fs.writeFileSync(file, sortedJson(fixture));
}

describe("Ink's input against the intents golden", () => {
  it("has cases to check (fails when the golden is missing)", () => {
    expect(fixture, "ink/test/input/intents.json is a committed golden").toBeDefined();
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

  it.each(fixture?.clicks ?? [])("click %j", (c: Click) => {
    expect(clickIntent(c)).toEqual(c.intent);
  });
});
