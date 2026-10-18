import { vi } from "vitest";
vi.hoisted(() => { process.env.FORCE_COLOR = "3"; });
import { afterEach, expect, it } from "vitest";
import { mount, until } from "./harness.js";
import { parseAnsi } from "../nodes/ansi.js";
import { HitRegistry } from "../../src/state/ui-state.js";
import { focusedScroller } from "../../src/nodes/index.js";
import { booksChrome } from "../chrome/fixture.js";
import { applyLook, defaultLook } from "../../src/text/look.js";

let stop: (() => void) | undefined;
afterEach(() => { stop?.(); stop = undefined; applyLook(defaultLook()); vi.restoreAllMocks(); });
const rowText = (cells: Array<{ ch?: string }> = []) => cells.map((c) => c.ch ?? " ").join("");

it("scroll and resize preserve the pinned row, stop the border track at its rule, and replace offscreen inline hits", async () => {
  applyLook(defaultLook("off"));
  const registry = new HitRegistry();
  const m = mount(80, 16, { registry }); stop = m.stop;
  m.welcome();
  const screen = { scope: "watch", panes: [{ pane: "detail", title: "report", body: { node: "extension", capability: "detail", version: 1, data: { header: { path: [{ text: "ankh-morpork/watch" }], title: [{ text: "WATCH-12" }] }, sections: [{ block: "section", title: "REPORT", blocks: Array.from({ length: 40 }, (_, i) => ({ block: "line", text: [{ text: `Evidence line ${i}`, target: `evidence:${i}` }] })) }] }, fallback: { node: "document", lines: [[{ text: "report fallback" }]] } }, foot_actions: { actions: [{ action: { id: "copy", label: "copy", key: { key: { char: "c" } }, mnemonic_at: 0 }, target: "detail-foot:1:0" }], menu: { action: { id: "menu", label: "extra actions", key: { key: { char: "x" } }, mnemonic_at: 1 }, target: "detail-foot:1:menu" } } }] };
  const chrome = { ...booksChrome({ width: 80, height: 16 }), sources: [], pane_jumps: [{ pane: "detail", key: "3" }], write_mode: "read_only" };
  m.snapshot({ chrome, screen, focus: "detail" }, 1);
  await until(() => !!focusedScroller()?.max() && registry.all().some((h) => h.target === "evidence:0"));
  const before = parseAnsi(m.frame());
  const controls = registry.all().filter((h) => h.target.startsWith("detail-foot:"));
  expect(controls).toHaveLength(2);
  const footY = controls[0]!.y;
  expect(before[footY - 1]![79]!.ch).toBe("│");
  expect(before[footY]![79]!.ch).toBe("│");
  expect(["█", "║"]).toContain(before[footY - 2]![79]!.ch);
  expect(registry.selectAreas().find((a) => a.id === "pane:detail:body")?.y! + registry.selectAreas().find((a) => a.id === "pane:detail:body")?.h!).toBe(footY - 1);
  const pinned = rowText(before[footY]);
  // A body-start drag and keyboard extension clamp before the pinned rule/control rows.
  const area = registry.selectAreas().find((a) => a.id === "pane:detail:body")!;
  m.type(`\x1b[<0;4;${area.y + 1}M\x1b[<32;70;${footY + 1}M\x1b[<0;70;${footY + 1}m`);
  await until(() => m.intents().some((i: any) => i.intent.intent === "copy"));
  m.type("\x1b[1;2B\x1b[1;2B\x03");
  const copies = m.intents().filter((i: any) => i.intent.intent === "copy").map((i: any) => i.intent.text);
  expect(copies.length).toBeGreaterThan(0);
  expect(copies.every((text: string) => !text.includes("extra actions") && !text.includes("copy"))).toBe(true);
  m.type("\x1b");
  focusedScroller()!.key("end");
  await until(() => registry.all().some((h) => h.target === "evidence:39") && registry.all().some((h) => h.target.startsWith("detail-foot:")));
  expect(registry.all().some((h) => h.target === "evidence:0")).toBe(false);
  expect(rowText(parseAnsi(m.frame())[footY])).toBe(pinned);
  expect(registry.all().filter((h) => h.target.startsWith("detail-foot:"))).toEqual(controls);
  expect(registry.all().some((h) => h.target === "evidence:39")).toBe(true);
  expect(registry.all().filter((h) => h.target.startsWith("evidence:")).every((h) => h.y + h.h <= footY - 1)).toBe(true);
  m.out.resize(20, 8);
  await until(() => registry.all().some((h) => h.target === "detail-foot:1:menu" && h.x + h.w <= 19 && h.y < 8));
  const narrow = registry.all().filter((h) => h.target.startsWith("detail-foot:"));
  expect(narrow).toHaveLength(1);
  expect(narrow[0]!.target).toBe("detail-foot:1:menu");
  expect(registry.all().every((h) => h.x >= 0 && h.x + h.w <= 20 && h.y >= 0 && h.y + h.h <= 8)).toBe(true);
  m.out.resize(20, 4);
  await until(() => !registry.all().some((h) => h.target.startsWith("detail-foot:")));
  expect(registry.all().some((h) => h.target.startsWith("evidence:"))).toBe(false);
});


