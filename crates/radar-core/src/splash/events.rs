//! The special events other than the fire: each a small scene in pixels,
//! two to a cell (`▀`, the mark's grain), [`W`] × [`H`] in the mark's and
//! the names' place. A scene is worked out from the time alone, so any
//! moment draws the same picture and a dropped frame costs nothing.
//!
//! Each tells its story in the time the event holds (from [`FIRE_FROM_MS`]
//! to the hand-off, about 2.8 s) and then keeps still, or keeps moving
//! gently, while the first screen is not ready:
//!
//! - snow: flakes fall in two depths over pines and a cabin with a lit
//!   window and a smoking chimney; the drifts build up.
//! - confetti: `1000` pops in digit by digit; two cannons fire from the
//!   corners, a burst goes off behind the number, then it rains confetti.
//! - tumbleweed: a dusk sun on the horizon, a mesa, a saguaro; a tumbleweed
//!   bounces across, raising dust, and leaves the desert empty.
//! - duck: a rubber duck floats in on the water, thinks for a moment (`…`),
//!   thinks better of it and blinks.
//! - ufo: a saucer drifts in under the stars, beams a cow up and is gone.
//! - steam: coffee pours into a mug in the mark's colours; steam rises.

use super::{Event, FIRE_FROM_MS, Grid, Ink, Inks, clamp01, ease_out, hash, mix, mixf};

/// The stage, in pixels: as wide as the scope, seven cell rows.
pub(super) const W: usize = 64;
pub(super) const H: usize = 14;

/// A scene's pixels, `None` where the background shows.
struct Pic {
    px: Vec<Option<Ink>>,
}

impl Pic {
    fn new() -> Self {
        Self { px: vec![None; W * H] }
    }

    fn inside(x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < W && (y as usize) < H
    }

    fn set(&mut self, x: i32, y: i32, ink: Ink) {
        if Self::inside(x, y) {
            self.px[y as usize * W + x as usize] = Some(ink);
        }
    }

    fn get(&self, x: i32, y: i32) -> Option<Ink> {
        Self::inside(x, y).then(|| self.px[y as usize * W + x as usize]).flatten()
    }

    /// `rows` with its top left at (`x`, `y`): each character a pixel in the
    /// colour `ink` gives it, `None` (and `.`) clear. `flip` mirrors it.
    fn sprite(&mut self, x: i32, y: i32, rows: &[&str], flip: bool, ink: impl Fn(char, i32) -> Option<Ink>) {
        for (dy, row) in rows.iter().enumerate() {
            let n = row.chars().count() as i32;
            for (dx, c) in row.chars().enumerate() {
                let dx = if flip { n - 1 - dx as i32 } else { dx as i32 };
                if c != '.'
                    && let Some(i) = ink(c, dx)
                {
                    self.set(x + dx, y + dy as i32, i);
                }
            }
        }
    }

    /// Onto `grid` from row `y`, centred, each colour `fade` of the way from
    /// the base.
    fn blit(&self, grid: &mut Grid, base: Ink, fade: f64, y: i32) {
        let x0 = (grid.width as i32 - W as i32) / 2;
        let ink = |c: Option<Ink>| c.map(|c| if fade >= 1.0 { c } else { mixf(base, c, fade) });
        for r in 0..H / 2 {
            for x in 0..W {
                let (top, bottom) = (ink(self.px[2 * r * W + x]), ink(self.px[(2 * r + 1) * W + x]));
                if top.is_none() && bottom.is_none() {
                    continue;
                }
                grid.put(x0 + x as i32, y + r as i32, '▀', top.unwrap_or(base), bottom.flatten(), false);
            }
        }
    }
}

