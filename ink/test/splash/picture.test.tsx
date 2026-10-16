import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import { afterEach, describe, expect, it } from "vitest";
import { blankUnder, kittySplash, kittySplashAt, kittySplashHide } from "../../src/media/splashPicture.js";
import { mount, tick } from "../app/harness.js";
import { parseAnsi } from "../nodes/ansi.js";
import type { SplashPicture } from "../../src/protocol/generated/SplashPicture.js";

// crates/radar-core/src/splash.rs: kitty_show, kitty_at, kitty_hide and clear_under, the strings the Ratatui runner writes.
const pic = (png: string): SplashPicture => ({ x: 3, y: 1, cols: 4, rows: 2, id: 7370, png });
const ESC = "\x1b";

describe("the splash's picture, as escape strings", () => {
  it("is one chunk with the id, placement, size and no cursor move", () => {
    expect(kittySplash(pic("QUJD"))).toBe(`${ESC}_Ga=T,f=100,i=7370,p=1,C=1,c=4,r=2,q=2,m=0;QUJD${ESC}\\`);
  });
  it("splits the base64 at 4096 with the head on the first chunk only and m=1 until the last", () => {
    const png = "A".repeat(4096) + "B".repeat(10);
    expect(kittySplash(pic(png))).toBe(
      `${ESC}_Ga=T,f=100,i=7370,p=1,C=1,c=4,r=2,q=2,m=1;${"A".repeat(4096)}${ESC}\\${ESC}_Gq=2,m=0;${"B".repeat(10)}${ESC}\\`,
    );
  });
  it("at its cell saves the cursor, goes to row y+1 column x+1 and restores it", () => {
    expect(kittySplashAt(pic("QUJD"))).toBe(`${ESC}7${ESC}[2;4H${kittySplash(pic("QUJD"))}${ESC}8`);
  });
  it("hides by id, once", () => {
    expect(kittySplashHide(7370)).toBe(`${ESC}_Ga=d,d=I,i=7370,q=2${ESC}\\`);
  });
  it("blanks the cells under it with spaces and keeps their colours", () => {
    const run = (text: string) => ({ text, fg: "#112233", bg: "#000000", bold: false });
    const out = blankUnder([[run("⣿⣿⣿⣿⣿⣿⣿⣿")], [run("⣿⣿⣿⣿⣿⣿⣿⣿")], [run("⣿⣿⣿⣿⣿⣿⣿⣿")], [run("⣿⣿⣿⣿⣿⣿⣿⣿")]], pic("x"));
    expect(out.map((r) => r.map((x) => x.text).join(""))).toEqual(["⣿⣿⣿⣿⣿⣿⣿⣿", "⣿⣿⣿    ⣿", "⣿⣿⣿    ⣿", "⣿⣿⣿⣿⣿⣿⣿⣿"]);
    expect(out[1]!.every((r) => r.fg === "#112233" && r.bg === "#000000")).toBe(true);
  });
});

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

const view = (picture?: SplashPicture) => ({
  width: 20,
  height: 6,
  rows: Array.from({ length: 6 }, () => [{ text: "⣿".repeat(20), fg: "#ffffff", bg: "#000000", bold: false }]),
  ...(picture ? { picture } : {}),
});

describe("the app writes the picture after its frame and takes it away once", () => {
  it("writes the picture at its cell for each frame, blanks under it, then deletes it when the splash goes", async () => {
    const m = mount(20, 6);
    live.push(m.stop);
    m.welcome();
    const writes = { get all() { return m.out.frames.filter((f) => f.includes("\x1b_G")); } };
    const drawn = () => parseAnsi(m.out.frames.filter((f) => !f.includes("\x1b_G")).at(-1) ?? "");
    m.snapshot({ splash: view(pic("QUJD")) }, 1);
    await tick(60);
    const cells = drawn();
    expect(cells[1]![3]!.ch).toBe(" ");
    expect(cells[0]![3]!.ch).toBe("⣿");
    expect(writes.all).toEqual([kittySplashAt(pic("QUJD"))]);
    m.snapshot({ splash: view(pic("REVG")) }, 2);
    await tick(60);
    expect(writes.all.at(-1)).toBe(kittySplashAt(pic("REVG")));
    m.snapshot({ screen: null }, 3);
    await tick(60);
    expect(writes.all.filter((w) => w === kittySplashHide(7370))).toHaveLength(1);
    m.snapshot({ screen: null }, 4);
    await tick(60);
    expect(writes.all.filter((w) => w === kittySplashHide(7370))).toHaveLength(1);
  });
  it("writes nothing for a splash without a picture", async () => {
    const m = mount(20, 6);
    live.push(m.stop);
    m.welcome();
    const writes = { get all() { return m.out.frames.filter((f) => f.includes("\x1b_G")); } };
    const drawn = () => parseAnsi(m.out.frames.filter((f) => !f.includes("\x1b_G")).at(-1) ?? "");
    m.snapshot({ splash: view() }, 1);
    await tick(60);
    m.snapshot({ screen: null }, 2);
    await tick(60);
    expect(writes.all).toEqual([]);
  });
});
