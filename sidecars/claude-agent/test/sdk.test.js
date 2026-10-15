import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { loadSdk, pin, sdkDir, untar } from "../src/sdk.js";
import { gunzipSync } from "node:zlib";

const here = dirname(fileURLToPath(import.meta.url));
const noBare = () => Promise.reject(new Error("not installed"));

// A stand-in for the SDK's npm tarball: package/sdk.mjs and package/package.json, packed by tar.
function tarball(root) {
  const pkg = join(root, "package");
  mkdirSync(join(pkg, "nested", "deeper"), { recursive: true });
  writeFileSync(join(pkg, "sdk.mjs"), "export const query = () => 'stand-in';\n");
  writeFileSync(join(pkg, "package.json"), JSON.stringify({ name: pin.name, version: pin.version }));
  writeFileSync(join(pkg, "nested", "deeper", "x.txt"), "x");
  const file = join(root, "sdk.tgz");
  // COPYFILE_DISABLE: macOS tar adds no ._ files.
  execFileSync("tar", ["-czf", file, "-C", root, "package"], { env: { ...process.env, COPYFILE_DISABLE: "1" } });
  return readFileSync(file);
}

const integrity = (bytes) => `sha512-${createHash("sha512").update(bytes).digest("base64")}`;

test("the pin is the lockfile's SDK and package.json's dependency", () => {
  const lock = JSON.parse(readFileSync(join(here, "..", "package-lock.json"), "utf8"));
  const entry = lock.packages[`node_modules/${pin.name}`];
  assert.equal(pin.version, entry.version);
  assert.equal(pin.resolved, entry.resolved);
  assert.equal(pin.integrity, entry.integrity);
  const pkg = JSON.parse(readFileSync(join(here, "..", "package.json"), "utf8"));
  assert.equal(pkg.dependencies[pin.name], pin.version);
});

test("the cache folder is radar's, per version, on every platform", () => {
  assert.equal(sdkDir({}, "darwin", "/Users/a"), `/Users/a/Library/Caches/radar/claude-agent-sdk/${pin.version}`);
  assert.equal(sdkDir({}, "linux", "/home/a"), `/home/a/.cache/radar/claude-agent-sdk/${pin.version}`);
  assert.equal(sdkDir({ XDG_CACHE_HOME: "/c" }, "linux", "/home/a"), `/c/radar/claude-agent-sdk/${pin.version}`);
  assert.equal(sdkDir({ RADAR_CLAUDE_SDK_DIR: "/x" }), "/x");
});

test("node_modules' SDK is used as it is and nothing is fetched", async () => {
  const mod = { query: () => "installed" };
  let fetched = 0;
  const got = await loadSdk({ env: {}, dir: "/nonexistent", importBare: async () => mod, fetchFn: async () => fetched++ });
  assert.equal(got, mod);
  assert.equal(fetched, 0);
});

test("without node_modules the SDK is downloaded once, checked, unpacked and imported", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "radar-sdk-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const bytes = tarball(root);
  const files = untar(gunzipSync(bytes)).map(([p]) => p).sort();
  assert.deepEqual(files, ["nested/deeper/x.txt", "package.json", "sdk.mjs"]);
  if (integrity(bytes) !== pin.integrity) {
    // The stand-in cannot carry the real checksum: a wrong one is refused and nothing is kept.
    const dir = join(root, "cache", "sdk");
    await assert.rejects(
      loadSdk({ env: {}, dir, importBare: noBare, fetchFn: async () => new Response(bytes) }),
      /does not match its pinned checksum/,
    );
    assert.equal(existsSync(dir), false);
    assert.deepEqual(readdirSync(join(root)).filter((f) => f.includes(".part-")), []);
  }
  // With the stand-in's own checksum pinned the same path installs it.
  pin.integrity = integrity(bytes);
  try {
    const dir = join(root, "cache2", "sdk");
    let fetched = 0;
    const fetchFn = async (url) => {
      fetched++;
      assert.equal(url, pin.resolved);
      return new Response(bytes);
    };
    const mod = await loadSdk({ env: {}, dir, importBare: noBare, fetchFn });
    assert.equal(mod.query(), "stand-in");
    assert.equal(readFileSync(join(dir, "nested", "deeper", "x.txt"), "utf8"), "x");
    await loadSdk({ env: {}, dir, importBare: noBare, fetchFn });
    assert.equal(fetched, 1, "the second start uses the cache");
  } finally {
    pin.integrity = JSON.parse(readFileSync(join(here, "..", "src", "sdk-pin.json"), "utf8")).integrity;
  }
});

test("offline, the error says what is missing and that it is fetched once; nothing is kept", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "radar-sdk-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const dir = join(root, "sdk");
  const fetchFn = async () => {
    throw new Error("getaddrinfo ENOTFOUND registry.npmjs.org");
  };
  await assert.rejects(loadSdk({ env: {}, dir, importBare: noBare, fetchFn }), (e) => {
    assert.match(e.message, /Claude Agent needs Anthropic's @anthropic-ai\/claude-agent-sdk/);
    assert.match(e.message, /ENOTFOUND/);
    assert.match(e.message, /fetched once, on first use/);
    return true;
  });
  assert.equal(existsSync(dir), false);
});

test("a sandbox never downloads", async () => {
  let fetched = 0;
  await assert.rejects(
    loadSdk({ env: { RADAR_SANDBOX: "1" }, dir: "/nonexistent/sdk", importBare: noBare, fetchFn: async () => fetched++ }),
    /not downloaded in a sandbox/,
  );
  assert.equal(fetched, 0);
});

test("a path that leaves the package is refused", () => {
  const head = Buffer.alloc(512);
  head.write("package/../../evil", 0);
  head.write("00000000001\0", 124);
  head.write("0", 156);
  const tar = Buffer.concat([head, Buffer.alloc(512, 0x41), Buffer.alloc(1024)]);
  assert.throws(() => untar(tar), /unsafe path/);
});
