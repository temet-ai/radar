// Startup: Yoga's wasm loader (the layout engine under Ink) asks for the module with `fetch(data:...)` when a global
// `fetch` exists, and the first call to Node's fetch loads undici, 30 to 45 ms of the renderer's 100 ms start-up
// (T21: node --cpu-prof of the bundle). Without a global fetch the loader decodes the same base64 itself, which is a
// few milliseconds. Imported before Ink so it runs before Yoga; `restoreFetch` (main.tsx) puts fetch back.
const saved = Object.getOwnPropertyDescriptor(globalThis, "fetch");
if (saved?.configurable) delete (globalThis as { fetch?: unknown }).fetch;

export function restoreFetch(): void {
  if (saved?.configurable) Object.defineProperty(globalThis, "fetch", saved);
}
