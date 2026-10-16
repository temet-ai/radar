//! Setup: a mode of the host that walks the user through what to set up,
//! over the same settings sections Settings shows.
//!
//! `Host.setup` holds where the run is. While it is on, the view stays
//! [`View::Settings`] and the host shows one step at a time: the host's own
//! steps (scan, choose, ready, and local code until a part offers it) are
//! drawn from [`Host::setup_section_view`]; every other step is a settings
//! section, so its card is the section's body and its sheet the section's
//! detail, edited by the same code as in Settings. The settings nav is never
//! drawn or focused. A part offers its step with [`Plugin::setup_step`] and
//! where its keys may be found with [`Plugin::setup_finds`].
//!
//! The flow: scan (may radar look for keys? asked every run, a yes never
//! kept) → choose (a tick per thing to set up) → one step per tick, in the
//! chooser's order → ready (what was set up, where it is saved, the mode,
//! the tour). `radar setup <step>` opens one step on its own.
//!
//! [`Plugin::setup_step`]: crate::Plugin::setup_step
//! [`Plugin::setup_finds`]: crate::Plugin::setup_finds

use std::path::{Component, Path};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::design::help_bar::KeyHint;
use crate::host::{Host, HostEffect, Owner, View};
use crate::motion::Level;
use crate::plugin::{Pane, SetupFinds, SetupGroup, SetupStep};
use crate::ui::icon::Icon;
use crate::view::chrome::SetupChrome;
use crate::view::{Block, Ink, ListRow, Node, SectionView, Seg, State, Target, Text};

#[cfg(test)]
mod tests;

/// The targets of the host steps' rows start with this.
pub const PREFIX: &str = "setup:";

/// The steps `radar setup <step>` takes besides the parts' own.
pub const HOST_STEPS: [&str; 7] = ["welcome", "scan", "choose", "look", "local", "mode", "ready"];

/// A step a tick in the chooser stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Look & terminal: the host's colours.
    Look,
    /// Local code: the first part offering a step on this machine, else
    /// the host's list of the parts that read this machine.
    Local,
    /// A part's step ([`crate::Plugin::setup_step`]).
    Part(usize),
    /// The mode: which writes radar may send. Every setup ends with it,
    /// and nothing is chosen until the user chooses.
    Mode,
}

/// Which screen of setup shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At {
    /// What radar is and what setup will do, read before anything is asked.
    Welcome,
    Scan,
    Choose,
    /// Step `i` of the plan.
    Step(usize),
    Ready,
}

/// How a step of the plan went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Upcoming,
    Done,
    Skipped,
}

/// The look for keys: asked every time setup runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scan {
    NotAsked,
    Declined,
    Looking,
    Found(Vec<Found>),
}

/// What the scan found for one part: names only, never a value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Found {
    /// The part's id.
    pub part: String,
    /// The variables found, each as `env:NAME`.
    pub env: Vec<String>,
    /// The programs whose sign-in file exists.
    pub signed_in: Vec<String>,
}

impl Found {
    pub fn is_empty(&self) -> bool {
        self.env.is_empty() && self.signed_in.is_empty()
    }

    /// What the chooser says of it: `token found`, `3 found`, `signed in`.
    pub fn words(&self) -> Option<String> {
        match (self.env.len(), self.signed_in.len()) {
            (0, 0) => None,
            (1, 0) => Some("token found".into()),
            (0, _) => Some("signed in".into()),
            (n, s) => Some(format!("{} found", n + s)),
        }
    }
}

/// What the scan is asked to look for: each part's id and its places.
pub type Asks = Vec<(String, SetupFinds)>;

/// The look for keys, as a pure function: which of the `names` (the
/// environment's variable names; values are never given) match each part's
/// [`SetupFinds::env`], and which of its sign-in files exist under `home`,
/// by `exists`. A path that is absolute or climbs out of `home` is not
/// looked at. Every asked part has a [`Found`], empty when nothing was.
pub fn scan<N: AsRef<str>>(
    asks: &[(String, SetupFinds)],
    names: impl IntoIterator<Item = N>,
    home: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> Vec<Found> {
    let names: Vec<String> = names.into_iter().map(|n| n.as_ref().to_string()).collect();
    asks.iter()
        .map(|(part, finds)| {
            let mut env: Vec<String> = names
                .iter()
                .filter(|name| {
                    finds.env.iter().any(|want| {
                        if want.ends_with('_') {
                            name.starts_with(want) && name.len() > want.len()
                        } else {
                            name.as_str() == *want
                        }
                    })
                })
                .map(|name| format!("env:{name}"))
                .collect();
            env.sort();
            env.dedup();
            let mut signed_in: Vec<String> = match home {
                Some(home) => finds
                    .files
                    .iter()
                    .filter(|(_, rel)| {
                        let rel = Path::new(rel);
                        !rel.as_os_str().is_empty()
                            && rel.components().all(|c| matches!(c, Component::Normal(_)))
                            && exists(&home.join(rel))
                    })
                    .map(|(program, _)| program.to_string())
                    .collect(),
                None => vec![],
            };
            signed_in.dedup();
            Found { part: part.clone(), env, signed_in }
        })
        .collect()
}

/// A choice on the look step.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LookRow {
    Icons(&'static str),
    Palette(String),
    Motion(Level),
}

/// The icon packs the look step offers: the pack, its word, what it needs.
const LOOK_ICONS: [(&str, &str, &str); 3] =
    [("nerd", "nerd", "needs a Nerd Font"), ("unicode", "plain", "any font"), ("emoji", "emoji", "wide; columns shift")];

const NERD_FONTS: &str = "nerdfonts.com/font-downloads";

/// The set-up page's line for a part with a setup step.
pub const RESUME: &str = "Press s to walk its setup step.";

/// A row of the chooser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub step: Step,
    /// One of the step's own rows ([`crate::plugin::SetupStep::options`]).
    pub option: Option<usize>,
    pub ticked: bool,
}

/// The steps the ticked rows take, each once, in the chooser's order.
fn plan_of(choices: &[Choice]) -> Vec<Step> {
    let mut plan: Vec<Step> = Vec::new();
    for c in choices.iter().filter(|c| c.ticked) {
        if !plan.contains(&c.step) {
            plan.push(c.step);
        }
    }
    plan
}

/// Where a setup run is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    pub at: At,
    pub scan: Scan,
    /// The chooser's rows, in its order.
    pub choices: Vec<Choice>,
    /// The ticked steps, in the chooser's order, once it was left.
    pub plan: Vec<Step>,
    /// The mode chosen on the mode step; `None` until the user chooses.
    pub mode: Option<crate::writes::Mode>,
    /// How each step of the plan went.
    pub marks: Vec<Mark>,
    /// The cursor on a host step's choices.
    pub row: usize,
    /// Opened at one step (`radar setup <step>`): `⏎` there goes on to
    /// ready.
    pub single: bool,
    /// The chooser's ticks were changed by hand: a scan answering late
    /// leaves them.
    touched: bool,
}

impl Setup {
    fn new(at: At) -> Self {
        Self {
            at,
            scan: Scan::NotAsked,
            choices: vec![],
            plan: vec![],
            mode: None,
            marks: vec![],
            row: 0,
            single: false,
            touched: false,
        }
    }

