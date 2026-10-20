// The golden scenes (ink/test/cells/<scene>-<w>x<h>.json, see ink/test/cells/README.md) as grid cells. Stories take the
// view data a scene holds and the size a component has on its screen. A cell names theme slots
// (`accent.primary/glow.purple` when slots share a colour); the file's `colours` says each name's hex, and a missing
// fg or bg is the canvas (text.primary on bg.base).
import type { Cell } from "./CellGrid";
import { SLOTS } from "../../src/text/generated/pack";

export interface GoldenScene {
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
function hexOf(scene: GoldenScene, name: string | undefined): string | undefined {
  if (!name) return undefined;
  return scene.colours[name]?.hex ?? scene.colours[name.split("/")[0]!]?.hex ?? SLOTS[name.split("/")[0]!];
}

export function sceneCells(scene: GoldenScene): Cell[][] {
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
export function sceneCanvas(scene: GoldenScene): { fg: string; bg: string } {
  return {
    fg: hexOf(scene, scene.canvas.fg) ?? SLOTS[scene.canvas.fg]!,
    bg: hexOf(scene, scene.canvas.bg) ?? SLOTS[scene.canvas.bg]!,
  };
}
