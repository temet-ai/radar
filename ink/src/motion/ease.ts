// The easings motion.rs uses (tachyonfx interpolation.rs). Each takes t in 0..=1.

const f = Math.fround;

export type Ease = "quadOut" | "quadIn" | "sineOut" | "linear";

/** tachyonfx `math::wave_sin`, the parabola its SineOut uses. */
function waveSin(t: number): number {
  const frac = t - Math.trunc(t);
  const x = frac < 0 ? frac + 1 : frac;
  const phase = f(1 - 2 * x);
  return f(4 * phase * (1 - Math.abs(phase)));
}

export function ease(kind: Ease, t: number): number {
  switch (kind) {
    case "quadOut": {
      const u = f(1 - t);
      return f(1 - f(u * u));
    }
    case "quadIn":
      return f(t * t);
    case "sineOut":
      return waveSin(f(t * 0.25));
    case "linear":
      return t;
  }
}

/** `Interpolation::flipped` for the ones used (a mirrored timer). */
export const flipped = (kind: Ease): Ease =>
  kind === "quadOut"
    ? "quadIn"
    : kind === "quadIn"
      ? "quadOut"
      : kind === "sineOut"
        ? "sineOut"
        : "linear";

const bits = new DataView(new ArrayBuffer(4));
/** micromath's `sqrt` (tachyonfx uses it for speed): a bit-shift estimate, within about 5% of the true root. The
 * sweep's CircOut is built on it, so its curve is the one the host's motion uses and not the exact one. */
export function fastSqrt(x: number): number {
  if (!(x >= 0)) return NaN;
  bits.setFloat32(0, x);
  bits.setUint32(0, (bits.getUint32(0) + 0x3f800000) >>> 1);
  return bits.getFloat32(0);
}

/** CircOut, which the sweep applies to its window alpha. */
export const circOut = (t: number): number =>
  f(fastSqrt(f(1 - f(f(t - 1) * f(t - 1)))));

/** tachyonfx EffectTimer: `remaining` counts down from `total` (ms); `alpha` is the eased progress. */
export class Timer {
  remaining: number;
  constructor(
    readonly total: number,
    private ease: Ease,
    private reverse = false,
  ) {
    this.remaining = total;
  }
  /** Direction reversed and curve flipped, so the visual shape is kept (`mirrored`). */
  mirrored(): Timer {
    return new Timer(this.total, flipped(this.ease), !this.reverse);
  }
  alpha(): number {
    if (this.total === 0) return this.reverse ? 0 : 1;
    const inv = f(this.remaining / this.total);
    return ease(this.ease, this.reverse ? inv : f(1 - inv));
  }
  /** Take `dt` ms; the overflow when it ran out, else undefined. */
  process(dt: number): number | undefined {
    if (this.remaining >= dt) {
      this.remaining -= dt;
      return undefined;
    }
    const over = dt - this.remaining;
    this.remaining = 0;
    return over;
  }
  done(): boolean {
    return this.remaining === 0;
  }
  reset() {
    this.remaining = this.total;
  }
}