    /// How many ticks would start.
    pub fn ticked(&self) -> usize {
        self.choices.iter().filter(|c| c.ticked).count()
    }
}

/// A muted capitals heading, as the settings sections draw one.
fn heading(target: &str, words: &str) -> ListRow {
    ListRow::heading(target, Text::new().push(Seg::new(words.to_uppercase(), Ink::Muted).strong()))
}

fn blank(n: usize) -> ListRow {
    ListRow::heading(format!("{PREFIX}blank:{n}"), Text::new())
}

/// A mark that says on or off: a tick, or a radio's dot.
fn tick(on: bool) -> Seg {
    if on {
        Seg::icon_or_lead(Icon::Done, "● ", Ink::Good)
    } else {
        Seg::icon_or_lead(Icon::Todo, "○ ", Ink::Muted)
    }
}

fn lines(words: &[&str]) -> Vec<Text> {
    words.iter().map(|w| Text::of(*w, Ink::Secondary)).collect()
}

/// A `$ command` row of the sheet; `y` copies it (the key bar says so).
fn command(target: &str, cmd: &str) -> Block {
    Block::action(target, Text::new().push(Seg::new("$ ", Ink::Muted)).push(Seg::new(cmd, Ink::Primary)))
}

fn sheet(scope: &str, body: Node, blocks: Vec<Block>) -> SectionView {
    SectionView {
        version: crate::view::VERSION,
        scope: scope.into(),
        body,
        detail_title: "about this step".into(),
        detail: vec![],
        detail_blocks: blocks,
        actions: vec![],
    }
}

impl Host {
    // --- the steps ------------------------------------------------------

    /// Part `p`'s step, if it offers one.
    fn part_step(&self, p: usize) -> Option<SetupStep> {
        self.plugins.get(p)?.setup_step()
    }

    /// The first part with a step on this machine: local code's card.
    fn local_part(&self) -> Option<usize> {
        (0..self.plugins.len())
            .find(|&p| self.part_step(p).is_some_and(|s| s.group == SetupGroup::Machine))
    }

    /// The parts that read this machine (no service, no agent dock), for
    /// local code's own card.
    fn local_parts(&self) -> Vec<usize> {
        (0..self.plugins.len())
            .filter(|&p| self.plugins[p].health().is_none() && self.plugins[p].dock_tabs().is_none())
            .collect()
    }

    /// The chooser's rows: this machine, then services, then agents.
    fn candidates(&self) -> Vec<Step> {
        let mut out = vec![];
        let local = self.local_part();
        if local.is_some() || !self.local_parts().is_empty() {
            out.push(Step::Local);
        }
        for group in [SetupGroup::Machine, SetupGroup::Service, SetupGroup::Agent] {
            for p in 0..self.plugins.len() {
                if Some(p) != local && self.part_step(p).is_some_and(|s| s.group == group) {
                    out.push(Step::Part(p));
                }
            }
        }
        // The look last: radar works as it is, so it is the least needed.
        out.push(Step::Look);
        out
    }

    /// The chooser's rows: a step's own rows where it has them, else one.
    fn choice_rows(&self) -> Vec<Choice> {
        let mut out = vec![];
        for step in self.candidates() {
            let options = match step {
                Step::Part(p) => self.part_step(p).map_or(0, |s| s.options.len()),
                _ => 0,
            };
            if options == 0 {
                out.push(Choice { step, option: None, ticked: false });
            } else {
                out.extend((0..options).map(|i| Choice { step, option: Some(i), ticked: false }));
            }
        }
        out
    }

    /// What a step is called on the rail and in the chooser, and its icon.
    pub fn setup_label(&self, step: Step) -> (String, Option<Icon>) {
        match step {
            Step::Look => ("look & terminal".into(), Some(Icon::Colours)),
            Step::Local => ("local code".into(), Some(Icon::Local)),
            Step::Mode => ("mode".into(), Some(self.mode().icon())),
            Step::Part(p) => match self.part_step(p) {
                Some(s) => (s.label.into(), s.icon),
                None => (self.plugins[p].name().into(), None),
            },
        }
    }

