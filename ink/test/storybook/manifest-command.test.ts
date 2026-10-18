import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";
import { generateManifest, interfacePermissionDenied, runCommand } from "../../.storybook/manifest.mjs";

const roots: string[] = [];
afterEach(() => { vi.restoreAllMocks(); for (const root of roots.splice(0)) fs.rmSync(root, { recursive: true, force: true }); });
function fixture() {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), "radar-manifest-")); roots.push(repo);
  const ink = path.join(repo, "ink"), from = path.join(ink, "storybook-static/manifests/components.json"), to = path.join(repo, "docs/storybook/components.json");
  for (const file of [from, to]) { fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, "previous manifest\n"); }
  const logs: string[] = [], errors: string[] = [];
  return { repo, ink, from, to, logs, errors, log: (s: string) => logs.push(s), error: (s: string) => errors.push(s) };
}
function syntheticBuild(ink: string, source: string) {
  let stdout = "", stderr = "";
  return {
    run: () => runCommand(process.execPath, ["-e", source], { cwd: ink,
      stdout: { write: (s: string) => { stdout += s; } }, stderr: { write: (s: string) => { stderr += s; } } }),
    stdout: () => stdout, stderr: () => stderr,
  };
}
function generated(f: ReturnType<typeof fixture>, content = JSON.stringify({ components: { watch: { importPath: `${f.repo}/ink/src/watch.tsx`, stories: [{ id: "watch--default" }] } } })) {
  return async () => { fs.writeFileSync(f.from, content); return { code: 0, output: "" }; };
}
const previous = (f: ReturnType<typeof fixture>) => expect(fs.readFileSync(f.to, "utf8")).toBe("previous manifest\n");

