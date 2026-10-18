// Shared by the contract tests. Self-contained on purpose: the tests are also run against the Ink source
// as it was before each fix (git archive <fix>^ ink), so they import only from ../../src and from here.
import React from "react";
import { EventEmitter } from "node:events";
import { readFileSync } from "node:fs";
import { render } from "ink";
import { App } from "../../src/app/App.js";
import { Session, capabilitiesFrom } from "../../src/app/session.js";
import { FakeConnection } from "../../src/transport/fake.js";
import { MouseBus, tapMouse } from "../../src/input/stdin-tap.js";

/** A stdout stub with a size (ink-testing-library fixes columns at 100). */
export class Out extends EventEmitter {
  frames: string[] = [];
  isTTY = true;
  constructor(
    public columns: number,
    public rows: number,
  ) {
    super();
  }
  write = (s: string) => {
    this.frames.push(s);
    return true;
  };
  last() {
    return this.frames.at(-1) ?? "";
  }
}

class In extends EventEmitter {
  isTTY = true;
  data: string | null = null;
  setEncoding() {}
  setRawMode() {}
  resume() {}
  pause() {}
  ref() {}
  unref() {}
  read = () => {
    const d = this.data;
    this.data = null;
    return d;
  };
  type(s: string) {
    this.data = s;
    this.emit("readable");
  }
}

export const tick = (ms = 20) => new Promise((r) => setTimeout(r, ms));

/** Wait on state, not on the clock: poll until `ok()` holds. */
export async function until(ok: () => boolean, ms = 3000) {
  const end = Date.now() + ms;
  while (!ok() && Date.now() < end) await new Promise((r) => setTimeout(r, 5));
}

/** The text of a frame, escape sequences taken out. */
export const plain = (s: string) => s.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "");

export type Mounted = ReturnType<typeof mount>;

const live: Array<() => void> = [];
/** Unmount every app mounted since the last call (a test's afterEach). */
export function unmountAll() {
  for (const f of live.splice(0)) f();
}

/** The App over a fake connection at `columns` x `rows`, welcomed. */
export function mount(columns: number, rows: number) {
  const out = new Out(columns, rows);
  const stdin = new In();
  const conn = new FakeConnection();
  const mouse = new MouseBus();
  tapMouse(stdin, mouse);
  const session = new Session(conn, capabilitiesFrom({}, out));
  session.start();
  const instance = render(<App session={session} mouse={mouse} />, {
    stdout: out as any,
    stdin: stdin as any,
    stderr: new Out(columns, rows) as any,
    debug: true,
    exitOnCtrlC: false,
    patchConsole: false,
  });
  live.push(() => instance.unmount());
  conn.receive({ type: "welcome", protocol: 1, view: 1, cursor: 0 });
  let cursor = 0;
  const m = {
    out,
    conn,
    session,
    /** A snapshot of `chrome` and what shows under it (the host's newest state). */
    snapshot(over: { chrome?: any; screen?: any; section?: any; focus?: string } = {}) {
      cursor += 1;
      conn.receive({
        type: "snapshot",
        cursor,
        chrome: over.chrome ?? chromeOf(columns, rows),
        screen: over.screen ?? null,
        section: over.section ?? null,
        focus: over.focus ?? "middle",
      });
    },
    intents: () => (conn.sent as any[]).filter((x) => x.type === "intent").map((x) => x.intent),
    type: (s: string) => stdin.type(s),
    stop: () => instance.unmount(),
    frame: () => plain(out.last()),
    lines: () => plain(out.last()).split("\n"),
    /** Let the frame settle after a change. */
    settle: async () => {
      // until no new frame has been written for 40 ms (a loaded machine renders late)
      let n = -1;
      let still = 0;
      for (let i = 0; i < 200 && still < 4; i++) {
        await tick(10);
        if (out.frames.length === n) still++;
        else still = 0;
        n = out.frames.length;
      }
    },

  };
  return m;
}

/** The chrome of the Books screen, as `radar shot` shows it, with `over` applied. */
export function chromeOf(width: number, height: number, over: Record<string, unknown> = {}): any {
  return {
    version: 1,
    width,
    height,
    top_bar: {
      brand: "radar",
      tabs: [
        { label: "Books", mnemonic: "B", active: true, target: "chrome:tab:0" },
        { label: "Work", mnemonic: "W", target: "chrome:tab:1" },
        { label: "history", mnemonic: "h", target: "chrome:history" },
        { label: "settings", mnemonic: "s", target: "chrome:settings" },
      ],
      mode: { text: "read-only", ink: "warn", target: "chrome:mode" },
    },
    status: {
      scope: "BOOKS",
      hints: [
        { keys: "q", label: "quit", keys_of: [{ label: "q", chord: { key: { char: "q" } }, target: "chrome:key:0:0" }] },
        { keys: "x", label: "menu", keys_of: [{ label: "x", chord: { key: { char: "x" } }, action: "chrome.menu", target: "chrome:key:1:0" }] },
      ],
    },
    keys_at: "body",
    ...over,
  };
}

/** A screen whose list shows `count` rows of text that is easy to find: `ZQXJ<i> ZQXJZQXJ...`. */
export function listScreen(count = 14, width = 150): any {
  const rows = Array.from({ length: count }, (_, i) => ({
    target: `row:${i}`,
    text: [{ text: `ZQXJ${i} ${"ZQXJ".repeat(Math.floor(width / 4))}`.slice(0, width) }],
  }));
  return { id: "list", scope: "list", panes: [{ pane: "middle", title: "list", body: { node: "list", selected: 0, rows } }], actions: [] };
}

