//! The host's side of the protocol: it holds the [`Host`] as
//! [`crate::headless::Headless`] does, answers the renderer's messages and
//! says what to send it. Intents go through [`Host::update`], so dispatch
//! spans, the mode's checks, confirmations and the journal stay in Rust;
//! the renderer only ever names targets and action ids from the views it
//! was sent, never a provider. Like `headless.rs`, it names no terminal
//! type (a test reads the source to keep it so).

use std::collections::VecDeque;

use crate::design::theme::Tier;
use crate::motion::{self, Cue, Level};
use crate::splash::{self, Splash, SplashView};
use crate::host::{Event, Host, HostEffect, JournalRecord, Owner, View};
use crate::theme::ThemeConfig;
use crate::view::{ChromeView, Intent, PaneId};

use super::{
    Capabilities, InputRequest, Look, MAX_CLIPBOARD, MAX_INPUT, MAX_LINE, MAX_NOTICE, MotionCue,
    MotionRegion, MotionView, Notice,
    PROTOCOL, Question, Snapshot, ToHost, ToRenderer, encode,
};

/// What a renderer's message came to.
#[derive(Debug, Default)]
pub struct Received {
    /// What the host asked for (work to start, files to write): the
    /// runner performs these as it does for the Ratatui renderer.
    pub effects: Vec<HostEffect>,
    /// The renderer said goodbye, or the host quit.
    pub quit: bool,
}

/// The host, driven by a renderer over the protocol.
pub struct Bridge {
    host: Host,
    width: u16,
    height: u16,
    capabilities: Option<Capabilities>,
    records: Vec<JournalRecord>,
    /// The renderer said hello and was welcomed.
    welcomed: bool,
    /// The last cursor given out (snapshots, questions, notices).
    cursor: u64,
    /// The snapshot sent and not yet acknowledged.
    in_flight: Option<u64>,
    /// What the last snapshot sent showed, to send only a change.
    last_state: Option<String>,
    /// The question or captured input open as last told, by its contents.
    asked: Option<String>,
    question_id: u64,
    /// The status line's words as last told.
    told: Option<String>,
    /// Messages that are never coalesced, oldest first.
    outbox: VecDeque<ToRenderer>,
    /// The colour tier the look is resolved at, as the Ratatui runner
    /// detects it.
    tier: Tier,
    /// The theme configuration the renderer's look was last resolved from.
    looked: Option<ThemeConfig>,
    /// The cell the pointer pressed, for the next intent.
    pointer: Option<(u16, u16)>,
    /// Motion cues raised and not yet sent.
    cues: Vec<MotionCue>,
    /// Over SSH motion is held back, as the Ratatui runner does.
    ssh: bool,
    /// The startup splash's timing, and its frame as last worked out.
    splash: Splash,
    splash_view: Option<SplashView>,
    /// The splash's clock: since the bridge was made, or as a test sets it.
    origin: std::time::Instant,
    clock_ms: Option<u64>,
}

/// Cues kept for a snapshot not sent yet.
const CUES_KEPT: usize = 32;

impl Bridge {
    /// Drive `host` for a renderer `width` × `height` cells big (until it
    /// says otherwise). Action recording is on.
    pub fn new(mut host: Host, width: u16, height: u16) -> Self {
        host.enable_journal();
        host.set_view_room(width, height);
        Self {
            host,
            width,
            height,
            capabilities: None,
            records: Vec::new(),
            welcomed: false,
            cursor: 0,
            in_flight: None,
            last_state: None,
            asked: None,
            question_id: 0,
            told: None,
            outbox: VecDeque::new(),
            tier: Tier::TrueColor,
            looked: None,
            pointer: None,
            cues: Vec::new(),
            ssh: crate::system::Env::detect().ssh,
            // Only a real start shows it ([`Bridge::show_splash`]).
            splash: Splash::new(splash::Setting::Off, Level::Off),
            splash_view: None,
            origin: std::time::Instant::now(),
            clock_ms: None,
        }
    }

    /// Show the startup splash, as `[theme] splash` says, from the first
    /// snapshot (the runner does, at radar's start).
    pub fn show_splash(&mut self) {
        self.splash = Splash::new(splash::Setting::parse(&self.host.colours.cfg.splash), Level::Off);
    }

    /// Set the splash's clock, in ms (tests).
    pub fn set_clock_ms(&mut self, ms: u64) {
        self.clock_ms = Some(ms);
    }

    fn now(&self) -> std::time::Duration {
        self.clock_ms.map_or_else(|| self.origin.elapsed(), std::time::Duration::from_millis)
    }

