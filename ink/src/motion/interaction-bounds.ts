// Match native motion/frames.rs before retaining metadata or constructing serialized identity keys.
import type { MotionContext } from "../protocol/generated/MotionContext.js";
import type { MotionScene, SceneMarks } from "./interaction.js";
import { perimeterLength } from "../text/frame.js";
import { bounded } from "./interaction-effects.js";

/** Count UTF-8 without allocating an encoded copy; stop as soon as the caller's budget is exceeded. */
function bytes(value: string, limit: number): number {
  let size = 0;
  for (let i = 0; i < value.length; i++) {
    const point = value.codePointAt(i)!;
    size += point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
    if (point >= 0x10000) i++;
    if (size > limit) break;
  }
  return size;
}
function stringsBounded(values: readonly (string | null | undefined)[], limit: number): boolean {
  let size = 0;
  for (const value of values) {
    if (value == null) continue;
    if (typeof value !== "string") return false;
    size += bytes(value, limit - size);
    if (size > limit) return false;
  }
  return true;
}
export function contextBounded(context: MotionContext | null | undefined): boolean {
  if (!context) return false;
  switch (context.kind) {
    case "screen": return Number.isSafeInteger(context.depth) && context.depth >= 0 && context.depth <= 64 && stringsBounded([context.plugin, context.screen, context.page], 16_384);
    case "settings": return stringsBounded([context.plugin, context.section], 16_384);
    case "history": return true;
    case "setup": return stringsBounded([context.step, context.plugin, context.section], 16_384);
    default: return false;
  }
}
/** Serialize only known, validated context fields, independent of object-property insertion order. */
export function contextKey(context: MotionContext | null): string | null {
  if (!contextBounded(context) || !context) return null;
  switch (context.kind) {
    case "screen": return JSON.stringify([context.kind, context.plugin, context.screen, context.page, context.depth, context.tab]);
    case "settings": return JSON.stringify([context.kind, context.plugin, context.section]);
    case "history": return JSON.stringify([context.kind]);
    case "setup": return JSON.stringify([context.kind, context.step, context.plugin, context.section]);
  }
}
export function pathBytes(path: readonly string[]): number | undefined {
  if (path.length > 64) return undefined;
  let size = 0;
  for (const component of path) {
    if (typeof component !== "string") return undefined;
    size += bytes(component, 4096 - size);
    if (size > 4096) return undefined;
  }
  return size;
}
export function sceneMetadataBounded(scene: MotionScene): boolean {
  if (!contextBounded(scene.context)) return false;
  const semantic = scene.interaction;
  if (semantic?.context && !contextBounded(semantic.context)) return false;
  const fold = semantic?.fold;
  return !fold || contextBounded(fold.context) && pathBytes(fold.path) !== undefined && stringsBounded([fold.pane], 1024) && stringsBounded([fold.slot], 1024);
}
export function frameMetadataBounded(frame: SceneMarks): boolean {
  if (!sceneMetadataBounded(frame.scene) || frame.panes.length > 32 || frame.trees.length > 16) return false;
  const panes = new Set<string>(), trees = new Map<string, Set<string>>();
  let perimeter = 0, metadata = 0;
  for (const pane of frame.panes) {
    if (!stringsBounded([pane.pane], 1024) || pane.keep.length > 64 || !bounded(pane.area) || panes.has(pane.pane)) return false;
    panes.add(pane.pane);
    perimeter += perimeterLength(pane.area.w, pane.area.h);
    if (perimeter > 65_536) return false;
  }
  for (const tree of frame.trees) {
    if (!stringsBounded([tree.pane], 1024) || !stringsBounded([tree.slot], 1024) || !bounded(tree.area) || tree.rows.length > tree.area.h || tree.open.length > tree.rows.length) return false;
    let slots = trees.get(tree.pane);
    if (slots?.has(tree.slot)) return false;
    if (!slots) trees.set(tree.pane, slots = new Set());
    slots.add(tree.slot);
    for (const rows of [tree.rows.map((row) => row.path), tree.open]) for (const path of rows) {
      const size = pathBytes(path);
      if (size === undefined) return false;
      metadata += size;
      if (metadata > 65_536) return false;
    }
  }
  return true;
}
