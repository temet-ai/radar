// Cell colours as CSS. Ink writes a slot's colour the way the host resolved it at the colour tier (look.ts):
// `#rrggbb` at truecolor, `ansi256(n)` at 256 colours, a terminal colour (SGR 30-37, 90-97) at 16. A real terminal
// paints the 16 and the 256 table from its own palette; the page uses xterm's defaults for them.

/** xterm's default 16 colours, by index (0-7 normal, 8-15 bright). */
export const XTERM_16 = [
  "#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000ee", "#cd00cd", "#00cdcd", "#e5e5e5",
  "#7f7f7f", "#ff0000", "#00ff00", "#ffff00", "#5c5cff", "#ff00ff", "#00ffff", "#ffffff",
];

const hex2 = (n: number) => n.toString(16).padStart(2, "0");

/** xterm's 256-colour table: 16 system colours, a 6x6x6 cube, 24 greys. */
export function xterm256(n: number): string {
  if (n < 16) return XTERM_16[n]!;
  if (n >= 232) {
    const v = 8 + (n - 232) * 10;
    return `#${hex2(v)}${hex2(v)}${hex2(v)}`;
  }
  const i = n - 16;
  const step = (c: number) => (c === 0 ? 0 : 55 + c * 40);
  return `#${hex2(step(Math.floor(i / 36)))}${hex2(step(Math.floor(i / 6) % 6))}${hex2(step(i % 6))}`;
}

/** The host's names for the 16 colours (ipc/look.rs colour_word), by xterm index. */
const NAMES = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];

/** A cell colour (`#rrggbb`, `ansi256(n)`, `ansi16(code)`) or a host colour word (`blackBright`) as CSS;
 * undefined for none. */
export function cssColour(c: string | undefined): string | undefined {
  if (!c) return undefined;
  if (c.startsWith("#")) return c;
  const named = NAMES.indexOf(c.replace(/Bright$/, ""));
  if (named >= 0) return XTERM_16[named + (c.endsWith("Bright") ? 8 : 0)];
  const m256 = /^ansi256\((\d+)\)$/.exec(c);
  if (m256) return xterm256(Number(m256[1]));
  const m16 = /^ansi16\((\d+)\)$/.exec(c);
  if (m16) {
    const code = Number(m16[1]);
    const base = code % 10;
    const bright = code >= 90;
    return XTERM_16[base + (bright ? 8 : 0)];
  }
  return undefined;
}
