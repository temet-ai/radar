// The goldens Ink owns: ink/test/cells (styled scenes), ink/test/fixtures/motion, ink/test/fixtures/wrap and
// ink/test/input/intents.json. They were first written from the reference; they are frozen and committed, and nothing in
// Rust writes them any more. A test that compares Ink with one moves it by rewriting it from Ink's own output:
//
//   INK_UPDATE_GOLDENS=1 npx vitest run test/<dir>
//
// then read `git diff` of the files it touched before committing. Without the variable the files are only read.
import fs from "node:fs";
import type { TCell } from "../nodes/ansi.js";
import { currentLook } from "../../src/text/look.js";

export const updatingGoldens = (): boolean => process.env.INK_UPDATE_GOLDENS === "1";

/** A cell of a scene file: every default left out (no `ch` is a space, no `fg`/`bg` the canvas, no `mods` none). */
export interface SceneCell {
  ch?: string;
  fg?: string;
  bg?: string;
  mods?: string[];
}

/** The parts of a scene file the update reads and rewrites; the rest of the file is kept as it is. */
export interface SceneFile {
  canvas: { fg: string; bg: string };
  colours: Record<string, { hex: string; ink?: string[] }>;
  rows: SceneCell[][];
  width?: number;
  height?: number;
  [key: string]: unknown;
}

const MODS: ReadonlyArray<[keyof TCell, string]> = [
  ["bold", "bold"],
  ["dim", "dim"],
  ["italic", "italic"],
  ["underline", "underline"],
  ["inverse", "reversed"],
  ["strike", "crossed_out"],
];

/** The slot names of a colour in the look in use, joined by `/` as a scene names them; the hex itself when no slot has it. */
function slotNames(look: Readonly<Record<string, string>>): Map<string, string> {
  const byHex = new Map<string, string[]>();
  for (const [slot, hex] of Object.entries(look)) {
    if (!hex) continue;
    const key = hex.toLowerCase();
    byHex.set(key, [...(byHex.get(key) ?? []), slot]);
  }
  return new Map([...byHex].map(([hex, slots]) => [hex, slots.join("/")]));
}

/** Ink's frame as the rows of a scene: `height` rows of `width` cells, colours as slot names where the look has one. */
export function sceneRows(scene: SceneFile, frame: TCell[][]): { rows: SceneCell[][]; colours: SceneFile["colours"] } {
  const names = slotNames(currentLook().slots);
  const colours = { ...scene.colours };
  const nameOf = (colour: string | undefined): string | undefined => {
    if (!colour) return undefined;
    const hex = colour.toLowerCase();
    const name = names.get(hex) ?? hex;
    if (name !== hex) {
      colours[name] ??= { hex, ink: [] };
      colours[name.split("/")[0]!] ??= { hex, ink: [] };
    }
    return name;
  };
  const width = scene.width ?? Math.max(...frame.map((r) => r.length));
  const height = scene.height ?? frame.length;
  const rows: SceneCell[][] = [];
  for (let y = 0; y < height; y++) {
    const row: SceneCell[] = [];
    for (let x = 0; x < width; x++) {
      const c = frame[y]?.[x];
      const out: SceneCell = {};
      if (c) {
        if (c.ch !== " ") out.ch = c.ch;
        const fg = nameOf(c.fg);
        const bg = nameOf(c.bg);
        if (fg && fg !== scene.canvas.fg) out.fg = fg;
        if (bg && bg !== scene.canvas.bg) out.bg = bg;
        const mods = MODS.filter(([key]) => c[key]).map(([, name]) => name);
        if (mods.length) out.mods = mods;
      }
      row.push(out);
    }
    rows.push(row);
  }
  return { rows, colours };
}

/** A scene file as it is laid out on disk: one line for each top-level key and for each row of cells. */
export function sceneText(scene: SceneFile): string {
  const lines = Object.entries(scene).map(([key, value]) =>
    key === "rows"
      ? `"rows": [\n${(value as SceneCell[][]).map((row) => JSON.stringify(row)).join(",\n")}\n]`
      : `${JSON.stringify(key)}: ${JSON.stringify(value)}`,
  );
  return `{\n${lines.join(",\n")}\n}\n`;
}

/** Rewrite a scene's rows from Ink's frame: in the object the test holds, and in the file. Update mode only. */
export function adoptInkFrame(file: string, scene: SceneFile, frame: TCell[][]): void {
  const { rows, colours } = sceneRows(scene, frame);
  scene.rows = rows;
  scene.colours = colours;
  fs.writeFileSync(file, sceneText(scene));
}

/** JSON with sorted keys, ending in a newline: two-space indent (the wrap and intents goldens) or, with 0, one line
 * (the motion goldens). */
export function sortedJson(value: unknown, indent = 2): string {
  const sort = (v: unknown): unknown =>
    Array.isArray(v)
      ? v.map(sort)
      : v && typeof v === "object"
        ? Object.fromEntries(Object.entries(v as Record<string, unknown>).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)).map(([k, x]) => [k, sort(x)]))
        : v;
  return JSON.stringify(sort(value), null, indent || undefined) + "\n";
}