    /// When to work out the splash's next frame: while it shows, once the
    /// renderer is welcomed.
    pub fn next_frame(&self) -> Option<std::time::Duration> {
        if self.welcomed { self.splash.next_frame() } else { None }
    }

    /// The splash's frame now ([`crate::splash`]), as the Ratatui runner
    /// works it out: at the end of its hold it hands off with the screen
    /// cue; where it cannot be drawn (too narrow) it ends at once.
    fn step_splash(&mut self) {
        if !self.splash.active() {
            self.splash_view = None;
            return;
        }
        self.splash.set_setting(splash::Setting::parse(&self.host.colours.cfg.splash));
        self.splash.set_level(self.motion_level().0);
        let parts = self.host.splash_parts();
        let now = self.now();
        self.splash_view = match self.splash.step(now, true, &parts) {
            splash::Step::Show(scene) => {
                let theme = self.host.build_theme(self.tier);
                let frame = splash::frame(&theme, &scene, self.width, self.height);
                if frame.is_none() {
                    self.splash.input();
                }
                frame.map(|f| splash::view(&f))
            }
            splash::Step::HandOff => {
                self.cues.push(MotionCue::Screen { region: MotionRegion::Body });
                None
            }
            splash::Step::Hidden => None,
        };
    }

    /// The splash as last worked out, while it shows.
    pub fn splash(&self) -> Option<&SplashView> {
        self.splash_view.as_ref()
    }

    /// Whether motion is held back as over SSH (a test sets it).
    pub fn set_ssh(&mut self, ssh: bool) {
        self.ssh = ssh;
    }

    /// The motion level that runs and why it is held back, as the Ratatui
    /// runner works them out.
    fn motion_level(&self) -> (Level, Option<String>) {
        let set = Level::parse(&self.host.colours.cfg.motion);
        let why = motion::held_back(self.tier, self.ssh).map(str::to_string);
        (motion::effective(set, self.tier, self.ssh), why)
    }

    /// Move the cues the host raised into those waiting to be sent: only
    /// ones that run at the level, as the Ratatui engine takes them.
    fn gather_cues(&mut self) {
        let (level, _) = self.motion_level();
        for cue in self.host.take_cues() {
            if !cue.runs_at(level) {
                continue;
            }
            if self.cues.len() >= CUES_KEPT {
                self.cues.remove(0);
            }
            self.cues.push(match cue {
                Cue::Screen => MotionCue::Screen { region: MotionRegion::Body },
                Cue::Page => MotionCue::Page { region: MotionRegion::Body },
                Cue::Popup => MotionCue::Popup { region: MotionRegion::Popup },
                Cue::Message => MotionCue::Message { region: MotionRegion::Message },
                Cue::MessageGone => MotionCue::MessageGone { region: MotionRegion::Message },
                Cue::Changed(c) => MotionCue::Changed {
                    changed: c.changed.into_iter().collect(),
                    added: c.added.into_iter().collect(),
                },
            });
        }
    }

    /// Resolve the look at `tier` (the runner passes `Tier::detect()`, as
    /// the Ratatui runner does); the renderer is sent it again.
    pub fn set_tier(&mut self, tier: Tier) {
        self.tier = tier;
        self.looked = None;
    }