/// The event `event` at `t` seconds into the splash, seven rows from `y`.
/// The scenery fades in over the 0.4 s before the story starts.
pub(super) fn draw(grid: &mut Grid, inks: &Inks, event: Event, t: f64, y: i32) {
    let s = (t - FIRE_FROM_MS as f64 / 1000.0).max(0.0);
    let fade = clamp01((t - (FIRE_FROM_MS as f64 / 1000.0 - 0.4)) / 0.4);
    if fade <= 0.0 {
        return;
    }
    let mut p = Pic::new();
    match event {
        Event::Fire => return,
        Event::Snow => snow(&mut p, inks, s),
        Event::Confetti => confetti(&mut p, inks, s),
        Event::Tumbleweed => tumbleweed(&mut p, inks, s),
        Event::Duck => duck(&mut p, inks, s),
        Event::Ufo => ufo(&mut p, inks, s),
        Event::Steam => steam(&mut p, inks, s),
    }
    p.blit(grid, inks.base, fade, y);
}

/// The fractional part, always in `0..1`.
fn fract(v: f64) -> f64 {
    v - v.floor()
}

/// A pine `h` pixels tall, its tip at (`cx`, `top`): tiers that widen as
/// they go down, lit from the left, snow on each tier's tips.
fn pine(p: &mut Pic, k: &Inks, cx: i32, top: i32, h: i32, seed: f64) {
    let (light, dark, snow) = (mix(k.green, k.base, 0.3), mix(k.green, k.base, 0.62), k.white);
    let trunk = mix(k.number, k.base, 0.6);
    for y in 0..h - 1 {
        let half = y % 3 + y / 3;
        for dx in -half..=half {
            let edge = dx.abs() == half && half > 0;
            let tip = edge && y % 3 == 2;
            let settled = y % 3 == 0 && dx.abs() <= 1 && y > 0 && hash(dx as f64 + seed, y as f64) > 0.55;
            let ink = if tip || settled {
                snow
            } else if dx > 0 && hash(dx as f64 * 3.1 + seed, y as f64) > 0.35 {
                dark
            } else {
                light
            };
            p.set(cx + dx, top + y, ink);
        }
    }
    p.set(cx, top + h - 1, trunk);
}

/// Points that fall and sway: `n` of them `speed` pixels a second, blown a
/// little to the right; `under` says where each column's ground starts.
fn flakes(p: &mut Pic, s: f64, n: usize, seed: f64, speed: (f64, f64), ink: impl Fn(usize) -> Ink, under: impl Fn(i32) -> i32) {
    let span = H as f64 + 3.0;
    for i in 0..n {
        let f = i as f64;
        let (a, b) = (hash(f, seed), hash(seed, f));
        let v = speed.0 + (speed.1 - speed.0) * b;
        let y = (a * span + v * s).rem_euclid(span) - 2.0;
        let x = hash(f * 1.7, seed + 3.0) * W as f64 + (s * 1.3 + a * std::f64::consts::TAU).sin() * 1.2 + s * 1.5;
        let (x, y) = (x.rem_euclid(W as f64).floor() as i32, y.round() as i32);
        if y < under(x) {
            p.set(x, y, ink(i));
        }
    }
}

