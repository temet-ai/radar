// Production notice rows/history with a controlled clock for inspecting drain and expiry.
import React, { useEffect, useState } from "react";
import { NoticeHistoryOverlay } from "../../src/notices/History";
import { layoutNotices, layoutNoticeHistory } from "../../src/notices/layout";
import { noticeCells } from "../../src/notices/text";
import { bgOf, slotStyle } from "../../src/text/look";
import type { NoticeView } from "../../src/protocol/generated/NoticeView";
import type { Cell } from "../../src/text/cells";
import { FRAME_MS } from "../../src/motion/engine";
import { Today } from "./parts";
import type { Pack, Tier } from "../support/look";
type Globals = { tier?: Tier; pack?: Pack };
const notices: NoticeView[] = [
  { id: 1, tone: "info", text: "City Watch report saved", created_ms: 0, lifetime_ms: 5000 },
  { id: 2, tone: "info", text: "Watch patrol updated · 界 👩‍💻", created_ms: 0, lifetime_ms: 5000 },
  { id: 3, tone: "error", text: "Watch report unavailable. Retry from its original page; this history is read-only.", created_ms: 0, lifetime_ms: 10000 },
];
function Stack({ now = 1500, motion = true }: { now?: number; motion?: boolean }) {
  const width = 80, height = 6, base = { ...slotStyle("text.primary"), backgroundColor: bgOf("bg.base") };
  const lines: Cell[][] = Array.from({ length: height }, () => noticeCells(" ".repeat(width), base));
  for (const row of layoutNotices(notices, 2, { x: 1, y: 1, w: width - 2, h: height - 2 }, now, motion)) {
    lines[row.rect.y] = [...noticeCells(" ".repeat(row.rect.x), base), ...row.cells, ...noticeCells(" ".repeat(width - row.rect.x - row.rect.w), base)];
  }
  return <NoticeHistoryOverlay layout={{ lines, hits: [] }} />;
}
function History() {
  return <NoticeHistoryOverlay layout={layoutNoticeHistory({ entries: notices, detail: notices[2], omitted: 2, selected: 0, scroll: 0, detail_scroll: 0 }, 80, 16)} />;
}
export function NoticeSurfaces({ globals }: { globals: Globals }) {
  return <div style={{ display: "flex", flexDirection: "column", gap: 20 }}>
    <Today globals={globals} cols={80} rows={6} caption="Production notice rows · newest at the foot, older dimmed, overflow opens ~ history"><Stack /></Today>
    <Today globals={globals} cols={80} rows={16} caption="Session-only notice history · inert summaries and the full selected detail"><History /></Today>
  </div>;
}
/** This demo clock runs only during an explicit replay; production uses Motion's existing30ms scheduler. */
export function NoticeDrainReplay({ globals, speed }: { globals: Globals; speed: "real" | "slow" }) {
  const [now, setNow] = useState(1500), [run, setRun] = useState(0), [off, setOff] = useState(false);
  useEffect(() => {
    if (!run) return;
    const start = performance.now(); let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = () => { const at = Math.min(10000, (performance.now() - start) / (speed === "slow" ? 4 : 1)); setNow(at); if (at < 10000) timer = setTimeout(tick, FRAME_MS); };
    tick(); return () => { if (timer !== undefined) clearTimeout(timer); };
  }, [run, speed]);
  return <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
    <Today globals={globals} cols={80} rows={6} caption="Production notice drain · info expires at 5 s, error at 10 s; motion off keeps a static ground"><Stack now={now} motion={!off} /></Today>
    <div style={{ display: "flex", gap: 16, alignItems: "center" }}>
      <button onClick={() => setRun((n) => n + 1)}>Replay lifetime</button>
      <label>Clock <input aria-label="Notice elapsed milliseconds" type="range" min={0} max={10000} value={now} onChange={(e) => { setRun(0); setNow(Number(e.currentTarget.value)); }} /> {Math.round(now)} ms</label>
      <label><input type="checkbox" checked={off} onChange={(e) => setOff(e.currentTarget.checked)} /> Motion off</label>
    </div>
  </div>;
}
