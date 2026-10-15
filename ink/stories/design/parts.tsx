// The pieces every Design story is made of: a label that says what a picture is (shipped today, or a target that is
// not built yet and the issue that builds it), today's real Ink component, and the canvas's target beside it.
import React from "react";
import { InkTerminal } from "../support/InkTerminal";
import { canvasOf, useLookOf, type Pack, type Tier } from "../support/look";
import { Board, type BoardName, type Pick } from "./Board";

const label: React.CSSProperties = {
  display: "inline-block",
  fontFamily: "var(--radar-font)",
  fontSize: 12,
  fontWeight: 700,
  letterSpacing: ".08em",
  textTransform: "uppercase",
  padding: "4px 10px",
  margin: "0 0 10px",
  alignSelf: "flex-start",
};

/** "Target · not built yet · issue #N": the canvas's proposal, so nobody takes it for the shipped look. */
export function TargetLabel({ issues, note }: { issues: number[]; note?: string }) {
  return (
    <div>
      <span style={{ ...label, color: "#0a0812", background: "#f1fa8c" }}>
        Target · not built yet · {issues.map((n) => `issue #${n}`).join(", ")}
      </span>
      {note ? <span style={{ marginLeft: 10, fontSize: 12, color: "#888" }}>{note}</span> : null}
    </div>
  );
}

/** "Shipped": what radar draws today (the canvas's own drawing of it, or the real component). */
export function ShippedLabel({ what }: { what: string }) {
  return <span style={{ ...label, color: "#82aaff", border: "1px solid #33396a" }}>Shipped · {what}</span>;
}

/** Today's real Ink component, drawn by Ink into cells in the look the toolbar chose. */
export function Today({ globals, cols, rows, children, caption }: { globals: { tier?: Tier; pack?: Pack }; cols: number; rows: number; children: React.ReactElement; caption: string }) {
  const tier = globals.tier ?? "truecolor";
  const version = useLookOf(tier, globals.pack ?? "off");
  return (
    <figure style={{ margin: 0 }}>
      <ShippedLabel what="today's Ink component" />
      <figcaption style={{ fontSize: 12, margin: "0 0 6px", color: "#888" }}>{caption}</figcaption>
      <InkTerminal cols={cols} rows={rows} canvas={canvasOf(tier)} version={version} label={caption}>
        {children}
      </InkTerminal>
    </figure>
  );
}

/** The canvas's target for the same element. */
export function Target({ board, picks, issues, note, props, width }: { board: BoardName; picks: Pick[]; issues: number[]; note?: string; props?: Record<string, unknown>; width?: number }) {
  return (
    <figure style={{ margin: 0 }}>
      <TargetLabel issues={issues} note={note} />
      <Board name={board} picks={picks} props={props} width={width} />
    </figure>
  );
}

/** A shipped part of a board, drawn by the canvas (its picture of today's look). */
export function Shipped({ board, picks, what, props, width }: { board: BoardName; picks: Pick[]; what: string; props?: Record<string, unknown>; width?: number }) {
  return (
    <figure style={{ margin: 0 }}>
      <ShippedLabel what={what} />
      <Board name={board} picks={picks} props={props} width={width} />
    </figure>
  );
}

/** Side by side, wrapping when narrow. */
export function Row({ children }: { children: React.ReactNode }) {
  return <div style={{ display: "flex", flexWrap: "wrap", gap: 24, alignItems: "flex-start" }}>{children}</div>;
}
