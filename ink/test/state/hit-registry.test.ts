import { describe, expect, it } from "vitest";
import { HitRegistry } from "../../src/state/ui-state.js";

// T22: a render can yield between its render phase (where App clears the registry) and its commit (where the nodes
// register), and keys and clicks arriving in that gap (a burst in one stdin read) must still find the last frame's hits.
const hit = (target: string) => ({ x: 0, y: 0, w: 1, h: 1, target });
describe("the hit registry across a frame", () => {
  it("answers with the last frame's hits until the new frame registers its first", () => {
    const r = new HitRegistry();
    r.add(hit("row:a"));
    r.clear();
    expect(r.all().map((h) => h.target)).toEqual(["row:a"]);
    r.add(hit("row:b"));
    r.add(hit("row:c"));
    expect(r.all().map((h) => h.target)).toEqual(["row:b", "row:c"]);
  });
  it("is empty once the frame settles having registered none", () => {
    const r = new HitRegistry();
    r.add(hit("row:a"));
    r.clear();
    r.settle();
    expect(r.all()).toEqual([]);
  });
  it("settle after a registering frame keeps its hits", () => {
    const r = new HitRegistry();
    r.clear();
    r.add(hit("row:a"));
    r.settle();
    expect(r.all().map((h) => h.target)).toEqual(["row:a"]);
  });
});
