// ink-testing-library fixes the terminal at 100 columns; the work area is checked at its real sizes, so this
// renders a <WorkArea capabilities={{columns, rows}}> on a stdout stub of that size and gives the same
// { lastFrame, unmount } (frames with the escape sequences taken out).
import React from "react";
import { render as inkRender } from "ink";
import chalk from "chalk";
import { Out, plain } from "../contract/support.js";

chalk.level = 1; // colours on, so a test can see which frame is lit

class In {
  isTTY = false;
  setEncoding() {}
  setRawMode() {}
  resume() {}
  pause() {}
  ref() {}
  unref() {}
  on() { return this; }
  off() { return this; }
  removeListener() { return this; }
  addListener() { return this; }
  read() { return null; }
}

/**
 * The panels drawn lit: each `╭ title` is looked up with the colour it is drawn in; the colour most panels share
 * is the resting one, and a panel in any other colour is lit (the focused border). Works whatever the theme's colour.
 */
export function litTitles(raw: string): Set<string> {
  const SGR = /\x1b\[[0-9;]*m/g;
  const found: Array<{ title: string; colour: string }> = [];
  for (let i = raw.indexOf("╭"); i >= 0; i = raw.indexOf("╭", i + 1)) {
    const after = raw.slice(i + 1, i + 200).replace(SGR, "");
    const title = / (.*?) ─/.exec(after)?.[1] ?? "";
    // the colour in force at the corner: the last escape before it that is not a reset
    const before = raw.slice(0, i).match(SGR) ?? [];
    const on = [...before].reverse().find((e) => e !== "\x1b[39m" && e !== "\x1b[49m" && e !== "\x1b[22m") ?? "";
    found.push({ title, colour: on + (raw.slice(i + 1, i + 40).match(SGR)?.[0] ?? "") });
  }
  const count = new Map<string, number>();
  for (const f of found) count.set(f.colour, (count.get(f.colour) ?? 0) + 1);
  const resting = [...count.entries()].sort((x, y) => y[1] - x[1])[0]?.[0];
  return new Set(found.filter((f) => f.colour !== resting).map((f) => f.title));
}

export function render(el: React.ReactElement) {
  const caps = (el.props as { capabilities: { columns: number; rows: number } }).capabilities;
  const out = new Out(caps.columns, caps.rows);
  const app = inkRender(el, { stdout: out as never, stdin: new In() as never, stderr: new Out(caps.columns, caps.rows) as never, debug: true, exitOnCtrlC: false, patchConsole: false });
  return { raw: () => out.last(), lastFrame: () => plain(out.last()).replace(/\n$/, ""), unmount: () => app.unmount() };
}

/** The node renderer the app hands the work area: the real one, with a work.* section drawn from its fallback document. */
import { renderNode as nodes } from "../../src/nodes/index.js";
import type { ViewNode } from "../../src/workarea/model.js";
export const realNodes = (node: ViewNode, c: { columns: number; focused: boolean }) =>
  nodes((node.node === "extension" && String(node.capability).startsWith("work.") && node.fallback ? node.fallback : node) as never, { width: c.columns, focused: c.focused } as never);
