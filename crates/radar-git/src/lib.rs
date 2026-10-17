//! radar-git: git on this machine, for any host, as a radar plugin.
//!
//! It finds the checkouts under the folders in `[git] roots` and matches
//! each to the repo its `origin` names (`local.rs`), and adds to the repos
//! and merge requests other parts draw (`join.rs`): the line under their
//! title saying where they are checked out, and the actions an editor on
//! it, a terminal in it, the merge request's branch checked out, look
//! again. Settings sections `checkouts`, `terminal`, `editors`, `links` and `branches` (`settings.rs`); the
//! `[git]` config table; the index in its cache.

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

mod source_navigation;
mod act;
mod agent_context;
mod branch;
mod demo;
mod join;
mod local;
mod ops;
mod prune;
mod review_worktree;
mod scoped;
mod screen;
mod settings;
mod setup;
mod terminal;

use radar_core::design::help_bar::KeyHint;
use radar_core::plugin::{Effect, MenuItem, Msg, Outcome, Pane, ScreenSpec, SectionSpec};
use radar_core::{Adds, Contribution, Plugin, Point, Subject};
use serde_json::Value;

/// The git part: its `[git]` table, and the checkouts it knows.
#[derive(Debug, Default)]
pub struct GitPlugin {
    pub(crate) cfg: local::Local,
    pub(crate) local: local::State,
    pub(crate) act: act::State,
    /// The Branches screen: what was read from each clone.
    pub(crate) branches: screen::Screen,
    pub(crate) now: i64,
    pub(crate) scoped: scoped::State,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fixture: Option<test_support::LocalCheckoutFixture>,
}

impl GitPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    /// Offer these folders holding checkouts (with how many), as if the
    /// walk of the home folder had found them (it never runs under
    /// `RADAR_SANDBOX`): for samples and tests.
    #[cfg(feature = "demo")]
    pub fn offer_places(&mut self, places: Vec<(String, usize)>) {
        #[cfg(any(test, feature = "test-support"))]
        if self.fixture_denied("offered discovery locations") {
            return;
        }
        local::walked(self, places);
    }

    #[cfg(test)]
    fn section_key(&mut self, section: &str, key: radar_core::ratatui::crossterm::event::KeyEvent) -> radar_core::plugin::Outcome {
        radar_core::view_tui::chord_of(&key).map_or(radar_core::plugin::Outcome::Ignored, |chord| {
            settings::intent(self, section, &radar_core::view::Intent::Key { chord })
        })
    }
}

