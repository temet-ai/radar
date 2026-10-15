//! Every element of radar's kit, live, without any part:
//!
//!     cargo run -p radar-core --example gallery
//!
//! `[`/`]` (or tab) switch tabs, `1`..`9` jump, `j`/`k` move, `space` folds,
//! `PgUp`/`PgDn` scroll the card or a story (the pages after Motion show each
//! element in each of its states), `c` steps through the colour levels
//! (the terminal's own, then full, 256, 16 and none), `q` quits. Resize the
//! terminal to watch columns drop. The last tab, Motion, starts each effect
//! with a key (`s m g p n t l`) and `f` steps through full, subtle and off;
//! switching tabs shows the screen switch. Motion runs only in full colour.

use std::time::{Duration, Instant};

use radar_core::design::theme::Tier;
use radar_core::motion::{self, Engine, SystemClock};
use radar_core::ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use radar_core::theme::{Theme, ThemeConfig};
use radar_core::ui::gallery::Gallery;

const LEVELS: [&str; 5] = ["auto", "truecolor", "256", "16", "none"];
/// How often the spinners step.
const TICK: Duration = Duration::from_millis(200);

fn main() -> std::io::Result<()> {
    let detected = Tier::detect();
    let mut level = 0;
    let theme_for = |level: usize| {
        let cfg = ThemeConfig {
            colors: LEVELS[level].into(),
            ..ThemeConfig::default()
        };
        Theme::build(&cfg, detected, &[])
    };
    let mut theme = theme_for(level);
    let mut gallery = Gallery::new();
    let mut fx = Engine::new(gallery.motion.level, Box::new(SystemClock::default()));
    let mut ticked = Instant::now();
    let mut terminal = radar_core::ratatui::init();
    let result = loop {
        fx.set_level(motion::effective(gallery.motion.level, theme.tier(), false));
        let note = format!(
            "colours: {} · motion: {}",
            radar_core::theme::color_level_label(LEVELS[level]),
            fx.level().word()
        );
        if let Err(e) = terminal.draw(|f| {
            let area = f.area();
            gallery.render(&theme, area, f.buffer_mut(), &note);
            fx.draw(f.buffer_mut(), &theme);
        }) {
            break Err(e);
        }
        // Frames come quickly only while an effect runs.
        let wait = fx.next_frame().unwrap_or(TICK);
        match event::poll(wait) {
            Ok(true) => match event::read() {
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                    fx.input();
                    if k.code == KeyCode::Char('c') {
                        level = (level + 1) % LEVELS.len();
                        theme = theme_for(level);
                    } else if !gallery.key(k) {
                        break Ok(());
                    }
                    for cue in gallery.take_cues() {
                        fx.cue(cue, &theme);
                    }
                }
                Ok(_) => {}
                Err(e) => break Err(e),
            },
            Ok(false) => {}
            Err(e) => break Err(e),
        }
        if ticked.elapsed() >= TICK {
            ticked = Instant::now();
            gallery.tick();
        }
    };
    radar_core::ratatui::restore();
    result
}
