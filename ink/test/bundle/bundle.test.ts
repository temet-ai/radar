import { describe, expect, it } from "vitest";
import { spawn, spawnSync } from "node:child_process";
import net from "node:net";
import path from "node:path";
import fs from "node:fs";
import os from "node:os";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

// Run the package's own `bundle` script through a shell (as npm does, so its quoting counts), writing to `outfile`.
function buildBundle(outfile: string) {
  const script: string = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).scripts.bundle;
  expect(script.startsWith("esbuild ")).toBe(true);
  return spawnSync("sh", ["-c", `${script} --outfile=${outfile}`], {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, PATH: `${path.join(root, "node_modules/.bin")}:${process.env.PATH}` },
  });
}

describe("bundle", () => {
  it("ships React's production build (NODE_ENV defined as the string \"production\")", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "radar-ink-prod-"));
    try {
      const out = path.join(dir, "radar-ink.js");
      const build = buildBundle(out);
      expect(build.status, build.stderr).toBe(0);
      const code = fs.readFileSync(out, "utf8");
      // React's development build carries warning texts the production build drops.
      expect(code).not.toContain("Each child in a list should have a unique");
      expect(code).not.toContain("process.env.NODE_ENV");
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  }, 60000);

  it("builds one file that exits 2 without RADAR_IPC_SOCKET, running with no node_modules beside it", async () => {
    const build = buildBundle(path.join(root, "dist/radar-ink.js"));
    expect(build.status, build.stderr).toBe(0);
    // Run a copy of dist/ in a folder with no node_modules above it.
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "radar-ink-bundle-"));
    try {
      fs.cpSync(path.join(root, "dist"), dir, { recursive: true });
      // A plain environment: the vitest worker's own variables change how Ink behaves.
      const env: NodeJS.ProcessEnv = { PATH: process.env.PATH, HOME: process.env.HOME, TERM: "xterm-256color" };
      const run = spawnSync(process.execPath, [path.join(dir, "radar-ink.js")], { env, encoding: "utf8" });
      expect(run.status).toBe(2);
      expect(run.stderr).toBe("radar ink renderer: RADAR_IPC_SOCKET is not set\n");
      expect(fs.readdirSync(dir)).toEqual(["radar-ink.js"]);

      // With a socket it connects and draws (this exercises Ink's Yoga layout from the bundle).
      const sock = path.join(dir, "s.sock");
      const server = net.createServer();
      const connected = new Promise<void>((r) => server.once("connection", () => r()));
      await new Promise<void>((r) => server.listen(sock, r));
      const logPath = path.join(os.tmpdir(), `radar-ink-bundle-${process.pid}.log`);
      const fd = fs.openSync(logPath, "w");
      const child = spawn(process.execPath, [path.join(dir, "radar-ink.js")], {
        env: { ...env, RADAR_IPC_SOCKET: sock },
        stdio: ["ignore", fd, fd],
      });
      // Without a TTY Ink writes its frame when it unmounts, so stop it and read what it wrote.
      const exited = new Promise((r) => child.once("exit", r));
      // Wait on state, not the clock: the renderer has connected (a loaded machine starts node slowly), then give it a beat to draw.
      await connected;
      await new Promise((r) => setTimeout(r, 500));
      child.kill("SIGTERM");
      await exited;
      server.close();
      fs.closeSync(fd);
      const out = fs.readFileSync(logPath, "utf8");
      fs.rmSync(logPath, { force: true });
      expect(out).toContain("radar ink renderer: waiting for the host");
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  }, 60000);
});
