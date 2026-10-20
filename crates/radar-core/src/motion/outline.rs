//! Loading outlines: while a panel waits for its first rows, its blank rows
//! show the shape the rows will have (an icon slot, a name, a state, an age),
//! and a light wave runs down them. Shared with Ink's `motion/outline.ts`.

/// The shapes rows take, in turn: name, state and age lengths in cells.
const SHAPES: [(u16, u16, u16); 5] = [(6, 0, 0), (12, 8, 2), (16, 10, 3), (11, 8, 2), (9, 8, 2)];
/// One pass of the wave down the panel's rows: each row lights and dims in turn.
pub const WAVE_MS: u32 = 1400;
const WAVE_RISE_MS: u32 = 170;
const WAVE_FALL_MS: u32 = 250;
/// How far an outline sits from the panel's ground toward its text, at rest and at the wave's peak.
pub const DIM: f32 = 0.12;
pub const LIGHT: f32 = 0.45;

/// The glyph at each cell of outline row `row` in a panel `width` cells wide; `None` leaves the cell blank.
pub fn row(row: usize, width: u16) -> Vec<Option<&'static str>> {
    let mut out = vec![None; usize::from(width)];
    if width < 8 {
        return out;
    }
    let (name, state, age) = SHAPES[row % SHAPES.len()];
    let last = width - 1;
    let bar = |from: u16, len: u16, out: &mut Vec<Option<&'static str>>| {
        for x in from..from.saturating_add(len).min(last) {
            out[usize::from(x)] = Some("━");
        }
    };
    out[1] = Some("●");
    bar(3, name, &mut out);
    if state > 0 {
        let at = (3 + name + 2).max(width * 5 / 8);
        bar(at, state, &mut out);
        bar(at + state + 2, age, &mut out);
    }
    out
}

/// How lit row `row` of `rows` is, `elapsed` ms into a pass of the wave: 0 at rest, 1 at the peak.
pub fn wave(elapsed: u32, row: usize, rows: usize) -> f32 {
    let lag = row as u32 * (WAVE_MS - WAVE_RISE_MS - WAVE_FALL_MS) / (rows as u32).max(1);
    let p = (elapsed % WAVE_MS + WAVE_MS - lag) % WAVE_MS;
    if p < WAVE_RISE_MS {
        p as f32 / WAVE_RISE_MS as f32
    } else if p < WAVE_RISE_MS + WAVE_FALL_MS {
        1.0 - (p - WAVE_RISE_MS) as f32 / WAVE_FALL_MS as f32
    } else {
        0.0
    }
}