describe("manifest command (synthetic builds, no Storybook or network interfaces)", () => {
  it.each(["uv_interface_addresses EPERM", "uv_interface_addresses: Unknown system error 1", "uv_interface_addresses { errno: 1, code: 'ERR_SYSTEM_ERROR' }"])(
    "recognizes permission-specific diagnostic: %s", (text) => expect(interfacePermissionDenied(text)).toBe(true));
  it.each(["EPERM opening components.json", "uv_interface_addresses ENOMEM", "uv_interface_addresses Unknown system error 12", "uv_interface_addresses errno: 12"])(
    "does not misclassify unrelated errors: %s", (text) => expect(interfacePermissionDenied(text)).toBe(false));

  it("streams both logs, keeps the previous manifest, and explains the known blocked build", async () => {
    const f = fixture(), build = syntheticBuild(f.ink, "console.log('building stories'); console.error('uv_interface_addresses EPERM'); process.exit(7)");
    expect(await generateManifest({ ...f, runBuild: build.run })).toBe(7);
    expect(build.stdout()).toContain("building stories"); expect(build.stderr()).toContain("uv_interface_addresses EPERM");
    expect(f.errors.join("\n")).toContain("Manifest generation blocked");
    expect(f.errors.join("\n")).toContain("cd ink && npm run manifest");
    expect(f.errors.join("\n")).toContain("may be stale");
    expect(f.logs).toEqual([]); previous(f); expect(fs.existsSync(f.from)).toBe(false);
  });
  it("propagates unknown failures and their original diagnostics", async () => {
    const f = fixture(), build = syntheticBuild(f.ink, "console.error('WATCH-12: invalid story'); process.exit(23)");
    expect(await generateManifest({ ...f, runBuild: build.run })).toBe(23);
    expect(build.stderr()).toContain("WATCH-12: invalid story");
    expect(f.errors.join("\n")).toContain("original build error");
    expect(f.errors.join("\n")).not.toContain("generation blocked"); previous(f);
  });
  it("cannot reuse an old generated file when a successful child produces none", async () => {
    const f = fixture();
    expect(await generateManifest({ ...f, runBuild: syntheticBuild(f.ink, "process.exit(0)").run })).toBe(1);
    expect(f.logs).toEqual([]); expect(f.errors.join("\n")).toContain("ENOENT"); previous(f);
  });
  it.each(["invalid json", "{}", '{"components":[]}', '{"components":{"watch":null}}', '{"components":{"watch":{"stories":1}}}'])(
    "preserves the committed file for invalid generated content: %s", async (content) => {
      const f = fixture(); expect(await generateManifest({ ...f, runBuild: generated(f, content) })).toBe(1);
      previous(f); expect(f.logs).toEqual([]); expect(fs.readdirSync(path.dirname(f.to))).toEqual(["components.json"]);
    });
  it("copies only a fresh valid manifest, normalizes paths, and cleans staging files", async () => {
    const f = fixture();
    const body = JSON.stringify({ components: { watch: { importPath: `${f.repo}/ink/src/watch.tsx`, stories: [{ id: "watch--default" }] } } });
    const build = syntheticBuild(f.ink, `require('node:fs').writeFileSync(${JSON.stringify(f.from)}, ${JSON.stringify(body)})`);
    expect(await generateManifest({ ...f, runBuild: build.run })).toBe(0);
    expect(JSON.parse(fs.readFileSync(f.to, "utf8")).components.watch.importPath).toBe("ink/src/watch.tsx");
    expect(f.logs).toEqual(["wrote docs/storybook/components.json: 1 components, 1 stories"]);
    expect(f.errors).toEqual([]); expect(fs.readdirSync(path.dirname(f.to))).toEqual(["components.json"]);
  });
  it("preserves the previous file on atomic replacement failure", async () => {
    const f = fixture(); vi.spyOn(fs, "renameSync").mockImplementation(() => { throw new Error("rename denied"); });
    expect(await generateManifest({ ...f, runBuild: generated(f) })).toBe(1);
    previous(f); expect(f.errors.join("\n")).toContain("rename denied");
    expect(fs.readdirSync(path.dirname(f.to))).toEqual(["components.json"]);
  });
  it("does not report a committed replacement as unchanged when cleanup fails", async () => {
    const f = fixture(), original = fs.rmSync;
    vi.spyOn(fs, "rmSync").mockImplementation((file, options) => {
      if (String(file).includes(".manifest-")) throw new Error("cleanup denied");
      original(file, options);
    });
    expect(await generateManifest({ ...f, runBuild: generated(f) })).toBe(0);
    expect(f.logs[0]).toContain("wrote docs/storybook/components.json");
    expect(f.errors.join("\n")).toContain("cleanup denied");
    expect(f.errors.join("\n")).not.toContain("not updated");
  });
  it("reports accurately if output reporting fails after replacement", async () => {
    const f = fixture();
    expect(await generateManifest({ ...f, runBuild: generated(f), log: () => { throw new Error("output closed"); } })).toBe(1);
    expect(f.errors.join("\n")).toContain("manifest was updated, but reporting failed");
  });
  it("treats signal termination as failure rather than a permission diagnosis", async () => {
    const f = fixture();
    expect(await generateManifest({ ...f, runBuild: async () => ({ code: null, signal: "SIGTERM", output: "uv_interface_addresses EPERM" }) })).toBe(1);
    expect(f.errors.join("\n")).toContain("SIGTERM"); expect(f.errors.join("\n")).not.toContain("generation blocked"); previous(f);
  });
  it("retains launch errors and returns failure", async () => {
    const f = fixture(); let diagnostics = "";
    const result = await runCommand(path.join(f.repo, "missing-command"), [], { stderr: { write: (s: string) => { diagnostics += s; } } });
    expect(result.error.code).toBe("ENOENT"); expect(diagnostics).toContain("ENOENT");
    expect(await generateManifest({ ...f, runBuild: async () => result })).not.toBe(0); previous(f);
  });
  it("runs the CLI through the npm build command and returns its failure status", () => {
    const f = fixture();
    // Disposable checkout: no production Storybook build or interface operation is exercised.
    const source = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../.storybook");
    const scripts = path.join(f.ink, ".storybook"); fs.mkdirSync(scripts);
    for (const name of ["manifest.mjs", "copy-manifest.mjs"]) fs.copyFileSync(path.join(source, name), path.join(scripts, name));
    const npm = path.join(f.repo, "fake-npm.cjs");
    fs.writeFileSync(npm, `if (JSON.stringify(process.argv.slice(2)) !== '["run","build-storybook"]') process.exit(99); console.error('uv_interface_addresses EPERM'); process.exit(9);`);
    const result = spawnSync(process.execPath, [path.join(scripts, "manifest.mjs")], {
      cwd: f.ink, env: { ...process.env, npm_execpath: npm }, encoding: "utf8", timeout: 5000,
    });
    expect(result.status).toBe(9); expect(result.stderr).toContain("uv_interface_addresses EPERM");
    expect(result.stderr).toContain("Manifest generation blocked"); previous(f);
    expect(result.stdout).not.toContain("wrote docs/storybook/components.json");
    const pkg = JSON.parse(fs.readFileSync(path.join(source, "../package.json"), "utf8"));
    expect(pkg.scripts.manifest).toBe("node .storybook/manifest.mjs");
  });
  it("preserves the original replacement error even when cleanup also fails", async () => {
    const f = fixture(), original = fs.rmSync;
    vi.spyOn(fs, "renameSync").mockImplementation(() => { throw new Error("rename denied"); });
    vi.spyOn(fs, "rmSync").mockImplementation((file, options) => {
      if (String(file).includes(".manifest-")) throw new Error("cleanup denied");
      original(file, options);
    });
    expect(await generateManifest({ ...f, runBuild: generated(f) })).toBe(1);
    previous(f); expect(f.errors.join("\n")).toContain("rename denied");
  });
  it("streams long logs with bounded diagnostic memory", async () => {
    const f = fixture(), build = syntheticBuild(f.ink, "process.stdout.write('x'.repeat(100000))");
    const result = await build.run(); expect(result.code).toBe(0);
    expect(build.stdout()).toHaveLength(100000); expect(result.output.length).toBeLessThanOrEqual(64 * 1024);
  });
});
