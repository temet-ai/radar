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
    MotionInteraction, MotionRegion, MotionView, Notice,
    PROTOCOL, Question, Snapshot, ToHost, ToRenderer,
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
    /// Without the dock's transcript, which `sent` holds.
    last_state: Option<Snapshot>,
    /// The dock's transcript as the renderer holds it, so the next snapshot
    /// sends only what changed ([`Bridge::send_snapshot`]).
    sent: Option<Sent>,
    /// The last transcript revision given out.
    revision: u64,
    /// The icon column's width, with the theme and tier it was worked out
    /// for: building the theme for it on every snapshot cost a millisecond.
    lead: std::cell::RefCell<Option<(ThemeConfig, Tier, usize)>>,
    /// The question or captured input open as last told, by its contents.
    asked: Option<String>,
    question_id: u64,
    /// Highest occurrence ID delivered immediately.
    told_notice_id: u64,
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
    /// Latest processed renderer input, including an ignored/no-op input.
    processed_seq: Option<u64>,
    /// Fresh scene causality, unlike the durable processed acknowledgement.
    scene_origin_seq: Option<u64>,
    /// Over SSH motion is held back, as the Ratatui runner does.
    ssh: bool,
    /// The startup splash's timing, and its frame as last worked out.
    splash: Splash,
    splash_view: Option<SplashView>,
    /// While the splash shows: the rest of the snapshot as last built. Only
    /// the splash moves between the host's own changes, so its frames (every
    /// [`splash::FRAME`]) do not rebuild the screen under it; anything that
    /// reaches the host drops this.
    held: Option<Snapshot>,
    /// The splash's clock: since the bridge was made, or as a test sets it.
    origin: std::time::Instant,
    clock_ms: Option<u64>,
    ui_origin_ms: u64,
    /// Latency tracing (`RADAR_TRACE_LATENCY=1`), when on.
    trace: Option<super::trace::Trace>,
}

/// Cues kept for a snapshot not sent yet.
const CUES_KEPT: usize = 32;