    /// The look the renderer is sent: the host's theme at this tier.
    pub fn look(&self) -> Look {
        Look::of(&self.host.build_theme(self.tier))
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut Host {
        &mut self.host
    }

    /// Give the host back (to the Ratatui renderer, when Ink cannot run):
    /// its parts, state and journal are as the renderer left them.
    pub fn into_host(self) -> Host {
        self.host
    }

    /// What the renderer said its terminal can do.
    pub fn capabilities(&self) -> Option<&Capabilities> {
        self.capabilities.as_ref()
    }

    /// Every journal record the renderer's intents produced, in order.
    pub fn records(&self) -> &[JournalRecord] {
        &self.records
    }

    /// The renderer is new (started or restarted): it is told everything
    /// again after its hello. Host state is untouched.
    pub fn reconnect(&mut self) {
        self.welcomed = false;
        self.in_flight = None;
        self.last_state = None;
        self.asked = None;
        self.told = None;
        self.looked = None;
        self.cues.clear();
        self.outbox.clear();
    }

    /// Handle one message from the renderer.
    pub fn receive(&mut self, message: ToHost) -> Received {
        let mut out = Received::default();
        match message.bounded() {
            ToHost::Hello {
                protocol,
                view,
                capabilities,
            } => {
                if protocol != PROTOCOL || view != crate::view::VERSION {
                    let message = if protocol != PROTOCOL {
                        format!(
                            "radar speaks renderer protocol {PROTOCOL}, this Ink renderer {protocol}: \
                             install them together, or run RADAR_RENDERER=ratatui radar"
                        )
                    } else {
                        format!(
                            "radar sends views of version {}, this Ink renderer reads {view}: \
                             install them together, or run RADAR_RENDERER=ratatui radar",
                            crate::view::VERSION
                        )
                    };
                    crate::log::warn(&message);
                    self.outbox.push_back(ToRenderer::Refuse {
                        wanted: PROTOCOL,
                        got: protocol,
                        message,
                    });
                    return out;
                }
                self.resize(capabilities.columns, capabilities.rows);
                self.capabilities = Some(capabilities);
                self.welcomed = true;
                self.cursor += 1;
                self.outbox.push_back(ToRenderer::Welcome {
                    protocol: PROTOCOL,
                    view: crate::view::VERSION,
                    cursor: self.cursor,
                });
            }
            ToHost::Resize { columns, rows } => self.resize(columns, rows),
            ToHost::Intent { intent, cursor, .. } => {
                if !self.welcomed {
                    return out;
                }
                if cursor > self.cursor {
                    crate::log::warn(format!(
                        "renderer intent at cursor {cursor}, past the host's {}",
                        self.cursor
                    ));
                }
                let pointer = self.pointer.take();
                let context = matches!(intent, Intent::Context { .. });
                let had_menu = self.host.menu.is_some();
                out.effects = self.send(intent);
                // A context menu opens where the pointer pressed.
                if let (true, false, Some((x, y)), Some(open)) = (context, had_menu, pointer, self.host.menu.as_mut()) {
                    open.anchor = Some((x, y).into());
                }
            }
            ToHost::Pointer { column, row } => self.pointer = Some((column, row)),
            ToHost::Ack { cursor } => {
                if self.in_flight.is_some_and(|c| cursor >= c) {
                    self.in_flight = None;
                }
            }
            ToHost::Log { level, text } => {
                let text = format!("ink: {text}");
                match level {
                    super::LogLevel::Error => crate::log::error(text),
                    super::LogLevel::Warn => crate::log::warn(text),
                    super::LogLevel::Info => crate::log::info(text),
                    super::LogLevel::Debug => crate::log::debug(|| text),
                }
            }
            ToHost::Bye => out.quit = true,
        }
        out.quit |= self.host.quit;
        out
    }

    /// Send what the user did, as [`crate::headless::Headless::send`]
    /// does; the host's effects come back and its journal records are
    /// kept.
    pub fn send(&mut self, intent: Intent) -> Vec<HostEffect> {
        // Whatever the user did ends the splash, and acts on the screen
        // under it.
        self.splash.input();
        self.update(Event::Intent(intent))
    }

    /// Hand the host an event (a tick, a finished task's message).
    pub fn update(&mut self, event: Event) -> Vec<HostEffect> {
        let out = self.host.update(event);
        self.records.extend(out.iter().filter_map(|e| match e {
            HostEffect::Journal(r) => Some(r.clone()),
            _ => None,
        }));
        out
    }

    fn resize(&mut self, columns: u16, rows: u16) {
        self.width = columns.max(1);
        self.height = rows.max(1);
        self.host.set_view_room(self.width, self.height);
    }

    /// Queue text for the terminal's clipboard; the renderer owns the
    /// terminal, so it writes it. False when it is too long to send.
    pub fn clipboard(&mut self, text: String) -> bool {
        if !self.welcomed || text.len() > MAX_CLIPBOARD {
            return false;
        }
        self.outbox.push_back(ToRenderer::Clipboard { text });
        true
    }

    /// The chrome, screen, section and focus as they are now.
    pub fn snapshot_now(&self, cursor: u64) -> Snapshot {
        let chrome = self.host.chrome_view(self.width, self.height);
        let (w, h) = (self.width, self.height);
        // The icon column the views are built for, as the Ratatui shell sets it
        // before it asks for a view: no frame is drawn here to set it.
        self.host
            .frontend
            .set_lead(crate::ui::icon::lead_width(&self.host.build_theme(self.tier)));
        // As Headless reads them: the screen showing, or the settings
        // section a part owns. The host keeps its section index while a
        // screen shows, so a section is sent only on the settings view: a
        // renderer draws what it is sent.
        let mut screen = crate::view::screen_showing(&self.host, w, h);
        let mut section = match self.host.current_section() {
            Some((Owner::Plugin(p), s)) if self.host.view == View::Settings => {
                self.host.section_view_in(p, s.id, w, h)
            }
            // The sections the host owns (parts, mode, colours, icons).
            Some(_) if self.host.view == View::Settings => crate::settings_view::section_view(&self.host, self.tier),
            _ => None,
        };
        // The renderer has no syntax highlighter: a diff arrives painted.
        if let Some(screen) = screen.as_mut() {
            screen.paint_diffs();
        }
        if let Some(section) = section.as_mut() {
            section.paint_diffs();
        }
        let dock = if screen.is_none() && section.is_some() { self.host.dock_view() } else { None };
        Snapshot {
            cursor,
            chrome,
            screen,
            section,
            dock,
            focus: PaneId::from(self.host.focus),
            motion: {
                let (level, held_back) = self.motion_level();
                MotionView {
                    level: level.into(),
                    held_back,
                    ..MotionView::default()
                }
            },
            splash: self.splash_view.clone(),
        }
    }

    /// What to send the renderer now, in order: whatever is never
    /// coalesced (a question, a notice, the clipboard), then a snapshot
    /// when what shows changed and the last one was acknowledged. A
    /// change while a snapshot is in flight waits; the next snapshot
    /// carries it (and every change before it), so nothing is lost.
    pub fn outgoing(&mut self) -> Vec<ToRenderer> {
        if self.welcomed {
            // The look first, so the snapshot after it draws in it.
            if self.looked.as_ref() != Some(&self.host.colours.cfg) {
                self.looked = Some(self.host.colours.cfg.clone());
                self.outbox.push_back(ToRenderer::Look(Box::new(self.look())));
            }
            self.ask_and_tell();
            self.step_splash();
            self.gather_cues();
            if self.in_flight.is_none() {
                let mut snap = self.snapshot_now(0);
                let state = state_key(&snap);
                if self.last_state.as_deref() != Some(state.as_str()) || !self.cues.is_empty() {
                    self.cursor += 1;
                    snap.cursor = self.cursor;
                    snap.motion.cues = std::mem::take(&mut self.cues);
                    if encode(&ToRenderer::Snapshot(Box::new(snap.clone()))).len() > MAX_LINE {
                        crate::log::warn("a view is over the protocol's line bound: sent without its body");
                        snap.screen = None;
                        snap.section = None;
                    }
                    self.in_flight = Some(self.cursor);
                    self.last_state = Some(state);
                    self.outbox.push_back(ToRenderer::Snapshot(Box::new(snap)));
                }
            }
        }
        self.outbox.drain(..).collect()
    }

    /// A question or captured input that opened, and the status line's
    /// words when they changed: queued whether or not a snapshot is in
    /// flight.
    fn ask_and_tell(&mut self) {
        let menu = self.host.menu_view();
        let key = menu.as_ref().map(|m| {
            // A menu is asked once; moving through it or typing in it
            // shows in snapshots.
            serde_json::to_string(&(&m.id, &m.title, m.question, &m.items)).unwrap_or_default()
        });
        if key != self.asked {
            self.asked = key;
            if let Some(m) = menu {
                self.cursor += 1;
                self.question_id += 1;
                self.outbox.push_back(match &m.typed {
                    Some(value) => ToRenderer::InputRequest(InputRequest {
                        id: self.question_id,
                        cursor: self.cursor,
                        title: m.title.chars().take(MAX_NOTICE).collect(),
                        value: value.chars().take(MAX_INPUT).collect(),
                    }),
                    None => ToRenderer::Question(Question {
                        id: self.question_id,
                        cursor: self.cursor,
                        menu: m,
                    }),
                });
            }
        }
        let words = self.host.message.as_ref().map(|m| m.text.clone());
        if words != self.told {
            self.told = words.clone();
            if let Some(text) = words.filter(|t| !t.is_empty()) {
                self.cursor += 1;
                self.outbox.push_back(ToRenderer::Notice(Notice {
                    cursor: self.cursor,
                    text: text.chars().take(MAX_NOTICE).collect(),
                }));
            }
        }
    }

    /// The snapshot in flight, if one is.
    pub fn in_flight(&self) -> Option<u64> {
        self.in_flight
    }

    pub fn welcomed(&self) -> bool {
        self.welcomed
    }

    /// The chrome as it is now.
    pub fn chrome(&self) -> ChromeView {
        self.host.chrome_view(self.width, self.height)
    }
}

/// What a snapshot shows, without its cursor.
fn state_key(s: &Snapshot) -> String {
    // Every part of the snapshot a renderer draws or routes keys by: the
    // dock beside a section too (left out, Ink kept the dock it was last
    // sent on Settings, and its keys went to the wrong place).
    serde_json::to_string(&(&s.chrome, &s.screen, &s.section, &s.dock, s.focus, s.motion.level, &s.splash))
        .unwrap_or_default()
}
