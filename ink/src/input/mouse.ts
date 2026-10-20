/**
 * The modes crossterm's EnableMouseCapture sets (the host runner's): clicks, drags, every move, urxvt and
 * SGR encodings. Reports arrive as SGR: `ESC [ < b ; x ; y M` press, `m` release, button + 32 a move.
 */
export const MOUSE_ON = "\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h";
export const MOUSE_OFF = "\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l";

export type MouseEvent = {
  kind: "press" | "release" | "wheel_up" | "wheel_down" | "motion";
  button: "left" | "middle" | "right" | "none";
  /** 0-based cell column and row. */
  x: number;
  y: number;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
};

const SGR = /\x1b\[<(\d+);(\d+);(\d+)([Mm])/g;

/** True when the whole chunk is SGR mouse reports (so keyboard handling can skip it). */
export function isMouseData(data: string): boolean {
  return data.length > 0 && data.replace(SGR, "") === "";
}

export function parseMouse(data: string): MouseEvent[] {
  const out: MouseEvent[] = [];
  for (const m of data.matchAll(SGR)) {
    const b = Number(m[1]);
    const x = Number(m[2]) - 1;
    const y = Number(m[3]) - 1;
    const final = m[4];
    const mods = { shift: (b & 4) !== 0, alt: (b & 8) !== 0, ctrl: (b & 16) !== 0 };
    if (b & 64) {
      out.push({ kind: (b & 1) === 0 ? "wheel_up" : "wheel_down", button: "none", x, y, ...mods });
      continue;
    }
    const button = (["left", "middle", "right", "none"] as const)[b & 3]!;
    if (b & 32) out.push({ kind: "motion", button, x, y, ...mods });
    else out.push({ kind: final === "M" ? "press" : "release", button, x, y, ...mods });
  }
  return out;
}

/**
 * Herdr sits between radar and the user's mouse and keeps right-clicks for its own pane menu unless the pane is told
 * otherwise. The host runner (terminal_host::route_herdr_right_click) runs `herdr pane input --current
 * --right-click pane` once mouse reporting is on; the child that owns the TTY does the same here. True when the
 * environment names a Herdr pane.
 */
export function inHerdrPane(env: NodeJS.ProcessEnv): boolean {
  return env.HERDR_ENV === "1" && !!env.HERDR_PANE_ID;
}

export const HERDR_ROUTE_TIMEOUT_MS = 2000;

/** The command that routes this pane's right-clicks to radar. */
export function herdrRouteCommand(env: NodeJS.ProcessEnv): { file: string; args: string[] } {
  return { file: env.HERDR_BIN_PATH || "herdr", args: ["pane", "input", "--current", "--right-click", "pane"] };
}

type Spawn = (file: string, args: string[], opts: { timeout: number; stdio: "ignore" }) => { on(ev: string, fn: (...a: any[]) => void): unknown; unref?: () => void };

/** Routes the pane's right-clicks to radar when in a Herdr pane; reports the outcome to `done` (never throws). */
export function routeHerdrRightClick(env: NodeJS.ProcessEnv, spawn: Spawn, done: (err: string | null) => void = () => {}): boolean {
  if (!inHerdrPane(env)) return false;
  const { file, args } = herdrRouteCommand(env);
  try {
    const child = spawn(file, args, { timeout: HERDR_ROUTE_TIMEOUT_MS, stdio: "ignore" });
    child.on("error", (e: Error) => done(`Herdr pane routing unavailable: ${e.message}`));
    child.on("exit", (code: number | null) => done(code === 0 ? null : "Herdr could not route right-clicks to this pane"));
  } catch (e) {
    done(`Herdr pane routing unavailable: ${(e as Error).message}`);
  }
  return true;
}