impl Bridge {
    /// Drive `host` for a renderer `width` × `height` cells big (until it
    /// says otherwise). Action recording is on.
    pub fn new(mut host: Host, width: u16, height: u16) -> Self {
        host.enable_journal();
        host.set_view_room(width, height);
        let ui_origin_ms = host.notices.now_ms();
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
            sent: None,
            revision: 0,
            lead: std::cell::RefCell::new(None),
            asked: None,
            question_id: 0,
            told_notice_id: 0,
            outbox: VecDeque::new(),
            tier: Tier::TrueColor,
            looked: None,
            pointer: None,
            cues: Vec::new(),
            processed_seq: None,
            scene_origin_seq: None,
            ssh: crate::system::Env::detect().ssh,
            // Only a real start shows it ([`Bridge::show_splash`]).
            splash: Splash::new(splash::Setting::Off, Level::Off),
            splash_view: None,
            held: None,
            origin: std::time::Instant::now(),
            clock_ms: None,
            ui_origin_ms,
            trace: super::trace::wanted(std::env::var(super::trace::VAR).ok().as_deref()).then(Default::default),
        }
    }

    /// Show the startup splash, as `[theme] splash` says, from the first
    /// snapshot (the runner does, at radar's start).
    pub fn show_splash(&mut self) {
        self.splash = Splash::new(splash::Setting::parse(&self.host.colours.cfg.splash), Level::Off);
        if self.splash.active() {
            let emoji = self.host.build_theme(self.tier).icons().width == 2;
            let facts = self.host.start_facts();
            self.splash.roll(self.motion_level().0 == Level::Full, emoji, facts);
        }
    }

    /// Trace latency (as `RADAR_TRACE_LATENCY=1` does): each snapshot sent
    /// carries what it answers and radar.log says what it cost.
    pub fn set_trace(&mut self, on: bool) {
        self.trace = on.then(Default::default);
    }

    /// Set the splash's clock, in ms (tests).
    pub fn set_clock_ms(&mut self, ms: u64) {
        self.clock_ms = Some(ms);
        self.advance_notices();
    }

    fn advance_notices(&mut self) {
        let revision = self.host.notices.revision();
        let now_ms = self.ui_origin_ms.saturating_add(self.now().as_millis().min(u64::MAX as u128) as u64);
        self.host.set_ui_clock_ms(now_ms);
        self.host.set_notice_motion_enabled(self.motion_level().0 != Level::Off);
        if revision != self.host.notices.revision() { self.held = None; }
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
        let theme = self.host.build_theme(self.tier);
        let parts = self.host.splash_parts(theme.icons());
        let now = self.now();
        self.splash_view = match self.splash.step(now, true, &parts) {
            splash::Step::Show(scene) => {
                let frame = splash::frame(&theme, &scene, self.width, self.height);
                if frame.is_none() {
                    self.splash.input();
                }
                let kitty = self.capabilities.as_ref().is_some_and(|c| c.image == crate::ipc::ImageProtocol::Kitty);
                frame.map(|f| {
                    let mut view = splash::view(&f);
                    // Kitty scales the picture to its cells: a usual cell's pixels.
                    if kitty {
                        view.picture = splash::picture(&f, (10, 20)).as_ref().map(splash::SplashPicture::of);
                    }
                    view
                })
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
        self.scene_origin_seq = None;
        self.host.set_motion_enabled(self.motion_level().0 == Level::Full);
        self.held = None;
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
        self.scene_origin_seq = None;
        self.host.set_motion_enabled(self.motion_level().0 == Level::Full);
        self.looked = None;
        self.held = None;
    }

    /// The look the renderer is sent: the host's theme at this tier.
    pub fn look(&self) -> Look {
        Look::of(&self.host.build_theme(self.tier))
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut Host {
        self.advance_notices();
        self.held = None;
        self.scene_origin_seq = None;
        self.host.take_motion_fold();
        &mut self.host
    }

    /// Give the host back (to the Ratatui renderer, when Ink cannot run):
    /// its parts, state and journal are as the renderer left them.
    pub fn into_host(mut self) -> Host {
        self.host.reset_notice_input_support();
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
        self.host.reset_notice_input_support();
        self.welcomed = false;
        self.in_flight = None;
        self.last_state = None;
        self.sent = None;
        self.asked = None;
        self.told_notice_id = self.host.notices.through_id();
        self.looked = None;
        self.cues.clear();
        self.processed_seq = None;
        self.scene_origin_seq = None;
        self.host.take_motion_fold();
        self.outbox.clear();
        self.held = None;
    }

    /// Handle one message from the renderer.
    pub fn receive(&mut self, message: ToHost) -> Received {
        // An intent reaches the host through `update` and a resize through
        // `resize`, both of which drop what is held; an ack or a log line
        // changes nothing that shows.
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
                self.told_notice_id = self.host.notices.through_id();
                self.cursor += 1;
                self.outbox.push_back(ToRenderer::Welcome {
                    protocol: PROTOCOL,
                    view: crate::view::VERSION,
                    cursor: self.cursor,
                });
            }
            ToHost::Resize { columns, rows } => self.resize(columns, rows),
            ToHost::Intent { intent, cursor, seq } => {
                if !self.welcomed {
                    return out;
                }
                if let Some(trace) = &mut self.trace {
                    trace.intent(seq, &intent, self.sent.as_ref().map_or(0, |s| s.items.len()));
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
                self.host.set_motion_origin(seq, cursor);
                self.processed_seq = Some(self.processed_seq.map_or(seq, |previous| previous.max(seq)));
                out.effects = self.send(intent);
                self.scene_origin_seq = (self.processed_seq == Some(seq)).then_some(seq);
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
            // The renderer lost the transcript a change was built on: the
            // next snapshot sends it whole.
            ToHost::Resync => {
                crate::log::info("renderer asked for the dock's transcript whole");
                self.sent = None;
                self.last_state = None;
            }
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
        if let Some(line) = self.splash.take_early_line() {
            self.host_mut().show_start_line(line);
        }
        self.update(Event::Intent(intent))
    }

    /// Hand the host an event (a tick, a finished task's message).
    pub fn update(&mut self, event: Event) -> Vec<HostEffect> {
        self.advance_notices();
        self.held = None;
        self.scene_origin_seq = None;
        if let Some(trace) = &mut self.trace {
            trace.event(&event);
        }
        self.host.set_motion_enabled(self.motion_level().0 == Level::Full);
        let out = self.host.update(event);
        self.records.extend(out.iter().filter_map(|e| match e {
            HostEffect::Journal(r) => Some(r.clone()),
            _ => None,
        }));
        out
    }

    fn resize(&mut self, columns: u16, rows: u16) {
        self.held = None;
        self.scene_origin_seq = None;
        self.width = columns.max(1);
        self.height = rows.max(1);
        self.host.set_view_room(self.width, self.height);
        self.host.take_motion_fold();
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

    /// The icon column's width for the theme as it is now: worked out again
    /// only when the theme configuration or the tier changed.
    fn lead(&self) -> usize {
        let cfg = &self.host.colours.cfg;
        let mut lead = self.lead.borrow_mut();
        if let Some((c, t, w)) = lead.as_ref()
            && c == cfg
            && *t == self.tier
        {
            return *w;
        }
        let w = crate::ui::icon::lead_width(&self.host.build_theme(self.tier));
        *lead = Some((cfg.clone(), self.tier, w));
        w
    }

    /// The chrome, screen, section and focus as they are now.
    pub fn snapshot_now(&self, cursor: u64) -> Snapshot {
        let (w, h) = (self.width, self.height);
        // The icon column the views are built for, as the Ratatui shell sets it
        // before it asks for a view: no frame is drawn here to set it.
        self.host.frontend.set_lead(self.lead());
        // As Headless reads them: the screen showing, or the settings
        // section a part owns. The host keeps its section index while a
        // screen shows, so a section is sent only on the settings view: a
        // renderer draws what it is sent.
        let mut screen = self.host.displayed_screen(w, h);
        // Responsive owner measurement precedes chrome pane-jump projection.
        let chrome = self.host.chrome_view(w, h);
        let mut section = match self.host.current_section() {
            // Setup's own steps (a step that is a section sends the section).
            _ if self.host.setup_host_step() => self.host.setup_section_view(),
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
        let interaction = (self.motion_level().0 == Level::Full).then(|| {
            let mut context = self.host.motion_context();
            // The already-built view is an additional fail-closed check for
            // legacy owners whose depth did not identify an actual item page.
            if screen.as_ref().is_some_and(|screen| screen.page.is_some())
                && matches!(&context, Some(super::MotionContext::Screen { page: None, .. }))
            {
                context = None;
            }
            let fold = self.host.motion_fold().filter(|fold| {
                context.as_ref() == Some(&fold.context)
                    && fold.origin_seq == self.processed_seq
                    && fold.origin_seq.is_some()
            }).cloned();
            MotionInteraction { context, processed_seq: self.processed_seq, origin_seq: self.scene_origin_seq, fold }
        });
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
                    interaction,
                    ..MotionView::default()
                }
            },
            splash: self.splash_view.clone(),
            trace: None,
        }
    }

    /// What to send the renderer now, in order: whatever is never
    /// coalesced (a question, a notice, the clipboard), then a snapshot
    /// when what shows changed and the last one was acknowledged. A
    /// change while a snapshot is in flight waits; the next snapshot
    /// carries it (and every change before it), so nothing is lost.
    pub fn outgoing(&mut self) -> Vec<ToRenderer> {
        self.advance_notices();
        if self.welcomed {
            self.host.set_motion_enabled(self.motion_level().0 == Level::Full);
            // The look first, so the snapshot after it draws in it.
            if self.looked.as_ref() != Some(&self.host.colours.cfg) {
                self.looked = Some(self.host.colours.cfg.clone());
                self.outbox.push_back(ToRenderer::Look(Box::new(self.look())));
            }
            self.ask_and_tell();
            self.step_splash();
            self.gather_cues();
            if self.in_flight.is_none() {
                self.send_snapshot();
            }
        }
        self.outbox.drain(..).collect()
    }

    /// The snapshot to send now. While the splash shows, the rest of it is
    /// built once per change of the host ([`Bridge::held`]) and only the
    /// splash is new.
    fn snapshot_to_send(&mut self) -> Snapshot {
        if !self.splash.active() {
            self.held = None;
            return self.snapshot_now(0);
        }
        let base = match self.held.take() {
            Some(held) => held,
            None => {
                let mut base = self.snapshot_now(0);
                base.splash = None;
                base
            }
        };
        let mut snap = base.clone();
        snap.splash = self.splash_view.clone();
        self.held = Some(base);
        snap
    }

    /// Queue a snapshot when what shows changed since the last one sent.
    ///
    /// The dock's transcript is the one part of a snapshot that grows
    /// without bound (a session's whole history), and it changes at its foot
    /// (a streamed word, a new message). It is taken out of the snapshot,
    /// compared with the transcript last sent item by item, and sent as
    /// what changed: the items after the unchanged head, with `kept` naming
    /// that head at the revision the renderer holds
    /// ([`crate::dock::DockView::kept`]).
    /// The rest of the snapshot is compared as values, not written out to
    /// find a change, so a check and a send both cost what changed plus the
    /// screen, whatever the transcript holds.
    fn send_snapshot(&mut self) {
        let started = std::time::Instant::now();
        let mut snap = self.snapshot_to_send();
        let built = std::time::Instant::now();
        let transcript = dock_mut(&mut snap).map(|d| (d.active.clone(), std::mem::take(&mut d.transcript)));
        // The unchanged head, when the renderer holds this tab's transcript.
        let head = match (&transcript, &self.sent) {
            (Some((tab, items)), Some(sent)) if sent.tab == *tab => {
                Some(sent.items.iter().zip(items).take_while(|(a, b)| a == b).count())
            }
            _ => None,
        };
        let same_transcript = match (&transcript, &self.sent, head) {
            (None, _, _) => true,
            (Some((_, items)), Some(sent), Some(k)) => k == items.len() && k == sent.items.len(),
            _ => false,
        };
        let same = same_transcript && self.last_state.as_ref().is_some_and(|last| same_state(last, &snap));
        let compared = std::time::Instant::now();
        if same && self.cues.is_empty() {
            if let Some(trace) = &mut self.trace {
                trace.unchanged();
            }
            return;
        }
        self.last_state = Some(snap.clone());
        self.cursor += 1;
        snap.cursor = self.cursor;
        snap.motion.cues = std::mem::take(&mut self.cues);
        self.host.take_motion_fold();
        self.scene_origin_seq = None;
        // What the renderer will hold once it has this snapshot.
        let mut held = None;
        let (mut items_all, mut items_sent) = (0, 0);
        if let Some((tab, items)) = transcript {
            items_all = items.len();
            let (kept, tail, revision) = match (self.sent.take(), head) {
                // Nothing to keep: an empty transcript is sent as it is, at
                // revision 0, so a dock with no session reads as it always did.
                _ if items.is_empty() => {
                    held = Some(Sent { tab, revision: 0, items });
                    (None, Vec::new(), 0)
                }
                (Some(sent), Some(k)) if same_transcript => {
                    // Unchanged: the renderer keeps all of it.
                    let revision = sent.revision;
                    held = Some(sent);
                    (Some((revision, k)), Vec::new(), revision)
                }
                (Some(mut sent), Some(k)) => {
                    self.revision += 1;
                    sent.items.truncate(k);
                    let tail = items[k..].to_vec();
                    sent.items.extend(items.into_iter().skip(k));
                    held = Some(Sent { tab, revision: self.revision, items: sent.items });
                    (Some((sent.revision, k)), tail, self.revision)
                }
                _ => {
                    self.revision += 1;
                    let tail = items.clone();
                    held = Some(Sent { tab, revision: self.revision, items });
                    (None, tail, self.revision)
                }
            };
            items_sent = tail.len();
            if let Some(dock) = dock_mut(&mut snap) {
                dock.transcript = tail;
                dock.revision = revision;
                dock.kept = kept.map(|(revision, items)| crate::dock::DockKept { revision, items: items as u32 });
            }
        }
        let bytes = encoded_len(&snap);
        if bytes > MAX_LINE {
            crate::log::warn("a view is over the protocol's line bound: sent without its body");
            snap.screen = None;
            snap.section = None;
            snap.dock = None;
            // The renderer gets no transcript: it holds what it held.
            held = None;
            self.last_state = None;
        }
        if held.is_some() {
            self.sent = held;
        }
        if let Some(trace) = &mut self.trace {
            let costs = super::trace::Costs {
                build_ms: (built - started).as_secs_f64() * 1000.0,
                compare_ms: (compared - built).as_secs_f64() * 1000.0,
                measure_ms: super::trace::ms_since(compared),
                bytes,
                items: items_all,
                items_sent,
            };
            snap.trace = trace.sent(self.cursor, dock_of(&snap), costs);
        }
        self.in_flight = Some(self.cursor);
        self.outbox.push_back(ToRenderer::Snapshot(Box::new(snap)));
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
        let notices: Vec<_> = self.host.notices.since(self.told_notice_id).cloned().collect();
        for occurrence in notices {
            self.told_notice_id = occurrence.id;
            self.cursor += 1;
            self.outbox.push_back(ToRenderer::Notice(Notice {
                cursor: self.cursor,
                text: occurrence.text.clone(),
                occurrence: Some(occurrence),
                revision: self.host.notices.revision(),
                now_ms: self.host.notices.now_ms(),
            }));
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

/// The dock a snapshot carries: a screen's, or the one beside a settings
/// section.
fn dock_of(s: &Snapshot) -> Option<&crate::dock::DockView> {
    s.screen.as_ref().and_then(|v| v.dock.as_deref()).or(s.dock.as_deref())
}

fn dock_mut(s: &mut Snapshot) -> Option<&mut crate::dock::DockView> {
    match s.screen.as_mut() {
        Some(v) => v.dock.as_deref_mut(),
        None => s.dock.as_deref_mut(),
    }
}

/// Whether two snapshots show the same, cursor and cues aside: every part
/// a renderer draws or routes keys by, the dock beside a section too (left
/// out, Ink kept the dock it was last sent on Settings, and its keys went
/// to the wrong place). Compared as values: writing both out to compare
/// them cost as much as sending them.
fn same_state(a: &Snapshot, b: &Snapshot) -> bool {
    a.chrome == b.chrome
        && a.screen == b.screen
        && a.section == b.section
        && a.dock == b.dock
        && a.focus == b.focus
        && a.motion.level == b.motion.level
        && a.motion.held_back == b.motion.held_back
        && interaction_state(&a.motion.interaction) == interaction_state(&b.motion.interaction)
        && a.splash == b.splash
}

// Fold evidence is a one-shot event, not persistent state. Context and the
// acknowledgement are state: both may change without any visible text changing.
fn interaction_state(interaction: &Option<MotionInteraction>) -> Option<(&Option<super::MotionContext>, Option<u64>)> {
    interaction.as_ref().map(|interaction| (&interaction.context, interaction.processed_seq))
}

/// A snapshot's line length, counted as it is written, without keeping it.
fn encoded_len(snap: &Snapshot) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0 += b.len();
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[derive(serde::Serialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Line<'a> {
        Snapshot(&'a Snapshot),
    }
    let mut n = Count(0);
    let _ = serde_json::to_writer(&mut n, &Line::Snapshot(snap));
    n.0 + 1
}

/// The dock's transcript as the renderer holds it: the tab it is of, the
/// revision it was sent at, its items.
struct Sent {
    tab: Option<String>,
    revision: u64,
    items: Vec<crate::dock::DockItem>,
}
