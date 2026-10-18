//! Settings → Agents (task T8): per CLI whether it is installed, its version,
//! protocol, who is signed in and what it can do; signing in; and the
//! defaults a new session starts from (provider, model, permitted roots,
//! standing allow and deny rules, local only, whether a reviewer may run
//! commands). It is a [`SectionView`]: nothing here is specific to a renderer.
//!
//! The section never reads a credential. It shows what each adapter says of
//! its CLI ([`AdapterStatus`]) and asks the plugin, through [`Request`]s, to
//! read a status again or to start a sign-in; the plugin owns the adapters.

pub mod config;
pub mod locations;
pub(crate) mod setup;
pub mod status;
mod view;

use std::collections::BTreeMap;

use radar_core::view::{Intent, Key, SectionView, ViewCx};
use radar_core::{Effect, Menu, MenuItem, Outcome, SectionSpec, Typing, Verdict};

use crate::adapter::AdapterStatus;
use crate::login::{self, Login, LoginEvent, Method};
use config::{AgentsConfig, PtyCli, Rule, contains_token as looks_like_token};

pub const SECTION: &str = "agents";

/// What the section asks of the plugin, which owns the adapters and the dock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Read this provider's status again.
    ReadStatus(String),
    /// Start the adapter's own sign-in (Copilot's device code).
    StartLogin(String),
    CancelLogin(String),
    /// Run the CLI's own login command in a terminal tab (T7), then read
    /// the status when it ends.
    RunInTab { provider: String, argv: Vec<String> },
    /// Read what a provider offers (its agents and skills), for the agent fields.
    ReadCatalog(String),
}

/// One row of the section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Row {
    Cli(String),
    /// `+ add a command`: a terminal CLI of your own.
    AddCli,
    Provider,
    Model,
    WorkDir,
    Roots,
    Skills,
    ProjectConfig,
    Allow,
    Deny,
    LocalOnly,
    Reviewer,
    RecipesDir,
    Recipe(String),
    ReviewsDir,
    /// Desktop notifications while you are away from a session.
    Notify,
}

impl Row {
    pub(crate) fn target(&self) -> String {
        match self {
            Row::Cli(id) => format!("cli:{id}"),
            Row::AddCli => "cli:add".into(),
            Row::Provider => "def:provider".into(),
            Row::Model => "def:model".into(),
            Row::WorkDir => "def:work_dir".into(),
            Row::Roots => "def:roots".into(),
            Row::Skills => "def:skills".into(),
            Row::ProjectConfig => "def:project_config".into(),
            Row::Allow => "def:allow".into(),
            Row::Deny => "def:deny".into(),
            Row::LocalOnly => "def:local_only".into(),
            Row::Reviewer => "def:reviewer_commands".into(),
            Row::RecipesDir => "rec:folder".into(),
            Row::Recipe(name) => format!("rec:{name}"),
            Row::ReviewsDir => "rec:reviews".into(),
            Row::Notify => "dock:notify".into(),
        }
    }

    /// The defaults rows, under "new sessions".
    pub(crate) fn is_default(&self) -> bool {
        !matches!(self, Row::Cli(_) | Row::AddCli | Row::RecipesDir | Row::Recipe(_) | Row::ReviewsDir | Row::Notify)
    }
}

/// A provider's own fields, edited from its row's `p` menu.
pub(crate) const CLI_FIELDS: [(char, &str, &str); 6] = [
    ('e', "executable", "the executable, when it is not on PATH (empty: PATH)"),
    ('f', "config_dir", "its own config folder (empty: its default)"),
    ('a', "agent", "the agent a new session starts as (empty: its own)"),
    ('s', "skills", "a skill folder to load (type a listed one to remove it)"),
    ('n', "instructions", "a folder of instruction files (type a listed one to remove it)"),
    ('p', "plugins", "a plugin folder to load (type a listed one to remove it)"),
];

/// The folder of the operator's own recipes: the one set, else
/// `$XDG_CONFIG_HOME/radar/recipes` (`~/.config/radar/recipes`).
pub fn default_recipes_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .map(|c| c.join("radar").join("recipes"))
}

/// Open `dir` in the operator's `$VISUAL`/`$EDITOR` (a terminal editor in a
/// new terminal); with no editor set, with the desktop's handler.
fn open_in_editor(dir: std::path::PathBuf) -> Effect {
    use radar_core::system::{Env, InFolder, folder_ways, start_folder};
    let env = Env::detect();
    let Some(editor) = env.editor.clone().filter(|e| !e.trim().is_empty()) else {
        return Effect::Open(format!("file://{}", dir.display()));
    };
    Effect::Spawn(Box::pin(async move {
        let _ = tokio::task::spawn_blocking(move || {
            let ways = folder_ways(&env, &dir.display().to_string(), InFolder::Editor(&editor), "", "recipes");
            start_folder(ways, &dir)
        })
        .await;
        Box::new(()) as radar_core::plugin::Msg
    }))
}

/// A command put on the clipboard, and said so.
fn copied(cmd: String) -> Vec<Effect> {
    vec![Effect::Notify(format!("copied: {cmd}")), Effect::Copy(cmd)]
}

#[derive(Default)]
pub struct AgentsSettings {
    pub cfg: AgentsConfig,
    statuses: BTreeMap<String, AdapterStatus>,
    login: Option<(String, Login)>,
    row: usize,
    /// The workspace whose own defaults are being edited; none: the shared ones.
    workspace: Option<String>,
    requests: Vec<Request>,
    /// The directories searched for a PTY CLI's executable; the process PATH when none.
    path: Option<String>,
    available: Option<Vec<String>>,
    /// The agents and skills each provider offers, as last read.
    catalogs: BTreeMap<String, crate::adapter::Catalog>,
    /// What the agents asked permission for, as rules, most asked first,
    /// with how often: what always allow and always deny offer.
    asked: Vec<(Rule, usize)>,
    /// The last section view was drawn as a setup step: the rows only
    /// Settings needs (the defaults, recipes, away notices) are not in the
    /// list, so keys and clicks walk what is drawn.
    setup_card: std::cell::Cell<bool>,
}

pub fn sections() -> Vec<SectionSpec> {
    vec![SectionSpec { id: SECTION, name: "Tools and defaults", group: "sessions" }]
}

/// The native providers, in the order they are listed.
pub(crate) const NATIVE: [(&str, &str); 3] = [("copilot", "Copilot"), ("codex", "Codex"), ("claude", "Claude Agent")];