fn snow(p: &mut Pic, k: &Inks, s: f64) {
    let ground = |x: i32| {
        let rise = clamp01(s / 2.8) * 0.7;
        12 - ((x as f64 * 0.21 + 1.3).sin() * 0.6 + 0.4 + rise).round() as i32
    };
    // The far flakes go behind everything.
    let far = mix(k.secondary, k.base, 0.45);
    flakes(p, s, 26, 1.0, (1.2, 2.2), |_| far, |_| H as i32);

    pine(p, k, 8, 0, 13, 0.0);
    pine(p, k, 16, 4, 9, 2.0);
    pine(p, k, 53, 2, 11, 4.0);
    pine(p, k, 60, 6, 7, 6.0);
    // The star on the tall pine.
    let twinkle = (s * 2.5).sin() > 0.0;
    p.set(8, 0, if twinkle { k.yellow } else { k.white });

    // The cabin: snow on the roof, log walls, a lit window, a door.
    let logs = mix(k.number, k.base, 0.55);
    let window = if hash((s * 6.0).floor(), 3.0) > 0.85 { mix(k.yellow, k.number, 0.5) } else { k.yellow };
    let door = mix(k.number, k.base, 0.75);
    let roof = mix(k.white, k.info, 0.25);
    p.sprite(
        26,
        5,
        &[
            ".........S...",
            "...SSSSS.B...",
            "..SSSSSSSSS..",
            ".SSSSSSSSSSS.",
            "..LLLLLLLLL..",
            "..LYYLLLDDL..",
            "..LYYLLLDDL..",
        ],
        false,
        |c, _| match c {
            'S' => Some(roof),
            'B' => Some(mix(k.red, k.base, 0.4)),
            'L' => Some(logs),
            'Y' => Some(window),
            'D' => Some(door),
            _ => None,
        },
    );
    // Chimney smoke, thinning as it rises.
    for i in 0..6 {
        let a = fract(s * 0.5 + i as f64 / 6.0);
        let x = 35.0 + a * 3.5 + (a * 9.0 + s).sin() * 0.6;
        p.set(x.round() as i32, (4.0 - a * 5.0).round() as i32, mixf(k.muted, k.base, 0.2 + a * 0.7));
    }

    // The ground: drifts, lit on top, blue in the hollows, warm by the window.
    for x in 0..W as i32 {
        let top = ground(x);
        for y in top..H as i32 {
            let ink = if y == top { k.white } else { mix(k.white, k.info, 0.3 + 0.2 * (y - top) as f32) };
            p.set(x, y, ink);
        }
    }
    // The near flakes, bright and quicker, settle on the drifts.
    flakes(p, s, 30, 9.0, (2.6, 4.2), |i| if i % 4 == 0 { mix(k.white, k.info, 0.3) } else { k.white }, ground);
}

/// The digits of the start count, 5 × 7.
const ONE: [&str; 7] = ["..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###."];
const ZERO: [&str; 7] = [".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."];

/// A piece of confetti from (`x0`, `y0`) at `v` pixels a second at angle
/// `a`, `age` seconds in: air drag, gravity, and a flutter once it slows.
/// `None` once it has fallen out.
fn piece(x0: f64, y0: f64, v: f64, a: f64, age: f64, phase: f64) -> Option<(i32, i32)> {
    if age < 0.0 {
        return None;
    }
    let (c, g) = (2.6, 20.0);
    let e = 1.0 - (-c * age).exp();
    let x = x0 + v * a.cos() * e / c + (age * 8.0 + phase).sin() * 1.1 * e;
    let y = y0 - ((v * a.sin() + g / c) * e / c - g * age / c);
    (y < H as f64 + 1.0).then(|| (x.round() as i32, y.round() as i32))
}

