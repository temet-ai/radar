#!/usr/bin/env node
// Writes ink/stories/design/Research.mdx from the design canvas's board 09 (stories/design/boards/Research.dc.html):
// its tables come from the board's own renderVals() data, its prose from the board's text, so the page says exactly
// what the board says, as text agents can read (Storybook's docs manifest carries MDX text, not pictures).
//
//   node .storybook/research-mdx.mjs
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const board = path.join(here, "../stories/design/boards/Research.dc.html");
const out = path.join(here, "../stories/design/Research.mdx");
const html = fs.readFileSync(board, "utf8");

// The board's data: its script's renderVals() with the canvas's DCLogic standing in.
const code = /<script type="text\/x-dc"[^>]*>([\s\S]*?)<\/script>/.exec(html)[1];
class DCLogic {
  constructor(props) {
    this.props = props;
  }
}
const Component = new Function("DCLogic", `${code}\nreturn Component;`)(DCLogic);
const v = new Component({}).renderVals();

const entities = { "&amp;": "&", "&lt;": "<", "&gt;": ">", "&quot;": '"', "&#39;": "'", "&nbsp;": " " };
const decode = (s) => s.replace(/&#x([0-9a-f]+);/gi, (_, h) => String.fromCodePoint(parseInt(h, 16))).replace(/&[a-z#0-9]+;/gi, (e) => entities[e] ?? e);
const textOf = (s) => decode(String(s ?? "").replace(/<[^>]+>/g, " ")).replace(/\s+/g, " ").trim();
const cell = (s) => textOf(s).replace(/\|/g, "\\|").replace(/[{}<>]/g, (c) => `&#${c.charCodeAt(0)};`);
const prose = (s) => cell(s);

/** The board's markup between one `.h2` and the next. */
function sectionHtml(title) {
  const parts = html.split(/<h2 class="h2"[^>]*>/);
  const i = parts.findIndex((p) => textOf(p.split("</h2>")[0]) === title);
  return i < 0 ? "" : parts[i].split("</h2>").slice(1).join("</h2>");
}
/** Paragraph-ish blocks of a section's static text (template loops left out). */
function blocks(title) {
  const s = sectionHtml(title).replace(/<sc-for[\s\S]*?<\/sc-for>/g, "");
  return s
    .split(/<\/(?:p|div)>/)
    .map(textOf)
    .filter((t) => t && !t.includes("{{"));
}

const ISSUES = {
  "Gradient focus border, wordmark": 31,
  "Gradient meters, ratio meters": 31,
  "Tinted chips": 30,
  "Scrim and shadow behind overlays": 29,
  "Command palette": 35,
  "Notices (stacking, draining)": 41,
  "Assemble on screen switch": 42,
  "Rainbow lap on focus": 32,
  "Loading outlines with a wave": 42,
  "Fold unroll": 32,
  "One tree, every part": 45,
  "Fixed two-cell icon slot": 43,
  "Key underlined in its word": 44,
  "Tabs without counts, attention dot": 33,
  "Charts: bars, braille, heat, big digits": 39,
  "Pipeline graph (DAG)": 34,
  "Mermaid diagrams": 40,
  "Markdown editor with preview": 37,
  "Pictures: Slack images, avatars": 38,
};

const lede = textOf(/<p class="lede"[^>]*>([\s\S]*?)<\/p>/.exec(html)?.[1]);
const lines = [];
lines.push(`{/* Written by .storybook/research-mdx.mjs from stories/design/boards/Research.dc.html; edit the board, then run it. */}`);
lines.push(`import { Meta } from "@storybook/addon-docs/blocks";`, "", `<Meta title="Design/Research" />`, "");
lines.push(`# Research and readiness`, "", `Board 09 of the design canvas, as text. ${prose(lede)}`, "");
lines.push(
  "Every proposal here is a **target, not built yet** unless its verdict says shipped. The issue column names the issue that builds it (issue #47 orders them).",
  "",
);
lines.push(`## Every proposal`, "", `| proposal | sheet | verdict | terminals | library | what it takes | issue |`, `|---|---|---|---|---|---|---|`);
for (const r of v.rows) lines.push(`| ${cell(r.name)} | ${cell(r.sheet)} | ${cell(r.verdict)} | ${cell(r.term)} | ${cell(r.lib)} | ${cell(r.note)} | ${ISSUES[textOf(r.name)] ? `#${ISSUES[textOf(r.name)]}` : ""} |`);
lines.push("");
lines.push(`## Decisions for you`, "");
for (const b of blocks("Decisions for you")) lines.push(`- ${prose(b)}`);
lines.push("", `## Libraries worth taking`, "", `| library | verdict | facts | note |`, `|---|---|---|---|`);
for (const l of v.libs) lines.push(`| ${cell(l.name)} | ${cell(l.verdict)} | ${cell(l.meta)} | ${cell(l.note)} |`);
lines.push("");
for (const b of blocks("Libraries worth taking")) lines.push(prose(b), "");
lines.push(`## How they integrate`, "");
for (const b of blocks("How they integrate").filter((t) => !t.startsWith("library tried"))) lines.push(prose(b), "");
lines.push(`| library | tried | lives in | host sends | what the trial showed |`, `|---|---|---|---|---|`);
for (const r of v.integ) lines.push(`| ${cell(r.name)} ${cell(r.meta)} | ${cell(r.verdict)} | ${cell(r.file)} | ${cell(r.host)} | ${cell(r.note)} |`);
lines.push("", "The dagre layout of a pipeline, drawn to cells by the trial's rasteriser:", "", "```text", ...String(v.dagreOut).split("\n"), "```", "");
lines.push(`## Order`, "");
for (const b of blocks("Order")) lines.push(`- ${prose(b)}`);
lines.push("", `## Adopted from other terminal apps`, "");
for (const p of v.prior) lines.push(`- **${cell(p.t)}** (${cell(p.src)}): ${cell(p.n)}`);
const rest = html.split(/Sources and limits/)[1] ?? "";
lines.push("", `## Sources and limits`, "");
for (const b of rest.split(/<\/(?:p|div)>/).map(textOf).filter((t) => t && !t.includes("{{") && !t.startsWith("class Component"))) {
  if (b.length > 40) lines.push(prose(b), "");
}
fs.writeFileSync(out, `${lines.join("\n").replace(/\n{3,}/g, "\n\n")}\n`);
console.log(`wrote ${path.relative(path.join(here, ".."), out)}: ${v.rows.length} proposals, ${v.libs.length} libraries, ${v.integ.length} integrations`);