/** A list of short rows: `!1`, `!2`, ... with a target each. */
export function shortList(n = 4): any {
  const rows = Array.from({ length: n }, (_, i) => ({ target: `row:!${i + 1}`, text: [{ text: `!${i + 1}` }] }));
  return { id: "mrs", scope: "mrs", panes: [{ pane: "middle", title: "mrs", body: { node: "list", selected: 0, rows } }], actions: [] };
}

/** The generated protocol types, read as text: the contract tests enumerate fields and kinds from them. */
const GENERATED = new URL("../../src/protocol/generated/", import.meta.url);
export const generatedSource = (name: string): string => readFileSync(new URL(`${name}.ts`, GENERATED), "utf8");

const stripComments = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/.*$/gm, "");

/** The top-level field names of `export type <name> = { ... }` (one object type), from the generated file. */
export function fieldsOf(name: string): string[] {
  const src = stripComments(generatedSource(name));
  const at = src.indexOf("= {");
  if (at < 0) return [];
  let depth = 0;
  let cur = "";
  const out: string[] = [];
  const flush = () => {
    const m = /^\s*([A-Za-z_][A-Za-z0-9_]*)\??\s*:/.exec(cur);
    if (m) out.push(m[1]!);
    cur = "";
  };
  depth = 1;
  for (let i = at + 3; i < src.length; i++) {
    const c = src[i]!;
    if (c === "{" || c === "[" || c === "(" || c === "<") depth++;
    else if (c === "}" || c === "]" || c === ")" || c === ">") {
      depth--;
      if (depth === 0) {
        flush();
        break;
      }
    }
    if (c === "," && depth === 1) flush();
    else cur += c;
  }
  return out;
}

/** The members of a string-literal union type (`"a" | "b"`), from the generated file. */
export function unionOf(name: string): string[] {
  const src = stripComments(generatedSource(name));
  return [...src.matchAll(/"([A-Za-z_:0-9]+)"/g)].map((m) => m[1]!);
}

/** The `node` kinds of the generated Node union. */
export function nodeKinds(): string[] {
  return [...stripComments(generatedSource("Node")).matchAll(/"node":\s*"([a-z_]+)"/g)].map((m) => m[1]!);
}

// ---- the whole protocol type graph, walked from the generated files' own imports ------------------------------

const GENERATED_DIR = new URL("../../src/protocol/generated/", import.meta.url);

/** Split `s` at `sep` where nesting depth is 0 (braces, brackets, parens, angle brackets). */
function splitTop(s: string, sep: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let cur = "";
  let q = false;
  for (const c of s) {
    if (c === '"') q = !q;
    if (!q) {
      if ("{[(<".includes(c)) depth++;
      else if ("}])>".includes(c)) depth--;
    }
    if (!q && depth === 0 && c === sep) {
      out.push(cur);
      cur = "";
    } else cur += c;
  }
  out.push(cur);
  return out.map((x) => x.trim()).filter((x) => x !== "");
}

/** Field names of an object type body `{ a: T, "b"?: U }`; quoted-name fields that are string literals are discriminators. */
function objectFields(body: string): { fields: string[]; tag: string | null } {
  const inner = body.trim().replace(/^\{/, "").replace(/\}$/, "");
  const fields: string[] = [];
  let tag: string | null = null;
  for (const part of splitTop(inner, ",")) {
    const lit = /^"([A-Za-z_]+)"\s*:\s*"([A-Za-z_0-9]+)"$/.exec(part);
    if (lit) {
      tag ??= lit[2]!;
      continue;
    }
    const m = /^"?([A-Za-z_][A-Za-z0-9_]*)"?\??\s*:/.exec(part);
    if (m) fields.push(m[1]!);
  }
  return { fields, tag };
}

/**
 * The object shapes a generated type declares: `[label, fields]` for each. A plain struct is `Name`; a tagged
 * union member is `Name.<tag>`; `{ "link": { url } }` style members are `Name.link`. Enums of strings have none.
 */
export function shapesOf(name: string): Array<[string, string[]]> {
  const src = stripComments(generatedSource(name));
  const at = src.indexOf(`export type ${name} =`);
  const body = src.slice(at + `export type ${name} =`.length).trim().replace(/;$/, "");
  const alts = splitTop(body, "|");
  const out: Array<[string, string[]]> = [];
  alts.forEach((alt, i) => {
    const parts = splitTop(alt, "&");
    const obj = parts.find((p) => p.startsWith("{"));
    if (!obj) return;
    const { fields, tag } = objectFields(obj);
    const wrapped = /^\{\s*"([A-Za-z_]+)"\s*:\s*(\{[\s\S]*\})\s*\}$/.exec(obj);
    if (wrapped) {
      out.push([`${name}.${wrapped[1]}`, objectFields(wrapped[2]!).fields]);
      return;
    }
    const label = alts.length > 1 ? `${name}.${tag ?? i}` : name;
    out.push([label, fields]);
  });
  return out;
}

/** Every generated type reachable from `roots` by the `import type` lines of the generated files. */
export function reachable(roots: string[]): string[] {
  const seen = new Set<string>();
  const queue = [...roots];
  while (queue.length) {
    const n = queue.pop()!;
    if (seen.has(n)) continue;
    seen.add(n);
    for (const m of generatedSource(n).matchAll(/import type \{ (\w+) \} from "\.\/\1\.js"/g)) queue.push(m[1]!);
  }
  return [...seen].sort();
}

/** `Label.field` for every field of every object shape reachable from what the host sends the renderer for a view. */
export function viewFields(roots = ["ChromeView", "ScreenView", "SectionView", "Question", "InputRequest", "Notice"]): string[] {
  return reachable(roots).flatMap((t) => shapesOf(t).flatMap(([label, fields]) => fields.map((f) => `${label}.${f}`)));
}