fn confetti(p: &mut Pic, k: &Inks, s: f64) {
    let colours = [k.accent[0], k.cyan, k.pink, k.yellow, k.info, k.green, k.accent[2]];
    let ink = |i: usize, age: f64| {
        let c = colours[i % colours.len()];
        // Edge on now and then, as it turns.
        if ((age * 9.0 + i as f64 * 0.7).floor() as i64) % 3 == 0 { mix(c, k.base, 0.45) } else { c }
    };
    // A burst behind the number.
    for i in 0..40 {
        let f = i as f64;
        let a = f / 40.0 * std::f64::consts::TAU + hash(f, 5.0) * 0.3;
        let v = 14.0 + hash(f, 6.0) * 22.0;
        let age = s - 1.45;
        if let Some((x, y)) = piece(32.0, 6.5, v, a, age, f) {
            p.set(x, y, ink(i, age));
        }
    }
    // The cannons in the two corners.
    for side in [0, 1] {
        for i in 0..30 {
            let f = i as f64 + side as f64 * 31.0;
            let tilt = (0.95 + hash(f, 1.0) * 0.45) * if side == 0 { 1.0 } else { -1.0 };
            let a = std::f64::consts::FRAC_PI_2 - tilt * 0.62;
            let v = 30.0 + hash(f, 2.0) * 22.0;
            let age = s - 0.35 - hash(f, 3.0) * 0.12;
            let x0 = if side == 0 { 2.0 } else { W as f64 - 3.0 };
            if let Some((x, y)) = piece(x0, 13.0, v, a, age, f) {
                p.set(x, y, ink(i + side, age));
            }
        }
    }
    // Then it rains confetti.
    let rain = clamp01((s - 2.4) / 0.8);
    if rain > 0.0 {
        let n = (36.0 * rain) as usize;
        flakes(p, s, n, 12.0, (2.0, 4.0), |i| ink(i, s), |_| H as i32);
    }
    // 1000, digit by digit, each with a flash and a hop, a shadow behind.
    let x0 = (W as i32 - 26) / 2;
    for (i, digit) in [ONE, ZERO, ZERO, ZERO].iter().enumerate() {
        let at = s - 0.12 * i as f64;
        if at < 0.0 {
            continue;
        }
        let dx = x0 + i as i32 * 7;
        let hop = if at < 0.12 { 1 } else { 0 };
        let shadow = mix(k.purple, k.base, 0.6);
        p.sprite(dx + 1, 4 - hop, digit, false, |_, _| Some(shadow));
        p.sprite(dx, 3 - hop, digit, false, |_, col| {
            let u = (dx + col - x0) as f32 / 25.0;
            Some(if at < 0.1 { k.white } else { k.ramp_at(u) })
        });
    }
}