impl AgentsSettings {
    pub fn configure(&mut self, table: Option<&toml::Value>) -> anyhow::Result<()> {
        self.cfg = AgentsConfig::from_table(table)?;
        self.refresh_pty();
        Ok(())
    }

    pub fn config(&self) -> Option<toml::Value> {
        self.cfg.to_table()
    }

    pub fn set_workspace(&mut self, name: Option<String>) {
        self.workspace = name;
    }

    /// For tests and the scratch-PATH exercise: where PTY CLIs are looked for.
    pub fn set_path(&mut self, path: Option<String>) {
        self.path = path;
        self.refresh_pty();
    }

    /// What a new session in the workspace starts from.
    pub fn defaults(&self) -> &config::Defaults {
        match &self.workspace {
            Some(w) => self.cfg.for_workspace(w),
            None => &self.cfg.defaults,
        }
    }

    /// What a session of `provider` in this workspace is set up with.
    pub fn session_setup(&self, provider: &str) -> crate::adapter::SessionSetup {
        self.cfg.session_setup(self.workspace.as_deref(), provider)
    }

    /// Where `provider`'s CLI is and keeps its state.
    pub fn provider_setup(&self, provider: &str) -> crate::adapter::ProviderSetup {
        self.cfg.provider_setup(provider)
    }

    /// Where a session started from the dock works: the workspace's folder
    /// when one is set and absolute, else `fallback`.
    pub fn work_dir(&self, fallback: std::path::PathBuf) -> std::path::PathBuf {
        locations::checkout_entry(&self.cfg.repositories, &fallback)
            .and_then(|r| r.work_dir.as_deref()).and_then(config::expand)
            .or_else(|| self.defaults().work_dir.as_deref().and_then(config::expand))
            .or_else(|| self.cfg.defaults.work_dir.as_deref().and_then(config::expand))
            .unwrap_or(fallback)
    }

    /// The selected repository's override for a "send to agent" session.
    pub fn context_work_dir(&self, contexts: &[radar_core::agent_context::AgentContext]) -> Option<std::path::PathBuf> {
        let context = contexts.first()?;
        let repository = context.fields.iter().find(|f| matches!(f.name.as_str(), "project" | "repository"));
        let entry = repository.and_then(|f| locations::repository_entry(&self.cfg.repositories, &context.subject.instance, &f.value))
            .or_else(|| {
                (context.subject.provider == "local").then(|| {
                    locations::checkout_entry(&self.cfg.repositories, std::path::Path::new(&context.subject.instance))
                }).flatten()
            })?;
        entry.work_dir.as_deref().and_then(config::expand)
    }

    pub fn repository_review_dirs(&self) -> BTreeMap<String, std::path::PathBuf> {
        locations::review_dirs(&self.cfg.repositories)
    }

    /// The folder of the operator's own recipes: the one set, else radar's config folder's `recipes`.
    pub fn recipes_dir(&self) -> Option<std::path::PathBuf> {
        self.cfg.recipes_dir.as_deref().and_then(config::expand).or_else(default_recipes_dir)
    }

    /// What a provider offers, as its adapter last read it.
    pub fn set_catalog(&mut self, provider: &str, catalog: crate::adapter::Catalog) {
        self.catalogs.insert(provider.to_string(), catalog);
    }

    pub fn catalog(&self, provider: &str) -> Option<&crate::adapter::Catalog> {
        self.catalogs.get(provider)
    }

