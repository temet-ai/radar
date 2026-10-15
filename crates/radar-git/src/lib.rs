//! radar-git: git on this machine, for any host, as a radar plugin.
//!
//! It finds the checkouts under the folders in `[git] roots` and matches
//! each to the repo its `origin` names (`local.rs`), and adds to the repos
//! and merge requests other parts draw (`join.rs`): the line under their
//! title saying where they are checked out, and the actions an editor on
//! it, a terminal in it, the merge request's branch checked out, look
//! again. Settings sections `checkouts`, `editors`, `links` and `branches` (`settings.rs`); the
//! `[git]` config table; the index in its cache.

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
}

impl GitPlugin {
    pub fn new() -> Self {
        Self::default()
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
        *self = demo::branches();
    }

    fn restore(&mut self, cache: Value) {
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
        act::on_bus(self, topic, payload)
    }

    fn capturing(&self) -> bool {
        self.local.edit.is_some()
    }

    fn paste(&mut self, text: &str) -> Vec<Effect> {
        if let Some((_, input)) = self.local.edit.as_mut() {
            input.paste(text);
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
        scoped::act(self, subject, action).unwrap_or_else(|| join::act(self, subject, action))
    }

    fn agent_context(
        &mut self,
        request: &radar_core::agent_context::AgentContextRequest,
    ) -> Option<radar_core::agent_context::AgentContext> {
        agent_context::agent_context(self, request)
    }

    fn ensure_worktree(
        &mut self,
        request: &radar_core::agent_context::WorktreeRequest,
    ) -> Option<(
        Result<radar_core::agent_context::WorktreeReady, radar_core::agent_context::WorktreeError>,
        Vec<Effect>,
    )> {
        Some(agent_context::ensure_worktree(self, request))
    }

    fn typing(&self, menu: &str, text: &str) -> radar_core::Typing {
        scoped::typing(self, menu, text).unwrap_or_else(|| act::typing(self, menu, text))
    }

    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
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

    fn section_icon(&self, section: &str) -> Option<radar_core::ui::icon::Icon> {
        use radar_core::ui::icon::Icon;
        Some(match section {
            "checkouts" => Icon::Local,
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
        settings::section_view(self, section, cx)
    }

    fn intent(&mut self, scope: &str, pane: Pane, intent: &radar_core::view::Intent) -> Outcome {
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
}

#[cfg(test)]
mod tests;
