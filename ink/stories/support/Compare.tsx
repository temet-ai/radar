import React from "react";
import { InkTerminal } from "./InkTerminal";
import { CellGrid, type Canvas, type Cell } from "./CellGrid";
import { canvasOf, useLookOf, type Pack, type Tier } from "./look";
import { sceneCanvas, sceneCells, type RatatuiScene } from "./ratatui";
import { paneRect, snapBody, textCells } from "./scene";

/** A region of a screen, in cells. */
export interface Crop {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Ratatui's picture of the same view data: its cells, where they came from, and the part of the screen the
 * component takes. */
export interface Reference {
  cells: Cell[][];
  canvas: Canvas;
  source: string;
  /** Text recordings carry no colour. */
  styled: boolean;
  crop?: Crop;
  note?: string;
}

/** A styled scene from ink/test/cells (tests/ratatui_cells.rs). */
export function fromScene(scene: RatatuiScene, opts: { pane?: string; paneIndex?: number; crop?: Crop; note?: string } = {}): Reference {
  const cells = sceneCells(scene);
  const crop = opts.crop ?? (opts.pane ? paneRect(cells, opts.pane, opts.paneIndex) : undefined);
  return { cells, canvas: sceneCanvas(scene), source: `ink/test/cells/${scene.scene}-${scene.width}x${scene.height}.json`, styled: true, crop, note: opts.note };
}

/** A text recording: an insta snapshot (`snap: true`) or a plain screen text. */
export function fromText(text: string, source: string, opts: { snap?: boolean; pane?: string; paneIndex?: number; crop?: Crop; note?: string } = {}): Reference {
  const cells = textCells(opts.snap ? snapBody(text) : text);
  const crop = opts.crop ?? (opts.pane ? paneRect(cells, opts.pane, opts.paneIndex) : undefined);
  return { cells, canvas: canvasOf("truecolor"), source, styled: false, crop, note: opts.note };
}

/** The size the component is drawn at so it lines up with the reference: the crop, else the whole screen. */
export function sizeOf(ref: Reference | undefined, fallback: { w: number; h: number }): { w: number; h: number } {
  if (!ref) return fallback;
  if (ref.crop) return { w: ref.crop.w, h: ref.crop.h };
  return { w: Math.max(...ref.cells.map((r) => r.length)), h: ref.cells.length };
}

function RatatuiReference({ reference }: { reference: Reference }) {
  const c = reference.crop ?? { x: 0, y: 0, ...sizeOf(reference, { w: 80, h: 24 }) };
  const cells = reference.cells.slice(c.y, c.y + c.h).map((row) => row.slice(c.x, c.x + c.w));
  return (
    <figure>
      <figcaption>
        Ratatui (reference): {reference.source}
        {reference.crop ? `, cells ${c.x},${c.y} ${c.w}x${c.h}` : ""}
        {reference.styled ? ", truecolour, icons off" : ", text only (no colours)"}
        {reference.note ? `. ${reference.note}` : ""}
      </figcaption>
      <CellGrid cells={cells} cols={c.w} rows={c.h} canvas={reference.canvas} label={`Ratatui ${reference.source}`} />
    </figure>
  );
}

/**
 * A story's picture: the real Ink component drawn by Ink into an in-page terminal, and beside it Ratatui's picture
 * of the same view data (`reference`), or why there is none (`missing`).
 */
export function Compare({
  ink,
  cols,
  rows,
  globals,
  reference,
  missing,
  paint,
}: {
  ink: React.ReactElement;
  cols: number;
  rows: number;
  globals: { tier?: Tier; pack?: Pack };
  reference?: Reference;
  missing?: string;
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
      {reference ? (
        <RatatuiReference reference={reference} />
      ) : (
        <p className="radar-missing">No Ratatui reference. {missing ?? "None is recorded for this view data."}</p>
      )}
    </div>
  );
}