fn tumbleweed(p: &mut Pic, k: &Inks, s: f64) {
    // The sun going down behind the horizon, cut in bands that widen as
    // it sinks; yellow at the top, red at the bottom.
    let (sx, r) = (15.0, 9.5);
    for y in 4..12 {
        if y == 8 || y == 10 {
            continue;
        }
        for x in 4..27 {
            let d = (x as f64 + 0.5 - sx).hypot((y as f64 + 0.5 - 12.0) * 1.25);
            if d > r {
                continue;
            }
            let hot = mix(k.yellow, k.red, (y - 4) as f32 / 7.0);
            p.set(x, y, mixf(k.base, hot, 0.8));
        }
    }
    // A mesa and a butte against it.
    let rock = mix(k.violet, k.base, 0.55);
    for x in 34..W as i32 {
        let top = match x {
            34..=37 => 11 - (x - 34),
            38..=50 => 7,
            51..=53 => 8 + (x - 51),
            56..=59 => 9,
            60..=61 => 10,
            _ => 12,
        };
        for y in top..12 {
            p.set(x, y, if y == top { mix(rock, k.number, 0.25) } else { rock });
        }
    }
    // A saguaro in front.
    let (cactus, shade) = (mix(k.green, k.base, 0.2), mix(k.green, k.base, 0.5));
    p.sprite(
        43,
        2,
        &[
            "...GD...", "...GD..G", "G..GD..G", "G..GD..G", "GGGGD.GG", "...GDGD.", "...GD...", "...GD...", "...GD...", "...GD...",
        ],
        false,
        |c, _| match c {
            'G' => Some(cactus),
            'D' => Some(shade),
            _ => None,
        },
    );
    // The ground, with stones.
    for x in 0..W as i32 {
        p.set(x, 12, mix(k.number, k.base, 0.5));
        let stone = hash(x as f64, 7.0) > 0.86;
        p.set(x, 13, if stone { mix(k.number, k.base, 0.45) } else { mix(k.number, k.base, 0.72) });
    }

    // The tumbleweed: rolls as it goes, hops a little lower each time.
    let (rad, period) = (3.3, 0.55);
    let cx_at = |s: f64| -5.0 + 24.0 * s;
    let cx = cx_at(s);
    let u = fract(s / period);
    let hop = 3.0 * (1.0 - s / 6.0).max(0.4);
    let cy = 12.0 - rad - 4.0 * u * (1.0 - u) * hop + 0.2;
    // Its shadow, smaller the higher it is.
    let sw = (2.5 - (12.0 - rad - cy) * 0.5).max(0.5);
    for x in (cx - sw).round() as i32..=(cx + sw).round() as i32 {
        p.set(x, 12, mix(k.number, k.base, 0.7));
    }
    // Dust where it last landed.
    let landed = (s / period).floor() * period;
    let age = s - landed;
    if age < 0.45 && landed > 0.0 {
        for side in [-1.0, 1.0] {
            for n in 0..2 {
                let x = cx_at(landed) + side * (1.5 + age * 7.0 + n as f64 * 1.5);
                let y = 11.0 - age * 3.0 - n as f64 * 0.5;
                p.set(x.round() as i32, y.round() as i32, mixf(mix(k.number, k.secondary, 0.4), k.base, age / 0.45));
            }
        }
    }
    let a = cx / rad;
    let (outer, inner, stem) = (mix(k.number, k.base, 0.45), mix(k.number, k.base, 0.15), mix(k.yellow, k.number, 0.5));
    for y in (cy - rad).floor() as i32..=(cy + rad).ceil() as i32 {
        for x in (cx - rad).floor() as i32..=(cx + rad).ceil() as i32 {
            let (dx, dy) = (x as f64 + 0.5 - cx, (y as f64 + 0.5 - cy) * 1.1);
            let d = dx.hypot(dy);
            if d > rad {
                continue;
            }
            // Turn the tangle with the ball, so it reads as rolling.
            let (u, v) = (dx * a.cos() + dy * a.sin(), -dx * a.sin() + dy * a.cos());
            let n = hash((u * 1.6).round(), (v * 1.6).round());
            let rim = d > rad - 0.8 && hash((v.atan2(u) * 4.0).round(), 2.0) > 0.3;
            if rim || n > 0.62 {
                p.set(x, y, if rim { outer } else if n > 0.85 { stem } else { inner });
            }
        }
    }
    // A breath of wind along the ground.
    for i in 0..4 {
        let f = i as f64;
        let x = (hash(f, 11.0) * W as f64 + s * 30.0).rem_euclid(W as f64);
        p.set(x as i32, 10 + i % 2, mix(k.muted, k.base, 0.35));
    }
}

fn duck(p: &mut Pic, k: &Inks, s: f64) {
    let surface = |x: i32| 10 + ((x as f64 * 0.35 - s * 3.0).sin() * 0.7).round() as i32;
    // It floats in, slows and stops, and bobs.
    let dx = (-16.0 + 41.0 * ease_out(s / 1.6)).round() as i32;
    let dy = 2 + ((s * 2.6).sin() * 0.8).round() as i32;
    let blink = (3.15..3.3).contains(&s);
    let (body, wing, beak) = (k.yellow, mix(k.yellow, k.number, 0.45), k.number);
    p.sprite(
        dx,
        dy,
        &[
            ".....YYYY.....",
            "....YYYYYY....",
            "....YYYYEYOO..",
            "....YYYYYYOOO.",
            ".....YYYYY....",
            ".Y..YYYYYY....",
            ".YYYYYYYYYYY..",
            ".YYYYDDDYYYYY.",
            "..YYYYDDYYYYY.",
            "...YYYYYYYYY..",
        ],
        false,
        |c, _| match c {
            'Y' => Some(body),
            'D' => Some(wing),
            'O' => Some(beak),
            'E' => Some(if blink { wing } else { k.base }),
            _ => None,
        },
    );
    // It thinks.
    for (i, at) in [1.9, 2.2, 2.5].iter().enumerate() {
        if s >= *at && s < 3.0 {
            p.set(dx + 12 + i as i32 * 2, dy - 2, k.secondary);
        }
    }
    // The water over the duck's bottom, a wake while it moves, light on the crests.
    let (deep, crest) = (mix(k.info, k.base, 0.55), mix(k.cyan, k.base, 0.15));
    let moving = s < 1.5;
    for x in 0..W as i32 {
        let top = surface(x);
        for y in top..H as i32 {
            let lit = y == top && (x as f64 * 0.35 - s * 3.0).sin() > 0.75;
            let wake = moving && y == top && x < dx + 2 && x > dx - 10 && (x + (s * 10.0) as i32) % 3 == 0;
            let water = if lit || wake { crest } else { mix(deep, k.base, (y - top) as f32 * 0.12) };
            let ink = match p.get(x, y) {
                Some(duck) if y <= top + 1 => mix(duck, water, 0.55),
                _ => water,
            };
            p.set(x, y, ink);
        }
    }
}