impl Plugin for GitPlugin {
    fn id(&self) -> &'static str {
        "git"
    }

    fn name(&self) -> &'static str {
        "Git"
    }

    fn configure(&mut self, table: Option<&toml::Value>) -> anyhow::Result<()> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(fixture) = &self.fixture {
            fixture.configure(table)?;
            return Ok(());
        }
        if self.scoped.invalidate() {
            self.local.scanning = false;
        }
        self.cfg = match table {
            Some(t) => t.clone().try_into()?,
            None => local::Local::default(),
        };
        self.cfg.depth = self.cfg.depth.clamp(1, local::DEPTH_MAX);
        Ok(())
    }

    /// The table, left out while it is the default.
    fn config(&self) -> Option<toml::Value> {
        (!self.cfg.is_default())
            .then(|| toml::Value::try_from(&self.cfg).ok())
            .flatten()
    }

    #[cfg(feature = "demo")]
    fn demo(&mut self) {
        #[cfg(any(test, feature = "test-support"))]
        if self.fixture.is_some() {
            return;
        }
        *self = demo::branches();
    }

    fn restore(&mut self, cache: Value) {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(fixture) = &self.fixture
            && !fixture.accept_cache(&cache)
        {
            return;
        }
        if let Ok(index) = serde_json::from_value(cache["index"].clone()) {
            self.local.index = Some(index);
        }
        if let Ok(used) = serde_json::from_value(cache["use"].clone()) {
            self.local.use_worktree = used;
        }
    }

    fn snapshot(&self) -> Option<Value> {
        Some(serde_json::json!({ "index": self.local.index, "use": self.local.use_worktree }))
    }

    fn tick(&mut self, now: i64) -> Vec<Effect> {
        self.now = now;
        let mut fx = local::tick(self);
        fx.extend(screen::tick(self));
        fx
    }

    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(fixture) = &self.fixture
            && !fixture.accept_message(msg.as_ref())
        {
            return vec![];
        }
        let msg = match msg.downcast::<scoped::Message>() {
            Ok(m) => return scoped::on_msg(self, *m),
            Err(m) => m,
        };
        let msg = match msg.downcast::<act::ActMsg>() {
            Ok(m) => return act::on_msg(self, *m),
            Err(m) => m,
        };
        let msg = match msg.downcast::<screen::BranchMsg>() {
            Ok(m) => return screen::on_msg(self, *m),
            Err(m) => m,
        };
        match msg.downcast::<local::LocalMsg>() {
            Ok(m) => local::on_msg(self, *m),
            Err(_) => vec![],
        }
    }

    fn on_bus(&mut self, topic: &str, payload: &Value) -> Vec<Effect> {
        #[cfg(any(test, feature = "test-support"))]
        if topic == act::START && self.fixture_denied("workspace start") {
            return vec![];
        }
        act::on_bus(self, topic, payload)
    }

    fn capturing(&self) -> bool {
        self.local.edit.is_some()
    }

    fn paste(&mut self, text: &str) -> Vec<Effect> {
        #[cfg(any(test, feature = "test-support"))]
        if self.fixture_denied("settings paste") {
            return vec![];
        }
        match self.local.edit.as_mut() {
            // Look down holds one digit: a pasted one takes its place.
            Some((settings::Field::Depth, input)) => {
                if let Some(c) = text.chars().rev().find(|c| settings::depth_digit(*c)) {
                    *input = radar_core::input::TextInput::new(&c.to_string());
                }
            }
            Some((_, input)) => input.paste(text),
            None => {}
        }
        vec![]
    }

    fn adds(&self) -> Vec<Adds> {
        join::ADDS.into_iter().chain(scoped::ADDS).collect()
    }

    fn offers(&self) -> Vec<radar_core::Offer> {
        vec![radar_core::Offer::new(Point::Column, "checkout", "the Branches tree")]
    }

    fn contribute(&self, point: Point, subject: &Subject) -> Option<Contribution> {
        scoped::contribute(self, point, subject).or_else(|| join::contribute(self, point, subject))
    }

    fn act(&mut self, subject: &Subject, action: &str) -> Vec<Effect> {
        #[cfg(any(test, feature = "test-support"))]
        if self.fixture_denied(action) {
            return vec![];
        }
        scoped::act(self, subject, action).unwrap_or_else(|| join::act(self, subject, action))
    }

    fn agent_context(
        &mut self,
        request: &radar_core::agent_context::AgentContextRequest,
    ) -> Option<radar_core::agent_context::AgentContext> {
        #[cfg(any(test, feature = "test-support"))]
        if self.fixture_denied("agent context / diff") {
            return None;
        }
        agent_context::agent_context(self, request)
    }

    fn ensure_worktree(
        &mut self,
        request: &radar_core::agent_context::WorktreeRequest,
    ) -> Option<(
        Result<radar_core::agent_context::WorktreeReady, radar_core::agent_context::WorktreeError>,
        Vec<Effect>,
    )> {
        #[cfg(any(test, feature = "test-support"))]
        if self.fixture_denied("ensure worktree") {
            return Some((Err(radar_core::agent_context::WorktreeError::Failed(
                "local fixture only permits checkout and Branches reads".into(),
            )), vec![]));
        }
        Some(agent_context::ensure_worktree(self, request))
    }

    fn typing(&self, menu: &str, text: &str) -> radar_core::Typing {
        #[cfg(any(test, feature = "test-support"))]
        if self.fixture_denied("folder / editor discovery") {
            return radar_core::Typing::default();
        }
        scoped::typing(self, menu, text).unwrap_or_else(|| act::typing(self, menu, text))
    }

    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        #[cfg(any(test, feature = "test-support"))]
        if self.fixture_denied("Git menu action") {
            return vec![];
        }
        if menu == "bulk" {
            return screen::on_bulk(self, value)
                .or_else(|| settings::on_bulk(self, value))
                .unwrap_or_default();
        }
        scoped::on_menu(self, menu, value)
            .or_else(|| screen::on_menu(self, menu, value))
            .or_else(|| join::on_menu(self, menu, value))
            .or_else(|| act::on_menu(self, menu, value))
            .or_else(|| settings::on_menu(self, menu, value))
            .unwrap_or_default()
    }

    // --- settings ------------------------------------------------------------

    fn sections(&self) -> Vec<SectionSpec> {
        settings::SECTIONS.to_vec()
    }
    fn setup_step(&self) -> Option<radar_core::plugin::SetupStep> {
        Some(setup::step(self))
    }

    /// The user let setup look: walk the home folder for the folders that
    /// hold checkouts, in the background, and offer them in the card.
    fn setup_looked(&mut self) {
        #[cfg(any(test, feature = "test-support"))]
        if self.fixture_denied("home discovery") {
            return;
        }
        local::want_home_walk(self);
    }

    fn section_icon(&self, section: &str) -> Option<radar_core::ui::icon::Icon> {
        use radar_core::ui::icon::Icon;
        Some(match section {
            "checkouts" => Icon::Local,
            "terminal" => Icon::Terminal,
            "editors" => Icon::Editor,
            "links" => Icon::Link,
            "branches" => Icon::Branch,
            _ => return None,
        })
    }

    fn section_hints(&self, _: &str) -> Vec<KeyHint<'static>> {
        settings::hints(self)
    }

    fn bulk(&self, scope: &str) -> Vec<MenuItem> {
        if scope == screen::SCREEN.id {
            screen::bulk(self)
        } else {
            settings::bulk(self, scope)
        }
    }

    // --- the Branches screen -------------------------------------------------

    /// A clone, as other parts may add columns to its branches' rows.
    fn subject_from_journal(&self, key: &radar_core::journal::SubjectKey) -> Option<radar_core::Subject> {
        source_navigation::from_journal(self, key)
    }

    fn related(&self, subject: &Subject) -> Vec<Subject> {
        scoped::related(self, subject)
    }

    fn show(&mut self, subject: &Subject) -> radar_core::Showing {
        source_navigation::show(self, subject)
    }

    fn subject(&self, scope: &str, pane: Pane) -> Option<Subject> {
        (scope == screen::SCREEN.id).then(|| screen::source_subject(self, pane)).flatten()
    }

    fn journal_subject(
        &self,
        subject: &radar_core::Subject,
    ) -> Option<radar_core::journal::SubjectSnapshot> {
        if !subject.is("checkout") {
            return None;
        }
        #[cfg(any(test, feature = "test-support"))]
        if let Some(fixture) = &self.fixture
            && subject.id != fixture.root().to_string_lossy()
        {
            fixture.blocked("checkout identity outside exact fixture root");
            return None;
        }
        let dir = local::journal_path(&subject.id)?.display().to_string();
        let key = radar_core::journal::SubjectKey::new("local", &dir, "checkout", &dir).ok()?;
        Some(radar_core::journal::SubjectSnapshot::new(
            key,
            radar_core::journal::SubjectRole::Primary,
        ))
    }

    fn owns(&self) -> Vec<radar_core::Owned> {
        vec![radar_core::Owned::page("checkout", "checkouts")]
    }

    fn screens(&self) -> Vec<ScreenSpec> {
        vec![screen::SCREEN]
    }

    /// Local git is one of the things radar brings together, though it syncs
    /// with no service.
    fn on_splash(&self) -> bool {
        true
    }

    fn icon(&self, _: &str) -> Option<radar_core::ui::icon::Icon> {
        Some(radar_core::ui::icon::Icon::Branch)
    }

    fn top_right(&self, _: &str) -> Option<String> {
        screen::top_right(self)
    }

    fn busy(&self, _: &str) -> bool {
        screen::busy(self)
    }

    fn status(&self, _: &str) -> Vec<(String, radar_core::Tone)> {
        screen::status(self)
    }

    fn view(&self, screen: &str, cx: &radar_core::view::ViewCx<'_>) -> Option<radar_core::view::ScreenView> {
        (screen == screen::SCREEN.id).then(|| screen::view(self, cx))
    }

    fn section_view(
        &self,
        section: &str,
        cx: &radar_core::view::ViewCx<'_>,
    ) -> Option<radar_core::view::SectionView> {
        #[cfg(any(test, feature = "test-support"))]
        if self.fixture_denied("Git settings discovery") {
            return None;
        }
        settings::section_view(self, section, cx)
    }

    fn intent(&mut self, scope: &str, pane: Pane, intent: &radar_core::view::Intent) -> Outcome {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(fixture) = &self.fixture
            && !fixture.accept_intent(scope, intent)
        {
            return Outcome::done();
        }
        if scope == screen::SCREEN.id {
            screen::intent(self, pane, intent)
        } else {
            settings::intent(self, scope, intent)
        }
    }

    fn panes(&self, _: &str) -> Vec<Pane> {
        screen::panes()
    }

    fn folds(&self, _: &str, pane: Pane) -> bool {
        screen::folds(pane)
    }

    fn row_hints(&self, _: &str, pane: Pane) -> Vec<KeyHint<'static>> {
        screen::row_hints(self, pane)
    }

    #[cfg(any(test, feature = "test-support"))]
    fn run_cli(&self, _: &clap::ArgMatches) -> radar_core::plugin::CliTask {
        let blocked = self.fixture_denied("Git CLI");
        Box::pin(async move {
            anyhow::ensure!(!blocked, "local fixture does not permit CLI execution");
            Ok(String::new())
        })
    }
}

#[cfg(test)]
mod tests;
