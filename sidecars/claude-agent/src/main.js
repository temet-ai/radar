#!/usr/bin/env node
import { createInterface } from "node:readline";
import { createServer } from "./server.js";
import { loadSdk, pin } from "./sdk.js";
// Imported, not read at run time, so a bundle (runtime/entry.mjs) carries it.
import pkg from "../package.json" with { type: "json" };

// stdout carries protocol lines only; diagnostics go to stderr.
const emit = (obj) => process.stdout.write(`${JSON.stringify(obj)}\n`);

// The SDK is loaded by the first request that needs it (fetched from npm once, sdk.js).
const server = createServer({
  sdk: () => loadSdk(),
  emit,
  sidecarVersion: pkg.version,
  sdkVersion: pin.version,
});

emit({ type: "ready", protocol: 1, sidecar_version: pkg.version, sdk_version: pin.version });

const rl = createInterface({ input: process.stdin });
let inflight = 0;
let eof = false;

const maybeExit = async () => {
  if (eof && inflight === 0) {
    await server.closeAll();
    process.exit(0);
  }
};

rl.on("line", (line) => {
  if (!line.trim()) return;
  let req;
  try {
    req = JSON.parse(line);
  } catch {
    emit({ type: "reply", ok: false, error: "invalid json" });
    return;
  }
  inflight++;
  server
    .handle(req)
    .then(emit)
    .finally(() => {
      inflight--;
      void maybeExit();
    });
});

// radar went away: end every session (pending permissions are denied), then exit.
rl.on("close", () => {
  eof = true;
  void maybeExit();
});
