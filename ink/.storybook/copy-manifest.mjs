#!/usr/bin/env node
// Copies Storybook's component manifest (storybook-static/manifests/components.json, written by `npm run
// build-storybook`) to docs/storybook/components.json, where agents read it without a running Storybook. Absolute
// paths of this checkout become repository paths (`ink/src/...`), so the file is the same on every machine.
// `npm run manifest` builds and copies; test/storybook/manifest.test.ts fails when a story is added or removed
// without running it.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ink = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repo = path.resolve(ink, "..");
const from = path.join(ink, "storybook-static/manifests/components.json");
const to = path.join(repo, "docs/storybook/components.json");
if (!fs.existsSync(from)) {
  console.error(`no ${path.relative(repo, from)}: run npm run build-storybook first (npm run manifest does both)`);
  process.exit(1);
}
const text = fs.readFileSync(from, "utf8").split(`${repo}/`).join("");
const manifest = JSON.parse(text);
fs.mkdirSync(path.dirname(to), { recursive: true });
fs.writeFileSync(to, `${JSON.stringify(manifest, null, 2)}\n`);
const stories = Object.values(manifest.components).reduce((n, c) => n + (c.stories?.length ?? 0), 0);
console.log(`wrote ${path.relative(repo, to)}: ${Object.keys(manifest.components).length} components, ${stories} stories`);
