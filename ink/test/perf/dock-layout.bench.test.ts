// What a transcript arriving whole costs the first time it is laid out (a resumed session, another tab, a resize):
// every item wrapped once. `RADAR_PERF=1 NODE_ENV=production npx vitest run test/perf/dock-layout.bench.test.ts`;
// Measured: 132, 84, 35 ms before wrapCells kept running widths; 25, 12, 9 after.
import { it } from "vitest";
import { transcriptView } from "../../src/dock/rows.js";

const P =
  "The change moves the transcript out of the snapshot's hot path, so a streamed word costs the same whether the session holds ten messages or five hundred; the renderer keeps what it already wrapped and draws only the rows that show, and the host compares versions instead of writing the whole view out to find what changed.";

it.skipIf(!process.env.RADAR_PERF)("first layout of 320 answers", () => {
  for (let round = 0; round < 3; round++) {
    const items = Array.from({ length: 320 }, (_, i) => ({ item: "assistant" as const, id: `m${i}-${round}`, text: `Answer ${i}: ${P}`, saved: true }));
    const t0 = performance.now();
    transcriptView(items, 84, 12);
    process.stderr.write(`320 answers laid out at 84 wide: ${(performance.now() - t0).toFixed(1)} ms\n`);
  }
});
