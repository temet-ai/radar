import React from "react";
import { EventEmitter } from "node:events";
import { render } from "ink";

// ink-testing-library's stdout is fixed at 100 columns; the 80x24 and 160x40 cases need their own, so this renders
// with Ink directly into a stub stream that has `columns`/`rows` and keeps the last frame.
class Out extends EventEmitter {
  frames: string[] = [];
  constructor(public columns: number, public rows: number) {
    super();
  }
  write = (s: string) => {
    this.frames.push(s);
    return true;
  };
  get isTTY() {
    return false;
  }
}

class In extends EventEmitter {
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

/** Render `element` at `columns` x `rows` and return the text of the last frame with the ANSI codes stripped. */
export function frameAt(element: React.ReactElement, columns: number, rows = 24): string {
  const stdout = new Out(columns, rows);
  const app = render(element, {
    stdout: stdout as unknown as NodeJS.WriteStream,
    stdin: new In() as unknown as NodeJS.ReadStream,
    debug: true,
    patchConsole: false,
    exitOnCtrlC: false,
  });
  app.unmount();
  const last = stdout.frames.filter((f) => f.trim().length > 0).at(-1) ?? "";
  // eslint-disable-next-line no-control-regex
  return last.replace(/\u001b\[[0-9;?]*[A-Za-z]/g, "").replace(/\s+$/gm, "");
}

/** The same with styles kept, for tests that look at colour. */
export function styledFrameAt(element: React.ReactElement, columns: number, rows = 24): string {
  const stdout = new Out(columns, rows);
  const app = render(element, {
    stdout: stdout as unknown as NodeJS.WriteStream,
    stdin: new In() as unknown as NodeJS.ReadStream,
    debug: true,
    patchConsole: false,
    exitOnCtrlC: false,
  });
  app.unmount();
  return stdout.frames.filter((f) => f.trim().length > 0).at(-1) ?? "";
}
