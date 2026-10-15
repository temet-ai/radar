import React from "react";
import { SLOTS } from "../../src/text/generated/pack";
import { cssColour } from "./colour";
import { lookFor, lookSource, tiers, type Tier } from "./look";

/** Every theme slot the host's look carries: its swatch at each colour tier the host was captured at, and the
 * colour word the host sends (`#rrggbb`, `ansi256(n)`, a terminal colour's name, or nothing at mono). */
export function Swatches() {
  const shown: Tier[] = tiers();
  const looks = shown.map((t) => lookFor(t, "off"));
  const groups = new Map<string, string[]>();
  for (const slot of Object.keys(SLOTS)) {
    const g = slot.split(".")[0]!;
    groups.set(g, [...(groups.get(g) ?? []), slot]);
  }
  const cell: React.CSSProperties = { padding: "4px 8px", borderBottom: "1px solid #8884", fontFamily: "var(--radar-font)", fontSize: 12, verticalAlign: "middle" };
  return (
    <div style={{ overflowX: "auto" }}>
      <table style={{ borderCollapse: "collapse" }}>
        <thead>
          <tr>
            <th style={{ ...cell, textAlign: "left" }}>slot</th>
            {shown.map((t) => (
              <th key={t} style={{ ...cell, textAlign: "left" }} title={lookSource(t)}>
                {t === "truecolor" ? "truecolour" : t}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {[...groups.entries()].map(([g, slots]) => (
            <React.Fragment key={g}>
              {slots.map((slot) => (
                <tr key={slot}>
                  <td style={cell}>{slot}</td>
                  {looks.map((look, i) => {
                    const word = look.slots[slot] ?? "";
                    const css = cssColour(word);
                    return (
                      <td key={i} style={cell}>
                        <span style={{ display: "inline-block", width: 28, height: 16, verticalAlign: "middle", marginRight: 6, background: css ?? "transparent", border: "1px solid #8888" }} />
                        {word || "terminal's own"}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </React.Fragment>
          ))}
        </tbody>
      </table>
    </div>
  );
}
