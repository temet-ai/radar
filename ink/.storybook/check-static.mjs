#!/usr/bin/env node
// Checks the static Storybook build renders: serves ink/storybook-static (run `npm run build-storybook` first) on a
// free port, opens one story's iframe in headless Chrome, waits for Ink's cell grid to hold the story's text, and
// writes a screenshot. Exits non-zero, with the reason, when the story does not render. Not part of `npm test`.
//
//   node .storybook/check-static.mjs [story-id] [screenshot.png]
//   (default: nodes-treenode--folded, storybook-static-check.png in the system temp folder)
//
// Chrome: $CHROME, else google-chrome-stable, google-chrome, chromium, or the macOS app.
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../storybook-static");
const story = process.argv[2] ?? "nodes-treenode--folded";
const shot = process.argv[3] ?? path.join(os.tmpdir(), "storybook-static-check.png");
const want = process.env.CHECK_TEXT ?? "team-b";
if (!fs.existsSync(path.join(root, "iframe.html"))) {
  console.error("no ink/storybook-static/iframe.html: run npm run build-storybook first");
  process.exit(2);
}

const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml", ".png": "image/png", ".woff2": "font/woff2" };
const server = http.createServer((req, res) => {
  const p = path.join(root, decodeURIComponent(new URL(req.url ?? "/", "http://x").pathname));
  const file = fs.existsSync(p) && fs.statSync(p).isDirectory() ? path.join(p, "index.html") : p;
  if (!file.startsWith(root) || !fs.existsSync(file)) {
    res.writeHead(404).end();
    return;
  }
  res.writeHead(200, { "content-type": types[path.extname(file)] ?? "application/octet-stream" });
  fs.createReadStream(file).pipe(res);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const port = server.address().port;

const candidates = [process.env.CHROME, "google-chrome-stable", "google-chrome", "chromium", "chromium-browser", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"].filter(Boolean);
const profile = fs.mkdtempSync(path.join(os.tmpdir(), "radar-sb-check-"));
const debugPort = 20000 + Math.floor(Math.random() * 20000);
let chrome;
for (const bin of candidates) {
  try {
    chrome = spawn(bin, ["--headless=new", "--disable-gpu", "--no-sandbox", `--remote-debugging-port=${debugPort}`, `--user-data-dir=${profile}`, "--window-size=1300,600", "about:blank"], { stdio: "ignore" });
    await new Promise((resolve, reject) => {
      chrome.once("error", reject);
      setTimeout(resolve, 300);
    });
    break;
  } catch {
    chrome = undefined;
  }
}
const finish = (code, message) => {
  console[code ? "error" : "log"](message);
  try {
    chrome?.kill();
  } catch {}
  server.close();
  setTimeout(() => {
    try {
      fs.rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
    } catch {}
    process.exit(code);
  }, 300);
};
if (!chrome) finish(2, "no Chrome found: set CHROME to its path");

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let ws;
for (let i = 0; i < 50 && !ws; i++) {
  await sleep(200);
  try {
    const page = (await (await fetch(`http://127.0.0.1:${debugPort}/json`)).json()).find((t) => t.type === "page");
    if (page) ws = new WebSocket(page.webSocketDebuggerUrl);
  } catch {}
}
if (!ws) finish(2, "Chrome's debugging port did not answer");
await new Promise((r) => ws.addEventListener("open", r));
let id = 0;
const pending = new Map();
const errors = [];
ws.addEventListener("message", (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) {
    pending.get(m.id)(m);
    pending.delete(m.id);
  }
  if (m.method === "Runtime.exceptionThrown") errors.push(m.params.exceptionDetails.exception?.description ?? m.params.exceptionDetails.text);
});
const send = (method, params = {}) =>
  new Promise((r) => {
    const i = ++id;
    pending.set(i, r);
    ws.send(JSON.stringify({ id: i, method, params }));
  });
await send("Runtime.enable");
await send("Page.enable");
const url = `http://127.0.0.1:${port}/iframe.html?id=${story}&viewMode=story`;
await send("Page.navigate", { url });
let text = "";
const deadline = Date.now() + 30000;
while (Date.now() < deadline) {
  await sleep(250);
  const r = await send("Runtime.evaluate", { expression: `[...document.querySelectorAll('.radar-term')].map((g) => g.textContent).join('\\n')` });
  text = r.result?.result?.value ?? "";
  if (text.includes(want)) break;
}
const png = await send("Page.captureScreenshot", { format: "png" });
fs.writeFileSync(shot, Buffer.from(png.result.data, "base64"));
if (!text.includes(want))
  finish(1, `${story} did not render from the static build in 30 s (no "${want}" in Ink's cells); screenshot ${shot}${errors.length ? `\nerrors:\n${errors.join("\n")}` : ""}`);
finish(0, `ok: ${story} renders from ink/storybook-static (Ink's cells hold "${want}"); screenshot ${shot}`);
