// The design canvas's boards (stories/design/boards/*.dc.html, saved from the canvas as they were agreed), drawn
// as the canvas draws them: each board's own markup, its own styles, and its own renderVals() (the grid builders
// that lay out its terminal pictures), expanded by a small stand-in for the canvas's template runtime (`sc-for`,
// `sc-if`, `{{ }}`). A board is drawn in a shadow root so its class names never meet Storybook's.
//
// The boards are design reference data. Nothing here is radar's renderer: every "target" a board shows is not built
// yet, and each story that shows one says so and names its issue.
import React, { useEffect, useRef } from "react";

const raw = import.meta.glob<string>("./boards/*.dc.html", { eager: true, query: "?raw", import: "default" });

export type BoardName = "Main" | "Colour" | "Type" | "Icons" | "Components" | "Data" | "Surfaces" | "Motion" | "Screen" | "Research";

/** The mono stack the stories use (terminal.css); the canvas's own GGMono font files are not in the saved boards. */
const MONO = `"0xProto Nerd Font Mono", "JetBrainsMono Nerd Font Mono", "JetBrainsMono NF", "JetBrains Mono", "Symbols Nerd Font Mono", ui-monospace, monospace`;

interface Parsed {
  css: string;
  body: Element;
  code: string;
  defaults: Record<string, unknown>;
}

const parsed = new Map<BoardName, Parsed>();

function parse(name: BoardName): Parsed {
  const hit = parsed.get(name);
  if (hit) return hit;
  const html = raw[`./boards/${name}.dc.html`];
  if (!html) throw new Error(`no board ${name}`);
  const doc = new DOMParser().parseFromString(html, "text/html");
  const css = [...doc.querySelectorAll("helmet style")]
    .map((s) => s.textContent ?? "")
    .join("\n")
    .replace(/@font-face\{[^}]*\}/g, "")
    .replace(/"GGMono",\s*monospace/g, MONO)
    .replace(/"GGMono"/g, MONO);
  const script = doc.querySelector("script[data-dc-script]");
  const props = JSON.parse(script?.getAttribute("data-props") ?? "{}") as Record<string, { default?: unknown }>;
  const defaults = Object.fromEntries(Object.entries(props).filter(([k]) => !k.startsWith("$")).map(([k, v]) => [k, v.default]));
  const body = doc.querySelector("x-dc")!;
  const out = { css, body, code: script?.textContent ?? "", defaults };
  parsed.set(name, out);
  return out;
}

/** The board's renderVals() with the given props (the canvas's DCLogic gives a component its props). */
function valuesOf(p: Parsed, props: Record<string, unknown>): Record<string, unknown> {
  if (!p.code.trim()) return {};
  class DCLogic {
    props: Record<string, unknown>;
    constructor(props: Record<string, unknown>) {
      this.props = props;
    }
  }
  const Component = new Function("DCLogic", `${p.code}\nreturn Component;`)(DCLogic);
  const c = new Component({ ...p.defaults, ...props });
  return typeof c.renderVals === "function" ? c.renderVals() : {};
}

function evaluate(expr: string, scope: Record<string, unknown>): unknown {
  try {
    return new Function(...Object.keys(scope), `return (${expr});`)(...Object.values(scope));
  } catch {
    return "";
  }
}

const interpolate = (s: string, scope: Record<string, unknown>) =>
  s.replace(/\{\{([\s\S]+?)\}\}/g, (_, e: string) => {
    const v = evaluate(e.trim(), scope);
    return v === undefined || v === null || v === false ? "" : String(v);
  });

/** Expand the canvas's template into live nodes of this document. */
function expand(node: Node, scope: Record<string, unknown>, into: Node) {
  if (node.nodeType === Node.TEXT_NODE) {
    into.appendChild(document.createTextNode(interpolate(node.textContent ?? "", scope)));
    return;
  }
  if (node.nodeType !== Node.ELEMENT_NODE) return;
  const el = node as Element;
  const tag = el.tagName.toLowerCase();
  if (tag === "helmet" || tag === "script") return;
  if (tag === "sc-for") {
    const list = evaluate(/^\{\{([\s\S]+)\}\}$/.exec(el.getAttribute("list") ?? "")?.[1] ?? "[]", scope);
    const as = el.getAttribute("as") ?? "item";
    (Array.isArray(list) ? list : []).forEach((item, i) => {
      for (const child of el.childNodes) expand(child, { ...scope, [as]: item, [`${as}Index`]: i }, into);
    });
    return;
  }
  if (tag === "sc-if") {
    const v = evaluate(/^\{\{([\s\S]+)\}\}$/.exec(el.getAttribute("value") ?? "")?.[1] ?? "false", scope);
    if (v) for (const child of el.childNodes) expand(child, scope, into);
    return;
  }
  const copy = document.importNode(el, false) as Element;
  for (const a of [...copy.attributes]) {
    if (a.name.startsWith("hint-")) copy.removeAttribute(a.name);
    else if (a.value.includes("{{")) copy.setAttribute(a.name, interpolate(a.value, scope));
  }
  for (const child of el.childNodes) expand(child, scope, copy);
  into.appendChild(copy);
}

