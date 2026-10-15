// Anthropic's Claude Agent SDK, fetched on first use. radar ships none of Anthropic's code:
// the first time the Claude agent needs the SDK, the pinned version (sdk-pin.json, the same as
// package-lock.json) is downloaded from the npm registry, checked against the lockfile's sha512
// and unpacked into radar's cache folder. Later starts import it from there. From a source
// checkout the SDK in node_modules is used and nothing is downloaded.
//
// The SDK's sdk.mjs needs only Node's own modules, so its package is all that is fetched.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { dirname, join, normalize, sep } from "node:path";
import { pathToFileURL } from "node:url";
import { gunzipSync } from "node:zlib";
import pin from "./sdk-pin.json" with { type: "json" };

export { pin };

/// Where the pinned SDK is kept: `RADAR_CLAUDE_SDK_DIR`, else `<cache>/radar/claude-agent-sdk/<version>`.
export function sdkDir(env = process.env, platform = process.platform, home = homedir()) {
  if (env.RADAR_CLAUDE_SDK_DIR) return env.RADAR_CLAUDE_SDK_DIR;
  const xdg = env.XDG_CACHE_HOME && env.XDG_CACHE_HOME.startsWith("/") ? env.XDG_CACHE_HOME : null;
  const cache = xdg ?? (platform === "darwin" ? join(home, "Library", "Caches") : join(home, ".cache"));
  return join(cache, "radar", "claude-agent-sdk", pin.version);
}

/// The SDK module: node_modules' when there is one, else the cached copy, else downloaded once.
/// `importBare` and `fetchFn` are for tests.
export async function loadSdk({
  env = process.env,
  dir = sdkDir(env),
  fetchFn = globalThis.fetch,
  importBare = () => import("@anthropic-ai/claude-agent-sdk"),
} = {}) {
  try {
    return await importBare();
  } catch {
    // Not installed beside this program (the runtime radar carries): the cache, then npm.
  }
  const main = join(dir, "sdk.mjs");
  if (!existsSync(main)) await download({ env, dir, fetchFn });
  try {
    return await import(pathToFileURL(main).href);
  } catch (e) {
    // In a Node single executable (the runtime radar carries) import() reaches only Node's own
    // modules; require() loads a file, an ES module included.
    if (e?.code !== "ERR_UNKNOWN_BUILTIN_MODULE") throw e;
    return createRequire(main)(main);
  }
}

async function download({ env, dir, fetchFn }) {
  const what = `${pin.name} ${pin.version}`;
  if (env.RADAR_SANDBOX) throw new Error(`Claude Agent: ${what} is not downloaded in a sandbox (RADAR_SANDBOX)`);
  let bytes;
  try {
    const res = await fetchFn(pin.resolved, { signal: AbortSignal.timeout(60_000) });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    bytes = Buffer.from(await res.arrayBuffer());
  } catch (e) {
    throw new Error(
      `Claude Agent needs Anthropic's ${what} and could not download it from the npm registry (${e.message}). ` +
        "It is fetched once, on first use; check the network and try again.",
    );
  }
  const got = `sha512-${createHash("sha512").update(bytes).digest("base64")}`;
  if (got !== pin.integrity) {
    throw new Error(`Claude Agent: the downloaded ${what} does not match its pinned checksum; nothing was installed`);
  }
  const tmp = `${dir}.part-${process.pid}`;
  rmSync(tmp, { recursive: true, force: true });
  try {
    for (const [path, data] of untar(gunzipSync(bytes))) {
      const out = join(tmp, path);
      mkdirSync(dirname(out), { recursive: true });
      writeFileSync(out, data);
    }
    if (!existsSync(join(tmp, "sdk.mjs"))) throw new Error(`Claude Agent: ${what} has no sdk.mjs`);
    mkdirSync(dirname(dir), { recursive: true });
    try {
      renameSync(tmp, dir);
    } catch (e) {
      // Another radar unpacked it first.
      if (!existsSync(join(dir, "sdk.mjs"))) throw e;
    }
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

/// The regular files of an npm tarball, as [path below `package/`, bytes]. Reads ustar names
/// with their prefix and pax `path` records; refuses a path that leaves the package.
export function untar(tar) {
  const files = [];
  let paxPath = null;
  for (let at = 0; at + 512 <= tar.length; ) {
    const head = tar.subarray(at, at + 512);
    if (head.every((b) => b === 0)) break;
    const field = (from, len) => head.subarray(from, from + len).toString("utf8").replace(/\0.*$/s, "");
    const size = parseInt(field(124, 12).trim() || "0", 8);
    const type = String.fromCharCode(head[156] || 48);
    const prefix = field(345, 155);
    const body = tar.subarray(at + 512, at + 512 + size);
    at += 512 + Math.ceil(size / 512) * 512;
    if (type === "x") {
      const m = /(?:^|\n)\d+ path=([^\n]*)\n/.exec(body.toString("utf8"));
      paxPath = m ? m[1] : null;
      continue;
    }
    const name = paxPath ?? (prefix ? `${prefix}/${field(0, 100)}` : field(0, 100));
    paxPath = null;
    if (type !== "0" && type !== "\0") continue;
    const clean = normalize(name);
    if (clean.startsWith("..") || clean.startsWith(sep)) throw new Error(`unsafe path in the SDK package: ${name}`);
    const rel = clean.split(sep).slice(1).join(sep);
    if (rel) files.push([rel, Buffer.from(body)]);
  }
  return files;
}
