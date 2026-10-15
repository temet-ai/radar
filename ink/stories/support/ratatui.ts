// Ratatui's painted cells (ink/test/cells/<scene>-<w>x<h>.json, written by tests/ratatui_cells.rs) as grid cells.
// A cell names theme slots (`accent.primary/glow.purple` when slots share a colour); the file's `colours` says each
// name's hex, and a missing fg or bg is the canvas (text.primary on bg.base).
import type { Cell } from "./CellGrid";
import { SLOTS } from "../../src/text/generated/pack";

export interface RatatuiScene {
  format: number;
  scene: string;
  width: number;
  height: number;
  canvas: { fg: string; bg: string };
  colours: Record<string, { hex: string }>;
  rows: Array<Array<{ ch?: string; fg?: string; bg?: string; mods?: string[] }>>;
  view?: { screen?: unknown; menu?: unknown; pointer?: unknown };
}

/** A slot name's colour in the scene (the first of joined names; the colours table is keyed by the joined name). */
function hexOf(scene: RatatuiScene, name: string | undefined): string | undefined {
  if (!name) return undefined;
  return scene.colours[name]?.hex ?? scene.colours[name.split("/")[0]!]?.hex ?? SLOTS[name.split("/")[0]!];
}

export function sceneCells(scene: RatatuiScene): Cell[][] {
  return scene.rows.map((row) =>
    row.map((c) => {
      const mods = new Set(c.mods ?? []);
      return {
        ch: c.ch === undefined ? " " : c.ch,
        fg: hexOf(scene, c.fg),
        bg: hexOf(scene, c.bg),
        ...(mods.has("bold") ? { bold: true } : {}),
        ...(mods.has("dim") ? { dim: true } : {}),
        ...(mods.has("italic") ? { italic: true } : {}),
        ...(mods.has("underline") ? { underline: true } : {}),
        ...(mods.has("reversed") ? { inverse: true } : {}),
        ...(mods.has("crossed_out") ? { strike: true } : {}),
      };
    }),
  );
}

/** The canvas colours; a slot the scene never painted with is missing from its table and comes from the host's
 * true-colour defaults (the tier the scenes are drawn at). */
export function sceneCanvas(scene: RatatuiScene): { fg: string; bg: string } {
  return {
    fg: hexOf(scene, scene.canvas.fg) ?? SLOTS[scene.canvas.fg]!,
    bg: hexOf(scene, scene.canvas.bg) ?? SLOTS[scene.canvas.bg]!,
  };
}
