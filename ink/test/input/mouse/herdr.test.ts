import { describe, expect, it } from "vitest";
import { MOUSE_ON, MOUSE_OFF, herdrRouteCommand, inHerdrPane, routeHerdrRightClick } from "../../../src/input/mouse.js";
import { RESTORE, Terminal } from "../../../src/app/terminal.js";

const herdr = { HERDR_ENV: "1", HERDR_PANE_ID: "w1:p1" } as NodeJS.ProcessEnv;

function fake() {
  const calls: { file: string; args: string[] }[] = [];
  const handlers: Record<string, (...a: any[]) => void> = {};
  const spawn = (file: string, args: string[]) => {
    calls.push({ file, args });
    return { on: (ev: string, fn: (...a: any[]) => void) => void (handlers[ev] = fn) };
  };
  return { calls, handlers, spawn };
}

describe("Herdr right-click routing (terminal_host::route_herdr_right_click)", () => {
  it("only a Herdr pane is routed", () => {
    expect(inHerdrPane(herdr)).toBe(true);
    expect(inHerdrPane({ HERDR_ENV: "1" })).toBe(false);
    expect(inHerdrPane({ HERDR_PANE_ID: "w1:p1" })).toBe(false);
    const f = fake();
    expect(routeHerdrRightClick({}, f.spawn)).toBe(false);
    expect(f.calls).toEqual([]);
  });

  it("runs `herdr pane input --current --right-click pane`, with HERDR_BIN_PATH when set", () => {
    const f = fake();
    expect(routeHerdrRightClick(herdr, f.spawn)).toBe(true);
    expect(f.calls).toEqual([{ file: "herdr", args: ["pane", "input", "--current", "--right-click", "pane"] }]);
    expect(herdrRouteCommand({ ...herdr, HERDR_BIN_PATH: "/opt/herdr" }).file).toBe("/opt/herdr");
  });

  it("reports success, failure and a missing binary", () => {
    const seen: (string | null)[] = [];
    const f = fake();
    routeHerdrRightClick(herdr, f.spawn, (e) => seen.push(e));
    f.handlers.exit!(0);
    f.handlers.exit!(1);
    f.handlers.error!(new Error("ENOENT"));
    expect(seen).toEqual([null, "Herdr could not route right-clicks to this pane", "Herdr pane routing unavailable: ENOENT"]);
  });
});

describe("mouse modes match crossterm's EnableMouseCapture and are all turned off on exit", () => {
  it("enables 1000, 1002, 1003, 1015, 1006 and disables each", () => {
    for (const n of ["1000", "1002", "1003", "1015", "1006"]) {
      expect(MOUSE_ON).toContain(`\x1b[?${n}h`);
      expect(MOUSE_OFF).toContain(`\x1b[?${n}l`);
      expect(RESTORE).toContain(`\x1b[?${n}l`);
    }
  });

  it("the terminal writes them on enter and the restore once", () => {
    const out: string[] = [];
    const t = new Terminal({ write: (s) => out.push(s) });
    t.enter();
    expect(out.join("")).toContain(MOUSE_ON);
    t.restore();
    t.restore();
    expect(out.filter((s) => s === RESTORE)).toHaveLength(1);
  });
});
