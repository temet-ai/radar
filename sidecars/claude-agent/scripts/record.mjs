#!/usr/bin/env node
// Records one live run of the sidecar against the installed `claude`, once, with tiny
// prompts, in a scratch git repo: ls (approved), write b.txt (denied), a third turn
// interrupted mid-way, then a second sidecar process resumes the session and reads its
// history. Output: a file with every line in both directions ({"dir":"in"|"out","line":..}).
//
//   node scripts/record.mjs <scratch-repo-dir> <out.ndjson>
//
// Scrub the output with scripts/scan-secrets before it is
// committed. The caller deletes the provider's session afterwards (see the skill).
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";

const [cwd, outPath] = process.argv.slice(2);
if (!cwd || !outPath) throw new Error("usage: record.mjs <scratch-repo-dir> <out.ndjson>");

const log = [];
const sessionId = randomUUID();

function start() {
  const child = spawn(process.execPath, [new URL("../src/main.js", import.meta.url).pathname], { stdio: ["pipe", "pipe", "inherit"] });
  const lines = [];
  const waiters = [];
  const rl = createInterface({ input: child.stdout });
  rl.on("line", (line) => {
    log.push({ dir: "out", line: JSON.parse(line) });
    lines.push(JSON.parse(line));
    for (const w of [...waiters]) w();
  });
  let n = 0;
  const call = async (req) => {
    const id = ++n;
    const full = { id, ...req };
    log.push({ dir: "in", line: full });
    child.stdin.write(`${JSON.stringify(full)}\n`);
    return waitFor((l) => l.type === "reply" && l.id === id);
  };
  const waitFor = (pred, ms = 120000) =>
    new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error("timeout waiting")), ms);
      const check = () => {
        const hit = lines.find((l, i) => i >= cursor && pred(l));
        if (hit) {
          clearTimeout(t);
          waiters.splice(waiters.indexOf(check), 1);
          resolve(hit);
          return true;
        }
        return false;
      };
      if (!check()) waiters.push(check);
    });
  let cursor = 0;
  const mark = () => (cursor = lines.length);
  return { call, waitFor, mark, child, lines };
}

const g = "radar-live-1";
let a = start();
await a.waitFor((l) => l.type === "ready");
console.error("status", JSON.stringify((await a.call({ op: "status" })).result.account ? "ok" : "no"));
await a.call({ op: "create", session: g, cwd, session_id: sessionId });

a.mark();
await a.call({ op: "send", session: g, text: "Run `ls` and tell me the file names. One sentence." });
const p1 = await a.waitFor((l) => l.kind === "permission_request");
await a.call({ op: "answer", session: g, request_id: p1.payload.request_id, behavior: "allow" });
await a.waitFor((l) => l.kind === "turn_end");

a.mark();
await a.call({ op: "send", session: g, text: "Create b.txt containing hi, using the Write tool. One sentence." });
const p2 = await a.waitFor((l) => l.kind === "permission_request");
await a.call({ op: "answer", session: g, request_id: p2.payload.request_id, behavior: "deny", message: "Denied in radar." });
await a.waitFor((l) => l.kind === "turn_end");

a.mark();
await a.call({ op: "send", session: g, text: "Run `sleep 30` with the shell tool, then say done." });
const p3 = await a.waitFor((l) => l.kind === "permission_request");
await a.call({ op: "answer", session: g, request_id: p3.payload.request_id, behavior: "allow" });
await a.waitFor((l) => l.kind === "tool_call" && l.payload.name === "Bash");
await a.call({ op: "interrupt", session: g });
await a.waitFor((l) => l.kind === "turn_end");

await a.call({ op: "close", session: g });
a.child.stdin.end();
await new Promise((r) => a.child.on("exit", r));

// A new sidecar process: resume and read history.
let b = start();
await b.waitFor((l) => l.type === "ready");
await b.call({ op: "create", session: g, cwd, resume: sessionId });
b.mark();
await b.call({ op: "send", session: g, text: "What file names did you list earlier? One sentence." });
await b.waitFor((l) => l.kind === "turn_end");
await b.call({ op: "history", session: g, provider_session_id: sessionId, dir: cwd });
await b.call({ op: "close", session: g });
b.child.stdin.end();
await new Promise((r) => b.child.on("exit", r));

writeFileSync(outPath, log.map((l) => JSON.stringify(l)).join("\n") + "\n");
console.error(`recorded ${log.length} lines; provider session ${sessionId}`);
