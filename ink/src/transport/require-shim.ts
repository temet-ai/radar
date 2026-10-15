// The ESM bundle contains CommonJS dependencies (signal-exit and others) that call
// `require("assert")`; give them one. Import this first in the entry point.
import { createRequire } from "node:module";

const g = globalThis as { require?: NodeJS.Require };
g.require ??= createRequire(import.meta.url);