it.each(["plain", "unknown-version", "malformed"])("%s detail fallback keeps complete identity and the final hidden-action reason reachable at End", async (mode) => {
  applyLook(defaultLook("off"));
  const m = mount(40, 12); stop = m.stop;
  if (mode === "plain") vi.spyOn(m.session, "plainText", "get").mockReturnValue(true);
  m.welcome();
  const fallback = { node: "rich", blocks: [
    { block: "line", text: [{ text: "Ankh-Morpork City Watch unabridged identity tail-marker" }] },
    ...Array.from({ length: 24 }, (_, i) => ({ block: "line", text: [{ text: `Full field value ${i}` }] })),
    { block: "line", text: [{ text: "approve (blocked until review)" }], key: "a", mnemonic_at: 0 },
    { block: "line", text: [{ text: "final-action unavailable-last-row" }], key: "z" },
  ] };
  const body = { node: "extension", capability: "detail", version: mode === "unknown-version" ? 9 : 1, data: mode === "malformed" ? { header: {}, facts: null } : { header: { path: [{ text: "Ankh-Morpork City Watch unabridged identity tail-marker" }], title: [] } }, fallback };
  m.snapshot({ chrome: { ...booksChrome({ width: 40, height: 12 }), sources: [] }, screen: { scope: "watch", panes: [{ pane: "detail", title: "report", body }] }, focus: "detail" }, 1);
  await until(() => !!focusedScroller()?.max());
  const first = rowText(parseAnsi(m.frame()).flat());
  expect(first).toContain("tail-marker");
  expect(first).not.toContain("…");
  focusedScroller()!.key("end");
  await until(() => m.frame().includes("unavailable-last-row"));
  const last = rowText(parseAnsi(m.frame()).flat());
  expect(last).toContain("unavailable-last-row");
  expect(last).toContain("[z]");
  expect(last).toContain("blocked until review");
  expect(last).not.toContain("shown simply");
});


it("same-caption message replacement resets detail scroll by selected target while refresh preserves it", async () => {
  const m = mount(80, 16); stop = m.stop;
  m.welcome();
  const detail = { pane: "detail", title: "message", body: { node: "extension", capability: "detail", version: 1,
    data: { header: { path: [{ text: "watch" }], title: [{ text: "same first line" }] }, sections: Array.from({ length: 30 }, (_, i) => ({ block: "line", text: [{ text: `reply ${i}` }] })) },
    fallback: { node: "document", lines: [] } } };
  const scene = (target: string) => ({ scope: "slack.watch", panes: [{ pane: "middle", title: "messages", body: { node: "list", selected: 0, rows: [{ target, text: [{ text: "same first line" }] }] } }, detail] });
  m.snapshot({ screen: scene("message:1"), focus: "detail" }, 1);
  await until(() => !!focusedScroller()?.max());
  focusedScroller()!.key("end");
  await until(() => (focusedScroller()?.at() ?? 0) > 0);
  const at = focusedScroller()!.at();
  m.snapshot({ screen: scene("message:1"), focus: "detail" }, 2);
  await until(() => m.frame().includes("reply 29"));
  expect(focusedScroller()!.at()).toBe(at);
  m.snapshot({ screen: scene("message:2"), focus: "detail" }, 3);
  await until(() => focusedScroller()?.at() === 0 && m.frame().includes("same first line"));
  expect(focusedScroller()!.at()).toBe(0);
  expect(m.frame()).toContain("same first line");
});
