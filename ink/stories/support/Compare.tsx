import React from "react";
import { InkTerminal } from "./InkTerminal";
import type { Cell } from "./CellGrid";
import { canvasOf, useLookOf, type Pack, type Tier } from "./look";
import { sceneCells, type GoldenScene } from "./golden";
import { paneRect, snapBody, textCells } from "./scene";

/** A region of a screen, in cells. */
export interface Crop {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** A recorded screen's cells and the part of it a component takes. It is not drawn: it says how large the story's
 * terminal is, so the component is drawn at the size it has on the recorded screen. */
export interface Region {
  cells: Cell[][];
  crop?: Crop;
}

/** The region of a golden scene (ink/test/cells): the pane titled `pane`, or `crop`, or the whole screen. */
export function regionOf(scene: GoldenScene, opts: { pane?: string; paneIndex?: number; crop?: Crop } = {}): Region {
  const cells = sceneCells(scene);
  return { cells, crop: opts.crop ?? (opts.pane ? paneRect(cells, opts.pane, opts.paneIndex) : undefined) };
}

/** The region of a text recording: an insta snapshot (`snap: true`) or a plain screen text. */
export function regionOfText(text: string, opts: { snap?: boolean; pane?: string; paneIndex?: number; crop?: Crop } = {}): Region {
  const cells = textCells(opts.snap ? snapBody(text) : text);
  return { cells, crop: opts.crop ?? (opts.pane ? paneRect(cells, opts.pane, opts.paneIndex) : undefined) };
}

/** The size the component is drawn at: the region's crop, else its whole screen, else the fallback. */
export function sizeOf(region: Region | undefined, fallback: { w: number; h: number }): { w: number; h: number } {
  if (!region) return fallback;
  if (region.crop) return { w: region.crop.w, h: region.crop.h };
  return { w: Math.max(...region.cells.map((r) => r.length)), h: region.cells.length };
}

/** A story's picture: the real Ink component drawn by Ink into an in-page terminal. */
export function Compare({
  ink,
  cols,
  rows,
  globals,
  paint,
}: {
  ink: React.ReactElement;
  cols: number;
  rows: number;
  globals: { tier?: Tier; pack?: Pack };
  /** What the renderer paints over Ink's finished frame (InkTerminal `paint`). */
  paint?: (frame: string) => string;
}) {
  const tier = globals.tier ?? "truecolor";
  const pack = globals.pack ?? "off";
  const version = useLookOf(tier, pack);
  return (
    <div className="radar-compare">
      <figure>
        <figcaption>
          Ink: {cols}x{rows}, {tier === "truecolor" ? "truecolour" : tier}, icons {pack}
        </figcaption>
        <InkTerminal cols={cols} rows={rows} canvas={canvasOf(tier)} version={version} label="Ink" paint={paint}>
          {ink}
        </InkTerminal>
      </figure>
    </div>
  );
}