fn ufo(p: &mut Pic, k: &Inks, s: f64) {
    // Stars, each twinkling in its own time.
    for i in 0..22 {
        let f = i as f64;
        let (x, y) = ((hash(f, 21.0) * W as f64) as i32, (hash(22.0, f) * 9.0) as i32);
        let b = 0.5 + 0.5 * (s * (2.0 + 3.0 * hash(f, 23.0)) + f * 20.0).sin();
        p.set(x, y, mixf(k.base, k.white, 0.2 + 0.6 * b));
    }
    // Hills, flat where the cow grazes, a tuft of grass here and there.
    let (hill, grass) = (mix(k.violet, k.base, 0.7), mix(k.green, k.base, 0.5));
    let ground = |x: i32| if (24..=40).contains(&x) { 12 } else { 12 + ((x as f64 * 0.17).sin() * 1.2).round() as i32 };
    for x in 0..W as i32 {
        let top = ground(x).min(13);
        for y in top..H as i32 {
            p.set(x, y, if y == top && hash(x as f64, 24.0) > 0.7 { grass } else { hill });
        }
    }

    // The saucer drifts in and hangs there; later it is gone in a blink.
    let e = ease_out(s / 1.1);
    let (mut ux, mut uy) = (-15.0 + 40.0 * e, -3.0 + 4.0 * e);
    let gone = (s - 2.6).max(0.0);
    ux += 70.0 * gone * gone + 6.0 * gone;
    uy -= 4.0 * gone;
    let (ux, uy) = (ux.round() as i32, uy.round() as i32);

    // The beam, and the cow going up it.
    let open = clamp01((s - 1.15) / 0.2) * clamp01((2.45 - s) / 0.2);
    if open > 0.0 {
        for y in uy + 5..12 {
            let half = (1.5 + (y - uy - 5) as f64 * 0.9) * open;
            let scan = if (y as f64 - s * 14.0).rem_euclid(3.0) < 1.0 { 0.2 } else { 0.0 };
            for x in (ux as f64 + 6.5 - half).round() as i32..=(ux as f64 + 6.5 + half).round() as i32 {
                p.set(x, y, mixf(k.base, k.cyan, 0.55 - (y - uy - 5) as f64 * 0.04 + scan));
            }
        }
    }
    let lift = clamp01((s - 1.45) / 0.8);
    if lift < 1.0 {
        let top = 8 - (5.0 * lift * lift).round() as i32;
        let flip = lift > 0.0 && ((s * 4.0) as i32) % 2 == 1;
        let spot = mix(k.base, k.muted, 0.3);
        // Into the hull: the rows above its underside are already inside.
        let hide = if lift > 0.0 { uy + 4 } else { i32::MIN };
        let cow = ["WW.....", "WWWKWWW", ".WWWWKW", ".W...W."];
        for (dy, row) in cow.iter().enumerate() {
            if top + dy as i32 >= hide {
                p.sprite(28, top + dy as i32, &[row], flip, |c, _| match c {
                    'W' => Some(k.white),
                    'K' => Some(spot),
                    _ => None,
                });
            }
        }
    }
    // A streak as it goes.
    if gone > 0.0 {
        for n in 1..5 {
            let g = (gone - 0.03 * n as f64).max(0.0);
            let x = (-15.0 + 40.0 + 70.0 * g * g + 6.0 * g).round() as i32 + 6;
            p.set(x, (1.0 - 4.0 * g).round() as i32 + 3, mixf(k.cyan, k.base, 0.2 * n as f64));
        }
    }
    let lights = [k.pink, k.yellow, k.cyan];
    let (dome, glint, hull, under) = (mix(k.cyan, k.base, 0.35), mix(k.cyan, k.white, 0.6), k.secondary, mix(k.secondary, k.base, 0.5));
    p.sprite(
        ux,
        uy,
        &["....GDDDD....", "..DDDDDDDDD..", "HHHHHHHHHHHHH", ".HLHLHLHLHLH.", "...UUUUUUU..."],
        false,
        |c, col| match c {
            'G' => Some(glint),
            'D' => Some(dome),
            'H' => Some(hull),
            'U' => Some(under),
            'L' => Some(lights[(col as usize / 2 + (s * 6.0) as usize) % lights.len()]),
            _ => None,
        },
    );
}

