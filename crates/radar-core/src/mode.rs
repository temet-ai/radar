//! The host's mode: which writes radar may send to the services
//! ([`crate::writes::Mode`]). Set by `[general] mode` and its row in
//! settings, general, mode; for one run by `RADAR_MODE` (or the old
//! `RADAR_NO_WRITES` switches, which mean read only). Always shown at the
//! right of the top bar; a click there, the `x` menu's `mode…` row and the
//! settings row open the same menu.

use crate::host::{Host, HostEffect, OpenMenu, Owner};
use crate::plugin::{Binding, Menu, MenuItem, Pane, SectionSpec};
use crate::view::{Chord, Key};
use crate::writes::{self, Mode};

/// The host's own settings section: the mode.
pub const MODE_SECTION: SectionSpec = SectionSpec {
    id: "mode",
    name: "mode",
    group: "general",
};

/// The config table the mode is kept in, `[general]`.
pub const GENERAL_TABLE: &str = "general";

/// The mode's menu.
pub const MODE_MENU: &str = "radar-mode";

impl Host {
    /// The mode in force: the environment's for this run, else the config's.
    pub fn mode(&self) -> Mode {
        self.mode_env.as_ref().map_or(self.mode_cfg, |(m, _)| *m)
    }

    /// Set what the environment says for this run (the runner, from
    /// [`writes::env_mode`]); `None` leaves the config's.
    pub fn set_env_mode(&mut self, env: Option<(Mode, String)>) {
        self.mode_env = env;
        self.apply_mode();
    }

    /// Tell this thread's writes the mode (parts ask it before sending).
    pub(crate) fn apply_mode(&self) {
        writes::set_mode(self.mode(), self.mode_env.as_ref().map(|(_, v)| v.clone()));
    }

    /// The mode's menu: each mode, what it allows, the one in force marked.
    pub fn open_mode_menu(&mut self) {
        let now = self.mode();
        let items = Mode::ALL
            .iter()
            .map(|m| {
                let key = m.key().chars().next().unwrap_or('e');
                let label = if *m == now {
                    format!("{} · now", m.word())
                } else {
                    m.word().to_string()
                };
                MenuItem::new(key, label, m.about(), m.key()).icon(m.icon())
            })
            .collect();
        let mut title = "what radar may change".to_string();
        if let Some((_, v)) = &self.mode_env {
            title = format!("{title} · {v} sets it for this run");
        }
        let row = Mode::ALL.iter().position(|m| *m == now).unwrap_or(0);
        self.menu = Some(OpenMenu {
            owner: Owner::Host,
            menu: Menu::new(MODE_MENU, title, items).icon(now.icon()),
            row,
            input: None,
            refused: None,
            viewport: Default::default(),
            anchor: None,
        });
    }

    /// A mode chosen: kept in the config (saved as any setting is).
    pub(crate) fn on_mode_menu(&mut self, value: &str) {
        let Some(m) = Mode::parse(value) else { return };
        self.mode_cfg = m;
        self.apply_mode();
        self.notify(
            match &self.mode_env {
                Some((now, v)) => format!(
                    "mode {} saved; {v} keeps radar {} for this run",
                    m.word(),
                    now.word()
                ),
                None => format!("radar is in {} mode: it {}", m.word(), m.about()),
            }
            .into(),
        );
    }

    /// A key in the mode section: ⏎ opens the menu, esc goes back.
    pub(crate) fn mode_key(&mut self, key: Chord) -> Vec<HostEffect> {
        match key.key {
            Key::Down | Key::Char('j') => self.pause.row = 1,
            Key::Up | Key::Char('k') => self.pause.row = 0,
            Key::Enter | Key::Char(' ') if self.pause.row == 1 => self.open_idle_menu(),
            Key::Enter | Key::Char(' ') => self.open_mode_menu(),
            Key::Esc => self.focus = Pane::Nav,
            _ => {}
        }
        vec![]
    }
}

/// Why every part's refreshing is paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pause {
    /// Nobody pressed a key for `pause if idle` minutes; any key resumes.
    Idle,
    /// `z`; only `z` resumes.
    Asked,
}

/// The host's pause: one for every part. `z` asks for it, `pause if idle`
/// (`[general] idle_minutes`, else a part's old `idle_minutes`) starts it.
#[derive(Debug, Clone, Copy)]
pub struct PauseState {
    pub now: Option<Pause>,
    /// Minutes without input before it pauses itself; 0 never.
    pub idle_minutes: u64,
    /// The unix time of the last key, click or paste.
    pub last_input: i64,
    /// The row the General section's cursor is on: 0 mode, 1 pause if idle.
    pub row: usize,
}

