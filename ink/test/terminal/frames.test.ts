import { describe, expect, it } from "vitest";
import { encodeFrame, FrameReader, IN_INPUT, inputFrame, MAX_PAYLOAD, OUT_OUTPUT, OUT_STATE, resizeFrame } from "../../src/terminal/frames.js";

const json = (o: unknown) => new TextEncoder().encode(JSON.stringify(o));

describe("terminal frames", () => {
  it("reads output and state frames, in order, however the bytes are cut", () => {
    const state = { state: "running", generation: 1, columns: 80, rows: 24 };
    const wire = new Uint8Array([...encodeFrame(OUT_STATE, json(state)), ...encodeFrame(OUT_OUTPUT, new TextEncoder().encode("hi"))]);
    const r = new FrameReader();
    const got = [];
    for (let i = 0; i < wire.length; i += 3) got.push(...r.push(wire.slice(i, i + 3)));
    expect(got).toEqual([
      { kind: "state", state },
      { kind: "output", bytes: new TextEncoder().encode("hi") },
    ]);
  });

  it("writes the layout the host reads (frame.rs): type, big-endian length, payload", () => {
    expect([...inputFrame("ls\r")]).toEqual([IN_INPUT, 0, 0, 0, 3, 0x6c, 0x73, 0x0d]);
    expect([...resizeFrame(90, 20)]).toEqual([2, 0, 0, 0, 4, 0, 90, 0, 20]);
  });

  it("refuses a frame longer than the host sends", () => {
    const head = new Uint8Array([OUT_OUTPUT, 0, 0, 0, 0]);
    new DataView(head.buffer).setUint32(1, MAX_PAYLOAD + 1, false);
    expect(() => new FrameReader().push(head)).toThrow(/too long/);
  });

  it("ignores a state it cannot read and a type it does not know", () => {
    const r = new FrameReader();
    expect(r.push(new Uint8Array([...encodeFrame(OUT_STATE, new TextEncoder().encode("{")), ...encodeFrame(9, new Uint8Array())]))).toEqual([]);
  });
});
