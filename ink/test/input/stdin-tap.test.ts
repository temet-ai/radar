import { PassThrough } from "node:stream";
import { describe, expect, it } from "vitest";
import { MouseBus, tapMouse } from "../../src/input/stdin-tap.js";
import type { InputEvent } from "../../src/input/tokens.js";

/** A stream with a `readable` listener that reads as Ink's does: `while ((chunk = stdin.read()) !== null)`. */
function inkLike() {
  const stdin = new PassThrough();
  stdin.setEncoding("utf8");
  const bus = new MouseBus();
  const events: InputEvent[] = [];
  bus.onInput((e) => events.push(...e));
  tapMouse(stdin as any, bus);
  stdin.on("readable", () => {
    let chunk;
    while ((chunk = stdin.read()) !== null) void chunk;
  });
  return { stdin, events };
}
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe("the tap on a stream read the way Ink reads it", () => {
  it("every read after a lone Esc still arrives (the stream is read to its end each time)", async () => {
    const { stdin, events } = inkLike();
    stdin.write("\x1b");
    await wait(20);
    stdin.write("j");
    await wait(20);
    stdin.write("\x1b[B");
    await wait(20);
    stdin.write("\x1b");
    await wait(20);
    stdin.write("x");
    await wait(20);
    expect(events.map((e) => (e.kind === "key" ? JSON.stringify(e.chord) : e.kind))).toEqual([
      '{"key":"esc"}',
      '{"key":{"char":"j"}}',
      '{"key":"down"}',
      '{"key":"esc"}',
      '{"key":{"char":"x"}}',
    ]);
  });

  it("alt-w arrives as one chord in each form a terminal writes it; a split ESC then w is Esc and w (a lone Esc is never held)", async () => {
    for (const bytes of [["\x1bw"], ["\x1b[119;3u"], ["\x1b[27;3;119~"]]) {
      const { stdin, events } = inkLike();
      for (const b of bytes) stdin.write(b);
      await wait(20);
      expect(events.map((e) => (e.kind === "key" ? JSON.stringify(e.chord) : e.kind)), JSON.stringify(bytes)).toEqual(['{"key":{"char":"w"},"alt":true}']);
    }
    const { stdin, events } = inkLike();
    stdin.write("\x1b");
    await wait(10);
    stdin.write("w");
    await wait(20);
    expect(events.map((e) => (e.kind === "key" ? JSON.stringify(e.chord) : e.kind))).toEqual(['{"key":"esc"}', '{"key":{"char":"w"}}']);
  });

  it("keys written together arrive together, in order, once", async () => {
    const { stdin, events } = inkLike();
    stdin.write("jk\x1b[A\r");
    await wait(30);
    expect(events).toHaveLength(4);
  });

  it("a sequence cut across two reads is joined; a cut one that is never finished is Esc and its characters", async () => {
    const { stdin, events } = inkLike();
    stdin.write("\x1b[");
    await wait(10);
    stdin.write("A");
    await wait(20);
    expect(events.map((e) => (e.kind === "key" ? e.chord.key : e.kind))).toEqual(["up"]);
    stdin.write("\x1b[");
    await wait(80);
    expect(events.slice(1).map((e) => (e.kind === "key" ? JSON.stringify(e.chord.key) : e.kind))).toEqual(['"esc"', '{"char":"["}']);
  });
});
