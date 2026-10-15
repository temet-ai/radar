import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { storyNameFromExport, toId } from "storybook/internal/csf";

/**
 * docs/storybook/components.json is Storybook's component manifest, committed so agents can read radar's component docs
 * without a running Storybook (`npm run manifest` writes it). This keeps it in step with the story files without a
 * build: the story ids the files declare (their `title` and exported stories) must be the ids the manifest holds.
 */
const here = path.dirname(fileURLToPath(import.meta.url));
const ink = path.resolve(here, "../..");
const storiesDir = path.join(ink, "stories");
const manifestPath = path.resolve(ink, "../docs/storybook/components.json");

const walk = (d: string): string[] =>
  fs.readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]));

/** The ids a CSF file declares: `title: "..."` in its meta and each `export const Name` (CSF's named stories). */
function declaredIds(file: string): string[] {
  const src = fs.readFileSync(file, "utf8");
  const title = /^\s*title:\s*"([^"]+)"/m.exec(src)?.[1];
  if (!title) return [];
  return [...src.matchAll(/^export const (\w+)\b/gm)].map((m) => toId(title, storyNameFromExport(m[1]!)));
}

describe("the committed Storybook manifest", () => {
  it("holds exactly the stories the story files declare (else run `npm run manifest` in ink/)", () => {
    expect(fs.existsSync(manifestPath), "docs/storybook/components.json is written by `npm run manifest`").toBe(true);
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as {
      components: Record<string, { stories?: Array<{ id: string }> }>;
    };
    const inManifest = Object.values(manifest.components)
      .flatMap((c) => (c.stories ?? []).map((s) => s.id))
      .sort();
    const inFiles = walk(storiesDir)
      .filter((f) => f.endsWith(".stories.tsx") || f.endsWith(".stories.ts"))
      .flatMap(declaredIds)
      .sort();
    expect(inFiles.length).toBeGreaterThan(0);
    expect(inManifest).toEqual(inFiles);
  });
});
