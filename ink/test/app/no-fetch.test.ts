import fs from "node:fs";
import { describe, expect, it } from "vitest";

describe("start-up does not load Node's fetch (T21)", () => {
  it("the shim removes the global fetch for Yoga's loader and gives it back", async () => {
    const before = globalThis.fetch;
    const { restoreFetch } = await import("../../src/transport/no-fetch.js");
    expect(typeof (globalThis as { fetch?: unknown }).fetch, "Yoga's loader takes the base64 path when there is no fetch").toBe("undefined");
    restoreFetch();
    expect(globalThis.fetch).toBe(before);
  });

  it("main.tsx imports the shim before the app (so before Yoga is loaded) and restores fetch before running", () => {
    const src = fs.readFileSync(new URL("../../src/main.tsx", import.meta.url), "utf8");
    const shim = src.indexOf("transport/no-fetch");
    expect(shim).toBeGreaterThan(-1);
    expect(shim).toBeLessThan(src.indexOf("app/run"));
    expect(src.indexOf("restoreFetch();")).toBeLessThan(src.indexOf("await run()"));
  });
});