fn steam(p: &mut Pic, k: &Inks, s: f64) {
    let coffee = mix(k.number, k.base, 0.65);
    // The saucer.
    for x in 20..44 {
        p.set(x, 13, if x == 20 || x == 43 { mix(k.secondary, k.base, 0.65) } else { mix(k.secondary, k.base, 0.4) });
    }
    // The mug, round from the light on its left, in the mark's colours.
    let full = clamp01(s / 0.9);
    let (lit, mid, dim) = (k.white, mix(k.white, k.secondary, 0.5), mix(k.secondary, k.base, 0.35));
    p.sprite(
        23,
        6,
        &[
            "MCCCCCCCCCCCCM....",
            "MMMMMMMMMMMMMMHHH.",
            "MMMMMMMMMMMMMM..H.",
            "BBBBBBBBBBBBBB..H.",
            "MMMMMMMMMMMMMMHHH.",
            "MMMMMMMMMMMMMM....",
            ".MMMMMMMMMMMM.....",
        ],
        false,
        |c, col| match c {
            'M' => Some(match col {
                0..=1 => lit,
                2..=9 => mid,
                _ => dim,
            }),
            'H' => Some(dim),
            'B' => Some(k.ramp_at(col as f32 / 13.0)),
            'C' => Some(mixf(mix(k.muted, k.base, 0.6), coffee, full)),
            _ => None,
        },
    );
    // The pour, then the last of it falls.
    if s < 1.05 {
        let from = ((s - 0.85) * 40.0).max(0.0).round() as i32;
        for y in from..6 {
            let x = 29 + if hash(y as f64, (s * 12.0).floor()) > 0.8 { 1 } else { 0 };
            p.set(x, y, mix(k.number, k.base, 0.45));
        }
        if s > 0.1 && ((s * 10.0) as i32) % 2 == 0 {
            p.set(27, 5, mix(k.number, k.base, 0.5));
            p.set(32, 5, mix(k.number, k.base, 0.5));
        }
    }
    // Steam: three wisps that curl as they rise and thin out.
    let warm = clamp01((s - 0.7) / 0.6);
    if warm <= 0.0 {
        return;
    }
    for (w, bx) in [26.0, 29.5, 33.0].iter().enumerate() {
        let wf = w as f64;
        for i in 0..8 {
            let a = fract(s * 0.55 + i as f64 / 8.0 + wf * 0.31);
            let y = (5.0 - a * 6.2).round() as i32;
            let x = bx + (a * 5.0 + s * 2.2 + wf * 2.0).sin() * (0.5 + a * 1.8);
            p.set(x.round() as i32, y, mixf(k.base, k.white, (1.0 - a * a) * 0.9 * warm));
        }
    }
}
