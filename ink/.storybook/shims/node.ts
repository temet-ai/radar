// node:stream, node:util and node:fs for the browser, reduced to what Ink touches (see main.ts `nodeShims`).
import { EventEmitter } from "./events.js";

/** node:stream: Ink only asks `stdout instanceof Stream` (a story's stdout is not one). */
export class Stream extends EventEmitter {}
export class Readable extends Stream {}
export class Writable extends Stream {}

/** node:util: Ink's ErrorBoundary and render-to-string check `types.isPromise`. */
export const types = {
  isPromise: (v: unknown): v is Promise<unknown> =>
    typeof (v as { then?: unknown })?.then === "function",
};
export const inspect = (v: unknown) => String(v);
export const format = (...args: unknown[]) => args.map(String).join(" ");

/** node:fs: Ink's ErrorOverview reads the failing source file when it exists; in a browser it never does. */
export const existsSync = () => false;
export const readFileSync = () => "";

export default { Stream, Readable, Writable, types, inspect, format, existsSync, readFileSync };
