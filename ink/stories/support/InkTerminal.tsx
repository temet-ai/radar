import React, { useEffect, useState } from "react";
import { render } from "ink";
import { EventEmitter } from "../../.storybook/shims/events";
import { parseAnsi } from "../../test/nodes/ansi";
import { CellGrid, type Canvas, type Cell } from "./CellGrid";
import { yogaReady } from "../../.storybook/shims/yoga";

/** A stdout for Ink with a size: every frame Ink writes is kept (Ink in debug mode writes the whole frame each time,
 * as test/nodes/helpers.tsx reads it). */
class Screen extends EventEmitter {
  isTTY = false;
  last = "";
  constructor(
    public columns: number,
    public rows: number,
    private onFrame: (frame: string) => void,
  ) {
    super();
  }
  write = (s: string) => {
    if (s.trim().length === 0) return true;
    this.last = s;
    this.onFrame(s);
    return true;
  };
}

class NoInput extends EventEmitter {
  isTTY = false;
  setRawMode() {}
  setEncoding() {}
  resume() {}
  pause() {}
  ref() {}
  unref() {}
  read() {
    return null;
  }
}

/** Ink's frame as cells: the ANSI Ink wrote (through radar's fast Output, app/fast-output.ts) parsed back into the
 * grid a terminal would show. */
export function frameCells(frame: string): Cell[][] {
  return parseAnsi(frame.replace(/\n$/, ""));
}

/**
 * Draws a real Ink element into an in-page terminal of `cols` x `rows` cells. The element is rendered by Ink itself
 * (its reconciler, Yoga layout and radar's Output), and the ANSI it writes is drawn cell by cell.
 */
export function InkTerminal({
  children,
  cols,
  rows,
  canvas,
  label = "Ink",
  version,
  paint,
}: {
  children: React.ReactElement;
  cols: number;
  rows: number;
  canvas: Canvas;
  label?: string;
  /** Changes when the look changes, so the element is drawn again in it. */
  version?: string;
  /** What the renderer paints over a finished frame that is not the component's own (the mouse selection,
   * app/selected.ts): given Ink's frame, the frame to show. */
  paint?: (frame: string) => string;
}) {
  const [frame, setFrame] = useState("");
  useEffect(() => {
    let app: { unmount(): void } | undefined;
    let gone = false;
    // Yoga's wasm loads once, asynchronously (.storybook/shims/yoga.ts); Ink lays out only after it has.
    yogaReady.then(() => {
      if (gone) return;
      const screen = new Screen(cols, rows, setFrame);
      app = render(children, {
        stdout: screen as unknown as NodeJS.WriteStream,
        stdin: new NoInput() as unknown as NodeJS.ReadStream,
        stderr: new Screen(cols, rows, () => {}) as unknown as NodeJS.WriteStream,
        debug: true,
        patchConsole: false,
        exitOnCtrlC: false,
      });
    });
    return () => {
      gone = true;
      app?.unmount();
    };
  }, [children, cols, rows, version]);
  return <CellGrid cells={frameCells(paint ? paint(frame.replace(/\n$/, "")) : frame)} cols={cols} rows={rows} canvas={canvas} label={label} />;
}
