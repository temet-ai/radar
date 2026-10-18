#!/usr/bin/env node
// Normalize a real Storybook manifest and replace the committed copy only after validation succeeds.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export function copyManifest({ from, to, repo }) {
  const text = fs.readFileSync(from, "utf8").split(`${repo}/`).join("");
  const manifest = JSON.parse(text);
  if (!manifest?.components || typeof manifest.components !== "object" || Array.isArray(manifest.components)) {
    throw new Error("generated manifest has no components object");
  }
  const components = Object.values(manifest.components);
  let stories = 0;
  for (const component of components) {
    if (!component || typeof component !== "object" || Array.isArray(component) ||
        (component.stories !== undefined && !Array.isArray(component.stories))) {
      throw new Error("generated manifest has an invalid component or stories list");
    }
    stories += component.stories?.length ?? 0;
  }
  const serialized = `${JSON.stringify(manifest, null, 2)}\n`;
  fs.mkdirSync(path.dirname(to), { recursive: true });
  const staging = fs.mkdtempSync(path.join(path.dirname(to), ".manifest-"));
  let cleanupWarning;
  try {
    const temporary = path.join(staging, "components.json");
    fs.writeFileSync(temporary, serialized);
    fs.renameSync(temporary, to);
  } finally {
    // Cleanup must not mask a write failure or turn a committed replacement into a reported failure.
    try { fs.rmSync(staging, { recursive: true, force: true }); }
    catch (error) { cleanupWarning = `Could not remove manifest staging directory ${staging}: ${error.message}`; }
  }
  return { message: `wrote ${path.relative(repo, to)}: ${components.length} components, ${stories} stories`, cleanupWarning };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const ink = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const repo = path.dirname(ink);
  try {
    const result = copyManifest({ from: path.join(ink, "storybook-static/manifests/components.json"), to: path.join(repo, "docs/storybook/components.json"), repo });
    console.log(result.message);
    if (result.cleanupWarning) console.error(result.cleanupWarning);
  } catch (error) {
    console.error(`Manifest copy failed: ${error.message}; run npm run manifest in ink/.`);
    process.exitCode = 1;
  }
}