    fn step_about(&self, step: Step) -> &'static str {
        match step {
            Step::Look => "icons, theme, motion",
            Step::Local => "checkouts, branches",
            Step::Mode => "which writes radar may send",
            Step::Part(p) => self.part_step(p).map_or("", |s| s.about),
        }
    }

    /// A step's one-line result, for the rail and the ready screen.
    pub fn setup_summary(&self, step: Step) -> Option<String> {
        match step {
            Step::Look => Some(format!("{} · {} icons", self.colours.cfg.palette, self.colours.cfg.icons)),
            Step::Mode => Some(self.mode().word().to_string()),
            Step::Local => match self.local_part() {
                Some(p) => self.part_step(p)?.summary,
                None => {
                    let parts = self.local_parts();
                    let ready = parts.iter().filter(|&&p| self.ready(p)).count();
                    (!parts.is_empty()).then(|| format!("{ready} of {} set up", parts.len()))
                }
            },
            Step::Part(p) => self.part_step(p)?.summary,
        }
    }

    /// The settings section a step's card is, when it is one.
    fn step_section(&self, step: Step) -> Option<usize> {
        let (owner, id) = match step {
            Step::Look | Step::Mode => return None,
            Step::Local => {
                let p = self.local_part()?;
                (Owner::Plugin(p), self.part_step(p)?.section)
            }
            Step::Part(p) => (Owner::Plugin(p), self.part_step(p)?.section),
        };
        self.sections.iter().position(|(o, s)| *o == owner && s.id == id)
    }

    /// Every step `radar setup <step>` takes, in the order they come.
    pub fn setup_step_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = HOST_STEPS[..5].iter().map(|s| s.to_string()).collect();
        // A part's step named as a host step (local code's part) is that step.
        for s in (0..self.plugins.len()).filter_map(|p| self.part_step(p)) {
            if !ids.iter().any(|id| id == s.id) {
                ids.push(s.id.to_string());
            }
        }
        ids.push("ready".into());
        ids.push("tour".into());
        ids
    }

    /// The step showing, when it is one of the plan's.
    fn setup_step_now(&self) -> Option<Step> {
        let setup = self.setup.as_ref()?;
        match setup.at {
            At::Step(i) => setup.plan.get(i).copied(),
            _ => None,
        }
    }

    /// Setup shows a settings section now: its keys go to the section.
    pub fn setup_on_section(&self) -> bool {
        self.setup_step_now().is_some_and(|s| self.step_section(s).is_some())
    }

    /// Setup shows one of the host's own steps (no section under it).
    pub fn setup_host_step(&self) -> bool {
        self.setup.is_some() && !self.setup_on_section()
    }

    // --- opening, moving, leaving --------------------------------------

    /// Open setup: at its start (the scan), or at `step` alone (`radar
    /// setup <step>`), the scan not done. An unknown step is refused with
    /// the list of steps.
    pub fn open_setup(&mut self, step: Option<&str>) -> Result<(), String> {
        let mut setup = Setup::new(At::Welcome);
        match step {
            Some("tour") => {
                self.open_tour();
                return Ok(());
            }
            None | Some("welcome") => {}
            Some("scan") => setup.at = At::Scan,
            Some("choose") => setup.at = At::Choose,
            Some("ready") => {
                setup.plan = self.candidates();
                setup.marks = vec![Mark::Upcoming; setup.plan.len()];
                setup.at = At::Ready;
                setup.single = true;
            }
            Some(id) => {
                let step = match id {
                    "look" => Step::Look,
                    "mode" => Step::Mode,
                    "local" => Step::Local,
                    id => match (0..self.plugins.len()).find(|&p| self.part_step(p).is_some_and(|s| s.id == id)) {
                        Some(p) => Step::Part(p),
                        None => {
                            return Err(format!(
                                "no setup step is called {id}; the steps are: {}",
                                self.setup_step_ids().join(", ")
                            ));
                        }
                    },
                };
                setup.plan = vec![step];
                setup.marks = vec![Mark::Upcoming];
                setup.single = true;
                setup.at = At::Step(0);
            }
        }
        let at = setup.at;
        self.setup = Some(setup);
        self.history = None;
        self.menu = None;
        self.bar = false;
        self.sources = false;
        self.tabs = false;
        self.back.clear();
        self.view = View::Settings;
        self.go(at);
        Ok(())
    }

    /// Show `at`: the chooser built when first reached, a section's step
    /// with that section selected; the keys in the card.
    fn go(&mut self, at: At) {
        if at == At::Choose && self.setup.as_ref().is_some_and(|s| s.choices.is_empty()) {
            let candidates = self.choice_rows();
            let found = self.setup.as_mut().and_then(|setup| {
                setup.choices = candidates;
                match &setup.scan {
                    Scan::Found(found) => Some(found.clone()),
                    _ => None,
                }
            });
            if let Some(found) = found {
                apply_found(self, &found);
            }
        }
        let Some(setup) = self.setup.as_mut() else { return };
        setup.at = at;
        setup.row = 0;
        self.focus = Pane::Middle;
        self.settings_detail.top();
        if let Some(i) = self.setup_step_now().and_then(|s| self.step_section(s)) {
            self.select_section(i);
        }
    }

    /// The step showing is done (or skipped): on to the next, or ready.
    fn advance(&mut self, mark: Mark) {
        let Some(setup) = self.setup.as_mut() else { return };
        let At::Step(i) = setup.at else { return };
        if let Some(m) = setup.marks.get_mut(i) {
            *m = mark;
        }
        let next = if !setup.single && i + 1 < setup.plan.len() { At::Step(i + 1) } else { At::Ready };
        self.go(next);
    }

    /// `←`: the screen before.
    fn setup_back(&mut self) {
        let Some(setup) = self.setup.as_ref() else { return };
        let to = match setup.at {
            At::Welcome => None,
            At::Scan => Some(At::Welcome),
            At::Choose => Some(At::Scan),
            At::Step(0) if setup.single => None,
            At::Step(0) => Some(At::Choose),
            At::Step(i) => Some(At::Step(i - 1)),
            At::Ready if setup.plan.is_empty() => Some(At::Choose),
            At::Ready if setup.single && setup.plan.len() > 1 => None,
            At::Ready => Some(At::Step(setup.plan.len() - 1)),
        };
        match to {
            Some(at) => self.go(at),
            None => self.message = Some("nothing before this step".into()),
        }
    }

    /// Leave setup for the first screen, saving as Settings does.
    fn leave_setup(&mut self) -> Vec<HostEffect> {
        self.setup = None;
        self.view = self.first_screen().map_or(View::Settings, View::Screen);
        self.focus = Pane::Middle;
        self.settings_detail.top();
        vec![self.save(false)]
    }

    /// `f`: leave setup at this step, keeping every answer so far. The
    /// status line says once where it paused and how to go on; a part not
    /// set up says so on its own page (`s`).
    fn finish_later(&mut self) -> Vec<HostEffect> {
        let paused = self.setup_step_now().map(|s| self.setup_label(s).0);
        let fx = self.leave_setup();
        if let Some(label) = paused {
            self.message = Some(format!("setup paused at {label} · radar setup, or s on a part not set up, goes on").into());
        }
        fx
    }

    /// The scan's answer.
    pub(crate) fn setup_scanned(&mut self, found: Vec<Found>) {
        let Some(setup) = self.setup.as_mut() else { return };
        if setup.scan != Scan::Looking {
            return;
        }
        setup.scan = Scan::Found(found.clone());
        if !setup.touched {
            apply_found(self, &found);
        }
    }

    /// What the scan may look for: each part's places, when it has any.
    fn asks(&self) -> Asks {
        self.plugins
            .iter()
            .map(|p| (p.id().to_string(), p.setup_finds()))
            .filter(|(_, f)| !f.env.is_empty() || !f.files.is_empty())
            .collect()
    }

    /// What the scan found for part `p`.
    fn found_for(&self, p: usize) -> Option<&Found> {
        let Scan::Found(found) = &self.setup.as_ref()?.scan else { return None };
        found.iter().find(|f| f.part == self.plugins[p].id())
    }

    // --- keys ------------------------------------------------------------

    /// A key while setup shows, before the host's own keys: `Some` when
    /// setup took it, `None` to let the host and the section have it.
    pub(crate) fn setup_key(&mut self, key: KeyEvent) -> Option<Vec<HostEffect>> {
        let setup = self.setup.as_ref()?;
        if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
            return None;
        }
        let at = setup.at;
        // Never the panes, the tabs, history or another screen: setup has
        // one place for the keys.
        let elsewhere = match key.code {
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Right => true,
            KeyCode::Char(c) => {
                matches!(c, 'h' | 'l' | 'b' | 'z' | 'w' | crate::host::SETTINGS_KEY | crate::host::HISTORY_KEY)
                    || c.is_ascii_digit()
                    || c == crate::dock::KEY
                    || self.claimed_capitals().contains(&c)
            }
            _ => false,
        };
        if elsewhere {
            return Some(vec![]);
        }
        if self.setup_on_section() {
            return self.setup_section_key(key);
        }
        let look = self.on_look().then(|| self.look_rows().len());
        if look.is_some() && key.code == KeyCode::Char(' ') {
            let row = self.setup.as_ref()?.row;
            self.look_choose(row);
            return Some(vec![]);
        }
        let mode = self.on_mode().then_some(crate::writes::Mode::ALL.len());
        if mode.is_some() && matches!(key.code, KeyCode::Char(' ') | KeyCode::Enter) {
            let row = self.setup.as_ref()?.row;
            let chosen = self.setup.as_ref()?.mode.is_some();
            self.mode_choose(row);
            // ⏎ goes on once a mode is chosen (choosing it now if none was).
            if key.code == KeyCode::Enter && (chosen || self.setup.as_ref()?.mode.is_some()) {
                self.advance(Mark::Done);
            }
            return Some(vec![]);
        }
        let look = look.or(mode);
        let setup = self.setup.as_mut()?;
        let choosing = at == At::Choose;
        let rows = match at {
            At::Welcome | At::Scan | At::Ready => 2,
            At::Choose => setup.choices.len(),
            At::Step(_) => look.unwrap_or(0),
        };
        match key.code {
            KeyCode::Char('q') => return None,
            KeyCode::Up | KeyCode::Char('k') => setup.row = setup.row.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => setup.row = (setup.row + 1).min(rows.saturating_sub(1)),
            KeyCode::Home | KeyCode::Char('g') => setup.row = 0,
            KeyCode::End | KeyCode::Char('G') => setup.row = rows.saturating_sub(1),
            KeyCode::Char(' ') if choosing => {
                let row = setup.row;
                if let Some(c) = setup.choices.get_mut(row) {
                    c.ticked = !c.ticked;
                    setup.touched = true;
                }
            }
            KeyCode::Char(c @ ('a' | 'n')) if choosing => {
                for choice in &mut setup.choices {
                    choice.ticked = c == 'a';
                }
                setup.touched = true;
            }
            KeyCode::Left => self.setup_back(),
            KeyCode::Enter => return Some(self.setup_enter()),
            KeyCode::Esc => return Some(self.setup_esc()),
            KeyCode::Char('f') if matches!(at, At::Step(_)) => return Some(self.finish_later()),
            KeyCode::Char('y') if matches!(at, At::Scan | At::Ready) => {
                return Some(vec![HostEffect::Copy("radar setup".into())]);
            }
            KeyCode::PageUp | KeyCode::PageDown => return None,
            _ => {}
        }
        Some(vec![])
    }

    /// A key on a step that is a settings section: `⏎` goes on, `esc`
    /// skips, `←` goes back, `space` chooses (the section's own space, else
    /// its `⏎`); everything else is the section's, then `f` finishes later.
    fn setup_section_key(&mut self, key: KeyEvent) -> Option<Vec<HostEffect>> {
        match key.code {
            KeyCode::Char('q' | 'x' | '?') | KeyCode::PageUp | KeyCode::PageDown => return None,
            KeyCode::Enter => {
                self.advance(Mark::Done);
                return Some(vec![]);
            }
            KeyCode::Esc => {
                self.advance(Mark::Skipped);
                return Some(vec![]);
            }
            KeyCode::Left => {
                self.setup_back();
                return Some(vec![]);
            }
            _ => {}
        }
        if let Some(fx) = self.setup_section_takes(key) {
            return Some(fx);
        }
        if key.code == KeyCode::Char(' ') {
            let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
            return Some(self.setup_section_takes(enter).unwrap_or_default());
        }
        if key.code == KeyCode::Char('f') {
            return Some(self.finish_later());
        }
        Some(vec![])
    }

    /// A key for the section the step is: the colours' rows, or the part's.
    /// `None` when it did not take it.
    pub(crate) fn setup_section_takes(&mut self, key: KeyEvent) -> Option<Vec<HostEffect>> {
        match self.current_section()? {
            (Owner::Host, _) => self.colours_take(key).then(Vec::new),
            (Owner::Plugin(_), _) => self.deliver(key),
            _ => None,
        }
    }

    fn setup_enter(&mut self) -> Vec<HostEffect> {
        let asks = self.asks();
        let Some(setup) = self.setup.as_mut() else { return vec![] };
        match setup.at {
            At::Welcome if setup.row == 0 => {
                self.go(At::Scan);
                vec![]
            }
            At::Welcome => self.leave_setup(),
            At::Scan if setup.row == 0 => {
                setup.scan = Scan::Looking;
                for p in &mut self.plugins {
                    p.setup_looked();
                }
                self.go(At::Choose);
                vec![HostEffect::Scan(asks)]
            }
            At::Scan => {
                setup.scan = Scan::Declined;
                self.go(At::Choose);
                vec![]
            }
            At::Choose => {
                // ⏎ on a row goes to that row's step: ticked if it was not,
                // the steps before it still reachable with ←.
                let row = setup.row;
                let here = setup.choices.get_mut(row).map(|c| {
                    c.ticked = true;
                    c.step
                });
                setup.plan = plan_of(&setup.choices);
                setup.plan.push(Step::Mode);
                setup.marks = vec![Mark::Upcoming; setup.plan.len()];
                let at = here.and_then(|h| setup.plan.iter().position(|s| *s == h)).unwrap_or(0);
                let to = At::Step(at);
                self.go(to);
                vec![]
            }
            At::Step(_) => {
                self.advance(Mark::Done);
                vec![]
            }
            At::Ready => {
                let tour = setup.row == 0;
                let fx = self.leave_setup();
                if tour {
                    self.open_tour();
                }
                fx
            }
        }
    }

    fn setup_esc(&mut self) -> Vec<HostEffect> {
        let on_mode = self.on_mode();
        let Some(setup) = self.setup.as_mut() else { return vec![] };
        match setup.at {
            At::Scan => {
                setup.scan = Scan::Declined;
                self.go(At::Choose);
                vec![]
            }
            At::Welcome | At::Choose | At::Ready => self.leave_setup(),
            // The mode is not skipped: the user chooses it.
            At::Step(_) if on_mode => {
                self.message = Some("choose a mode: ↑↓ and ⏎".into());
                vec![]
            }
            At::Step(_) => {
                self.advance(Mark::Skipped);
                vec![]
            }
        }
    }

    /// A row of a host step pressed (`setup:<screen>:<n>`) or a stop of the
    /// rail activated (`chrome:setup:<n>`): a press selects the row (and
    /// ticks a chooser's row); activating a choice chooses it.
    pub(crate) fn setup_target(&mut self, target: &Target, activate: bool) -> Vec<HostEffect> {
        let on_mode = self.on_mode();
        let look = self.on_look().then(|| self.look_rows().len()).or(on_mode.then_some(crate::writes::Mode::ALL.len()));
        let Some(setup) = self.setup.as_mut() else { return vec![] };
        let Some(rest) = target.as_str().strip_prefix(PREFIX) else { return vec![] };
        let Some((screen, n)) = rest.split_once(':') else { return vec![] };
        let Ok(n) = n.parse::<usize>() else { return vec![] };
        let here = match setup.at {
            At::Welcome => "welcome",
            At::Scan => "scan",
            At::Choose => "choose",
            At::Ready => "ready",
            At::Step(_) if on_mode => "mode",
            At::Step(_) if look.is_some() => "look",
            At::Step(_) => "",
        };
        if screen != here {
            return vec![];
        }
        let rows = match (setup.at, look) {
            (At::Choose, _) => setup.choices.len(),
            (At::Step(_), Some(n)) => n,
            _ => 2,
        };
        if n >= rows {
            return vec![];
        }
        setup.row = n;
        // A move clears the status line's message, as a key does.
        if !activate {
            self.message = None;
        }
        let Some(setup) = self.setup.as_mut() else { return vec![] };
        if look.is_some() {
            if activate {
                if on_mode {
                    self.mode_choose(n);
                } else {
                    self.look_choose(n);
                }
            }
            return vec![];
        }
        if setup.at == At::Choose {
            if activate && let Some(c) = setup.choices.get_mut(n) {
                c.ticked = !c.ticked;
                setup.touched = true;
            }
            return vec![];
        }
        if activate { self.setup_enter() } else { vec![] }
    }

    /// The rail's stop `i` activated: back to a step reached before.
    pub(crate) fn setup_stop(&mut self, i: usize) {
        let Some(setup) = self.setup.as_ref() else { return };
        let reached = match setup.at {
            At::Step(now) => i <= now || setup.marks.get(i).is_some_and(|m| *m != Mark::Upcoming),
            At::Ready => i < setup.plan.len(),
            _ => false,
        };
        if reached && i < setup.plan.len() {
            self.go(At::Step(i));
        }
    }

    /// The key bar while setup shows, unless a menu or a field has it.
    pub(crate) fn setup_hints(&self) -> Option<Vec<KeyHint<'static>>> {
        let setup = self.setup.as_ref()?;
        let mut h = match setup.at {
            At::Welcome => vec![
                KeyHint::new("⏎", if setup.row == 0 { "set it up" } else { "not now" }),
                KeyHint::new("↑↓", "move"),
                KeyHint::new("esc", "not now"),
            ],
            At::Scan => vec![
                KeyHint::new("⏎", "continue"),
                KeyHint::new("↑↓", "move"),
                KeyHint::new("esc", "no thanks"),
                KeyHint::new("y", "copy"),
            ],
            At::Choose => vec![
                KeyHint::new("⏎", "start here"),
                KeyHint::new("space", "tick"),
                KeyHint::new("a", "all"),
                KeyHint::new("n", "none"),
                KeyHint::new("esc", "not now"),
                KeyHint::new("←", "back"),
            ],
            At::Step(_) if self.on_mode() => vec![
                KeyHint::new("⏎", "choose and continue"),
                KeyHint::new("↑↓", "move"),
                KeyHint::new("←", "back"),
            ],
            At::Step(_) => {
                let mut h = vec![KeyHint::new("⏎", "continue")];
                if self.on_look() || self.on_mode() {
                    h.push(KeyHint::new("↑↓", "move"));
                }
                if self.setup_on_section() || self.on_look() || self.on_mode() {
                    h.push(KeyHint::new("space", "choose"));
                }
                h.extend([
                    KeyHint::new("esc", "skip this"),
                    KeyHint::new("←", "back"),
                    KeyHint::new("f", "finish later"),
                ]);
                h
            }
            At::Ready => vec![
                KeyHint::new("⏎", if setup.row == 0 { "yes, a tour" } else { "no thanks" }),
                KeyHint::new("↑↓", "move"),
                KeyHint::new("←", "back"),
                KeyHint::new("y", "copy"),
            ],
        };
        if self.settings_detail.more() {
            h.push(KeyHint::new("PgUp/PgDn", "scroll"));
        }
        Some(h)
    }

    // --- view data -------------------------------------------------------

    /// Setup's chrome: the title, `step n of m`, and the card's heading and
    /// lead line. No rail: progress is `step n of m` (a rail that redrew as
    /// ticks changed only flickered).
    pub fn setup_chrome(&self) -> Option<SetupChrome> {
        let setup = self.setup.as_ref()?;
        let n = setup.plan.len();
        let (title, step, heading, lead): (String, Option<String>, &str, &str) = match setup.at {
            At::Welcome => (
                "welcome".into(),
                None,
                "Let's get you set up.",
                "",
            ),
            At::Scan => (
                "look for keys?".into(),
                None,
                "May radar look for keys on this machine?",
                "If you have tokens or sign-ins already, radar can use them instead of you pasting them.",
            ),
            At::Choose => (
                "choose".into(),
                None,
                "What would you like to set up?",
                "Tick the ones you use. Each one ticked gets a short step, in this order.",
            ),
            At::Step(now) => {
                let s = setup.plan[now];
                let (heading, lead) = match s {
                    Step::Look => ("How radar should look in this terminal.", "radar measured the terminal; pick what reads best."),
                    Step::Mode => ("What may radar send?", "Choose one. Every write is shown and asked before it is sent, in any mode."),
                    Step::Local if self.local_part().is_none() => ("Where your code lives on this machine.", "The parts that read this machine."),
                    Step::Local => self.local_part().and_then(|p| self.part_step(p)).map_or(("", ""), |p| (p.heading, p.lead)),
                    Step::Part(p) => self.part_step(p).map_or(("", ""), |p| (p.heading, p.lead)),
                };
                (self.setup_label(s).0, Some(format!("step {} of {n}", now + 1)), heading, lead)
            }
            At::Ready => ("ready".into(), None, "Setup is done.", "This is what radar has now."),
        };
        Some(SetupChrome { title, step, rail: vec![], current: None, heading: heading.into(), lead: lead.into() })
    }

    /// The card and the sheet of a host step, as a section's view; `None`
    /// off setup or on a step that is a settings section (the section's own
    /// view is sent then).
    pub fn setup_section_view(&self) -> Option<SectionView> {
        let setup = self.setup.as_ref()?;
        if self.setup_on_section() {
            return None;
        }
        Some(match setup.at {
            At::Welcome => self.welcome_view(setup),
            At::Scan => self.scan_view(setup),
            At::Choose => self.choose_view(setup),
            At::Ready => self.ready_view(setup),
            At::Step(_) if self.on_look() => self.look_view(setup),
            At::Step(_) if self.on_mode() => self.mode_view(setup),
            At::Step(_) => self.local_view(),
        })
    }

    // --- mode ---------------------------------------------------------------

    /// The mode step shows.
    fn on_mode(&self) -> bool {
        self.setup_step_now() == Some(Step::Mode)
    }

    /// Choose the mode step's row `i`: kept in the config as Settings' mode
    /// section keeps it.
    fn mode_choose(&mut self, i: usize) {
        let Some(m) = crate::writes::Mode::ALL.get(i).copied() else { return };
        self.on_mode_menu(m.key());
        // The card shows the choice; the status line keeps its keys.
        self.message = None;
        if let Some(setup) = self.setup.as_mut() {
            setup.mode = Some(m);
        }
    }

    fn mode_view(&self, setup: &Setup) -> SectionView {
        let mut rows = vec![];
        let w = crate::writes::Mode::ALL.iter().map(|m| crate::ui::width(m.word())).max().unwrap_or(0) + 3;
        for (i, m) in crate::writes::Mode::ALL.iter().enumerate() {
            rows.push(ListRow::new(
                format!("{PREFIX}mode:{i}"),
                Text::new()
                    .push(tick(setup.mode == Some(*m)))
                    .push(Seg::icon(m.icon()))
                    .push(Seg::new(crate::ui::pad(m.word(), w), Ink::Primary)),
            ));
        }
        let blocks = vec![
            Block::section_of(
                Some(Icon::Info),
                "what each allows",
                None,
                lines(&[
                    "Read only: radar reads and sends nothing. A write you start is shown, and not sent.",
                    "Write: radar may add comments, replies, reactions and new items.",
                    "Edit: radar may also approve, resolve, move, assign, rebase and merge.",
                ]),
            ),
            Block::section_of(
                Some(Icon::Settings),
                "change it later",
                None,
                lines(&["The mode is at the top right of every screen; a click on it, or Settings (,), changes it."]),
            ),
        ];
        sheet("setup.mode", Node::List { rows, selected: Some(setup.row), editing: None }, blocks)
    }

    // --- look & terminal --------------------------------------------------

    /// The look step shows.
    fn on_look(&self) -> bool {
        self.setup_step_now() == Some(Step::Look)
    }

    /// The look step's choices, in the card's order.
    fn look_rows(&self) -> Vec<LookRow> {
        let mut rows: Vec<LookRow> = LOOK_ICONS.iter().map(|(pack, _, _)| LookRow::Icons(pack)).collect();
        let mut palettes: Vec<String> =
            crate::theme::palettes().into_iter().filter(|p| p.starts_with("silkcircuit")).collect();
        if !palettes.contains(&self.colours.cfg.palette) {
            palettes.push(self.colours.cfg.palette.clone());
        }
        rows.extend(palettes.into_iter().map(LookRow::Palette));
        rows.extend([Level::Subtle, Level::Full, Level::Off].map(LookRow::Motion));
        rows
    }

    /// Choose the look step's row `i`: it applies at once.
    fn look_choose(&mut self, i: usize) {
        let Some(row) = self.look_rows().into_iter().nth(i) else { return };
        let (menu, value) = match &row {
            LookRow::Icons(pack) => ("icons", pack.to_string()),
            LookRow::Palette(p) => ("palette", p.clone()),
            LookRow::Motion(l) => ("motion", l.word().to_string()),
        };
        if let Some(m) = self.colours.on_menu(menu, &value) {
            self.message = Some(m.into());
        }
    }

    /// Whether the look step's row is the choice in use.
    fn look_on(&self, row: &LookRow) -> bool {
        let cfg = &self.colours.cfg;
        match row {
            LookRow::Icons(pack) => {
                let now = if cfg.icons == "auto" { crate::ui::icon::auto_choice() } else { cfg.icons.clone() };
                now == *pack
            }
            LookRow::Palette(p) => cfg.palette == *p,
            LookRow::Motion(l) => Level::parse(&cfg.motion) == *l,
        }
    }

    /// Settings' look section: setup's look card, with its own cursor.
    pub fn look_settings_view(&self) -> SectionView {
        let mut view = self.look_view_at(self.look_row);
        // In Settings already: no step, no "change it later".
        view.detail_title = "about".into();
        view.detail_blocks.retain(|b| !matches!(b, Block::Section { title, .. } if title == "CHANGE IT LATER"));
        view
    }

    /// A key on Settings' look section: move, or choose (space, ⏎).
    pub(crate) fn look_settings_key(&mut self, key: KeyEvent) -> bool {
        let n = self.look_rows().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.look_row = self.look_row.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.look_row = (self.look_row + 1).min(n.saturating_sub(1)),
            KeyCode::Home | KeyCode::Char('g') => self.look_row = 0,
            KeyCode::End | KeyCode::Char('G') => self.look_row = n.saturating_sub(1),
            KeyCode::Enter | KeyCode::Char(' ') => self.look_choose(self.look_row),
            _ => return false,
        }
        true
    }

    /// A row of Settings' look section pressed (`setup:look:<n>`).
    pub(crate) fn look_settings_target(&mut self, target: &Target, activate: bool) {
        if let Some(n) = target.as_str().strip_prefix("setup:look:").and_then(|n| n.parse::<usize>().ok())
            && n < self.look_rows().len()
        {
            self.look_row = n;
            if activate {
                self.look_choose(n);
            }
        }
    }

    fn look_view(&self, setup: &Setup) -> SectionView {
        self.look_view_at(setup.row)
    }

    fn look_view_at(&self, row_at: usize) -> SectionView {
        let user = crate::ui::icon::user_packs();
        let kv = |target: &str, key: &str, value: String| {
            ListRow::heading(
                target,
                Text::new().push(Seg::new(crate::ui::pad(key, 10), Ink::Muted)).push(Seg::new(value, Ink::Primary)),
            )
        };
        let terminal = self
            .terminal
            .as_ref()
            .map(|t| t.stack.description())
            .unwrap_or_else(|| crate::terminal_host::Stack::detect().description());
        let colours = match crate::design::theme::Tier::detect() {
            crate::design::theme::Tier::TrueColor => "truecolor",
            crate::design::theme::Tier::Ansi256 => "256 colours",
            crate::design::theme::Tier::Ansi16 => "16 colours",
            crate::design::theme::Tier::Mono => "no colour",
        };
        let mut rows = vec![
            heading("setup:h:terminal", "terminal"),
            kv("setup:h:term", "terminal", terminal),
            kv("setup:h:colours", "colours", colours.into()),
        ];
        let mut selected = None;
        let mut group = "";
        let default_palette = crate::theme::ThemeConfig::default().palette;
        for (i, row) in self.look_rows().iter().enumerate() {
            let (g, words, sample, about) = match row {
                LookRow::Icons(pack) => {
                    let (_, label, about) = LOOK_ICONS.iter().find(|(p, _, _)| p == pack).copied().unwrap_or((pack, pack, ""));
                    ("icons", label.to_string(), crate::ui::icon::preview(pack, &user), about.to_string())
                }
                LookRow::Palette(p) => {
                    let about = if *p == default_palette { "default" } else { "" };
                    ("theme", p.replace('-', " "), String::new(), about.to_string())
                }
                LookRow::Motion(l) => ("motion", l.word().to_string(), String::new(), l.about().to_string()),
            };
            if g != group {
                rows.push(blank(rows.len()));
                rows.push(heading(&format!("setup:h:{g}"), g));
                group = g;
            }
            if row_at == i {
                selected = Some(rows.len());
            }
            let mut text = Text::new().push(tick(self.look_on(row))).push(Seg::new(crate::ui::pad(&words, 20), Ink::Primary));
            if !sample.is_empty() {
                text.add(Seg::new(sample, Ink::Secondary));
            }
            let mut r = ListRow::new(format!("{PREFIX}look:{i}"), text);
            if !about.is_empty() {
                r.right = Some(Text::of(about, Ink::Muted));
            }
            rows.push(r);
        }
        rows.push(blank(rows.len()));
        rows.push(heading("setup:h:preview", "preview"));
        let preview = [
            (Icon::Change, "!264", "Tidy the build", Icon::RunPassed, "passed", Ink::Good, "2h"),
            (Icon::Change, "!263", "Add a stats endpoint", Icon::Thread, "1 thread open", Ink::Warn, "25m"),
            (Icon::Change, "!118", "Update dependencies", Icon::RunFailed, "pipeline failed", Ink::Bad, "3d"),
        ];
        for (n, (icon, id, title, mark, state, ink, age)) in preview.into_iter().enumerate() {
            let branch = if n == 2 { "└─ " } else { "├─ " };
            let text = Text::new()
                .push(Seg::new(branch, Ink::Muted))
                .push(Seg::icon(icon))
                .push(Seg::new(format!("{id} "), Ink::Number))
                .push(Seg::new(crate::ui::pad(title, 22), Ink::Primary))
                .push(Seg::icon(mark))
                .push(Seg::new(state, ink));
            let mut r = ListRow::heading(format!("setup:h:preview:{n}"), text);
            r.right = Some(Text::of(age, Ink::Muted));
            rows.push(r);
        }
        let blocks = vec![
            Block::section_of(
                Some(Icon::Info),
                "icons show as boxes?",
                None,
                lines(&["Your font lacks them. Choose plain, or install a Nerd Font, set it in your terminal and come back."]),
            ),
            Block::section(Some(Icon::Link), "nerd fonts", None, vec![Block::line(
                Text::new().push(Seg::new(NERD_FONTS, Ink::Accent).linked(format!("https://{NERD_FONTS}"))),
            )]),
            Block::section_of(Some(Icon::Settings), "change it later", None, lines(&["Settings (,) / general / appearance."])),
        ];
        sheet("setup.look", Node::List { rows, selected, editing: None }, blocks)
    }

    /// Before anything is asked: a short hello, what radar does, and set it
    /// up now or not now.
    fn welcome_view(&self, setup: &Setup) -> SectionView {
        let mut rows = vec![];
        let choices = [("yes, let's set it up", "a few minutes"), ("not now", "Settings (,) has every step")];
        for (i, (words, note)) in choices.into_iter().enumerate() {
            rows.push(ListRow::new(
                format!("{PREFIX}welcome:{i}"),
                Text::new()
                    .push(tick(setup.row == i))
                    .push(Seg::new(crate::ui::pad(words, 24), Ink::Primary))
                    .push(Seg::new(note, Ink::Muted)),
            ));
        }
        let blocks = vec![
            Block::line(Text::new().push(Seg::new("Welcome to radar.", Ink::Emphasis).strong())),
            Block::line(Text::new()),
            Block::line(Text::of("radar shows the work from the services you use together, linked to each", Ink::Primary)),
            Block::line(Text::of("other and to the code on this machine.", Ink::Primary)),
            Block::line(Text::new()),
            Block::line(Text::of("Nothing is sent to a service until you confirm it.", Ink::Secondary)),
            Block::line(Text::of("You pick the services you use, and each one gets a short step.", Ink::Secondary)),
            Block::line(Text::of("esc skips a step. f stops setup, and radar setup carries on later.", Ink::Secondary)),
        ];
        let body = Node::List { rows, selected: Some(setup.row), editing: None };
        let mut view = sheet("setup.welcome", body, blocks);
        view.detail_title = "about radar".into();
        view
    }

    fn scan_view(&self, setup: &Setup) -> SectionView {
        let asks = self.asks();
        let env: Vec<String> = asks
            .iter()
            .flat_map(|(_, f)| f.env.iter().map(|e| if e.ends_with('_') { format!("{e}…") } else { e.to_string() }))
            .collect();
        // A program may have a sign-in file in more than one place: named once.
        let mut programs: Vec<&str> = Vec::new();
        for p in asks.iter().flat_map(|(_, f)| f.files.iter().map(|(p, _)| *p)) {
            if !programs.contains(&p) {
                programs.push(p);
            }
        }
        let list = |words: Vec<String>| if words.is_empty() { "none".to_string() } else { words.join(", ") };
        let kv = |target: &str, key: &str, value: String| {
            ListRow::heading(
                target,
                Text::new().push(Seg::new(crate::ui::pad(key, 10), Ink::Muted)).push(Seg::new(value, Ink::Primary)),
            )
        };
        let mut rows = vec![
            heading("setup:h:where", "where it looks"),
            kv("setup:h:env", "variables", list(env)),
            kv("setup:h:files", "sign-ins", list(programs.iter().map(|p| p.to_string()).collect())),
            blank(0),
            heading("setup:h:choice", "your choice"),
        ];
        let first = rows.len();
        for (i, words) in ["yes, look", "no thanks"].into_iter().enumerate() {
            rows.push(ListRow::new(
                format!("{PREFIX}scan:{i}"),
                Text::new().push(tick(setup.row == i)).push(Seg::new(words, Ink::Primary)),
            ));
        }
        let blocks = vec![
            Block::section_of(
                Some(Icon::Search),
                "what it reads",
                None,
                lines(&[
                    "The names of the variables listed, and whether the sign-in files listed exist.",
                    "It does not read a token's value or a file's contents.",
                    "Nothing is sent anywhere, and nothing is saved until you finish a step.",
                ]),
            ),
            Block::section_of(
                Some(Icon::Settings),
                "if you say no",
                None,
                lines(&["Nothing is ticked. Each service's step asks for a token instead."]),
            ),
            Block::section_of(
                Some(Icon::Updated),
                "next time",
                None,
                lines(&["radar asks again each time setup runs."]),
            ),
        ];
        let body = Node::List { rows, selected: Some(first + setup.row), editing: None };
        sheet("setup.scan", body, blocks)
    }

    fn choose_view(&self, setup: &Setup) -> SectionView {
        let icons = crate::ui::icon::current();
        let option = |c: &Choice| match (c.step, c.option) {
            (Step::Part(p), Some(i)) => self.part_step(p).and_then(|s| s.options.get(i).cloned()),
            _ => None,
        };
        let label_w = setup
            .choices
            .iter()
            .map(|c| crate::ui::width(&option(c).map_or_else(|| self.setup_label(c.step).0, |o| o.label)))
            .max()
            .unwrap_or(0)
            + 2;
        // The look has its own heading at the end: optional, radar works
        // as it is.
        let group_of = |step: Step| match step {
            Step::Look | Step::Mode => None,
            Step::Local => Some(SetupGroup::Machine),
            Step::Part(p) => Some(self.part_step(p).map_or(SetupGroup::Service, |s| s.group)),
        };
        let mut rows = Vec::new();
        let mut selected = None;
        let mut shown = None;
        for (i, choice) in setup.choices.iter().enumerate() {
            let group = group_of(choice.step);
            if shown != Some(group) {
                if shown.is_some() {
                    rows.push(blank(rows.len()));
                }
                let words = match group {
                    Some(SetupGroup::Machine) => "this machine",
                    Some(SetupGroup::Service) => "services",
                    Some(SetupGroup::Agent) => "agents",
                    None => "optional",
                };
                rows.push(heading(&format!("setup:h:{words}"), words));
                shown = Some(group);
            }
            let own = option(choice);
            let (label, icon) = match &own {
                Some(o) => (o.label.clone(), o.icon),
                None => self.setup_label(choice.step),
            };
            let mut text = Text::new().push(tick(choice.ticked));
            let mark = match (choice.step, &own) {
                (Step::Part(p), None) => self.plugins[p].health().map(|h| h.mark.in_pack(&icons)).filter(|m| !m.is_empty()),
                _ => None,
            };
            match (mark, icon) {
                (Some(mark), _) => text.add(Seg::new(format!("{mark} "), Ink::Secondary)),
                (None, Some(icon)) => text.add(Seg::icon(icon)),
                (None, None) => {}
            }
            text.add(Seg::new(crate::ui::pad(&label, label_w), Ink::Primary));
            if own.is_none() {
                text.add(Seg::new(self.step_about(choice.step), Ink::Muted));
            }
            let found = match (choice.step, &setup.scan) {
                _ if let Some(o) = &own => o.note.clone().map(|n| Text::of(n, if o.ready { Ink::Good } else { Ink::Muted })),
                (Step::Part(_), Scan::Looking) => Some(Text::of("looking…", Ink::Muted)),
                (Step::Part(p), _) => self.found_for(p).and_then(Found::words).map(|w| Text::of(w, Ink::Good)),
                _ => None,
            };
            if setup.row == i {
                selected = Some(rows.len());
            }
            let mut row = ListRow::new(format!("{PREFIX}choose:{i}"), text);
            row.right = found;
            rows.push(row);
        }
        if rows.is_empty() {
            rows.push(heading("setup:h:none", "nothing to set up in this build"));
        }
        let why: &[&str] = match &setup.scan {
            Scan::Found(_) => &["radar found a token or a sign-in for each service ticked."],
            Scan::Looking => &["Looking for tokens and sign-ins…"],
            Scan::Declined | Scan::NotAsked => &["Nothing was looked for, so nothing is ticked."],
        };
        let blocks = vec![
            Block::section_of(Some(Icon::Search), "ticked", None, lines(why)),
            Block::section_of(
                Some(Icon::Navigate),
                "keys",
                None,
                lines(&[
                    "⏎ starts at the row you are on. esc in a step skips it, and ← goes back one.",
                    "Anything you leave unticked can be set up later in Settings (,).",
                ]),
            ),
        ];
        sheet("setup.choose", Node::List { rows, selected, editing: None }, blocks)
    }

    fn ready_view(&self, setup: &Setup) -> SectionView {
        let label_w = setup.plan.iter().map(|s| crate::ui::width(&self.setup_label(*s).0)).max().unwrap_or(0) + 3;
        let mut rows = vec![heading("setup:h:setup", "setup")];
        for (i, (step, mark)) in setup.plan.iter().zip(&setup.marks).enumerate() {
            let (label, icon) = self.setup_label(*step);
            let mut text = Text::new();
            if let Some(icon) = icon {
                text.add(Seg::icon(icon));
            }
            text.add(Seg::new(crate::ui::pad(&label, label_w), Ink::Primary));
            let (words, ink) = match (mark, self.setup_summary(*step)) {
                (Mark::Skipped, _) => ("skipped".to_string(), Ink::Muted),
                (_, Some(s)) => (s, Ink::Secondary),
                (_, None) => ("not set up".to_string(), Ink::Muted),
            };
            text.add(Seg::new(words, ink));
            rows.push(ListRow::heading(format!("setup:h:step:{i}"), text));
        }
        if setup.plan.is_empty() {
            rows.push(ListRow::heading("setup:h:nothing", Text::of("nothing chosen", Ink::Muted)));
        }
        let mode = self.mode();
        let key = |words: &str| Seg::new(crate::ui::pad(&words.to_uppercase(), 10), Ink::Muted).strong();
        let place = self.config_place.clone().unwrap_or_else(|| "config.toml".into());
        rows.extend([
            blank(0),
            ListRow::heading("setup:h:saved", Text::new().push(key("saved in")).push(Seg::new(place, Ink::Primary))),
            ListRow::heading(
                "setup:h:mode",
                Text::new().push(key("mode")).push(Seg::icon(mode.icon())).push(Seg::new(mode.word(), Ink::Info).strong()),
            ),
            blank(1),
            heading("setup:h:tour", "a quick tour?"),
        ]);
        let first = rows.len();
        for (i, words) in ["yes, show me around", "no thanks"].into_iter().enumerate() {
            rows.push(ListRow::new(
                format!("{PREFIX}ready:{i}"),
                Text::new().push(tick(setup.row == i)).push(Seg::new(words, Ink::Primary)),
            ));
        }
        let blocks = vec![
            Block::section_of(
                Some(Icon::File),
                "saved",
                None,
                lines(&[
                    "In the config file shown below. A token is saved as the name of the variable that holds it.",
                    "radar config path prints where the file is.",
                ]),
            ),
            Block::section(Some(Icon::Settings), "change it later", None, vec![
                Block::line(Text::of("Settings (,) has every step's fields. To run setup again:", Ink::Secondary)),
                command("setup:copy", "radar setup"),
            ]),
        ];
        let body = Node::List { rows, selected: Some(first + setup.row), editing: None };
        sheet("setup.ready", body, blocks)
    }

    /// Local code with no part offering it: the parts that read this
    /// machine, and whether each is set up.
    fn local_view(&self) -> SectionView {
        let parts = self.local_parts();
        let body = if parts.is_empty() {
            Node::State { state: State::Empty, text: "no part in this build reads this machine".into() }
        } else {
            let w = parts.iter().map(|&p| crate::ui::width(self.plugins[p].name())).max().unwrap_or(0) + 3;
            let mut rows = vec![heading("setup:h:local", "on this machine")];
            for &p in &parts {
                let (mark, words) = if self.ready(p) { (tick(true), "set up") } else { (tick(false), "not set up") };
                rows.push(ListRow::heading(
                    format!("setup:h:local:{p}"),
                    Text::new()
                        .push(mark)
                        .push(Seg::new(crate::ui::pad(self.plugins[p].name(), w), Ink::Primary))
                        .push(Seg::new(words, Ink::Muted)),
                ));
            }
            Node::List { rows, selected: None, editing: None }
        };
        let blocks = vec![Block::section_of(
            Some(Icon::Settings),
            "change it later",
            None,
            lines(&["each part's own section in Settings (,)"]),
        )];
        sheet("setup.local", body, blocks)
    }
}

