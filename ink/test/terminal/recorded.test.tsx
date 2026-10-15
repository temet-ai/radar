import React from "react";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { render } from "ink-testing-library";
import { TerminalScreen } from "../../src/terminal/screen.js";
import { TerminalView } from "../../src/terminal/TerminalView.js";
import type { TerminalLink } from "../../src/terminal/link.js";
import type { Frame } from "../../src/terminal/frames.js";

// Recordings of `less -R` over a coloured file, made by the host's own PTY (crates/radar-sessions-pty/examples/record.rs)
// at the two sizes the screens are checked at. They hold the bytes as the host replays them to a renderer.
interface Recording {
  command: string[];
  columns: number;
  rows: number;
  state: { state: "running" | "exited" | "failed"; generation: number; columns: number; rows: number };
  base64: string;
}
const load = (name: string): Recording => JSON.parse(readFileSync(new URL(`./recorded/${name}.json`, import.meta.url), "utf8"));
const bytesOf = (r: Recording) => new Uint8Array(Buffer.from(r.base64, "base64"));

/** A link that replays the recording as the host does on attach: the state, then the ring. */
const replayLink = (r: Recording): TerminalLink => ({
  onFrame(fn: (f: Frame) => void) {
    fn({ kind: "state", state: r.state });
    fn({ kind: "output", bytes: bytesOf(r) });
  },
  onClose() {},
  input() {},
  resize() {},
  restart() {},
  kill() {},
  close() {},
});

for (const name of ["less-80x24", "less-160x40"]) {
  describe(`recorded ${name}`, () => {
    const rec = load(name);

    it("parses to a screen of the recorded size whose rows are all that wide", async () => {
      const s = new TerminalScreen(rec.columns, rec.rows);
      await s.write(bytesOf(rec));
      const text = s.text();
      expect(text).toHaveLength(rec.rows);
      expect(text[0]).toContain("step 01");
      expect(text.at(-1)).toMatch(/\(END\)|:|sample/); // less's status row
      expect(s.modes.altScreen).toBe(true);
      expect(text.join("\n")).toMatchSnapshot();
      // The program's colours survive the parse: "warn" is bold yellow, "err" red.
      const first = s.runs()[0]!;
      expect(first[0]).toMatchObject({ text: "warn", color: "yellow", bold: true });
      expect(s.runs()[2]![0]).toMatchObject({ text: "err", color: "red" });
      for (const row of s.runs()) expect(row.reduce((n, r) => n + [...r.text].reduce((m, c) => m + (/[一-鿿]/.test(c) ? 2 : 1), 0), 0)).toBe(rec.columns);
    });

    it("draws through TerminalView in the same cells", async () => {
      const r = render(<TerminalView open={() => replayLink(rec)} openKey={name} columns={rec.columns} rows={rec.rows} focused={false} />);
      const end = Date.now() + 3000;
      while (Date.now() < end && !(r.lastFrame() ?? "").includes("step 01")) await new Promise((x) => setTimeout(x, 10));
      const lines = (r.lastFrame() ?? "").split("\n");
      expect(lines).toHaveLength(rec.rows);
      expect(lines[0]).toContain("step 01");
      r.unmount();
    });
  });
}
