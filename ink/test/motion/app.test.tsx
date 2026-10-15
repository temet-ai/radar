// Motion through the whole App: the host's snapshot carries a cue, the screen is drawn through Ink, and the frames
// Ink writes while the effect runs are faded, then the frame after is exactly the one without motion. A key drops
// the effect at once; level off draws nothing extra.
import { afterEach, describe, expect, it } from "vitest";
import React from "react";
import { render } from "ink";
import { EventEmitter } from "node:events";
import { Out, tick, until } from "../app/harness.js";
import { App } from "../../src/app/App.js";
import { Session, capabilitiesFrom } from "../../src/app/session.js";
import { FakeConnection } from "../../src/transport/fake.js";
import { MouseBus, tapMouse } from "../../src/input/stdin-tap.js";
import { booksChrome } from "../chrome/fixture.js";
import fs from "node:fs";
import { motion, installMotionOutput } from "../../src/motion/motion.js";
import { attachMotion } from "../../src/motion/attach.js";
import { SCREEN_MS } from "../../src/motion/engine.js";

const SLOTS = {
  base: "bg.base",
  surface: "bg.surface",
  overlay: "bg.overlay",
  glow: "accent.primary",
  text: "text.primary",
};
const motionView = (level: string, cues: unknown[] = []) => ({
  level,
  cues,
  timings: {},
  slots: SLOTS,
});
/** The words of a frame: no colours, no trailing blanks (the faded frame writes blank cells the plain one omits). */
const strip = (s: string) =>
  s
    .replace(/\x1b\[[0-9;]*m/g, "")
    .split("\n")
    .map((l) => l.trimEnd())
    .join("\n")
    .trimEnd();

/** A stdin the test types into (the harness's own is not exported). */
class In extends EventEmitter {
  isTTY = true;
  data: string | null = null;
  setEncoding() {}
  setRawMode() {}
  resume() {}
  pause() {}
  ref() {}
  unref() {}
  read = () => {
    const d = this.data;
    this.data = null;
    return d;
  };
  type(s: string) {
    this.data = s;
    this.emit("readable");
  }
}

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

/** The harness's mount, keeping the MouseBus so motion hears the same keys the app does. */
function open(level: string) {
  const out = new Out(80, 24);
  const stdin = new In();
  const conn = new FakeConnection();
  const mouse = new MouseBus();
  tapMouse(stdin, mouse);
  const session = new Session(conn, capabilitiesFrom({}, out));
  session.start();
  const instance = render(
    <App session={session} mouse={mouse} onExit={() => {}} />,
    {
      stdout: out as any,
      stdin: stdin as any,
      stderr: new Out(80, 24) as any,
      debug: true,
      exitOnCtrlC: false,
      patchConsole: false,
    },
  );
  // The nodes mark rows on the renderer's own motion, so the test uses it too.
  motion.reset();
  installMotionOutput(motion);
  live.push(attachMotion(session, mouse, out, motion));
  live.push(() => instance.unmount());
  conn.receive({ type: "welcome", protocol: 1, view: 1, cursor: 0 });
  const snapshot = (
    cues: unknown[],
    cursor: number,
    lvl = level,
    screen: unknown = null,
  ) =>
    conn.receive({
      type: "snapshot",
      cursor,
      chrome: booksChrome({ width: 80, height: 24 }),
      screen,
      section: null,
      focus: "middle",
      motion: motionView(lvl, cues),
    });
  snapshot([], 1);
  return { out, stdin, motion, snapshot, frame: () => out.last() };
}

describe("motion through the app", () => {
  it("a screen cue fades the body over ~200 ms, then draws exactly as without motion", async () => {
    const m = open("subtle");
    await tick(60);
    const plain = m.frame();
    expect(m.motion.active).toBe(false);
    m.snapshot([{ type: "screen", region: "body" }], 2, "subtle");
    await until(() => m.frame() !== plain, 500);
    const during = m.frame();
    expect(during).not.toBe(plain);
    expect(strip(during)).toBe(strip(plain));
    expect(m.motion.active).toBe(true);
    await until(() => !m.motion.active, 1500);
    await tick(80);
    expect(m.frame()).toBe(plain);
    // frames were drawn on a tick while it ran
    expect(m.out.frames.length).toBeGreaterThan(3);
  });

  it("a key drops the effect at once", async () => {
    const m = open("full");
    await tick(60);
    const plain = m.frame();
    m.snapshot([{ type: "screen", region: "body" }], 2, "full");
    await until(() => m.motion.active, 500);
    m.stdin.type("j");
    await until(() => !m.motion.active, 500);
    await tick(80);
    expect(m.frame()).toBe(plain);
  });

  it("level off draws nothing extra and ticks nothing", async () => {
    const m = open("off");
    await tick(60);
    const before = m.out.frames.length;
    m.snapshot([], 2);
    await tick(SCREEN_MS + 100);
    expect(m.motion.active).toBe(false);
    expect(m.out.frames.length - before).toBeLessThanOrEqual(2);
  });

  it("a row a sync changed glows where it is drawn, and only that row", async () => {
    const screen = JSON.parse(
      fs.readFileSync(
        new URL("../fixtures/gitlab_repos_80x24.json", import.meta.url),
        "utf8",
      ),
    );
    // Every group open, so the changed row is on screen.
    const tree = screen.panes[1].body;
    const paths: string[][] = [];
    const walk = (rows: any[], prefix: string[]) =>
      rows.forEach((r) => {
        const p = [...prefix, r.id];
        if (r.children?.length) {
          paths.push(p);
          walk(r.children, p);
        }
      });
    walk(tree.rows, []);
    tree.open = paths;
    const m = open("subtle");
    m.snapshot([], 2, "subtle", screen);
    await tick(80);
    const plain = m.frame().split("\n");
    m.snapshot(
      [
        {
          type: "changed",
          changed: ["merge-request:acme/team-a/api!263"],
          added: [],
        },
      ],
      3,
      "subtle",
      screen,
    );
    await until(() => m.motion.active, 500);
    await tick(40);
    const during = m.frame().split("\n");
    const moved = during
      .map((l, i) => (l === plain[i] ? -1 : i))
      .filter((i) => i >= 0);
    expect(moved.length, "exactly one row glows").toBe(1);
    expect(strip(during[moved[0]!]!)).toBe(strip(plain[moved[0]!]!));
    await until(() => !m.motion.active, 1500);
    await tick(80);
    expect(m.frame().split("\n")).toEqual(plain);
  });
});
