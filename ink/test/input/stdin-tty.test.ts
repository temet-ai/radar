import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { describe, expect, it } from "vitest";

/**
 * The tap on a real terminal. A fake or PassThrough stdin re-arms `readable` by itself; a tty.ReadStream does not
 * until a read() returns null, so a tap that takes one chunk and returns null leaves every later key unread (the
 * renderer sat there alive and deaf after the first key). This runs the tap in a child on a pseudo-terminal
 * (python's pty module, on Linux and macOS) and types five keys into it.
 */
const here = path.dirname(fileURLToPath(import.meta.url));

describe("the tap on a pseudo-terminal", () => {
  it("every key typed, one read each, reaches the bus (not only the first)", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "radar-tap-"));
    try {
      const out = path.join(dir, "tap.mjs");
      await build({ entryPoints: [path.join(here, "tty/tap-main.ts")], bundle: true, platform: "node", format: "esm", outfile: out, logLevel: "silent" });
      const text = execFileSync("python3", [path.join(here, "tty/drive.py"), process.execPath, out], { encoding: "utf8", timeout: 15000 });
      const seen = text
        .split(/\r?\n/)
        .filter((l) => l.startsWith("{"))
        .map((l) => JSON.parse(l).chord.key);
      expect(seen).toEqual(["esc", { char: "j" }, "down", "esc", { char: "x" }]);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  }, 20000);
});
