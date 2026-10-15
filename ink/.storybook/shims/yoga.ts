// yoga-layout for the browser build. The package's main module loads its wasm with a top-level await; in the static
// build (Rolldown) that await turns every chunk that reaches Ink into an async module, and the stories' chunks import
// each other in a cycle, so a story's module never finished evaluating and the preview stayed on its spinner (the dev
// server, which does not bundle, was fine). This stands in for `yoga-layout` without a top-level await: it loads
// Yoga through `yoga-layout/load` and forwards to it once loaded. Ink only touches Yoga when it lays a frame out,
// and every story waits for `yogaReady` first (preview.tsx loaders, InkTerminal); until then the enum constants
// answer (Ink reads only `Yoga.Node` and constants such as `Yoga.EDGE_TOP`).
import { loadYoga } from "yoga-layout/load";
import YGEnums from "radar-yoga-enums";

type YogaApi = Awaited<ReturnType<typeof loadYoga>>;
let real: YogaApi | undefined;

export const yogaReady: Promise<void> = loadYoga().then((y) => {
  real = y;
});

const Yoga = new Proxy({} as YogaApi, {
  get(_target, key) {
    const from = (real ?? YGEnums) as unknown as Record<PropertyKey, unknown>;
    return from[key];
  },
});

export default Yoga;
export * from "yoga-layout/load";
