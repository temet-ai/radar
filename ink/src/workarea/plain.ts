import { plainBlocks } from "../nodes/plain.js";
import type { Block } from "../nodes/view.js";
import { displayChord } from "../nodes/actions.js";
import type { ChordName } from "../nodes/view.js";
import type { ViewNode, WorkSection } from "./model.js";

function textOf(t: unknown): string {
  // view::Text serialises as a list of segments {text,...}; accept a string too.
  if (typeof t === "string") return t;
  if (!Array.isArray(t)) return "";
  return t.map((s) => {
    if (typeof s === "string") return s;
    const seg = s as { text?: string; key_hint?: string };
    return (seg?.text ?? "") + (seg?.key_hint ? ` [${seg.key_hint}]` : "");
  }).join("");
}

/** The rail's actions as `key  label` lines, so a screen reader keeps the controls and their keys. */
function railActions(data: unknown): string[] | null {
  const list = (data as { actions?: { label?: string; key?: ChordName }[] } | null)?.actions;
  if (!Array.isArray(list)) return null;
  return list.map((a) => {
    const key = a.key ? displayChord(a.key) : "";
    return `${key ? `[${key}] ` : ""}${a.label ?? ""}`;
  });
}

/** A node's content as plain lines (no layout). Unknown nodes say so. */
export function plainLines(node: ViewNode): string[] {
  switch (node.node) {
    case "state":
      return [`${String(node.state)}: ${String(node.text ?? "")}`];
    case "rich":
      return plainBlocks((node.blocks ?? []) as Block[]);
    case "document":
      return ((node.lines as unknown[]) ?? []).map((l) => textOf(l).replace(/^[▸▶›❯] /, "▎ ")); // GUIDE 2.12: the cursor mark, not a chevron
    case "list": {
      const sel = node.selected as number | undefined;
      return ((node.rows as { text?: unknown; label?: unknown }[]) ?? []).map(
        (r, i) => `${i === sel ? "▎ " : "  "}${textOf(r.text ?? r.label)}`,
      );
    }
    case "entries":
    case "table": {
      const rows = (node.rows as { text?: unknown; cells?: unknown[]; title?: unknown }[]) ?? [];
      const out = rows.map((r) =>
        r.cells ? r.cells.map(textOf).join(" | ") : textOf(r.text ?? r.title),
      );
      return out.length ? out : [String(node.empty ?? "")].filter(Boolean);
    }
    case "extension": {
      const actions = node.capability === "work.rail" ? railActions(node.data) : null;
      if (actions?.length) return actions;
      const fb = node.fallback as ViewNode | undefined;
      return fb ? plainLines(fb) : [`(${String(node.capability)} not shown)`];
    }
    default:
      return [`(${node.node})`];
  }
}

/** The work area as labelled lines in reading order: pending first, then the view's order. */
export function plainText(sections: WorkSection[], focus?: string): string[] {
  const ordered = [...sections].sort((a, b) =>
    a.pending === b.pending ? 0 : a.pending ? -1 : 1,
  );
  const out: string[] = [];
  for (const s of ordered) {
    const flags = [s.pending ? "PENDING" : "", s.status ?? "", s.target === focus ? "focused" : ""]
      .filter(Boolean)
      .join(", ");
    out.push(`[${s.kind}] ${s.title}${flags ? ` (${flags})` : ""}`);
    const line = (t: unknown[]) => (t.length ? out.push(`  ${textOf(t)}`) : 0);
    line(s.head);
    // The selected map item is a style on screen; in plain text it is the cursor mark, as lists have.
    const items = s.kind === "map" ? ((s.body.data as { items?: { selected?: boolean }[] } | undefined)?.items ?? []) : [];
    plainLines(s.body).forEach((l, i) => out.push(items[i - 1]?.selected ? `▎ ${l}` : `  ${l}`));
    s.notes.forEach(line);
    line(s.footer);
  }
  return out;
}
