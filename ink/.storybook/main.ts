import path from "node:path";
import { readFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { fileURLToPath } from "node:url";
import type { StorybookConfig } from "@storybook/react-vite";

// react-docgen-typescript (the props in the docs pages and in the component manifest, which @storybook/react builds)
// needs TypeScript's JS API, which TypeScript 7.0 (radar's compiler) does not have. Both get TypeScript 5 from
// .storybook/docgen-typescript; `tsc` stays 7.
registerHooks({
  resolve(specifier, context, next) {
    if (specifier === "typescript" && /react-docgen-typescript|@storybook\/react\/dist/.test(context.parentURL ?? ""))
      return next("radar-docgen-typescript", context);
    return next(specifier, context);
  },
});

const here = path.dirname(fileURLToPath(import.meta.url));
const ink = path.resolve(here, "..");
const shim = (f: string) => path.join(here, "shims", f);

/**
 * Ink is written for Node. The stories run the real Ink components in the browser, so each Node module Ink or its
 * dependencies import is replaced by a small shim (`.storybook/shims/`); the two aliases radar's own bundle script sets
 * (`react-devtools-core`, `ink-output`) are set here the same way, so app/fast-output.ts patches the Output Ink uses.
 */
const nodeShims: Array<{ find: RegExp; replacement: string }> = [
  { find: /^(node:)?process$/, replacement: shim("process.ts") },
  { find: /^(node:)?events$/, replacement: shim("events.ts") },
  { find: /^(node:)?module$/, replacement: shim("module.ts") },
  { find: /^(node:)?(os|path|url)$/, replacement: shim("os-path-url.ts") },
  // No top-level await in the build (shims/yoga.ts says why); the enums come from the package's own file.
  { find: /^yoga-layout$/, replacement: shim("yoga.ts") },
  { find: /^radar-yoga-enums$/, replacement: path.join(ink, "node_modules/yoga-layout/dist/src/generated/YGEnums.js") },
  { find: /^(node:)?(stream|util|fs)$/, replacement: shim("node.ts") },
  { find: /^signal-exit$/, replacement: shim("signal-exit.ts") },
  { find: /^terminal-size$/, replacement: shim("terminal-size.ts") },
  { find: /^cli-cursor$/, replacement: shim("cli-cursor.ts") },
  { find: /^is-in-ci$/, replacement: shim("is-in-ci.ts") },
  { find: /^patch-console$/, replacement: shim("patch-console.ts") },
  { find: /^react-devtools-core$/, replacement: path.join(ink, "src/transport/devtools-stub.ts") },
  { find: /^ink-output$/, replacement: path.join(ink, "node_modules/ink/build/output.js") },
];

/**
 * No top-level await may reach the stories' chunks: in the static build (Rolldown) a module with one turns every
 * chunk that imports it into an async module, the story chunks import each other in a cycle, and a story's module then
 * never finishes evaluating (the preview stays on its spinner, with no error; the dev server does not bundle, so it
 * was fine). Ink has three: yoga-layout's wasm load (shims/yoga.ts replaces the module) and, in reconciler.js,
 * `await import('./devtools.js')` and `await loadPackageJson()`, both behind `DEV=true`, removed here as the file loads
 * (radar's bundle swaps devtools for a stub too).
 */
const noTopLevelAwait = {
  name: "radar-no-top-level-await",
  enforce: "pre" as const,
  load(id: string) {
    if (!id.replace(/\\/g, "/").endsWith("/ink/build/reconciler.js")) return null;
    const code = readFileSync(id, "utf8");
    const swaps: Array<[string, string]> = [
      ["await import('./devtools.js');", "/* react-devtools: not in Storybook */"],
      ["const loaded = await loadPackageJson();", "const loaded = {}; /* package.json: not read in Storybook */"],
    ];
    let out = code;
    for (const [from, to] of swaps) {
      if (!out.includes(from)) throw new Error(`ink/build/reconciler.js changed: \`${from}\` not found (see .storybook/main.ts)`);
      out = out.replace(from, to);
    }
    return { code: out, map: null };
  },
};

const config: StorybookConfig = {
  stories: ["../stories/**/*.mdx", "../stories/**/*.stories.@(ts|tsx)"],
  addons: ["@storybook/addon-docs", "@storybook/addon-a11y", "@storybook/addon-mcp"],
  framework: "@storybook/react-vite",
  features: { componentsManifest: true },
  typescript: {
    reactDocgen: "react-docgen-typescript",
    // tsconfig.json covers src only; the stories' own helpers (stories/support/nodes.tsx) need to be in the program.
    reactDocgenTypescriptOptions: { tsconfigPath: path.join(ink, "tsconfig.storybook.json") },
  },
  core: { disableTelemetry: true },
  async viteFinal(vite) {
    const existing = vite.resolve?.alias;
    const before = Array.isArray(existing)
      ? existing
      : Object.entries(existing ?? {}).map(([find, replacement]) => ({ find, replacement: replacement as string }));
    vite.resolve = { ...vite.resolve, alias: [...nodeShims, ...before] };
    vite.plugins = [noTopLevelAwait, ...(vite.plugins ?? [])];
    // yoga-layout (Ink's layout engine) loads its wasm with a top-level await.
    vite.build = { ...vite.build, target: "esnext", chunkSizeWarningLimit: 4096 };
    vite.optimizeDeps = { ...vite.optimizeDeps, include: [...(vite.optimizeDeps?.include ?? []), "ink"] };
    vite.define = { ...vite.define, "process.env.NODE_ENV": JSON.stringify("production") };
    return vite;
  },
};

export default config;
