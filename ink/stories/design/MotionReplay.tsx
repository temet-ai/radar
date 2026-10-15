// radar's shipped motion, run for real: Ink's own Engine (src/motion/engine.ts, the effects in effects.ts) over a real
// shell frame, with the marks and the cue the host sent, exactly as test/motion/fixtures.test.ts replays them. The
// frames, marks and cues are the Rust side's motion fixtures (ink/test/fixtures/motion/<effect>-<level>-<moment>.json,
// written by tests/motion_cells.rs); each also holds Ratatui's cells at the effect's start, middle and end, shown
// beside the live run as the reference.
import React, { useEffect, useState } from "react";
import { Engine, type Cue, type InkColours, type Level, type Marks } from "../../src/motion/engine";
import { cueOf } from "../../src/motion/attach";
import { parseHex, hex } from "../../src/motion/color";
import { Grid } from "../../src/motion/grid";
import { charWidth } from "../../src/chrome/width";
import { SLOTS } from "../../src/text/generated/pack";
import { CellGrid, type Cell } from "../support/CellGrid";

type Run = [string, string, string, string];
export interface MotionFixture {
  effect: string;
  level: Level;
  moment: string;
  t_ms: number;
  duration_ms: number;
  cue: any;
  motion: { level: Level; slots: Record<keyof InkColours, string> };
  marks: { body: number[] | null; popup: number[] | null; message: number[] | null; rows: Record<string, number[]>; loading: number[][] };
  before: Run[][];
  after: Run[][];
  rows: Run[][];
  width: number;
  height: number;
}

const fixtures = import.meta.glob<MotionFixture>("../../test/fixtures/motion/*.json", { eager: true, import: "default" });
export const motionFixture = (name: string): MotionFixture => {
  const f = fixtures[`../../test/fixtures/motion/${name}.json`];
  if (!f) throw new Error(`no motion fixture ${name}`);
  return f;
};

const colour = (s: string): number => (s === "reset" ? -1 : s === "white" ? 0xc0c0c0 : parseHex(s));
const rect = (r: number[]) => ({ x: r[0]!, y: r[1]!, w: r[2]!, h: r[3]! });

/** A recorded frame (runs of text, fg, bg, modifiers per row) as the engine's grid; test/motion/fixtures.test.ts's gridOf. */
export function gridOf(rows: Run[][], w = 80, h = 24): Grid {
  const g = new Grid(w, h);
  rows.forEach((runs, y) => {
    let x = 0;
    for (const [text, fg, bg, mods] of runs) {
      const attr = [mods.includes("bold") ? "1" : "", mods.includes("italic") ? "3" : "", mods.includes("underline") ? "4" : "", mods.includes("reversed") ? "7" : ""].filter(Boolean).join(";");
      for (const ch of text) {
        const cw = Math.max(1, charWidth(ch.codePointAt(0)!));
        for (let k = 0; k < cw && x + k < w; k++) Object.assign(g.at(x + k, y), { ch: k === 0 ? ch : "", fg: colour(fg), bg: colour(bg), attr });
        x += cw;
      }
    }
  });
  return g;
}

export const marksOf = (f: MotionFixture): Marks => ({
  body: f.marks.body ? rect(f.marks.body) : undefined,
  popup: f.marks.popup ? rect(f.marks.popup) : undefined,
  message: f.marks.message ? rect(f.marks.message) : undefined,
  rows: Object.entries(f.marks.rows).map(([k, r]) => [k, rect(r)] as const),
  loading: f.marks.loading.map(rect),
});

/** The theme colours the effects blend with, from the slots the host named (truecolor defaults). */
const inkOf = (f: MotionFixture): InkColours => {
  const s = f.motion.slots;
  return { base: parseHex(SLOTS[s.base]), surface: parseHex(SLOTS[s.surface]), overlay: parseHex(SLOTS[s.overlay]), glow: parseHex(SLOTS[s.glow]), text: parseHex(SLOTS[s.text]) };
};