impl Default for PauseState {
    fn default() -> Self {
        Self {
            now: None,
            idle_minutes: DEFAULT_IDLE_MINUTES,
            last_input: 0,
            row: 0,
        }
    }
}

pub const DEFAULT_IDLE_MINUTES: u64 = 30;
/// The pause-if-idle menu.
pub const IDLE_MENU: &str = "radar-idle";
/// What the menu offers, in minutes; 0 is never.
const IDLE_CHOICES: [u64; 6] = [0, 5, 15, 30, 60, 120];

impl Host {
    /// Whether every part is paused, and why.
    pub fn paused(&self) -> Option<Pause> {
        self.pause.now
    }

    /// The top bar's words while paused, once for all parts.
    pub fn paused_words(&self) -> Option<&'static str> {
        self.pause.now.map(|p| match p {
            Pause::Asked => "paused · z resumes",
            Pause::Idle => "paused while you were away · any key resumes",
        })
    }

    /// `z`: pause every part, or resume.
    pub(crate) fn toggle_pause(&mut self) {
        if self.pause.now.take().is_some() {
            self.notify("refreshing resumed".into());
        } else {
            self.pause.now = Some(Pause::Asked);
            self.notify("refreshing paused · z resumes".into());
        }
    }

    /// Someone pressed, clicked or pasted: an idle pause ends.
    pub(crate) fn input_seen(&mut self, now: i64) {
        self.pause.last_input = now;
        if self.pause.now == Some(Pause::Idle) {
            self.pause.now = None;
            self.notify("refreshing resumed".into());
        }
    }

    /// On each tick: pause after the idle minutes.
    pub(crate) fn check_idle(&mut self, now: i64) {
        if self.plugins.iter().any(|p| p.background_work()) { return; }
        let idle = self.pause.idle_minutes as i64 * 60;
        if self.pause.now.is_none() && idle > 0 && now - self.pause.last_input >= idle {
            self.pause.now = Some(Pause::Idle);
            self.notify(
                format!(
                    "refreshing paused after {} idle",
                    minutes(self.pause.idle_minutes)
                )
                .into(),
            );
        }
    }

    /// Read `[general] idle_minutes`; without it the first part's old
    /// `idle_minutes` (each part used to keep its own).
    pub(crate) fn read_idle(&mut self, table: &toml::Table) {
        let int = |v: &toml::Value| v.as_integer().and_then(|n| u64::try_from(n).ok());
        let general = table
            .get(GENERAL_TABLE)
            .and_then(|g| g.get("idle_minutes"))
            .and_then(int);
        let old = || {
            self.plugins.iter().find_map(|p| {
                table
                    .get(p.id())
                    .and_then(|t| t.get("idle_minutes"))
                    .and_then(int)
            })
        };
        self.pause.idle_minutes = general.or_else(old).unwrap_or(DEFAULT_IDLE_MINUTES);
    }

    pub fn open_idle_menu(&mut self) {
        let now = self.pause.idle_minutes;
        let mut choices = IDLE_CHOICES.to_vec();
        if !choices.contains(&now) {
            choices.push(now);
            choices.sort_unstable();
        }
        let items = choices
            .iter()
            .map(|m| {
                let label = if *m == now {
                    format!("{} · now", idle_words(*m))
                } else {
                    idle_words(*m)
                };
                MenuItem::pick(label, "", m.to_string())
            })
            .collect();
        let row = choices.iter().position(|m| *m == now).unwrap_or(0);
        self.menu = Some(OpenMenu {
            owner: Owner::Host,
            menu: Menu::new(IDLE_MENU, "pause refreshing every part when idle", items),
            row,
            input: None,
            refused: None,
            viewport: Default::default(),
            anchor: None,
        });
    }

    pub(crate) fn on_idle_menu(&mut self, value: &str) {
        let Ok(m) = value.parse::<u64>() else { return };
        self.pause.idle_minutes = m;
        self.notify(format!("pause if idle: {}", idle_words(m)).into());
    }
}

fn minutes(m: u64) -> String {
    if m == 1 {
        "1 minute".into()
    } else {
        format!("{m} minutes")
    }
}

/// `never`, `after 1 minute`, `after 30 minutes`.
pub fn idle_words(m: u64) -> String {
    if m == 0 {
        "never".into()
    } else {
        format!("after {}", minutes(m))
    }
}

/// The section's `x` menu.
pub(crate) fn keys(_: &Host) -> Vec<Binding> {
    vec![
        Binding::heading("mode"),
        Binding::new("⏎", "choose what radar may change"),
    ]
}

/// The top bar's mode chip: the mode's icon by name and its word. Which
/// glyph the icon is belongs to the renderer's pack, not to view data.
pub(crate) fn chip(host: &Host) -> (crate::ui::icon::Icon, &'static str) {
    let m = host.mode();
    (m.icon(), m.word())
}
