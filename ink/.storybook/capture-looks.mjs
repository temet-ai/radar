#!/usr/bin/env node
// Writes ink/stories/looks/<tier>.json: the Look message the real radar host sends at each colour tier (truecolor, 256,
// 16, mono; ToRenderer::Look, ipc/look.rs), for Storybook's tier switch. The page never resolves a colour itself.
//
//   node .storybook/capture-looks.mjs [path/to/radar]      (default: <repo>/target/release/radar)
//
// Each tier runs radar once in a private tmux server with RADAR_RENDERER=ink and RADAR_INK_BUNDLE pointing at
// capture-child.mjs, which records the host's first look and leaves. The tier comes from the environment as radar reads
// it (design/theme/capability.rs): COLORTERM=truecolor, TERM=xterm-256color for 256, TERM=xterm for 16, NO_COLOR=1
// for mono.
// radar gets scratch XDG folders and an empty config, so it reads nothing of yours and calls no service.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const ink = path.resolve(here, "..");
const repo = path.resolve(ink, "..");
const radar = path.resolve(process.argv[2] ?? path.join(repo, "target/release/radar"));
if (!fs.existsSync(radar)) {
  console.error(`no radar binary at ${radar}; build it (cargo build --release --bin radar) or pass its path`);
  process.exit(1);
}
const outDir = path.join(ink, "stories/looks");
fs.mkdirSync(outDir, { recursive: true });

const tiers = [
  { tier: "truecolor", term: "xterm-256color", extra: ["COLORTERM=truecolor"] },
  { tier: "256", term: "xterm-256color", extra: [] },
  { tier: "16", term: "xterm", extra: [] },
  { tier: "mono", term: "xterm", extra: ["NO_COLOR=1"] },
];

for (const { tier, term, extra } of tiers) {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), "radar-look-"));
  for (const sub of ["cfg/radar", "state", "cache", "data", "home"]) fs.mkdirSync(path.join(d, sub), { recursive: true });
  fs.writeFileSync(path.join(d, "cfg/radar/config.toml"), "");
  const out = path.join(d, "look.json");
  const env = [
    `HOME=${d}/home`, `XDG_CONFIG_HOME=${d}/cfg`, `XDG_STATE_HOME=${d}/state`, `XDG_CACHE_HOME=${d}/cache`,
    `XDG_DATA_HOME=${d}/data`, `TERM=${term}`, "LANG=C.UTF-8", "RADAR_MODE=read-only", "RADAR_NO_WRITES=1",
    "RADAR_RENDERER=ink", `RADAR_INK_BUNDLE=${path.join(here, "capture-child.mjs")}`, `RADAR_CAPTURE_OUT=${out}`, ...extra,
  ];
  const server = `radar-look-${process.pid}-${tier}`;
  const cmd = `env -u COLORTERM -u NO_COLOR ${env.map((e) => `'${e}'`).join(" ")} '${radar}' 2>'${d}/stderr'`;
  execFileSync("tmux", ["-L", server, "-f", "/dev/null", "new-session", "-d", "-x", "80", "-y", "24", cmd]);
  const end = Date.now() + 15000;
  while (!fs.existsSync(out) && Date.now() < end) execFileSync("sleep", ["0.2"]);
  try {
    execFileSync("tmux", ["-L", server, "kill-server"], { stdio: "ignore" });
  } catch {}
  if (!fs.existsSync(out)) {
    console.error(`tier ${tier}: the host sent no look in 15 s; stderr:\n${fs.readFileSync(path.join(d, "stderr"), "utf8")}`);
    process.exit(1);
  }
  const look = JSON.parse(fs.readFileSync(out, "utf8"));
  const kept = { tier: look.tier, source: `${execFileSync(radar, ["--version"]).toString().trim()}, ${[`TERM=${term}`, ...extra].join(" ")}`, slots: look.slots, modifiers: look.modifiers };
  if (!kept.slots) {
    console.error(`tier ${tier}: unexpected look message: ${JSON.stringify(look).slice(0, 300)}`);
    process.exit(1);
  }
  fs.writeFileSync(path.join(outDir, `${tier}.json`), `${JSON.stringify(kept, null, 2)}\n`);
  console.log(`wrote stories/looks/${tier}.json (tier ${kept.tier}, ${Object.keys(kept.slots).length} slots)`);
  fs.rmSync(d, { recursive: true, force: true });
}
