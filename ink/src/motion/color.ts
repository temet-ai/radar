// Colour maths of tachyonfx 0.25 (color_space.rs, interpolation.rs), the maths the host's motion engine blends with.
// A colour is 0xRRGGBB, or RESET (-1) for a cell colour the terminal decides. Reset reads as black.

export const RESET = -1;

const f = Math.fround;

export const rgb = (r: number, g: number, b: number): number =>
  (r << 16) | (g << 8) | b;
export const parseHex = (s: string | undefined): number => {
  if (!s) return RESET;
  const m = /^#([0-9a-fA-F]{6})$/.exec(s);
  return m ? parseInt(m[1]!, 16) : RESET;
};
export const hex = (c: number): string =>
  c < 0 ? "reset" : `#${c.toString(16).padStart(6, "0")}`;

/** Reset reads as black (tachyonfx `ToRgbComponents`). */
function parts(c: number): [number, number, number] {
  if (c < 0) return [0, 0, 0];
  return [(c >> 16) & 255, (c >> 8) & 255, c & 255];
}

function rgbToHsl(r: number, g: number, b: number): [number, number, number] {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const delta = max - min;
  const sum = max + min;
  if (delta === 0) return [0, 0, f(sum * f(50 / 255))];
  const l = f(sum * f(50 / 255));
  const denom = 255 - Math.abs(sum - 255);
  const invDelta = f(1 / delta);
  const invDenom = f(1 / denom);
  const s = f(f(delta * 100) * invDenom);
  const hr = f((g - b) * invDelta);
  const hg = f(f((b - r) * invDelta) + 2);
  const hb = f(f((r - g) * invDelta) + 4);
  const rMask = r >= g && r >= b ? 1 : 0;
  const gMask = (g >= b ? 1 : 0) * (1 - rMask);
  const bMask = 1 - rMask - gMask;
  const hr2 = hr + (g < b ? 6 : 0);
  const h = f(f(rMask * hr2 + gMask * hg + bMask * hb) * 60);
  return [h, s, l];
}

function hslToRgb(h: number, s: number, l: number): [number, number, number] {
  s = f(s / 100);
  l = f(l / 100);
  if (s === 0) {
    const gray = Math.round(l * 255);
    return [gray, gray, gray];
  }
  const hh = f(f(((h % 360) + 360) % 360) / 60);
  const c = f((1 - Math.abs(2 * l - 1)) * s);
  const m = f(l - c * 0.5);
  const sector = Math.floor(hh);
  const fr = hh - sector;
  const h2 = (sector & 1) + fr;
  const x = f(c * (1 - Math.abs(h2 - 1)));
  let r: number, g: number, b: number;
  switch (sector) {
    case 0:
      [r, g, b] = [c + m, x + m, m];
      break;
    case 1:
      [r, g, b] = [x + m, c + m, m];
      break;
    case 2:
      [r, g, b] = [m, c + m, x + m];
      break;
    case 3:
      [r, g, b] = [m, x + m, c + m];
      break;
    case 4:
      [r, g, b] = [x + m, m, c + m];
      break;
    default:
      [r, g, b] = [c + m, m, x + m];
  }
  const q = (v: number) => Math.max(0, Math.min(255, Math.round(f(v) * 255)));
  return [q(r), q(g), q(b)];
}

/** `ColorSpace::Hsl.lerp` (tachyonfx's default, and what `Color::lerp` uses): from → to by alpha. */
export function lerp(from: number, to: number, alpha: number): number {
  alpha = Math.max(0, Math.min(1, alpha));
  if (alpha === 0) return from;
  if (alpha === 1) return to;
  const [h1, s1, l1] = rgbToHsl(...parts(from));
  const [h2, s2, l2] = rgbToHsl(...parts(to));
  let hd = h2 - h1;
  if (hd > 180) hd -= 360;
  else if (hd < -180) hd += 360;
  let h = h1 + hd * alpha;
  if (h < 0) h += 360;
  if (h >= 360) h -= 360;
  const [r, g, b] = hslToRgb(h, s1 + (s2 - s1) * alpha, l1 + (l2 - l1) * alpha);
  return rgb(r, g, b);
}
