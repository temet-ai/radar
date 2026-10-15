// The npm packages an esbuild bundle carries, with their licence texts, as Markdown.
// usage: node scripts/notices-npm.mjs META.json PACKAGE_ROOT > out.md
// META.json is esbuild's --metafile of the bundle; PACKAGE_ROOT is the folder its input
// paths are relative to (where node_modules is).
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const [meta, root] = process.argv.slice(2);
const inputs = Object.keys(JSON.parse(readFileSync(meta, "utf8")).inputs);

// node_modules/a/node_modules/@s/b/x.js -> node_modules/a/node_modules/@s/b
const packages = new Set();
for (const input of inputs) {
  const m = /^(.*node_modules\/(?:@[^/]+\/)?[^/]+)\//.exec(input);
  if (m) packages.add(m[1]);
}

const entries = [];
for (const dir of packages) {
  const full = join(root, dir);
  const pkg = JSON.parse(readFileSync(join(full, "package.json"), "utf8"));
  const file = readdirSync(full).find((f) => /^(licen[cs]e|copying)(\.|$)/i.test(f));
  const text = file ? readFileSync(join(full, file), "utf8").trim() : null;
  const license = typeof pkg.license === "string" ? pkg.license : pkg.license?.type ?? "unknown";
  if (!text && !existsSync(full)) throw new Error(`${dir} is missing`);
  // The same version installed in more than one place is listed once.
  if (entries.some((e) => e.name === pkg.name && e.version === pkg.version)) continue;
  entries.push({ name: pkg.name, version: pkg.version, license, text, repo: pkg.repository?.url ?? pkg.repository ?? "" });
}
entries.sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));

const out = ["## npm packages (the Ink renderer)", ""];
for (const e of entries) {
  out.push(`### ${e.name} ${e.version} (${e.license})`, "");
  if (e.repo) out.push(`${String(e.repo).replace(/^git\+/, "")}`, "");
  out.push(e.text ? ["```", e.text, "```"].join("\n") : `Licensed under ${e.license}; the package ships no licence file.`, "");
}
process.stdout.write(`${out.join("\n")}\n`);
