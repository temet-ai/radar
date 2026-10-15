#!/usr/bin/env node
// A live check: a session set up with radar's own MCP
// server (setup.tool_server, from radar-sessions' mcp_listen example) calls its echo tool, then a
// second sidecar resumes the session and calls it again. Records every line both ways, as
// record.mjs does ({"dir":"in"|"out","line":..}).
//
//   cargo run -p radar-sessions --example mcp_listen -- <sock> <radar> <server.json> &
//   node scripts/record-mcp.mjs <scratch-repo-dir> <out.ndjson> <server.json>
//
// The session gets radar's policy: permissionMode default, no settings sources, ask on Bash. A
// permission request is printed; one for an mcp__radar__ tool is approved once, any other denied
// (with allowedTools naming mcp__radar__echo, none should come). Scrub the output with
// scripts/scan-secrets; the caller deletes the provider's
// session afterwards (see the skill).
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { readFileSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";

const [cwd, outPath, serverPath] = process.argv.slice(2);
if (!cwd || !outPath || !serverPath) throw new Error("usage: record-mcp.mjs <scratch-repo-dir> <out.ndjson> <server.json>");
const toolServer = JSON.parse(readFileSync(serverPath, "utf8"));

const log = [];
const sessionId = randomUUID();

function start() {
  const child = spawn(process.execPath, [new URL("../src/main.js", import.meta.url).pathname], { stdio: ["pipe", "pipe", "inherit"] });
  const lines = [];
  const waiters = [];
  const rl = createInterface({ input: child.stdout });
  rl.on("line", (line) => {
    const l = JSON.parse(line);
    log.push({ dir: "out", line: l });
    lines.push(l);
    if (l.kind === "permission_request") {
      const p = l.payload;
      console.error("permission", p.tool_name ?? p.toolName ?? JSON.stringify(p).slice(0, 200));
      const name = String(p.tool_name ?? p.toolName ?? p.input?.tool ?? "");
      call({ op: "answer", session: l.session, request_id: p.request_id, ...(name.startsWith("mcp__radar__") ? { behavior: "allow" } : { behavior: "deny", message: "Only radar's echo is approved in this check." }) });
    }
    if (l.kind === "tool_call" || l.kind === "tool_result") console.error(l.kind, JSON.stringify(l.payload).slice(0, 300));
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
  let cursor = 0;
  const waitFor = (pred, ms = 180000) =>
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
  const mark = () => (cursor = lines.length);
  return { call, waitFor, mark, child, lines };
}

const g = "radar-live-mcp";
const setup = { tool_server: toolServer, project_config: false };

let a = start();
await a.waitFor((l) => l.type === "ready");
await a.call({ op: "create", session: g, cwd, session_id: sessionId, setup });
a.mark();
await a.call({ op: "send", session: g, text: "Call the echo tool from the radar server with text 'ping' and tell me the result." });
await a.waitFor((l) => l.kind === "turn_end");
await a.call({ op: "close", session: g });
a.child.stdin.end();
await new Promise((r) => a.child.on("exit", r));

// A new sidecar process resumes the session with the same setup: the server must be there again.
let b = start();
await b.waitFor((l) => l.type === "ready");
await b.call({ op: "create", session: g, cwd, resume: sessionId, setup });
b.mark();
await b.call({ op: "send", session: g, text: "Call the echo tool from the radar server again, with text 'pong', and tell me the result." });
await b.waitFor((l) => l.kind === "turn_end");
await b.call({ op: "close", session: g });
b.child.stdin.end();
await new Promise((r) => b.child.on("exit", r));

writeFileSync(outPath, log.map((l) => JSON.stringify(l)).join("\n") + "\n");
console.error(`recorded ${log.length} lines; provider session ${sessionId}`);
