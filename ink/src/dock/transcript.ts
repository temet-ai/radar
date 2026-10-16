import type { DockItem, DockView } from "./model.js";

/** The transcript Ink holds: the items as last taken in, at the host's revision. */
export type HeldTranscript = { revision: number; items: DockItem[] };

/**
 * Take in a dock as the host sent it (crates/radar-core/src/ipc/bridge.rs, "the dock's transcript"): whole, or, with
 * `kept`, the items kept from the transcript held at `kept.revision` followed by the changed tail it carries. The dock
 * is given its whole transcript back (everything after this reads `transcript` as before), and the items kept are
 * the same objects as before, so what was laid out for them is reused (dock/rows.ts). `ok` is false when the dock
 * builds on a revision Ink does not hold: the host is asked to send it whole (`resync`), and until then it draws the
 * tail it has.
 */
export function takeTranscript(held: HeldTranscript | undefined, dock: DockView): { held: HeldTranscript; ok: boolean } {
  const tail = dock.transcript ?? [];
  const kept = dock.kept;
  delete dock.kept;
  let items = tail;
  let ok = true;
  if (kept) {
    if (held && held.revision === kept.revision && kept.items <= held.items.length) items = kept.items === 0 ? tail : held.items.slice(0, kept.items).concat(tail);
    else ok = false;
  }
  dock.transcript = items;
  return { held: { revision: dock.revision ?? 0, items }, ok };
}