/** The board's sheet, expanded. */
export function renderBoard(name: BoardName, props: Record<string, unknown> = {}): HTMLElement {
  const p = parse(name);
  const scope = valuesOf(p, props);
  const holder = document.createElement("div");
  for (const child of p.body.childNodes) expand(child, scope, holder);
  return (holder.querySelector(".sheet") as HTMLElement) ?? holder;
}

/** Picks parts of an expanded sheet. */
export type Pick = (sheet: HTMLElement) => Element[];

const text = (e: Element | null | undefined) => (e?.textContent ?? "").replace(/\s+/g, " ").trim();

/** The sheet's opening block (eyebrow, title, lede). */
export const intro: Pick = (sheet) => (sheet.firstElementChild ? [sheet.firstElementChild] : []);

/** The block a heading (`.h2`, else `.h3`) reading `heading` heads: the largest block around it that holds no
 * other heading of its kind (two sections side by side in one grid come apart). */
export const section =
  (heading: string): Pick =>
  (sheet) => {
    const h = [...sheet.querySelectorAll(".h2"), ...sheet.querySelectorAll(".h3")].find((e) => text(e) === heading);
    if (!h) return [];
    const kind = h.classList.contains("h2") ? ".h2" : ".h3";
    let block: Element = h;
    while (block.parentElement && block.parentElement !== sheet && block.parentElement.querySelectorAll(kind).length === 1) block = block.parentElement;
    return [block];
  };

/** The nth child (0-based) of the block found by `pick`. */
export const childOf =
  (pick: Pick, n: number): Pick =>
  (sheet) =>
    pick(sheet).flatMap((e) => (e.children[n] ? [e.children[n]!] : []));

/** The panel (`.panel`, else the heading's block) whose `.h3` (or `.lab`) begins with `prefix`. */
export const card =
  (prefix: string): Pick =>
  (sheet) =>
    [...sheet.querySelectorAll(".h3, .lab")]
      .filter((h) => text(h).startsWith(prefix))
      .slice(0, 1)
      .map((h) => h.closest(".panel") ?? h.parentElement?.parentElement ?? h);

/** Every top-level block of the sheet but the opening one and those whose heading is in `except`. */
export const sectionsBut =
  (except: string[]): Pick =>
  (sheet) =>
    [...sheet.children].slice(1).filter((c) => ![...c.querySelectorAll(".h2")].some((h) => except.includes(text(h))));

/** Every element under the sheet matching a CSS selector. */
export const select =
  (css: string): Pick =>
  (sheet) => [...sheet.querySelectorAll(css)];

/**
 * A board, or parts of it, drawn as the canvas draws it. `picks` choose the parts (in order); none draws the whole
 * board. `props` are the board's own props (Motion's `speed`).
 */
export function Board({ name, picks, props, width }: { name: BoardName; picks?: Pick[]; props?: Record<string, unknown>; width?: number }) {
  const host = useRef<HTMLDivElement>(null);
  const key = JSON.stringify(props ?? {});
  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const root = el.shadowRoot ?? el.attachShadow({ mode: "open" });
    const sheet = renderBoard(name, props);
    let shown: HTMLElement = sheet;
    if (picks && picks.length) {
      const parts = picks.flatMap((p) => p(sheet)).map((e) => e.cloneNode(true) as Element);
      shown = sheet.cloneNode(false) as HTMLElement;
      shown.classList.add("radar-part");
      for (const e of parts) shown.appendChild(e);
    }
    const style = document.createElement("style");
    style.textContent = `${parse(name).css}\n:host{display:block}\n.sheet.radar-part{width:${width ?? 1440}px;max-width:none;padding:32px 36px;gap:32px}`;
    root.replaceChildren(style, shown);
  }, [name, key, picks, width]);
  return <div ref={host} />;
}
