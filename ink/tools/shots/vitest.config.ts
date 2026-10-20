import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

// Runs tools/shots/draw.test.tsx on its own; the normal suite (vitest.config.ts) does not include it.
const ink = fileURLToPath(new URL("../..", import.meta.url));
export default defineConfig({
  root: ink,
  resolve: { alias: { "ink-output": fileURLToPath(new URL("../../node_modules/ink/build/output.js", import.meta.url)) } },
  test: { include: ["tools/shots/draw.test.tsx"], testTimeout: 120000 },
});
