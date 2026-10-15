import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

export default defineConfig({
  // Ink's output.js, which src/app/fast-output.ts patches (Ink's package exports do not offer it).
  resolve: { alias: { "ink-output": fileURLToPath(new URL("./node_modules/ink/build/output.js", import.meta.url)) } },
  test: {
    include: ["test/**/*.test.ts", "test/**/*.test.tsx"],
  },
});