    /// The permission requests the agents made, newest last: always allow
    /// and always deny offer them as rules, one key each.
    pub fn set_asked<'a>(&mut self, requests: impl IntoIterator<Item = &'a crate::event::PermissionRequest>) {
        let mut asked: Vec<(Rule, usize)> = vec![];
        for r in requests.into_iter().flat_map(Rule::suggest) {
            match asked.iter_mut().find(|(have, _)| *have == r) {
                Some((_, n)) => *n += 1,
                None => asked.push((r, 1)),
            }
        }
        asked.sort_by_key(|a| std::cmp::Reverse(a.1));
        self.asked = asked;
    }

    /// Where reviews are saved when set; else radar's state folder.
    pub fn reviews_dir(&self) -> Option<std::path::PathBuf> {
        self.cfg.reviews_dir.as_deref().and_then(config::expand)
    }

    /// The configured terminal CLIs.
    pub fn pty_clis(&self) -> &[PtyCli] {
        &self.cfg.pty
    }

    /// An adapter's reading of its CLI.
    pub fn set_status(&mut self, provider: &str, status: AdapterStatus) {
        self.statuses.insert(provider.to_string(), status);
    }

    pub fn status(&self, provider: &str) -> Option<&AdapterStatus> {
        self.statuses.get(provider)
    }

    /// Which providers this build has an adapter for; the others are shown as
    /// not in this build instead of waiting for a status that never comes.
    pub fn set_available(&mut self, providers: Vec<String>) {
        self.available = Some(providers);
    }

    pub fn state(&self, provider: &str) -> status::State {
        if !self.statuses.contains_key(provider) && self.available.as_ref().is_some_and(|a| !a.iter().any(|p| p == provider)) {
            return status::State::NoAdapter;
        }
        status::state(self.statuses.get(provider))
    }

    pub fn login_state(&self) -> Option<&(String, Login)> {
        self.login.as_ref()
    }

    /// What the plugin must do next; empties the queue.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// The terminal tab the plugin opened for a login command.
    pub fn tab_opened(&mut self, provider: &str, tab: &str) {
        if let Some((p, l)) = &mut self.login
            && p == provider
        {
            l.open_tab(tab);
        }
    }

    /// The terminal tab running a login command ended.
    pub fn tab_exited(&mut self, provider: &str, code: Option<i32>) {
        let Some((p, l)) = &mut self.login else { return };
        if p == provider && l.tab_exited(code) {
            self.requests.push(Request::ReadStatus(provider.to_string()));
        }
    }

    /// A sign-in moved (the adapter's flow). A finished one asks for the
    /// status to be read again.
    pub fn login_event(&mut self, provider: &str, event: LoginEvent) {
        let Some((p, l)) = &mut self.login else { return };
        if p != provider {
            return;
        }
        if l.on_event(event) {
            self.requests.push(Request::ReadStatus(provider.to_string()));
        }
    }

    /// A PTY CLI is "installed" when its command is found; it has no status
    /// of its own to read.
    pub fn refresh_pty(&mut self) {
        let path = self.path.clone().or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
        for c in &self.cfg.pty {
            let found = find_on_path(&c.command, &path);
            self.statuses.insert(
                c.id.clone(),
                AdapterStatus {
                    installed: found,
                    version: None,
                    protocol: None,
                    auth: crate::adapter::AuthStatus::Unknown,
                    error: None,
                    capabilities: crate::adapter::Capabilities {
                        structured_events: false,
                        permissions: false,
                        unsupported: vec![
                            crate::adapter::Unsupported { control: "permissions".into(), reason: "it is the CLI's own terminal".into() },
                            crate::adapter::Unsupported { control: "structured events".into(), reason: "it is the CLI's own terminal".into() },
                        ],
                        ..Default::default()
                    },
                },
            );
        }
    }

    pub(crate) fn rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = NATIVE.iter().map(|(id, _)| Row::Cli((*id).into())).collect();
        rows.extend(self.cfg.pty.iter().map(|c| Row::Cli(c.id.clone())));
        rows.push(Row::AddCli);
        if self.setup_card.get() {
            return rows;
        }
        rows.extend([
            Row::Provider,
            Row::Model,
            Row::WorkDir,
            Row::Roots,
            Row::Skills,
            Row::ProjectConfig,
            Row::Allow,
            Row::Deny,
            Row::LocalOnly,
            Row::Reviewer,
            Row::RecipesDir,
        ]);
        let dir = self.recipes_dir();
        rows.extend(crate::brief::Recipe::names(dir.as_deref()).into_iter().map(Row::Recipe));
        rows.push(Row::ReviewsDir);
        rows.push(Row::Notify);
        rows
    }

    pub(crate) fn selected(&self) -> Row {
        let rows = self.rows();
        rows[self.row.min(rows.len() - 1)].clone()
    }

    pub(crate) fn name_of(&self, id: &str) -> String {
        NATIVE
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, n)| (*n).to_string())
            .or_else(|| self.cfg.pty.iter().find(|c| c.id == id).map(|c| c.name.clone()))
            .unwrap_or_else(|| id.to_string())
    }

    fn editing(&mut self) -> &mut config::Defaults {
        let w = self.workspace.clone();
        self.cfg.editing(w.as_deref())
    }

    // --- sections ---------------------------------------------------------

    pub fn section_view(&self, section: &str, cx: &ViewCx<'_>) -> Option<SectionView> {
        self.setup_card.set(cx.setup);
        (section == SECTION).then(|| view::section(self, cx))
    }

    pub fn intent(&mut self, section: &str, intent: &Intent) -> Outcome {
        if section != SECTION {
            return Outcome::Ignored;
        }
        match intent {
            Intent::Select { target } | Intent::Context { target } => {
                self.select(target.as_str());
                Outcome::done()
            }
            // A command line of the sheet: a click copies it.
            Intent::Activate { target } if setup::copy_of(target.as_str()).is_some() => {
                Outcome::Handled(setup::copy_of(target.as_str()).map(copied).unwrap_or_default())
            }
            Intent::Activate { target } => {
                self.select(target.as_str());
                Outcome::Handled(self.activate())
            }
            Intent::Invoke { action } => match action.as_str() {
                "sessions.agents.sign_in" => Outcome::Handled(self.sign_in()),
                "sessions.agents.cancel" => Outcome::Handled(self.cancel()),
                "sessions.agents.refresh" => Outcome::Handled(self.refresh()),
                "sessions.agents.copy" => Outcome::Handled(self.copy_command()),
                "sessions.agents.open" => Outcome::Handled(self.open_page()),
                "sessions.agents.add" => Outcome::Handled(self.activate_add()),
                "sessions.agents.edit" => Outcome::Handled(self.activate()),
                "sessions.agents.paths" => Outcome::Handled(self.cli_menu()),
                "sessions.agents.repositories" => Outcome::Handled(self.repository_menu()),
                "sessions.agents.recipe.edit" => Outcome::Handled(self.edit_recipes()),
                _ => intent.chord().map_or(Outcome::Ignored, |c| self.key(c)),
            },
            Intent::Key { chord } => self.key(*chord),
            _ => Outcome::Ignored,
        }
    }

    fn key(&mut self, chord: radar_core::view::Chord) -> Outcome {
        if chord.ctrl || chord.alt {
            return Outcome::Ignored;
        }
        match chord.key {
            Key::Enter => Outcome::Handled(self.activate()),
            Key::Esc if self.login.as_ref().is_some_and(|(_, l)| l.active()) => Outcome::Handled(self.cancel()),
            Key::Char('i') => Outcome::Handled(self.sign_in()),
            Key::Char('r') => Outcome::Handled(self.refresh()),
            Key::Char('p') if matches!(self.selected(), Row::Cli(_)) => Outcome::Handled(self.cli_menu()),
            Key::Char('p') if matches!(self.selected(), Row::WorkDir | Row::ReviewsDir) => Outcome::Handled(self.repository_menu()),
            // The command still needed, for the terminal: its install while it is missing, else its sign-in.
            Key::Char('y') if matches!(self.selected(), Row::Cli(_)) => Outcome::Handled(self.copy_command()),
            Key::Char('o') if matches!(self.selected(), Row::Cli(_)) => Outcome::Handled(self.open_page()),
            Key::Char('a') if matches!(self.selected(), Row::Cli(_) | Row::AddCli) => Outcome::Handled(self.activate_add()),
            Key::Char('e') if matches!(self.selected(), Row::Recipe(_) | Row::RecipesDir) => Outcome::Handled(self.edit_recipes()),
            _ => Outcome::Ignored,
        }
    }

    /// `y` on a CLI: copy the command it still needs.
    fn copy_command(&mut self) -> Vec<Effect> {
        let Row::Cli(id) = self.selected() else { return vec![] };
        match setup::command_for(self, &id) {
            Some(cmd) => copied(cmd),
            None => vec![Effect::Notify(format!("{} has no command to copy", self.name_of(&id)))],
        }
    }

    /// `o` on a CLI: open its own page about installing it.
    fn open_page(&mut self) -> Vec<Effect> {
        let Row::Cli(id) = self.selected() else { return vec![] };
        match setup::help(&id) {
            Some(h) => vec![Effect::Open(h.page.into())],
            None => vec![Effect::Notify(format!("{} has no page to open", self.name_of(&id)))],
        }
    }

    /// `a`: type a command of your own to add.
    fn activate_add(&mut self) -> Vec<Effect> {
        vec![Effect::Menu(Menu::typed(
            "agents.pty_add",
            "Add a command: what you would type in a terminal (aider, opencode --yes)",
            "",
        ))]
    }

    /// The terminal CLI a typed command makes: its id is the program's name.
    /// `Err` says why it cannot be added.
    fn pty_from(&self, text: &str) -> Result<PtyCli, String> {
        let text = text.trim();
        if looks_like_token(text) || text.split_whitespace().any(looks_like_token) {
            return Err("that looks like a token; radar never stores one".into());
        }
        let mut words = text.split_whitespace();
        let Some(command) = words.next() else { return Err("type the command".into()) };
        let id: String = command
            .rsplit('/')
            .next()
            .unwrap_or(command)
            .to_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            .collect();
        if id.is_empty() {
            return Err("type the command".into());
        }
        if NATIVE.iter().any(|(i, _)| *i == id) || self.cfg.pty.iter().any(|c| c.id == id) {
            return Err(format!("{id} is listed already"));
        }
        Ok(PtyCli { id: id.clone(), name: id, command: command.to_string(), args: words.map(str::to_string).collect(), login: None })
    }

    fn select(&mut self, target: &str) {
        if let Some(i) = self.rows().iter().position(|r| r.target() == target) {
            self.row = i;
        }
    }

    fn refresh(&mut self) -> Vec<Effect> {
        let Row::Cli(id) = self.selected() else { return vec![] };
        if self.cfg.pty.iter().any(|c| c.id == id) {
            self.refresh_pty();
        } else {
            self.requests.push(Request::ReadStatus(id));
        }
        vec![]
    }

    fn cancel(&mut self) -> Vec<Effect> {
        let Some((p, l)) = &mut self.login else { return vec![] };
        if !l.active() {
            return vec![];
        }
        l.cancel();
        self.requests.push(Request::CancelLogin(p.clone()));
        vec![Effect::Notify("sign-in cancelled".into())]
    }

    fn sign_in(&mut self) -> Vec<Effect> {
        let Row::Cli(id) = self.selected() else { return vec![] };
        if self.login.as_ref().is_some_and(|(_, l)| l.active()) {
            return vec![Effect::Notify("a sign-in is already running; esc cancels it".into())];
        }
        match self.state(&id) {
            status::State::Missing => {
                return vec![Effect::Notify(format!("{} is not installed", self.name_of(&id)))];
            }
            status::State::Ready { .. } => {
                return vec![Effect::Notify(format!("{} is already signed in", self.name_of(&id)))];
            }
            _ => {}
        }
        match login::method(&id, &self.cfg.pty) {
            Method::DeviceCode => {
                let mut l = Login::default();
                l.start();
                self.login = Some((id.clone(), l));
                self.requests.push(Request::StartLogin(id));
                vec![]
            }
            Method::Command(argv) => {
                let mut l = Login::default();
                l.start();
                self.login = Some((id.clone(), l));
                self.requests.push(Request::RunInTab { provider: id, argv });
                vec![]
            }
            Method::None => vec![Effect::Notify(format!("{} has no login command set", self.name_of(&id)))],
        }
    }

    /// `⏎` on the selected row.
    fn activate(&mut self) -> Vec<Effect> {
        let d = self.defaults().clone();
        match self.selected() {
            Row::Cli(id) => match self.state(&id) {
                status::State::SignedOut => self.sign_in(),
                status::State::Missing => vec![Effect::Notify(self.state(&id).advice())],
                _ => self.refresh(),
            },
            Row::AddCli => self.activate_add(),
            Row::Provider => {
                let mut ids: Vec<(String, String)> = NATIVE.iter().map(|(i, n)| ((*i).into(), (*n).into())).collect();
                ids.extend(self.cfg.pty.iter().map(|c| (c.id.clone(), c.name.clone())));
                // Native names have stable letter mnemonics. Custom names are
                // arbitrary, so use numbers instead of the old a/b/d/e/f/g/h.
                // Keep the same seven custom rows, order and provider values.
                let keys = ['c', 'x', 'l', '1', '2', '3', '4', '5', '6', '7'];
                let items = ids
                    .iter()
                    .zip(keys)
                    .map(|((id, name), k)| MenuItem::new(k, name.clone(), if *id == d.provider { "now" } else { "" }, id.clone()))
                    .collect();
                vec![Effect::Menu(Menu::new("agents.provider", "Default provider", items).at(d.provider))]
            }
            Row::Model => vec![Effect::Menu(Menu::typed(
                "agents.model",
                "Default model (empty: the provider's own)",
                d.model.unwrap_or_default(),
            ))],
            Row::Roots => vec![Effect::Menu(Menu::typed(
                "agents.roots",
                "Add a folder sessions may use (type one that is listed to remove it)",
                "",
            ))],
            Row::WorkDir => vec![Effect::Menu(Menu::typed(
                "agents.work_dir",
                "Where a session started from the dock works (empty: where radar was started)",
                d.work_dir.unwrap_or_default(),
            ))],
            Row::Skills => vec![Effect::Menu(Menu::typed(
                "agents.skills",
                "Add a skill folder every CLI loads (type one that is listed to remove it)",
                "",
            ))],
            Row::ProjectConfig => {
                let d = self.editing();
                d.project_config = !d.project_config;
                vec![Effect::Notify(if d.project_config {
                    "each CLI loads its own agents, skills, instructions and MCP servers".into()
                } else {
                    "each CLI loads only what these settings name".into()
                })]
            }
            Row::RecipesDir => vec![Effect::Menu(Menu::typed(
                "agents.recipes_dir",
                "Your recipes folder (empty: radar's config folder's recipes)",
                self.cfg.recipes_dir.clone().unwrap_or_default(),
            ))],
            Row::ReviewsDir => vec![Effect::Menu(Menu::typed(
                "agents.reviews_dir",
                "Default review folder (empty: workspace notes, then radar's state folder)",
                self.cfg.reviews_dir.clone().unwrap_or_default(),
            ))],
            Row::Recipe(name) => self.recipe_menu(&name),
            Row::Allow => self.rules_menu("agents.allow"),
            Row::Deny => self.rules_menu("agents.deny"),
            Row::LocalOnly => {
                let d = self.editing();
                d.local_only = !d.local_only;
                vec![Effect::Notify(if d.local_only { "sessions run on this machine only".into() } else { "remote runs allowed".into() })]
            }
            Row::Notify => {
                self.cfg.notify = !self.cfg.notify;
                vec![Effect::Notify(
                    if self.cfg.notify { "a desktop notification when a session needs you or ends out of sight".into() } else { "no desktop notifications".into() },
                )]
            }
            Row::Reviewer => {
                let d = self.editing();
                d.reviewer_may_run_commands = !d.reviewer_may_run_commands;
                vec![Effect::Notify(
                    if d.reviewer_may_run_commands { "a reviewer may ask to run commands, each approved".into() } else { "a reviewer only reads".into() },
                )]
            }
        }
    }

    /// ⏎ on always allow or always deny: what your agents asked for, one key
    /// each to add it; what is on the list now (⏎ takes one off); typing one
    /// of your own last.
    fn rules_menu(&self, menu: &str) -> Vec<Effect> {
        let allow = menu == "agents.allow";
        let rules = self.rules(menu);
        let other = self.rules(if allow { "agents.deny" } else { "agents.allow" });
        let mut items = vec![];
        let offered: Vec<&(Rule, usize)> = self.asked.iter().filter(|(r, _)| !rules.contains(r) && !other.contains(r)).take(9).collect();
        if !offered.is_empty() {
            items.push(MenuItem::heading(if allow { "your agents asked for · press its number to always allow it" } else { "your agents asked for · press its number to always deny it" }));
            items.extend(offered.iter().zip('1'..='9').map(|((r, n), k)| {
                MenuItem::new(k, rule_words(r), if *n > 1 { format!("asked {n} times") } else { "asked once".into() }, format!("add\t{r}"))
            }));
        }
        items.push(MenuItem::heading(if allow { "allowed now · ⏎ on one takes it off" } else { "denied now · ⏎ on one takes it off" }));
        if rules.is_empty() {
            items.push(MenuItem::note("  nothing yet"));
        }
        items.extend(rules.iter().map(|r| MenuItem::pick(rule_words(r), "take off", format!("remove\t{r}"))));
        items.push(MenuItem::heading("or type one of your own"));
        items.extend(RULE_WAYS.iter().map(|(k, kind, what, _)| MenuItem::new(*k, *what, "", format!("type\t{kind}"))));
        vec![Effect::Menu(Menu::new(menu, if allow { "Always allow" } else { "Always deny" }, items))]
    }

    // --- a provider's own fields and the recipes --------------------------

    /// `p` on a CLI row: its executable, config folder, agent and folders.
    fn cli_menu(&mut self) -> Vec<Effect> {
        let Row::Cli(id) = self.selected() else { return vec![] };
        let native = id.strip_prefix("pty:").unwrap_or(&id).to_string();
        let p = self.cfg.provider(&native);
        let now = |k: &str| -> String {
            match k {
                "executable" => p.executable.clone().unwrap_or_default(),
                "config_dir" => p.config_dir.clone().unwrap_or_default(),
                "agent" => p.agent.clone().unwrap_or_default(),
                "skills" => p.skill_dirs.join(", "),
                "instructions" => p.instruction_dirs.join(", "),
                _ => p.plugin_dirs.join(", "),
            }
        };
        let items = CLI_FIELDS
            .iter()
            .map(|(k, field, _)| MenuItem::new(*k, field.replace('_', " "), now(field), format!("{native}\t{field}")))
            .collect();
        self.requests.push(Request::ReadCatalog(native.clone()));
        vec![Effect::Menu(Menu::new("agents.cli", format!("{} · paths, agent and folders", self.name_of(&native)), items))]
    }

    /// ⏎ on a recipe: edit it, set its agent or skills, or go back to the built-in.
    fn recipe_menu(&mut self, name: &str) -> Vec<Effect> {
        let (recipe, source, _) = crate::brief::Recipe::load(name, self.recipes_dir().as_deref());
        let r = recipe.as_ref();
        self.requests.push(Request::ReadCatalog(self.defaults().provider.clone()));
        let mut items = vec![
            MenuItem::new('e', "edit it in your editor", "", format!("{name}\tedit")),
            MenuItem::new('a', "agent", r.and_then(|r| r.agent.clone()).unwrap_or_default(), format!("{name}\tagent")),
            MenuItem::new('s', "skills", r.map(|r| r.skills.join(", ")).unwrap_or_default(), format!("{name}\tskills")),
        ];
        if matches!(source, crate::brief::recipe::Source::File(_)) && crate::brief::Recipe::built_in(name).is_some() {
            items.push(MenuItem::new('b', "use the built-in again (yours is kept as .bak)", "", format!("{name}\tbuilt-in")));
        }
        vec![Effect::Menu(Menu::new("agents.recipe", format!("recipe {name}"), items))]
    }

    /// The operator's copy of recipe `name`: made from the built-in the first
    /// time, so editing never starts from an empty file.
    fn own_recipe(&self, name: &str) -> Result<std::path::PathBuf, String> {
        let dir = self.recipes_dir().ok_or("no recipes folder: set one")?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
        let path = dir.join(format!("{name}.md"));
        if !path.exists() {
            let text = crate::brief::recipe::BUILT_IN.iter().find(|(n, _)| *n == name).map(|(_, t)| *t).ok_or("no such built-in recipe")?;
            std::fs::write(&path, text).map_err(|e| format!("could not write {}: {e}", path.display()))?;
        }
        Ok(path)
    }

    /// `e`: every built-in recipe copied into the recipes folder (those not
    /// there yet), then the folder opened in the operator's editor.
    fn edit_recipes(&mut self) -> Vec<Effect> {
        let names: Vec<&str> = match self.selected() {
            Row::Recipe(n) if crate::brief::Recipe::built_in(&n).is_none() => vec![],
            _ => crate::brief::recipe::BUILT_IN.iter().map(|(n, _)| *n).collect(),
        };
        for n in names {
            if let Err(e) = self.own_recipe(n) {
                return vec![Effect::error(None, e)];
            }
        }
        let Some(dir) = self.recipes_dir() else { return vec![] };
        vec![Effect::Notify(format!("opening {} in your editor", dir.display())), open_in_editor(dir)]
    }

    fn cli_field(&mut self, native: &str, field: &str, value: &str) -> Vec<Effect> {
        if looks_like_token(value) {
            return vec![Effect::error(None, "that looks like a token; not saved")];
        }
        let path_like = !matches!(field, "agent");
        if path_like && !value.is_empty() && config::expand(value).is_none() {
            return vec![Effect::error(None, "give the whole path (or ~/…); not saved")];
        }
        let p = self.cfg.providers.entry(native.to_string()).or_default();
        let one = |v: &str| (!v.is_empty()).then(|| v.to_string());
        let said = match field {
            "executable" => {
                p.executable = one(value);
                "saved: applies when the CLI next starts"
            }
            "config_dir" => {
                p.config_dir = one(value);
                "saved: applies when the CLI next starts"
            }
            "agent" => {
                p.agent = one(value);
                "saved: new sessions start as this agent"
            }
            _ => {
                let kind = match field {
                    "skills" => config::FolderKind::Skills,
                    "instructions" => config::FolderKind::Instructions,
                    _ => config::FolderKind::Plugins,
                };
                if value.is_empty() {
                    return vec![];
                }
                let list = p.folders_mut(kind);
                if let Some(i) = list.iter().position(|r| r == value) {
                    list.remove(i);
                    "removed"
                } else {
                    list.push(value.to_string());
                    "added"
                }
            }
        };
        if *p == config::ProviderSettings::default() {
            self.cfg.providers.remove(native);
        }
        vec![Effect::Notify(said.into())]
    }

    fn recipe_field(&mut self, name: &str, field: &str, value: &str) -> Vec<Effect> {
        let path = match self.own_recipe(name) {
            Ok(p) => p,
            Err(e) => return vec![Effect::error(None, e)],
        };
        let Ok(text) = std::fs::read_to_string(&path) else { return vec![Effect::error(None, format!("could not read {}", path.display()))] };
        let Ok(now) = crate::brief::Recipe::parse(&text) else {
            return vec![Effect::error(None, format!("{} is not a valid recipe: fix it in your editor first", path.display()))];
        };
        let (agent, skills) = match field {
            "agent" => (Some(value.to_string()), now.skills.clone()),
            _ => (now.agent.clone(), value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
        };
        let out = crate::brief::Recipe::with_agent_and_skills(&text, agent.as_deref(), &skills);
        match std::fs::write(&path, out) {
            Ok(()) => vec![Effect::Notify(format!("saved in {}", path.display()))],
            Err(e) => vec![Effect::error(None, format!("could not write {}: {e}", path.display()))],
        }
    }

    // --- menus ------------------------------------------------------------

    pub fn typing(&self, menu: &str, text: &str) -> Typing {
        if menu.starts_with("agents.repository_path:") {
            return Typing { verdict: folder_verdict(text, false), offers: vec![] };
        }
        if let Some(rest) = menu.strip_prefix("agents.cli:") {
            return self.typing_cli(rest, text);
        }
        if menu.starts_with("agents.recipe:") && menu.ends_with(":agent") {
            let provider = &self.defaults().provider;
            return Typing { verdict: None, offers: self.agent_offers(provider, text) };
        }
        if matches!(menu, "agents.work_dir" | "agents.recipes_dir" | "agents.reviews_dir" | "agents.skills") {
            return Typing { verdict: folder_verdict(text, menu == "agents.skills" && self.defaults().skill_dirs.iter().any(|r| r == text.trim())), offers: vec![] };
        }
        let verdict = match menu {
            "agents.pty_add" if text.trim().is_empty() => Some(Verdict::Unsure("a program and its arguments: aider, opencode --yes".into())),
            "agents.pty_add" => Some(match self.pty_from(text) {
                Err(why) => Verdict::Bad(why),
                Ok(cli) => {
                    let path = self.path.clone().or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
                    if find_on_path(&cli.command, &path) {
                        Verdict::Good(format!("found on PATH · ⏎ adds {}", cli.id))
                    } else {
                        Verdict::Unsure(format!("not found on PATH yet · ⏎ adds {}", cli.id))
                    }
                }
            }),
            "agents.model" if looks_like_token(text) => Some(Verdict::Bad("that looks like a token; radar never stores one".into())),
            "agents.model" => None,
            "agents.roots" => {
                let t = text.trim();
                if t.is_empty() {
                    None
                } else if looks_like_token(t) {
                    Some(Verdict::Bad("that looks like a token; radar never stores one".into()))
                } else if self.defaults().permitted_roots.iter().any(|r| r == t) {
                    Some(Verdict::Unsure("listed: ⏎ removes it".into()))
                } else if !std::path::Path::new(t).is_absolute() {
                    Some(Verdict::Bad("give the whole path".into()))
                } else if std::path::Path::new(t).is_dir() {
                    Some(Verdict::Good("added".into()))
                } else {
                    Some(Verdict::Unsure("no such folder yet".into()))
                }
            }
            m if let Some((list, kind)) = rule_menu(m) => {
                let t = text.trim();
                // What the agents asked for of this kind, matching what is typed: ↑↓ puts one in the field.
                let offers = self
                    .asked
                    .iter()
                    .filter(|(r, _)| r.kind == kind && r.argument.contains(t) && !self.rules(list).contains(r))
                    .take(9)
                    .map(|(r, _)| MenuItem::pick(r.argument.clone(), "asked by an agent", r.argument.clone()))
                    .collect();
                let example = RULE_WAYS.iter().find(|w| w.1 == kind).map_or("", |w| w.3);
                let verdict = if t.is_empty() {
                    Verdict::Unsure(example.into())
                } else {
                    match Rule::parse(&format!("{kind}({t})")) {
                        Ok(r) if self.rules(list).contains(&r) => Verdict::Unsure("on the list already: ⏎ takes it off".into()),
                        Ok(_) => Verdict::Good("⏎ adds it".into()),
                        Err(config::RuleError::Secret) => Verdict::Bad("that looks like a token; radar never stores one".into()),
                        Err(_) => Verdict::Bad(example.into()),
                    }
                };
                return Typing { verdict: Some(verdict), offers };
            }
            _ => None,
        };
        Typing { verdict, offers: vec![] }
    }

    fn typing_cli(&self, rest: &str, text: &str) -> Typing {
        let Some((native, field)) = rest.split_once(':') else { return Typing { verdict: None, offers: vec![] } };
        if field == "agent" {
            return Typing { verdict: None, offers: self.agent_offers(native, text) };
        }
        let p = self.cfg.provider(native);
        let listed = match field {
            "skills" => p.skill_dirs.iter().any(|r| r == text.trim()),
            "instructions" => p.instruction_dirs.iter().any(|r| r == text.trim()),
            "plugins" => p.plugin_dirs.iter().any(|r| r == text.trim()),
            _ => false,
        };
        let verdict = if field == "executable" && !text.trim().is_empty() {
            match config::expand(text) {
                None => Some(Verdict::Bad("give the whole path".into())),
                Some(p) if find_on_path(&p.display().to_string(), "") => Some(Verdict::Good("found".into())),
                Some(_) => Some(Verdict::Unsure("no executable there".into())),
            }
        } else {
            folder_verdict(text, listed)
        };
        Typing { verdict, offers: vec![] }
    }

    /// The agents a provider offers, as choices for a typed agent name.
    fn agent_offers(&self, provider: &str, text: &str) -> Vec<MenuItem> {
        let native = provider.strip_prefix("pty:").unwrap_or(provider);
        let Some(c) = self.catalogs.get(native) else { return vec![] };
        let t = text.trim().to_lowercase();
        c.agents
            .iter()
            .filter(|a| t.is_empty() || a.name.to_lowercase().contains(&t))
            .take(9)
            .zip('1'..='9')
            .map(|(a, k)| MenuItem::new(k, a.name.clone(), a.source.clone().unwrap_or_default(), a.name.clone()))
            .collect()
    }

    fn rules(&self, menu: &str) -> &[Rule] {
        let d = self.defaults();
        if menu.starts_with("agents.allow") { &d.allow } else { &d.deny }
    }

    /// "always allow" from the dock: each rule goes on the allow list (of
    /// the workspace being edited) unless it is there already.
    pub fn always_allow(&mut self, rules: &[Rule]) {
        let d = self.editing();
        for r in rules {
            if !d.allow.contains(r) {
                d.allow.push(r.clone());
            }
        }
    }

    /// Add `rule` to the allow or deny list, or remove it when it is listed.
    fn toggle_rule(&mut self, menu: &str, rule: Rule) -> &'static str {
        let d = self.editing();
        let list = if menu.starts_with("agents.allow") { &mut d.allow } else { &mut d.deny };
        if let Some(i) = list.iter().position(|r| *r == rule) {
            list.remove(i);
            "removed"
        } else {
            list.push(rule);
            "added"
        }
    }

    /// The user chose or typed `value` in a menu this section opened.
    pub fn on_menu(&mut self, menu: &str, value: &str) -> Option<Vec<Effect>> {
        let value = value.trim();
        if let Some(effects) = self.repository_location_menu(menu, value) { return Some(effects); }
        // A field chosen from a provider's menu or a recipe's: ask for its value.
        if let ("agents.cli" | "agents.recipe", Some((who, field))) = (menu, value.split_once('\t')) {
            let (who, field) = (who.to_string(), field.to_string());
            if menu == "agents.recipe" {
                return Some(match field.as_str() {
                    "edit" => self.edit_recipes(),
                    "built-in" => self.back_to_built_in(&who),
                    "agent" => vec![Effect::Menu(Menu::typed(
                        format!("agents.recipe:{who}:agent"),
                        "Agent for this recipe: a name, or provider=name pairs (copilot=reviewer, claude=code-reviewer); empty: the setting's",
                        crate::brief::Recipe::load(&who, self.recipes_dir().as_deref()).0.and_then(|r| r.agent).unwrap_or_default(),
                    ))],
                    _ => vec![Effect::Menu(Menu::typed(
                        format!("agents.recipe:{who}:skills"),
                        "Skills this recipe asks for, comma separated (empty: none named)",
                        crate::brief::Recipe::load(&who, self.recipes_dir().as_deref()).0.map(|r| r.skills.join(", ")).unwrap_or_default(),
                    ))],
                });
            }
            let words = CLI_FIELDS.iter().find(|(_, f, _)| *f == field).map_or("", |(_, _, w)| *w);
            let p = self.cfg.provider(&who);
            let single = match field.as_str() {
                "executable" => p.executable.unwrap_or_default(),
                "config_dir" => p.config_dir.unwrap_or_default(),
                "agent" => p.agent.unwrap_or_default(),
                _ => String::new(),
            };
            return Some(vec![Effect::Menu(Menu::typed(format!("agents.cli:{who}:{field}"), format!("{}: {words}", self.name_of(&who)), single))]);
        }
        if let Some((native, field)) = menu.strip_prefix("agents.cli:").and_then(|r| r.split_once(':')) {
            let (native, field) = (native.to_string(), field.to_string());
            return Some(self.cli_field(&native, &field, value));
        }
        if let Some((name, field)) = menu.strip_prefix("agents.recipe:").and_then(|r| r.split_once(':')) {
            let (name, field) = (name.to_string(), field.to_string());
            return Some(self.recipe_field(&name, &field, value));
        }
        let path_ok = |v: &str| v.is_empty() || config::expand(v).is_some();
        match menu {
            "agents.work_dir" | "agents.recipes_dir" | "agents.reviews_dir" if !path_ok(value) || looks_like_token(value) => {
                return Some(vec![Effect::error(None, "give the whole path (or ~/…); not saved")]);
            }
            "agents.work_dir" => {
                self.editing().work_dir = (!value.is_empty()).then(|| value.to_string());
                return Some(vec![Effect::Notify("saved".into())]);
            }
            "agents.recipes_dir" => {
                self.cfg.recipes_dir = (!value.is_empty()).then(|| value.to_string());
                return Some(vec![Effect::Notify("saved".into())]);
            }
            "agents.reviews_dir" => {
                self.cfg.reviews_dir = (!value.is_empty()).then(|| value.to_string());
                return Some(vec![Effect::Notify("saved: new reviews go there".into())]);
            }
            "agents.skills" => {
                if value.is_empty() || !path_ok(value) || looks_like_token(value) {
                    return Some(vec![]);
                }
                let d = self.editing();
                let said = if let Some(i) = d.skill_dirs.iter().position(|r| r == value) {
                    d.skill_dirs.remove(i);
                    "removed"
                } else {
                    d.skill_dirs.push(value.to_string());
                    "added"
                };
                return Some(vec![Effect::Notify(said.into())]);
            }
            _ => {}
        }
        match menu {
            "agents.pty_add" => {
                if value.trim().is_empty() {
                    return Some(vec![]);
                }
                Some(match self.pty_from(value) {
                    Err(why) => vec![Effect::error(None, format!("{why}; not saved"))],
                    Ok(cli) => {
                        let (id, found) = (cli.id.clone(), cli.command.clone());
                        self.add_pty(cli);
                        let installed = self.state(&id) == status::State::Installed;
                        vec![Effect::Notify(if installed { format!("added {id}") } else { format!("added {id}: {found} is not on PATH yet") })]
                    }
                })
            }
            "agents.provider" => {
                let known = NATIVE.iter().any(|(i, _)| *i == value) || self.cfg.pty.iter().any(|c| c.id == value);
                if !known {
                    return Some(vec![Effect::error(None, "no such provider; not saved")]);
                }
                self.editing().provider = value.to_string();
                Some(vec![Effect::Notify(format!("new sessions start with {}", self.name_of(value)))])
            }
            "agents.model" => {
                if looks_like_token(value) {
                    return Some(vec![Effect::error(None, "that looks like a token; not saved")]);
                }
                self.editing().model = (!value.is_empty()).then(|| value.to_string());
                Some(vec![Effect::Notify("saved".into())])
            }
            "agents.roots" => {
                if value.is_empty() || looks_like_token(value) {
                    return Some(vec![]);
                }
                let d = self.editing();
                let said = if let Some(i) = d.permitted_roots.iter().position(|r| r == value) {
                    d.permitted_roots.remove(i);
                    "removed"
                } else {
                    d.permitted_roots.push(value.to_string());
                    "added"
                };
                Some(vec![Effect::Notify(said.into())])
            }
            "agents.allow" | "agents.deny" => {
                // From the menu: add a rule of a kind (asks what to match), or remove a listed one.
                if let Some(kind) = value.strip_prefix("type\t") {
                    let (_, _, what, _) = RULE_WAYS.iter().find(|w| w.1 == kind)?;
                    let verb = if menu == "agents.allow" { "Always allow" } else { "Always deny" };
                    return Some(vec![Effect::Menu(Menu::typed(format!("{menu}:{kind}"), format!("{verb} {what}"), ""))]);
                }
                let text = value.strip_prefix("remove\t").or_else(|| value.strip_prefix("add\t")).unwrap_or(value);
                let Ok(rule) = Rule::parse(text) else {
                    return Some(vec![Effect::error(None, "not a rule; not saved")]);
                };
                let words = rule_words(&rule);
                let said = self.toggle_rule(menu, rule);
                // Back to the list, so the change is in view.
                let mut fx = vec![Effect::Notify(format!("{said}: {words}"))];
                fx.extend(self.rules_menu(menu));
                Some(fx)
            }
            m if let Some((list, kind)) = rule_menu(m) => {
                if value.is_empty() {
                    return Some(self.rules_menu(list));
                }
                let Ok(rule) = Rule::parse(&format!("{kind}({value})")) else {
                    return Some(vec![Effect::error(None, "that looks like a token; not saved")]);
                };
                let words = rule_words(&rule);
                let said = self.toggle_rule(list, rule);
                // Back to the list, so what was added (or removed) is in view.
                let mut fx = vec![Effect::Notify(format!("{said}: {words}"))];
                fx.extend(self.rules_menu(list));
                Some(fx)
            }
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn clone_for_test(&self) -> AgentsSettings {
        AgentsSettings {
            cfg: self.cfg.clone(),
            statuses: self.statuses.clone(),
            catalogs: self.catalogs.clone(),
            asked: self.asked.clone(),
            login: self.login.clone(),
            row: self.row,
            workspace: self.workspace.clone(),
            requests: self.requests.clone(),
            path: self.path.clone(),
            available: self.available.clone(),
            setup_card: self.setup_card.clone(),
        }
    }

    /// Add a PTY CLI by hand (also what a `[[sessions.pty]]` entry means).
    pub fn add_pty(&mut self, cli: PtyCli) {
        self.cfg.pty.retain(|c| c.id != cli.id);
        self.cfg.pty.push(cli);
        self.refresh_pty();
    }
}

/// The kinds of request a standing rule can match: the menu key, the rule's
/// kind, what it is in words, and what to type for it.
pub(crate) const RULE_WAYS: [(char, &str, &str, &str); 5] = [
    ('c', "shell", "a command", "type the command's start: git diff, cargo test, npm run *"),
    ('r', "read", "reading files", "type a file or folder: src/*, /home/me/notes/*"),
    ('w', "write", "writing files", "type a file or folder: docs/*, CHANGELOG.md"),
    ('u', "url", "URL access", "type a site: github.com, docs.rs"),
    ('m', "mcp", "an MCP tool", "type a server, or server/tool: github, github/get_issue"),
];

/// `agents.allow:shell` → (`agents.allow`, `shell`): the field that asks what a new rule matches.
fn rule_menu(menu: &str) -> Option<(&str, &str)> {
    let (list, kind) = menu.split_once(':')?;
    (matches!(list, "agents.allow" | "agents.deny") && RULE_WAYS.iter().any(|w| w.1 == kind)).then_some((list, kind))
}

/// A rule in words: `command git diff`, `website github.com`.
pub(crate) fn rule_words(rule: &Rule) -> String {
    let what = match rule.kind.as_str() {
        "shell" => "command",
        "read" => "read files in",
        "write" => "change files in",
        "url" => "website",
        "mcp" => "MCP tool",
        k => k,
    };
    let mut arg = rule.argument.clone();
    if matches!(rule.kind.as_str(), "read" | "write") {
        arg = arg.trim_end_matches("/*").to_string();
        if let Some(h) = std::env::var("HOME").ok().filter(|h| !h.is_empty())
            && let Some(rest) = arg.strip_prefix(&h)
        {
            arg = format!("~{rest}");
        }
    }
    format!("{what} {arg}")
}

/// What a typed folder says as it is typed: listed (⏎ removes it), not a
/// whole path, there, or not there yet.
fn folder_verdict(text: &str, listed: bool) -> Option<Verdict> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if looks_like_token(t) {
        return Some(Verdict::Bad("that looks like a token; radar never stores one".into()));
    }
    if listed {
        return Some(Verdict::Unsure("listed: ⏎ removes it".into()));
    }
    Some(match config::expand(t) {
        None => Verdict::Bad("give the whole path (or ~/…)".into()),
        Some(p) if p.is_dir() => Verdict::Good("there".into()),
        Some(_) => Verdict::Unsure("no such folder yet".into()),
    })
}

impl AgentsSettings {
    /// Rename the operator's copy of a built-in recipe to `.bak`, so the built-in is used again.
    fn back_to_built_in(&mut self, name: &str) -> Vec<Effect> {
        let Some(dir) = self.recipes_dir() else { return vec![] };
        let path = dir.join(format!("{name}.md"));
        match std::fs::rename(&path, dir.join(format!("{name}.md.bak"))) {
            Ok(()) => vec![Effect::Notify(format!("the built-in {name} is used again; yours is {name}.md.bak"))],
            Err(e) => vec![Effect::error(None, format!("could not move {}: {e}", path.display()))],
        }
    }
}

/// Whether `command` is an executable in one of the `path` directories (or
/// is itself a path to one).
pub fn find_on_path(command: &str, path: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let runnable = |p: &std::path::Path| p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if command.contains('/') {
        return runnable(std::path::Path::new(command));
    }
    std::env::split_paths(path).any(|d| runnable(&d.join(command)))
}

#[cfg(test)]
mod tests;
