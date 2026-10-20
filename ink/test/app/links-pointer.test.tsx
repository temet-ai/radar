import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import Output from "ink-output";
import { afterEach, describe, expect, it } from "vitest";
import { paintHover } from "../../src/app/hover.js";
import { PointerShape, shapeSequence } from "../../src/app/pointer.js";
import { restyle, useFastOutput } from "../../src/app/fast-output.js";
import { escaped, hyperlink, openable } from "../../src/text/link.js";
import { applyLook, defaultLook } from "../../src/text/look.js";

// ui::link and ui::pointer for Ink: OSC 8 links ride in the cells' style through the frame writer, and the pointer's
// shape (OSC 22) is written when what is under it changes.
const URL = "https://example.org/watch/12";
const open = (u: string) => `\x1b]8;;${u}\x1b\\`;
const CLOSE = open("");

afterEach(() => applyLook(defaultLook()));

describe("links (OSC 8)", () => {
  it("wraps words only when the host turned links on and the address may be opened", () => {
    expect(hyperlink(URL, "WATCH-12", false)).toBe("WATCH-12");
    expect(hyperlink(URL, "WATCH-12", true)).toBe(`${open(URL)}WATCH-12${CLOSE}`);
    expect(hyperlink("javascript:alert(1)", "x", true)).toBe("x");
    expect(hyperlink("https://a.b/\x07", "x", true)).toBe("x");
    expect(openable("mailto:vimes@watch.example")).toBe(true);
    applyLook({ ...defaultLook(), hyperlinks: true });
    expect(hyperlink(URL, "WATCH-12")).toContain(open(URL));
  });

  it("encodes what Ink's width code would not read as part of the sequence", () => {
    expect(escaped("https://a.b/x y+(1)")).toBe("https://a.b/x%20y%2B%281%29");
    expect(escaped("https://a.b/é")).toBe("https://a.b/%C3%A9");
  });

  it("are written around their cells in the frame, closed where they end, and take no width", () => {
    useFastOutput(true);
    const o = new (Output as any)({ width: 20, height: 2 });
    o.write(0, 0, `see ${hyperlink(URL, "WATCH-12", true)} now`, { transformers: [] });
    // A clip that cuts the link closes it where the clip ends.
    o.clip({ x1: 0, x2: 6 });
    o.write(0, 1, `ab${hyperlink(URL, "WATCH-12", true)}`, { transformers: [] });
    o.unclip();
    const [first, second] = o.get().output.split("\n");
    expect(first).toBe(`see ${open(URL)}WATCH-12${CLOSE} now`);
    expect(second).toBe(`ab${open(URL)}WATC${CLOSE}`);
  });

  it("survive the hover's paint over a finished frame", () => {
    const line = `ab${open(URL)}cd${CLOSE}ef`;
    const lit = paintHover(line, 10, { x: 0, y: 0, w: 6, h: 1 });
    expect(lit).toContain(open(URL));
    expect(lit.indexOf(open(URL))).toBeLessThan(lit.indexOf("cd"));
    expect(lit.indexOf(CLOSE)).toBeGreaterThan(lit.indexOf("cd"));
    // A row read back and written again keeps it.
    expect(restyle(line, 10, { x: 0, y: 0, w: 10, h: 1 }, (id) => id)).toBe(line);
  });
});

describe("the pointer's shape (OSC 22)", () => {
  it("is written only when it changes, follows a drag, and goes back to the arrow", () => {
    const out: string[] = [];
    const p = new PointerShape((s) => out.push(s), () => true);
    p.set("pointer");
    p.set("pointer");
    p.drag(true);
    p.set("default");
    p.drag(false);
    p.restore();
    expect(out).toEqual([shapeSequence("pointer"), shapeSequence("text"), shapeSequence("default")]);
    p.restore();
    expect(out).toHaveLength(3);
  });

  it("writes nothing where the host did not turn it on", () => {
    const out: string[] = [];
    const p = new PointerShape((s) => out.push(s), () => false);
    p.set("pointer");
    p.restore();
    expect(out).toEqual([]);
  });
});
