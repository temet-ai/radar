// The agent dock under a long transcript: a streamed word and a key typed into
// the composer, with 500 items in the transcript against 20, at 160x40, as the host's bridge sends them (the transcript
// whole once, then the items after its unchanged head). Costs are this thread's CPU time per frame (it holds on a busy
// machine better than wall time). RADAR_PERF=1 prints the numbers; RADAR_PERF_WHOLE=1 sends the transcript whole every time,
// as fresh objects, as the host did before (it fails: that is the cost this test keeps out).
import { afterEach, describe, expect, it } from "vitest";
import { fixture, mount, threadCpu, until } from "./harness.js";

const W = 160;
const H = 40;
/** One frame at 60 Hz: a key's echo and a streamed word each fit in it, with the rest of the App. */
const FRAME_CPU_BUDGET_MS = 16;
/** What 480 more items may add to a frame's CPU: noise, not a transcript laid out again (that was 15 to 25 ms). */
const MORE_ITEMS_MS = 6;
const WORDS = "the change moves the transcript out of the hot path so a streamed word costs the same whatever the session holds and the renderer keeps what it already wrapped".split(" ");

let stops: Array<() => void> = [];
afterEach(() => {
  stops.forEach((f) => f());
  stops = [];
});

function transcript(n: number): any[] {
  return Array.from({ length: n }, (_, i) =>
    i % 3 === 0
      ? { item: "user", id: `user-${i}`, text: `question ${i}: ${WORDS.join(" ")}` }
      : i % 3 === 1
        ? { item: "assistant", id: `m${i}`, text: `answer ${i}: ${WORDS.join(" ")}\n${WORDS.join(" ")}`, saved: true }
        : { item: "tool", id: `call${i}`, title: `ls -la src/${i}`, kind: "shell", state: "done", open: false, result: ["a", "b"] },
  );
}

/** The dock as the host's bridge sends it: whole at revision 1, then (`kept`) the items after the unchanged head. */
const dock = (items: any[], revision = 1, kept?: { revision: number; items: number }) => ({
  visible: true,
  focused: true,
  tabs: [{ id: "s1", icon: "bot", title: "one", state: "streaming" }],
  active: "s1",
  transcript: items,
  revision,
  ...(kept ? { kept } : {}),
  composer: { draft: "", can_send: true, can_stop: true, commands: [], history: [] },
  status: { provider: "fake" },
});

const cpuMs = (c: NodeJS.CpuUsage) => (c.user + c.system) / 1000;
const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)]!;

/** A dock of `n` items mounted at 160x40: the median CPU of a streamed word and of a key typed into the composer. */
async function costs(n: number, whole: boolean) {
  const chrome = fixture("host_chrome_160x40.json");
  const base = fixture("gitlab_repos_160x40.json");
  const m = mount(W, H, true);
  stops.push(m.stop);
  const items = transcript(n);
  let text = "";
  const live = () => ({ item: "assistant", id: "live", text, saved: false });
  await m.frameAfter({ chrome, screen: { ...base, dock: dock([...items, live()]) } });
  const words: number[] = [];
  for (let w = 0; w < 7; w++) {
    text += `${WORDS[w % WORDS.length]} `;
    // The word changed the last item: the host keeps the items before it and sends that one.
    const sent = whole ? dock(structuredClone([...items, live()])) : dock([live()], w + 2, { revision: w + 1, items: items.length });
    words.push((await m.frameAfter({ chrome, screen: { ...base, dock: sent } })).cpu);
  }
  // What Ink drew is the whole transcript with the words in it.
  expect(m.out.writes.map((x) => x.text).join("")).toContain(WORDS[6]);
  const keys: number[] = [];
  for (const k of process.env.RADAR_PERF_KEYS ?? "hello w") {
    const from = m.out.writes.length;
    const cpu0 = threadCpu();
    m.type(k);
    await until(() => m.out.writes.length > from, 1000);
    keys.push(cpuMs(threadCpu(cpu0)));
  }
  if (process.env.RADAR_PERF)
    process.stderr.write(
      `dock, ${n} items${whole ? ", sent whole" : ""}: streamed word ${median(words).toFixed(1)} ms CPU (${words.map((x) => x.toFixed(1)).join(" ")}), key ${median(keys).toFixed(1)} ms CPU (${keys.map((x) => x.toFixed(1)).join(" ")})\n`,
    );
  return { word: median(words), key: median(keys) };
}

describe("the dock with 500 items in its transcript", () => {
  // Under vitest React runs its development build, whose own cost is most of a frame here, so the frame's budget is
  // held only in production (NODE_ENV=production, as the bundle runs); what 480 more items add is held in both.
  it("draws a streamed word and a typed key as fast as with 20 items, within a frame's CPU", async () => {
    const whole = !!process.env.RADAR_PERF_WHOLE;
    const short = await costs(20, whole);
    stops.forEach((f) => f());
    stops = [];
    const long = await costs(500, whole);
    expect(long.word - short.word).toBeLessThan(MORE_ITEMS_MS);
    expect(long.key - short.key).toBeLessThan(MORE_ITEMS_MS);
    if (process.env.NODE_ENV === "production") {
      expect(long.word).toBeLessThan(FRAME_CPU_BUDGET_MS);
      expect(long.key).toBeLessThan(FRAME_CPU_BUDGET_MS);
    }
  }, 30_000);
});