/// A section shown as a setup step: its sheet is the step's help (the
/// sections a part puts after the selected row's own detail), not the
/// Settings detail of that row; a row with no help keeps its detail.
pub(crate) fn help_only(view: &mut SectionView) {
    if let Some(first) = view.detail_blocks.iter().position(|b| matches!(b, Block::Section { .. })) {
        view.detail_blocks.drain(..first);
        view.detail_title = "about this step".into();
    }
}

/// A section shown as a setup step: its help's `⏎ …` lines (`⏎ uses it`)
/// say `space …`, the key that presses the section's ⏎ there.
pub(crate) fn space_for_enter(view: &mut SectionView) {
    fn text(t: &mut Text) {
        if let Some(first) = t.0.first_mut()
            && let Some(rest) = first.text.strip_prefix("⏎ ")
        {
            first.text = format!("space {rest}");
        }
    }
    fn blocks(bs: &mut [Block]) {
        for b in bs {
            match b {
                Block::Line { text: t, .. } => text(t),
                Block::Section { blocks: inner, .. } => blocks(inner),
                _ => {}
            }
        }
    }
    view.detail.iter_mut().for_each(text);
    blocks(&mut view.detail_blocks);
}

/// Tick the chooser's rows the scan found something for.
fn apply_found(host: &mut Host, found: &[Found]) {
    let ids: Vec<(usize, String)> = (0..host.plugins.len()).map(|p| (p, host.plugins[p].id().to_string())).collect();
    let ready: Vec<(usize, Vec<bool>)> = (0..host.plugins.len())
        .map(|p| (p, host.plugins[p].setup_step().map(|s| s.options.iter().map(|o| o.ready).collect()).unwrap_or_default()))
        .collect();
    let Some(setup) = host.setup.as_mut() else { return };
    for choice in &mut setup.choices {
        if let Step::Part(p) = choice.step
            && let Some((_, id)) = ids.iter().find(|(i, _)| *i == p)
            && found.iter().any(|f| f.part == *id && !f.is_empty())
        {
            // A step's own row is ticked when it is ready to use.
            choice.ticked = match choice.option {
                None => true,
                Some(i) => ready.iter().find(|(q, _)| *q == p).is_some_and(|(_, r)| r.get(i) == Some(&true)),
            };
        }
    }
}
