#!/usr/bin/env node
// Run the supported build without loading Storybook in this process: even importing its server can fail.
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { copyManifest } from "./copy-manifest.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const tailLimit = 64 * 1024;

export function interfacePermissionDenied(output) {
  return /uv_interface_addresses/.test(output) &&
    /\bEPERM\b|operation not permitted|Unknown system error 1\b|errno:\s*1\b/i.test(output);
}

// Stream all original diagnostics, retaining only a bounded tail for classification.
export function runCommand(command, args, { cwd, stdout = process.stdout, stderr = process.stderr } = {}) {
  return new Promise((resolve) => {
    let output = "";
    let error;
    const child = spawn(command, args, { cwd, stdio: ["ignore", "pipe", "pipe"] });
    for (const [stream, sink] of [[child.stdout, stdout], [child.stderr, stderr]]) {
      stream.setEncoding("utf8");
      stream.on("data", (chunk) => {
        sink.write(chunk);
        output = (output + chunk).slice(-tailLimit);
      });
    }
    child.on("error", (cause) => {
      error = cause;
      stderr.write(`${cause.message}\n`);
    });
    child.on("close", (code, signal) => resolve({ code, signal, error, output }));
  });
}

export async function generateManifest({ ink = path.resolve(here, ".."), runBuild, log = console.log, error = console.error } = {}) {
  const repo = path.dirname(ink);
  const from = path.join(ink, "storybook-static/manifests/components.json");
  const to = path.join(repo, "docs/storybook/components.json");
  let updated = false;
  try {
    // A successful command must actually produce this manifest, never reuse an earlier build's copy.
    fs.rmSync(from, { force: true });
    const result = await (runBuild ?? (() => {
      const npm = process.env.npm_execpath;
      return runCommand(npm ? process.execPath : "npm", npm ? [npm, "run", "build-storybook"] : ["run", "build-storybook"], { cwd: ink });
    }))();
    if (result.code !== 0 || result.signal || result.error) {
      if (!result.signal && !result.error && interfacePermissionDenied(result.output)) {
        error("Manifest generation blocked: Storybook cannot enumerate network interfaces (uv_interface_addresses permission error).\n" +
          "In an authorized environment that permits network-interface enumeration, from the repository root run: cd ink && npm run manifest\n" +
          "The committed manifest was not updated; it may be stale. Do not disable sandbox protections.");
      } else {
        error(`Manifest generation failed${result.signal ? ` (${result.signal})` : ""}; see the original build error above. The committed manifest was not updated.`);
      }
      return Number.isInteger(result.code) && result.code > 0 ? result.code : 1;
    }
    const copied = copyManifest({ from, to, repo });
    updated = true;
    log(copied.message);
    if (copied.cleanupWarning) error(copied.cleanupWarning);
    return 0;
  } catch (cause) {
    error(`Manifest generation failed: ${cause.message}. ${updated ? "The committed manifest was updated, but reporting failed." : "The committed manifest was not updated."}`);
    return 1;
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = await generateManifest();
}
