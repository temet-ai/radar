import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Snapshot } from "../../src/protocol/generated/Snapshot.js";
import { applyLook, currentLook, defaultLook } from "../../src/text/look.js";
import { freezeClock, mount, until } from "../app/harness.js";
import { plain } from "../contract/support.js";
import { attachMotion } from "../../src/motion/attach.js";

// A production hosted-provider refresh, reduced by Scenario, supplies this checkpoint.
// tests/provider_scenarios.rs checks these files against a fresh journey and renders
// the very same checkpoint with Ratatui. This is renderer replay, not socket E2E.
// Keep the case explicit: deleting fixtures must fail, never silently skip coverage.
const checkpoint = new URL("../fixtures/scenarios/github_refresh_120x40.json", import.meta.url);
const ratatui = new URL("../fixtures/scenarios/github_refresh_120x40.txt", import.meta.url);
type Recording = Pick<Snapshot, "chrome" | "screen" | "section" | "focus"> & {
  width: number;
  height: number;
};

const rows = (text: string): string[] => {
  const result = plain(text).split("\n").map((line) => line.trimEnd());
  while (result.at(-1) === "") result.pop();
  return result;
};

// Opt-in diagnostic capture only; this is never a fixture-update mechanism.
// The wrapper provides a new private directory outside the checkout. Exclusive
// creation refuses existing files and symlinks, including fixture destinations.
const captureActual = (frame: string) => {
  const output = process.env.RADAR_SCENARIO_ACTUAL_OUTPUT;
  if (!output || !frame) return;
  const root = fileURLToPath(new URL("../../..", import.meta.url));
  const resolved = path.resolve(output);
  const relative = path.relative(root, resolved);
  if (!path.isAbsolute(output) || output !== resolved ||
      (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative)) ||
      fs.realpathSync(path.dirname(resolved)) !== path.dirname(resolved)) {
    throw new Error("diagnostic output requires an absolute path outside the checkout with no symlink parents");
  }
  if (Buffer.byteLength(frame, "utf8") > 512 * 1024) throw new Error("diagnostic frame exceeds 512 KiB");
  fs.writeFileSync(resolved, frame, { encoding: "utf8", flag: "wx", mode: 0o600 });
};

const look = currentLook();
let unfreeze = () => {};
let stop = () => {};
beforeAll(() => {
  unfreeze = freezeClock();
  applyLook(defaultLook("off"));
});
afterEach(() => stop());
afterAll(() => {
  unfreeze();
  applyLook(look);
});

describe("scenario-derived renderer checkpoint", () => {
  it("draws the same visible GitHub refresh text as Ratatui", async () => {
    expect(fs.existsSync(checkpoint), "missing scenario JSON: regenerate with tests/provider_scenarios.rs").toBe(true);
    expect(fs.existsSync(ratatui), "missing scenario Ratatui text: regenerate with tests/provider_scenarios.rs").toBe(true);
    const view: Recording = JSON.parse(fs.readFileSync(checkpoint, "utf8"));
    expect([view.width, view.height]).toEqual([120, 40]);
    expect(view.screen?.scope).toBe("github");
    expect(view.screen?.page).toBeFalsy();
    expect(JSON.stringify(view.screen)).toContain("Patrol the Brass Bridge");
    const expected = rows(fs.readFileSync(ratatui, "utf8"));
    expect(expected.join("\n")).toContain("Patrol the Brass Bridge");

    // Production attaches the output compositor before rendering App; it paints notices after layout.
    // This replay attaches before delivering the captured snapshot, using the same input bus.
    // Hold the notice clock at the captured host time until the initial frame is compared.
    let now = 0;
    const renderer = mount(view.width, view.height, {}, {}, () => now);
    const detach = attachMotion(renderer.session, renderer.mouse, renderer.out);
    stop = () => { renderer.stop(); detach(); };
    let actual = "";
    try {
      renderer.welcome();
      renderer.snapshot({ chrome: view.chrome, screen: view.screen, section: view.section, focus: view.focus });
      await until(() => plain(renderer.frame()).includes("Patrol the Brass Bridge"));
      actual = renderer.frame();
      expect(rows(actual)).toEqual(expected);
      const notices = view.chrome?.notices;
      if (notices?.active.length) {
        // Inside the bottom-right panel a long notice may end in "…"; its start and its click target are there.
        for (const notice of notices.active) expect(plain(actual)).toContain(notice.text.slice(0, 16));
        expect(renderer.session.noticeHits.map((hit) => hit.target)).toEqual(notices.active.map((notice) => `chrome:notice:${notice.id}`));
        // Expiry must clear the presentation without leaving notice pixels or targets behind.
        now = Math.max(...notices.active.map((notice) => notice.created_ms + notice.lifetime_ms)) - notices.now_ms;
        renderer.snapshot({ chrome: view.chrome, screen: view.screen, section: view.section, focus: view.focus }, 2);
        await until(() => notices.active.every((notice) => !plain(renderer.frame()).includes(notice.text.slice(0, 16))));
        for (const notice of notices.active) expect(plain(renderer.frame())).not.toContain(notice.text.slice(0, 16));
        expect(renderer.session.noticeHits).toEqual([]);
        expect(plain(renderer.frame())).toContain("Patrol the Brass Bridge");
      }
    } finally {
      // Preserve what Ink really drew on success or mismatch. No synthetic frame
      // is substituted when mount/replay fails before producing output.
      captureActual(actual || renderer.frame());
    }
  });
});