/** The engine's grid as page cells. */
export function cellsOf(g: Grid): Cell[][] {
  const rows: Cell[][] = [];
  for (let y = 0; y < g.height; y++) {
    const row: Cell[] = [];
    for (let x = 0; x < g.width; x++) {
      const c = g.at(x, y);
      const a = c.attr.split(";");
      row.push({
        ch: c.ch,
        fg: c.fg >= 0 ? hex(c.fg) : undefined,
        bg: c.bg >= 0 ? hex(c.bg) : undefined,
        ...(a.includes("1") ? { bold: true } : {}),
        ...(a.includes("3") ? { italic: true } : {}),
        ...(a.includes("4") ? { underline: true } : {}),
        ...(a.includes("7") ? { inverse: true } : {}),
      });
    }
    rows.push(row);
  }
  return rows;
}

const CANVAS = { fg: SLOTS["text.primary"]!, bg: SLOTS["bg.base"]! };

/**
 * The fixture's frame with its effect running on Ink's Engine, again and again: the cue, the effect frame by frame
 * (every 30 ms, as the renderer ticks), the plain frame, a pause, and the cue again. `slow` runs the clock at a quarter
 * of real time.
 */
export function MotionReplay({ name, speed, cue: cueOverride }: { name: string; speed: "real" | "slow"; cue?: Cue }) {
  const f = motionFixture(name);
  const k = speed === "slow" ? 4 : 1;
  const [cells, setCells] = useState<Cell[][]>(() => cellsOf(gridOf(f.after, f.width, f.height)));
  const [t, setT] = useState(0);
  useEffect(() => {
    let alive = true;
    let start = performance.now();
    const clock = () => (performance.now() - start) / k;
    const ink = inkOf(f);
    const marks = marksOf(f);
    let engine: Engine;
    let restartAt = 0;
    const begin = () => {
      start = performance.now();
      engine = new Engine(f.motion.level, clock);
      if (f.effect === "message_gone") engine.draw(gridOf(f.before, f.width, f.height), { message: rect(motionFixture("message-full-start").marks.message!) }, ink);
      const cue = cueOverride ?? (f.cue ? cueOf(f.cue) : undefined);
      if (cue) engine.cue(cue, ink);
      restartAt = 0;
    };
    begin();
    const tick = () => {
      if (!alive) return;
      const g = gridOf(f.after, f.width, f.height);
      engine.draw(g, marks, ink);
      setCells(cellsOf(g));
      setT(Math.round(clock()));
      if (!engine.running()) {
        if (!restartAt) restartAt = performance.now() + 900 * k;
        else if (performance.now() >= restartAt) begin();
      }
    };
    const id = setInterval(tick, 30);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [name, k, cueOverride]);
  return (
    <figure style={{ margin: 0 }}>
      <figcaption style={{ fontSize: 12, margin: "0 0 6px", color: "#888" }}>
        Ink's Engine, live: {f.effect} ({f.level}), {f.duration_ms} ms{speed === "slow" ? " at a quarter speed" : ""}, t = {t} ms
      </figcaption>
      <CellGrid cells={cells} cols={f.width} rows={f.height} canvas={CANVAS} label={`Ink motion ${f.effect}`} />
    </figure>
  );
}

/** Ratatui's cells for the same effect at its start, middle and end (the fixtures' `rows`). */
export function MotionReference({ base }: { base: string }) {
  return (
    <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
      {(["start", "mid", "end"] as const).map((m) => {
        const f = motionFixture(`${base}-${m}`);
        return (
          <figure key={m} style={{ margin: 0 }}>
            <figcaption style={{ fontSize: 12, margin: "0 0 6px", color: "#888" }}>
              Ratatui (reference), t = {f.t_ms} ms: ink/test/fixtures/motion/{base}-{m}.json
            </figcaption>
            <div style={{ zoom: 0.5 }}>
              <CellGrid cells={cellsOf(gridOf(f.rows, f.width, f.height))} cols={f.width} rows={f.height} canvas={CANVAS} label={`Ratatui ${base} ${m}`} />
            </div>
          </figure>
        );
      })}
    </div>
  );
}
