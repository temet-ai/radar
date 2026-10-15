//! The part: owns the sessions and feeds the dock.
//!
//! The host calls this on its own thread; the providers are async. The plugin
//! therefore keeps an inbox: tasks it spawns push what an adapter says (or an
//! error) into it and wake the host ([`radar_core::wake`]), which has the inbox
//! applied to the store at once (`woken`, at most one call per 40 ms however
//! fast text streams), and `tick` (every 200 ms) applies whatever is left.
//! The dock is read from the store, so a renderer that detaches or restarts
//! never stops a session, and a permission request simply waits in the store
//! until an `Approve` or `Deny` intent answers it. Nothing here answers for
//! the operator.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use radar_core::dock::{self as view, DockIntent, PickerKind};
use radar_core::view::{Intent as ViewIntent, SectionView, ViewCx};
use radar_core::{Effect, Outcome, Pane, Plugin, SectionSpec, Typing};
use tokio::runtime::{Handle, Runtime};

use crate::adapter::*;
use crate::dock::{self, Inputs};
use crate::event::*;
use crate::settings::{self, AgentsSettings, Request};
use crate::store::{SessionMeta, Store};
use crate::warm::{self, Cache, Spare};

/// A reviewed merge request's `x` menu: open its local review, copy its path.
const REVIEW_OPEN: &str = "sessions.review.open";
const REVIEW_COPY: &str = "sessions.review.copy";

/// What a spawned task tells the plugin.
enum Inbound {
    Update(ProviderId, Update),
    Status(String, AdapterStatus),
    Models(String, Vec<ModelInfo>),
    Created(RadarSessionId, ProviderSessionId),
    Failed(Option<RadarSessionId>, String),
    /// The provider refused this message: (session, text, why).
    SendFailed(RadarSessionId, String, String),
    /// One provider's sessions, replacing what was listed of it before.
    Sessions(String, Vec<ProviderSessionInfo>),
    /// What a provider offers: its agents and skills, with the cache key it was read for.
    Catalog { provider: String, key: String, catalog: Catalog },
    /// The provider could not say. What was cached stays; with nothing cached, an empty
    /// catalog and the reason as a notice.
    CatalogFailed { provider: String, key: String, why: String },
    Login(String, LoginEvent),
}

/// What the spawned tasks brought, and the host's waker, rung on each push.
#[derive(Default)]
struct InboxInner {
    items: Mutex<Vec<Inbound>>,
    waker: Mutex<Option<radar_core::wake::Waker>>,
}

/// Whether a status says the CLI can be asked for models, lists and a session: installed, no
/// error, and not known to be signed out or blocked.
fn usable(status: &AdapterStatus) -> bool {
    status.installed && status.error.is_none() && !matches!(status.auth, AuthStatus::SignedOut | AuthStatus::PolicyBlocked { .. })
}

type Inbox = Arc<InboxInner>;

/// Makes the adapter of a terminal CLI configured in `[[sessions.pty]]`.
type TerminalFactory = Box<dyn Fn(&crate::settings::config::PtyCli) -> Box<dyn Adapter> + Send + Sync>;

/// The part. The binary adds the adapters it was built with (`sessions-copilot`
/// and the like), so this crate never names an adapter crate.
pub struct SessionsPlugin {
    adapters: Vec<Arc<dyn Adapter>>,
    settings: AgentsSettings,
    store: Store,
    order: Vec<RadarSessionId>,
    active: Option<RadarSessionId>,
    folds: BTreeMap<String, bool>,
    picker: Option<PickerKind>,
    history: Vec<String>,
    models: BTreeMap<String, Vec<ModelInfo>>,
    chosen_model: Option<String>,
    /// The agent chosen in the dock for new sessions: (provider, agent).
    chosen_agent: Option<(String, String)>,
    /// Makes the adapter of a configured terminal CLI (the binary gives it).
    terminal_factory: Option<TerminalFactory>,
    provider_sessions: Vec<(String, ProviderSessionInfo)>,
    logins: BTreeMap<String, tokio::sync::mpsc::UnboundedSender<()>>,
    /// The terminal tab running a CLI's login command: (provider, tab).
    login_tab: Option<(String, RadarSessionId)>,
    inbox: Inbox,
    handle: Option<Handle>,
    runtime: Option<Runtime>,
    started: bool,
    restored: Vec<SessionMeta>,
    counter: u64,
    now: i64,
    /// Local reviews in progress and the ones saved (task T11).
    review: crate::review::driver::Driver,
    /// Effects the review journey produced between ticks.
    review_fx: Vec<Effect>,
    /// When each session's turns ran, at this plugin's clock.
    clocks: BTreeMap<RadarSessionId, dock::Clock>,
    /// Sessions whose last turn ended out of sight.
    unread: BTreeSet<RadarSessionId>,
    /// The host shows the dock ([`Plugin::dock_visible`]).
    dock_visible: bool,
    /// Requests a standing rule or a session approval answered: they never wait for the user.
    auto_answered: BTreeSet<String>,
    /// What the pickers and the `/` menu show at once (saved with the part's state).
    cache: Cache,
    /// Providers whose client was started (and status read) in the background.
    warmed: BTreeSet<String>,
    /// Providers the last status read called usable ([`usable`]).
    ready: BTreeSet<String>,
    /// The dock has been shown in this run (the first [`Plugin::dock_visible`] with `true`).
    dock_open: bool,
    /// Providers whose catalog and session list were read when the dock opened.
    refreshed: BTreeSet<String>,
    /// A session created ahead of the next new tab, for a provider that can keep one for free.
    spare: Option<Spare>,
    /// Creating the spare failed; no more tries until the settings change.
    spare_off: bool,
    /// Settings → Agents was drawn: the CLIs nobody has asked yet are asked then.
    settings_seen: Arc<AtomicBool>,
    statuses_asked: bool,
    /// A tab opened from the spare is not saved by any event yet: save with the next drain.
    persist_soon: bool,
}

impl Default for SessionsPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionsPlugin {
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
            settings: AgentsSettings::default(),
            store: Store::new(),
            order: Vec::new(),
            active: None,
            folds: BTreeMap::new(),
            picker: None,
            history: Vec::new(),
            models: BTreeMap::new(),
            chosen_model: None,
            chosen_agent: None,
            terminal_factory: None,
            provider_sessions: Vec::new(),
            logins: BTreeMap::new(),
            login_tab: None,
            inbox: Inbox::default(),
            handle: None,
            runtime: None,
            started: false,
            restored: Vec::new(),
            counter: 0,
            now: 0,
            review: Default::default(),
            review_fx: Vec::new(),
            clocks: BTreeMap::new(),
            unread: BTreeSet::new(),
            dock_visible: false,
            auto_answered: BTreeSet::new(),
            cache: Cache::default(),
            warmed: BTreeSet::new(),
            ready: BTreeSet::new(),
            dock_open: false,
            refreshed: BTreeSet::new(),
            spare: None,
            spare_off: false,
            settings_seen: Arc::new(AtomicBool::new(false)),
            statuses_asked: false,
            persist_soon: false,
        }
    }

    /// What the dock is built from; `caps`, `models` and the catalog are the active session's provider's.
    fn inputs<'a>(
        &'a self,
        caps: Option<&'a Capabilities>,
        models: &'a [ModelInfo],
        terminals: &'a BTreeMap<String, view::TerminalView>,
        provider: &'a str,
    ) -> Inputs<'a> {
        Inputs {
            store: &self.store,
            order: &self.order,
            active: self.active.as_ref(),
            folds: &self.folds,
            picker: self.picker,
            history: &self.history,
            capabilities: caps,
            models,
            chosen_model: self.chosen_model.as_deref(),
            provider_sessions: &self.provider_sessions,
            terminals,
            catalog: self.settings.catalog(provider.strip_prefix("pty:").unwrap_or(provider)),
            cached_commands: self
                .active
                .as_ref()
                .and_then(|id| self.store.get(id))
                .and_then(|s| self.cache.commands(&warm::commands_key(s.meta.provider.as_str(), &s.meta.cwd))),
            chosen_agent: self.chosen_agent.as_ref().filter(|(p, _)| p == provider).map(|(_, a)| a.as_str()),
            now: self.now,
            clocks: &self.clocks,
            unread: &self.unread,
        }
    }

    /// The tab showing in a shown dock has been seen.
    fn seen(&mut self) {
        if let (true, Some(radar)) = (self.dock_visible, self.active.clone()) {
            self.unread.remove(&radar);
        }
    }

    /// Bring every session's clock up to the store at `now`: a turn that
    /// ended out of sight is unread, and what needs the user or ended out of
    /// sight goes to the desktop when Settings → Agents asks for it.
    fn time_turns(&mut self) -> Vec<Effect> {
        let mut told = Vec::new();
        for s in self.store.sessions() {
            let radar = s.meta.radar_session_id.clone();
            let needs = dock::needs_you(s, &self.auto_answered);
            let Some(change) = self.clocks.entry(radar.clone()).or_default().step(s, needs, self.now) else { continue };
            if self.dock_visible && self.active.as_ref() == Some(&radar) {
                continue;
            }
            if change == dock::Change::Finished {
                self.unread.insert(radar.clone());
            }
            told.push((radar, change));
        }
        if !self.settings.cfg.notify {
            return vec![];
        }
        told.into_iter().filter_map(|(radar, change)| self.desktop(&radar, change)).collect()
    }

    /// A desktop notification of `change` in session `radar`.
    fn desktop(&self, radar: &RadarSessionId, change: dock::Change) -> Option<Effect> {
        let s = self.store.get(radar)?;
        let a = dock::activity(s, self.clocks.get(radar), self.now);
        let body = match (change, a.as_ref().map(|a| a.doing)) {
            (dock::Change::NeedsYou, Some(view::Doing::Approval)) => format!("needs your approval: {}", a.map(|a| a.detail).unwrap_or_default()),
            (dock::Change::NeedsYou, _) => "waits for your answer".to_string(),
            (_, Some(view::Doing::Failed)) => "failed".to_string(),
            (_, Some(view::Doing::Stopped)) => "stopped".to_string(),
            _ => "done".to_string(),
        };
        let (title, body) = (format!("radar · {}", s.meta.title), body);
        Some(Effect::Spawn(Box::pin(async move {
            let _ = tokio::task::spawn_blocking(move || {
                let env = radar_core::system::Env::detect();
                radar_core::system::try_ways(&radar_core::system::notify_ways(&env, &title, &body), None)
            })
            .await;
            Box::new(()) as radar_core::plugin::Msg
        })))
    }

    /// Where a review's note goes when no part keeps notes (and where the
    /// saved reviews are read back from).
    pub fn with_review_dir(mut self, dir: std::path::PathBuf) -> Self {
        crate::review::driver::load(&dir, &mut self.review.reviews);
        self.review.fallback_dir = Some(dir);
        self
    }

    /// The row's word for a change request that was reviewed locally
    /// (`reviewed abc1234`, `review stale (abc1234)`), by its id in its
    /// service and, when known, its head sha now.
    pub fn review_chip(&self, id: &str, head_sha: Option<&str>) -> Option<String> {
        let record = self.review.reviews.of_id(id)?;
        self.review.reviews.chip(&record.subject, head_sha)
    }

    /// What the review journey asks the dock to do.
    fn review_todo(&mut self, todo: Vec<crate::review::driver::Todo>) {
        use crate::review::driver::Todo;
        for t in todo {
            match t {
                Todo::Notice(text) => self.review_fx.push(Effect::Notify(text)),
                Todo::StopSession(radar) => self.stop_session(&radar),
                Todo::StartSession { run, spec } => self.start_review_session(run, spec),
            }
        }
    }

    /// A session in the review's worktree with the default provider; the
    /// brief is its first message. Its permission requests are decided by
    /// [`crate::review::policy`] in `drain`, and what that leaves waits for
    /// the operator in the dock.
    fn start_review_session(&mut self, run: u64, spec: crate::review::journey::SessionSpec) {
        let Some(provider) = self.default_provider() else {
            self.review_fx.push(Effect::Notify("no agent provider to review with".into()));
            return;
        };
        let Some(adapter) = self.adapter(&provider) else { return };
        let radar = self.next_id();
        let model = self.chosen_model.clone().or_else(|| self.settings.defaults().model.clone());
        let meta = SessionMeta {
            radar_session_id: radar.clone(),
            provider: adapter.id(),
            provider_session_id: ProviderSessionId::new(""),
            cwd: spec.cwd.display().to_string(),
            title: spec.title.clone(),
            model: model.clone(),
            created_at: self.now,
            cursor: None,
            agent: None,
        };
        // The brief goes with the create: anything typed meanwhile waits for its turn.
        self.store.insert(meta).sending();
        self.order.push(radar.clone());
        self.active = Some(radar.clone());
        let todo = self.review.bind(run, radar.clone(), model.clone(), self.now);
        self.review_todo(todo);
        let inbox = self.inbox.clone();
        let (cwd, first) = (spec.cwd, spec.brief);
        let mut setup = self.setup(&provider);
        // The review recipe's agent and skills win over the settings' default agent.
        if spec.agent.is_some() {
            setup.agent = spec.agent;
        }
        setup.skills = spec.skills;
        setup.review = true;
        if let Some(s) = self.store.get_mut(&radar) {
            s.meta.agent = setup.agent.clone();
        }
        self.spawn(async move {
            match adapter.create(CreateSession { radar_session_id: radar.clone(), cwd, model, setup }).await {
                Ok(psid) => {
                    Self::push(&inbox, Inbound::Created(radar.clone(), psid));
                    if let Err(e) = adapter.send(&radar, SendInput { text: first }).await {
                        Self::push(&inbox, Inbound::Failed(Some(radar), format!("could not send: {e}")));
                    }
                }
                Err(e) => Self::push(&inbox, Inbound::Failed(Some(radar), format!("could not start: {e}"))),
            }
        });
    }

    fn stop_session(&mut self, radar: &RadarSessionId) {
        let Some(adapter) = self.store.get(radar).and_then(|s| self.adapter(s.meta.provider.as_str())) else { return };
        let (radar, inbox) = (radar.clone(), self.inbox.clone());
        self.spawn(async move {
            if let Err(e) = adapter.stop(&radar).await {
                Self::push(&inbox, Inbound::Failed(Some(radar), format!("could not stop: {e}")));
            }
        });
    }

    /// What the review journey does with one event of a review session: the
    /// reviewer's permission requests are decided here (reads and git
    /// diff/log/show run, writes and posting tools are denied, anything else
    /// waits for the operator), then the journey sees the event.
    fn review_event(&mut self, event: Event, answered: bool) {
        if let (false, EventKind::PermissionRequested { request }) = (answered, &event.kind) {
            let cwd = self.store.get(&event.radar_session_id).map(|s| s.meta.cwd.clone()).unwrap_or_default();
            use crate::review::policy::{Verdict, decide};
            match decide(request, std::path::Path::new(&cwd)) {
                Verdict::Allow => self.answer_for(&event.radar_session_id, &request.request_id, PermissionDecision::ApproveOnce),
                Verdict::Deny(why) => {
                    self.answer_for(&event.radar_session_id, &request.request_id, PermissionDecision::Deny { reason: Some(why.into()) })
                }
                Verdict::Ask if !self.settings.defaults().reviewer_may_run_commands && matches!(request.target, crate::event::PermissionTarget::Command { .. }) => self.answer_for(
                    &event.radar_session_id,
                    &request.request_id,
                    PermissionDecision::Deny { reason: Some("this review session may only read and run git diff, log and show".into()) },
                ),
                Verdict::Ask => {}
            }
        }
        let todo = self.review.event(&event, self.now);
        self.review_todo(todo);
    }

    /// Register a provider adapter; the first added is the default.
    pub fn add_adapter(&mut self, adapter: Box<dyn Adapter>) {
        adapter.configure(self.settings.provider_setup(adapter.id().as_str()));
        self.adapters.push(Arc::from(adapter));
        self.settings.set_available(self.adapters.iter().map(|a| a.id().to_string()).collect());
    }

    /// How the binary makes the adapter of a terminal CLI configured in
    /// `[[sessions.pty]]` (this crate names no adapter crate).
    pub fn with_terminal_factory(
        mut self,
        factory: impl Fn(&crate::settings::config::PtyCli) -> Box<dyn Adapter> + Send + Sync + 'static,
    ) -> Self {
        self.terminal_factory = Some(Box::new(factory));
        self
    }

    /// Every adapter takes its CLI's place and state folder from settings,
    /// and every configured terminal CLI has an adapter.
    fn apply_settings(&mut self) {
        // Settings changed: a session that could not be made ahead may be now.
        self.spare_off = false;
        self.review.recipes_dir = self.settings.recipes_dir();
        if let Some(dir) = self.settings.reviews_dir()
            && self.review.fallback_dir.as_ref() != Some(&dir)
        {
            crate::review::driver::load(&dir, &mut self.review.reviews);
            self.review.fallback_dir = Some(dir);
        }
        for a in &self.adapters {
            a.configure(self.settings.provider_setup(a.id().as_str()));
        }
        let missing: Vec<_> = self
            .settings
            .pty_clis()
            .iter()
            .filter(|c| !self.adapters.iter().any(|a| a.id().as_str() == c.id))
            .cloned()
            .collect();
        if let Some(make) = &self.terminal_factory {
            let made: Vec<_> = missing.iter().map(make.as_ref()).collect();
            for a in made {
                self.add_adapter(a);
            }
        }
    }

    /// What a session of `provider` is set up with: settings, and the agent
    /// chosen in the dock for that provider.
    fn setup(&self, provider: &str) -> SessionSetup {
        let mut s = self.settings.session_setup(provider);
        if let Some((p, agent)) = &self.chosen_agent
            && p == provider
        {
            s.agent = Some(agent.clone());
        }
        s
    }

    pub fn adapters(&self) -> &[Arc<dyn Adapter>] {
        &self.adapters
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn active(&self) -> Option<&RadarSessionId> {
        self.active.as_ref()
    }

    /// Add the deterministic fake provider when `RADAR_FAKE_AGENT` is set, so the
    /// dock can be exercised with no CLI installed.
    pub fn with_fake_from_env(mut self) -> Self {
        if std::env::var_os("RADAR_FAKE_AGENT").is_some() {
            self.add_adapter(Box::new(crate::fake::FakeAdapter::new()));
        }
        self
    }

    fn adapter(&self, provider: &str) -> Option<Arc<dyn Adapter>> {
        self.adapters.iter().find(|a| a.id().as_str() == provider).cloned()
    }

    fn default_provider(&self) -> Option<String> {
        let wanted = self.settings.defaults().provider.clone();
        self.adapters
            .iter()
            .map(|a| a.id().to_string())
            .find(|id| *id == wanted)
            .or_else(|| self.adapters.first().map(|a| a.id().to_string()))
    }

    // --- running things in the background ---------------------------------

    fn spawn<F: Future<Output = ()> + Send + 'static>(&mut self, fut: F) {
        if self.handle.is_none() {
            self.handle = Handle::try_current().ok();
        }
        if self.handle.is_none() {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("a runtime for agent sessions");
            self.handle = Some(rt.handle().clone());
            self.runtime = Some(rt);
        }
        if let Some(handle) = &self.handle {
            handle.spawn(fut);
        }
    }

    fn push(inbox: &Inbox, item: Inbound) {
        inbox.items.lock().expect("inbox").push(item);
        if let Some(waker) = inbox.waker.lock().expect("waker").as_ref() {
            waker.wake();
        }
    }

    /// The first tick with an adapter: every adapter's updates start flowing and the sessions
    /// of the last run come back; the default provider's client is started in the background
    /// (and any other provider that has such a session). Nothing else starts a CLI: the others
    /// start when something needs them (a new tab, a picker, Settings → Agents).
    fn start(&mut self) {
        self.started = true;
        let adapters = self.adapters.clone();
        let default = self.default_provider();
        for adapter in adapters {
            let inbox = self.inbox.clone();
            let provider = adapter.id();
            let restored: Vec<SessionMeta> =
                self.restored.iter().filter(|m| m.provider == provider).cloned().collect();
            let warm = default.as_deref() == Some(provider.as_str()) || !restored.is_empty();
            let name = provider.to_string();
            let setup = self.setup(provider.as_str());
            // Listening comes first: nothing a session says is lost to the client starting.
            self.spawn(async move {
                let mut updates = match adapter.subscribe().await {
                    Ok(updates) => updates,
                    Err(e) => {
                        Self::push(&inbox, Inbound::Failed(None, format!("{provider}: {e}")));
                        return;
                    }
                };
                // Sessions from the last run: the provider replays its history.
                for meta in restored {
                    let req = ResumeSession {
                        radar_session_id: meta.radar_session_id.clone(),
                        provider_session_id: meta.provider_session_id.clone(),
                        cwd: meta.cwd.clone().into(),
                        cursor: None,
                        setup: SessionSetup { agent: meta.agent.clone().or_else(|| setup.agent.clone()), ..setup.clone() },
                    };
                    if let Err(e) = adapter.resume(req).await {
                        Self::push(&inbox, Inbound::Failed(Some(meta.radar_session_id), format!("could not resume: {e}")));
                    }
                }
                while let Some(update) = updates.recv().await {
                    Self::push(&inbox, Inbound::Update(provider.clone(), update));
                }
                Self::push(
                    &inbox,
                    Inbound::Update(provider.clone(), Update::Disconnected { reason: "the adapter stopped".into() }),
                );
            });
            if warm {
                self.warm_provider(&name);
            }
        }
    }

    /// Start `provider`'s client and read its status, then its models when the cache has none
    /// fresh and the CLI is usable. Once per run per provider; the rest (catalogs, a spare) waits
    /// for the dock to open ([`SessionsPlugin::settle`]). Never waited on.
    fn warm_provider(&mut self, provider: &str) {
        if !self.warmed.insert(provider.to_string()) {
            return;
        }
        let Some(adapter) = self.adapter(provider) else { return };
        let want_models = adapter.capabilities().list_models
            && !self.cache.models(provider).is_some_and(|m| m.fresh(self.now, warm::MODELS_FRESH));
        let (inbox, provider) = (self.inbox.clone(), provider.to_string());
        self.spawn(async move {
            // An error is not news here: the status says what is wrong.
            let _ = adapter.warm().await;
            let status = adapter.status().await;
            let go = usable(&status);
            Self::push(&inbox, Inbound::Status(provider.clone(), status));
            if go
                && want_models
                && let Ok(models) = adapter.list_models().await
            {
                Self::push(&inbox, Inbound::Models(provider, models));
            }
        });
    }

    /// Read `provider`'s status again (Settings → Agents asks; so does a finished sign-in).
    fn read_status(&mut self, provider: &str) {
        let Some(adapter) = self.adapter(provider) else { return };
        let (inbox, provider) = (self.inbox.clone(), provider.to_string());
        self.spawn(async move {
            Self::push(&inbox, Inbound::Status(provider, adapter.status().await));
        });
    }

    /// Settings → Agents was drawn: ask the CLIs nobody has asked yet, so its rows fill in.
    fn read_unread_statuses(&mut self) {
        let unread: Vec<String> = self
            .adapters
            .iter()
            .map(|a| a.id().to_string())
            .filter(|p| !self.warmed.contains(p) && self.settings.status(p).is_none())
            .collect();
        for p in unread {
            self.read_status(&p);
        }
    }

    /// What the dock needs once it is open and the default provider is usable: its catalog and
    /// session list read in the background (shown from the cache meanwhile), and a session
    /// ready for the next new tab. Also keeps that session true to the settings.
    fn settle(&mut self) {
        self.check_spare();
        if !self.dock_open {
            return;
        }
        let Some(provider) = self.default_provider() else { return };
        if !self.ready.contains(&provider) {
            return;
        }
        if self.refreshed.insert(provider.clone()) {
            self.read_catalog(&provider, false);
            self.read_sessions(&provider);
        }
        self.ensure_spare(&provider);
    }

    fn handle_requests(&mut self) {
        for request in self.settings.take_requests() {
            match request {
                Request::ReadStatus(provider) => self.read_status(&provider),
                Request::StartLogin(provider) => self.start_login(provider),
                Request::CancelLogin(provider) => {
                    if let Some(cancel) = self.logins.get(&provider) {
                        let _ = cancel.send(());
                    }
                }
                Request::RunInTab { provider, argv } => self.run_in_tab(provider, argv),
                Request::ReadCatalog(provider) => self.read_catalog(&provider, true),
            }
        }
    }

    /// A CLI's own login command (`codex login`) in a terminal tab: the PTY
    /// adapter for that provider (else any PTY adapter) runs `argv`; radar shows
    /// the screen and reads nothing of it. The tab ending reads the status again.
    fn run_in_tab(&mut self, provider: String, argv: Vec<String>) {
        let wanted = format!("pty:{provider}");
        let adapter = self
            .adapter(&wanted)
            .or_else(|| self.adapters.iter().find(|a| a.id().as_str().starts_with("pty:")).cloned());
        let Some(adapter) = adapter else {
            self.settings.login_event(&provider, LoginEvent::Failed("terminal tabs are not available in this build".into()));
            return;
        };
        let radar = self.next_id();
        let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
        let meta = SessionMeta {
            radar_session_id: radar.clone(),
            provider: adapter.id(),
            provider_session_id: ProviderSessionId::new(""),
            cwd: cwd.display().to_string(),
            title: format!("{provider} login"),
            model: None,
            created_at: self.now,
            cursor: None,
            agent: None,
        };
        self.store.insert(meta);
        self.order.push(radar.clone());
        self.active = Some(radar.clone());
        self.settings.tab_opened(&provider, radar.as_str());
        self.login_tab = Some((provider.clone(), radar.clone()));
        // The sign-in is the operator's to answer: the dock shows it, on Settings too.
        self.review_fx.push(Effect::ShowDock);
        let inbox = self.inbox.clone();
        self.spawn(async move {
            match adapter.open_command(CreateSession { radar_session_id: radar.clone(), cwd, model: None, setup: SessionSetup::default() }, argv).await {
                Ok(psid) => Self::push(&inbox, Inbound::Created(radar, psid)),
                Err(e) => {
                    Self::push(&inbox, Inbound::Failed(Some(radar), format!("could not start: {e}")));
                    Self::push(&inbox, Inbound::Login(provider, LoginEvent::Failed(e.to_string())));
                }
            }
        });
    }

    /// The login tab's program ended: tell Settings, which reads the status again.
    fn watch_login_tab(&mut self) {
        let Some((provider, radar)) = self.login_tab.clone() else { return };
        let Some(adapter) = self
            .store
            .get(&radar)
            .and_then(|s| self.adapter(s.meta.provider.as_str()))
        else {
            self.login_tab = None;
            return;
        };
        let code = match adapter.terminal(&radar).map(|t| t.state) {
            Some(TerminalStatus::Exited(code)) => Some(code),
            _ => return,
        };
        self.login_tab = None;
        self.settings.tab_exited(&provider, code);
    }

    fn start_login(&mut self, provider: String) {
        let Some(adapter) = self.adapter(&provider) else {
            self.settings.login_event(&provider, LoginEvent::Failed("no such provider in this build".into()));
            return;
        };
        let (cancel_tx, mut cancel_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        self.logins.insert(provider.clone(), cancel_tx);
        let inbox = self.inbox.clone();
        self.spawn(async move {
            let mut handle = match adapter.start_login(None).await {
                Ok(handle) => handle,
                Err(e) => {
                    Self::push(&inbox, Inbound::Login(provider, LoginEvent::Failed(e.to_string())));
                    return;
                }
            };
            loop {
                let event = tokio::select! {
                    event = handle.next_event() => event,
                    _ = cancel_rx.recv() => {
                        handle.cancel();
                        LoginEvent::Cancelled
                    }
                };
                let last = !matches!(event, LoginEvent::DeviceCode { .. });
                Self::push(&inbox, Inbound::Login(provider.clone(), event));
                if last {
                    return;
                }
            }
        });
    }

    fn drain(&mut self) -> Vec<Effect> {
        let items: Vec<Inbound> = std::mem::take(&mut *self.inbox.items.lock().expect("inbox"));
        let mut persist = false;
        for item in items {
            match item {
                Inbound::Update(_, Update::Event(event)) if self.spare.as_ref().is_some_and(|s| s.radar == event.radar_session_id) => {
                    // Not shown yet: what a session created ahead says waits for the tab that takes it.
                    if let Some(s) = self.spare.as_mut() {
                        s.events.push(event);
                    }
                }
                Inbound::Update(provider, Update::Event(mut event)) => {
                    // A review session keeps its own stricter policy: nothing in it is approved for the session.
                    if let EventKind::PermissionRequested { request } = &mut event.kind
                        && self.review.is_review(&event.radar_session_id)
                    {
                        request.can_offer_session_approval = false;
                    }
                    let review = self.review.is_review(&event.radar_session_id).then(|| event.clone());
                    let asked = matches!(event.kind, EventKind::PermissionRequested { .. }).then(|| event.clone());
                    // The cursor moves on persisted events only: streamed text writes no cache.
                    persist |= event.durability == Durability::Persisted;
                    let ingested = self.store.ingest(event);
                    let _ = provider;
                    let mut standing = false;
                    if let (Some(event), Some(crate::store::Ingested::Applied)) = (asked, ingested) {
                        standing = self.standing_rule(&event) || self.session_approval(&event);
                        if let (true, EventKind::PermissionRequested { request }) = (standing, &event.kind) {
                            self.auto_answered.insert(request.request_id.clone());
                        }
                    }
                    if let (Some(event), Some(crate::store::Ingested::Applied)) = (review, ingested) {
                        self.review_event(event, standing);
                    }
                }
                Inbound::Update(provider, Update::Disconnected { reason }) => {
                    // A session made ahead on a connection that is gone is no use; the next settle makes another.
                    if self.spare.as_ref().is_some_and(|s| s.provider == provider.as_str()) {
                        self.spare = None;
                    }
                    self.store.set_connected(&provider, false);
                    for s in self.store.sessions().map(|s| s.meta.radar_session_id.clone()).collect::<Vec<_>>() {
                        if let Some(s) = self.store.get_mut(&s)
                            && s.meta.provider == provider
                        {
                            s.note(format!("connection lost: {reason}"));
                        }
                    }
                }
                Inbound::Update(_, Update::Commands { session, commands }) => {
                    if let Some(s) = self.spare.as_mut().filter(|s| s.radar == session) {
                        let key = warm::commands_key(&s.provider, &s.cwd.display().to_string());
                        persist |= self.cache.set_commands(&key, commands.clone(), self.now);
                        s.commands = Some(commands);
                    } else if let Some(s) = self.store.get_mut(&session) {
                        // What the provider says a session of this folder offers after `/` is
                        // kept, so the next session's menu is full before it has said it again.
                        let key = warm::commands_key(s.meta.provider.as_str(), &s.meta.cwd);
                        persist |= self.cache.set_commands(&key, commands.clone(), self.now);
                        s.commands = Some(commands);
                    }
                }
                Inbound::Update(provider, Update::Reconnected) => {
                    self.store.set_connected(&provider, true);
                    self.resume_all(&provider);
                }
                Inbound::Status(provider, status) => {
                    if usable(&status) {
                        self.ready.insert(provider.clone());
                    } else {
                        self.ready.remove(&provider);
                    }
                    self.settings.set_status(&provider, status);
                }
                Inbound::Models(provider, models) => {
                    self.cache.set_models(&provider, models.clone(), self.now);
                    self.models.insert(provider, models);
                    persist = true;
                }
                Inbound::Created(radar, psid) => {
                    if let Some(s) = self.spare.as_mut().filter(|s| s.radar == radar) {
                        s.psid = Some(psid);
                    } else {
                        if let Some(s) = self.store.get_mut(&radar) {
                            s.meta.provider_session_id = psid;
                        }
                        persist = true;
                    }
                }
                // A session made ahead that could not be: no notice (nobody asked for it yet), no more tries.
                Inbound::Failed(Some(radar), _) if self.spare.as_ref().is_some_and(|s| s.radar == radar) => {
                    self.spare = None;
                    self.spare_off = true;
                }
                Inbound::Failed(Some(radar), message) => {
                    let todo = self.review.start_failed(&radar, message.clone(), self.now);
                    self.review_todo(todo);
                    if let Some(s) = self.store.get_mut(&radar) {
                        s.fail(message);
                    }
                }
                Inbound::SendFailed(radar, text, message) => {
                    if let Some(s) = self.store.get_mut(&radar) {
                        s.send_failed(&text);
                        s.fail(message);
                    }
                }
                Inbound::Failed(None, message) => {
                    if let Some(radar) = self.active.clone()
                        && let Some(s) = self.store.get_mut(&radar)
                    {
                        s.note(message);
                    }
                }
                Inbound::Sessions(provider, list) => {
                    self.provider_sessions.retain(|(p, _)| *p != provider);
                    self.provider_sessions.extend(list.into_iter().map(|i| (provider.clone(), i)));
                }
                Inbound::Catalog { provider, key, catalog } => {
                    self.cache.set_catalog(&key, catalog.clone(), self.now);
                    self.settings.set_catalog(&provider, catalog);
                    persist = true;
                }
                // What was read before stays: a refresh that failed is not a reason to empty the picker.
                Inbound::CatalogFailed { provider, key, why } => {
                    if self.cache.catalog(&key).is_none() {
                        self.settings.set_catalog(&provider, Catalog::default());
                        self.review_fx.push(Effect::Notify(format!("{provider} could not list its agents and skills: {why}")));
                    }
                }
                Inbound::Login(provider, event) => self.settings.login_event(&provider, event),
            }
        }
        self.send_due();
        self.watch_login_tab();
        if self.settings_seen.load(Ordering::Relaxed) && !self.statuses_asked {
            self.statuses_asked = true;
            self.read_unread_statuses();
        }
        self.handle_requests();
        self.settle();
        let todo = self.review.tick(self.now);
        self.review_todo(todo);
        let told = self.time_turns();
        let mut fx = std::mem::take(&mut self.review_fx);
        fx.extend(told);
        if persist || std::mem::take(&mut self.persist_soon) {
            fx.push(Effect::PersistCache);
        }
        fx
    }

    fn resume_all(&mut self, provider: &ProviderId) {
        let Some(adapter) = self.adapter(provider.as_str()) else { return };
        let sessions: Vec<SessionMeta> = self
            .store
            .sessions()
            .filter(|s| &s.meta.provider == provider)
            .map(|s| s.meta.clone())
            .collect();
        let inbox = self.inbox.clone();
        let setup = self.setup(provider.as_str());
        self.spawn(async move {
            for meta in sessions {
                let req = ResumeSession {
                    radar_session_id: meta.radar_session_id.clone(),
                    provider_session_id: meta.provider_session_id.clone(),
                    cwd: meta.cwd.clone().into(),
                    cursor: meta.cursor.clone(),
                    setup: SessionSetup { agent: meta.agent.clone().or_else(|| setup.agent.clone()), ..setup.clone() },
                };
                if let Err(e) = adapter.resume(req).await {
                    Self::push(&inbox, Inbound::Failed(Some(meta.radar_session_id), format!("could not resume: {e}")));
                }
            }
        });
    }

    // --- what the dock asks ------------------------------------------------

    fn next_id(&mut self) -> RadarSessionId {
        self.counter += 1;
        RadarSessionId::new(format!("radar-{}-{}", self.now, self.counter))
    }

    fn new_session(&mut self, provider: Option<String>, resume: Option<ProviderSessionId>) {
        self.new_session_with(provider, resume, None, None)
    }

    /// A new session; `first` is sent as its first message once it exists,
    /// and `recipe` names the agent and skills it runs with.
    fn new_session_with(
        &mut self,
        provider: Option<String>,
        resume: Option<ProviderSessionId>,
        first: Option<String>,
        recipe: Option<crate::brief::Recipe>,
    ) {
        let Some(provider) = provider.or_else(|| self.default_provider()) else { return };
        let Some(adapter) = self.adapter(&provider) else { return };
        // A provider nothing has needed yet starts now (the default one started at launch).
        self.warm_provider(&provider);
        let (cwd, model, mut setup) = self.session_inputs(&provider);
        if let Some(r) = recipe {
            if let Some(a) = r.agent_for(&provider) {
                setup.agent = Some(a);
            }
            setup.skills = r.skills;
        }
        // A session already created for exactly this takes the new tab: it only attaches.
        let spare = if resume.is_none() { self.take_spare(&provider, &cwd, &model, &setup, first.is_some()) } else { None };
        let radar = spare.as_ref().map(|s| s.radar.clone()).unwrap_or_else(|| self.next_id());
        // The first number no open tab of this name uses: a tab closed in the middle leaves its number to the next one,
        // and two tabs never read alike.
        let name = setup.agent.clone().unwrap_or_else(|| provider.clone());
        let taken = |n: usize| self.store.sessions().any(|s| s.meta.title == format!("{name} {n}"));
        let n = (1..).find(|&n| !taken(n)).unwrap_or(1);
        let meta = SessionMeta {
            radar_session_id: radar.clone(),
            provider: adapter.id(),
            provider_session_id: resume
                .clone()
                .or_else(|| spare.as_ref().and_then(|s| s.psid.clone()))
                .unwrap_or_else(|| ProviderSessionId::new("")),
            cwd: cwd.display().to_string(),
            title: match &setup.agent {
                Some(a) => format!("{a} {n}"),
                None => format!("{provider} {n}"),
            },
            model: model.clone(),
            created_at: self.now,
            cursor: None,
            agent: setup.agent.clone(),
        };
        let s = self.store.insert(meta);
        // The first message goes with the create: anything typed meanwhile waits for its turn.
        if first.is_some() {
            s.sending();
        }
        self.order.push(radar.clone());
        self.active = Some(radar.clone());
        let inbox = self.inbox.clone();
        if let Some(spare) = spare {
            // What the session said while it waited is its history now, in the order it came.
            for event in spare.events {
                self.store.ingest(event);
            }
            if let Some(s) = self.store.get_mut(&radar) {
                s.commands = spare.commands;
            }
            if let Some(text) = first {
                self.spawn(async move {
                    if let Err(e) = adapter.send(&radar, SendInput { text }).await {
                        Self::push(&inbox, Inbound::Failed(Some(radar), format!("could not send: {e}")));
                    }
                });
            }
            // The next one is made now, so the tab after this is as quick.
            self.settle();
            return;
        }
        self.spawn(async move {
            match resume {
                Some(psid) => {
                    let req = ResumeSession { radar_session_id: radar.clone(), provider_session_id: psid, cwd, cursor: None, setup };
                    if let Err(e) = adapter.resume(req).await {
                        Self::push(&inbox, Inbound::Failed(Some(radar), format!("could not resume: {e}")));
                    }
                }
                None => match adapter.create(CreateSession { radar_session_id: radar.clone(), cwd, model, setup }).await {
                    Ok(psid) => {
                        Self::push(&inbox, Inbound::Created(radar.clone(), psid));
                        if let Some(text) = first
                            && let Err(e) = adapter.send(&radar, SendInput { text }).await
                        {
                            Self::push(&inbox, Inbound::Failed(Some(radar), format!("could not send: {e}")));
                        }
                    }
                    Err(e) => Self::push(&inbox, Inbound::Failed(Some(radar), format!("could not start: {e}"))),
                },
            }
        });
    }

    fn active_adapter(&self) -> Option<(RadarSessionId, Arc<dyn Adapter>)> {
        let radar = self.active.clone()?;
        let provider = self.store.get(&radar)?.meta.provider.clone();
        Some((radar, self.adapter(provider.as_str())?))
    }

    fn answer(&mut self, id: &str, decision: PermissionDecision) {
        let Some(radar) = self.active.clone() else { return };
        self.answer_for(&radar, id, decision)
    }

    /// A standing allow or deny rule from Settings → Agents that matches the
    /// request answers it, with a line in the transcript naming the rule.
    /// Only a match answers; everything else waits for the operator. Returns
    /// whether a rule answered.
    fn standing_rule(&mut self, event: &Event) -> bool {
        let EventKind::PermissionRequested { request } = &event.kind else { return false };
        let verdict = self.settings.defaults().standing(request).map(|(allow, rule)| (allow, rule.to_string()));
        let Some((allow, rule)) = verdict else { return false };
        let radar = &event.radar_session_id;
        // A review session's own refusals (writes, posting tools, the network)
        // stand over an allow rule.
        if allow && self.review.is_review(radar) {
            let cwd = self.store.get(radar).map(|s| s.meta.cwd.clone()).unwrap_or_default();
            if matches!(crate::review::policy::decide(request, std::path::Path::new(&cwd)), crate::review::policy::Verdict::Deny(_)) {
                return false;
            }
        }
        if let Some(s) = self.store.get_mut(radar) {
            s.note(if allow { format!("approved once by your allow rule {rule}") } else { format!("denied by your deny rule {rule}") });
        }
        let decision = if allow {
            PermissionDecision::ApproveOnce
        } else {
            PermissionDecision::Deny { reason: Some(format!("denied by the operator's standing rule {rule}")) }
        };
        self.answer_for(radar, &request.request_id, decision);
        true
    }

    /// The operator approved the same kind and target for this session
    /// before ("approve for session"): the request is approved again, with a
    /// line in the transcript. A provider that remembers the approval itself
    /// does not ask again; this answers the ones that do. Standing rules come
    /// first (a deny rule stands over it). Returns whether it answered.
    fn session_approval(&mut self, event: &Event) -> bool {
        let EventKind::PermissionRequested { request } = &event.kind else { return false };
        let radar = &event.radar_session_id;
        if self.review.is_review(radar) {
            return false;
        }
        let Some(s) = self.store.get_mut(radar) else { return false };
        if !s.approved_for_session(request) {
            return false;
        }
        s.note(format!("approved for this session: {}", crate::dock::target_text(&request.target)));
        s.approve_for_session(request);
        self.answer_for(radar, &request.request_id, PermissionDecision::ApproveOnce);
        true
    }

    fn answer_for(&mut self, radar: &RadarSessionId, id: &str, decision: PermissionDecision) {
        let radar = radar.clone();
        let Some(adapter) = self.store.get(&radar).and_then(|s| self.adapter(s.meta.provider.as_str())) else { return };
        // Only a request that is waiting in the store can be answered.
        let Some(request) = self.store.get(&radar).and_then(|s| s.pending().into_iter().find(|r| r.request_id == id).cloned()) else {
            return;
        };
        // Approve for session only where the request offers it (never in a review session); radar remembers it too.
        let decision = match decision {
            PermissionDecision::ApproveForSession if !request.can_offer_session_approval || self.review.is_review(&radar) => {
                PermissionDecision::ApproveOnce
            }
            PermissionDecision::ApproveForSession => {
                if let Some(s) = self.store.get_mut(&radar) {
                    s.approve_for_session(&request);
                }
                PermissionDecision::ApproveForSession
            }
            other => other,
        };
        let answer = PermissionAnswer { request_id: id.to_string(), decision };
        let inbox = self.inbox.clone();
        self.spawn(async move {
            if let Err(e) = adapter.answer_permission(&radar, answer).await {
                Self::push(&inbox, Inbound::Failed(Some(radar), format!("could not answer: {e}")));
            }
        });
    }

    /// The operator answered one question of a waiting request in the active
    /// session: the choices picked, or their own words. Once every question
    /// of the request has a reply, the replies go to the agent. A reply that
    /// does not fit the question (a label it does not offer, words where it
    /// takes none) is dropped: nothing is sent that the operator did not pick.
    fn reply(&mut self, id: &str, question: &str, choices: &[String], text: Option<&str>) {
        let Some(radar) = self.active.clone() else { return };
        let Some(s) = self.store.get_mut(&radar) else { return };
        let (values, own_words) = match text.map(str::trim) {
            Some(t) if !t.is_empty() => (vec![t.to_string()], true),
            _ => (choices.to_vec(), false),
        };
        let fits = s
            .pending_questions()
            .into_iter()
            .find(|r| r.request_id == id)
            .and_then(|r| r.questions.iter().find(|q| q.id == question))
            .is_some_and(|q| {
                if own_words {
                    q.free_text
                } else {
                    !values.is_empty()
                        && (q.multiple || values.len() == 1)
                        && values.iter().all(|v| q.choices.iter().any(|c| &c.label == v))
                }
            });
        if !fits {
            return;
        }
        let reply = QuestionReply { question_id: question.to_string(), values, own_words };
        if let Some(replies) = s.draft(id, reply) {
            self.answer_question_for(&radar, id, QuestionDecision::Answer { replies });
        }
    }

    fn answer_question_for(&mut self, radar: &RadarSessionId, id: &str, decision: QuestionDecision) {
        let radar = radar.clone();
        let Some(adapter) = self.store.get(&radar).and_then(|s| self.adapter(s.meta.provider.as_str())) else { return };
        // Only a question that is waiting in the store can be answered.
        let waiting = self.store.get(&radar).is_some_and(|s| s.pending_questions().iter().any(|r| r.request_id == id));
        if !waiting {
            return;
        }
        let answer = QuestionAnswer { request_id: id.to_string(), decision };
        let inbox = self.inbox.clone();
        self.spawn(async move {
            if let Err(e) = adapter.answer_question(&radar, answer).await {
                Self::push(&inbox, Inbound::Failed(Some(radar), format!("could not answer: {e}")));
            }
        });
    }

    /// The operator's message. It is never refused for a turn that runs: a
    /// provider that steers takes it now, one that queues takes it now and
    /// runs it after the turn (shown queued until it does), and for any other
    /// radar keeps it, shown queued, and sends it when the turn ends. So does a
    /// session not connected or not started yet.
    fn send(&mut self, text: String) {
        let Some((radar, adapter)) = self.active_adapter() else { return };
        if text.trim().is_empty() {
            return;
        }
        self.history.push(text.clone());
        let caps = adapter.capabilities();
        let Some(s) = self.store.get_mut(&radar) else { return };
        let ready = s.connected() && !s.meta.provider_session_id.as_str().is_empty();
        let held = s.queued.iter().any(|q| q.holder == crate::store::Holder::Radar);
        if !ready || held || (s.busy() && !caps.steer && !caps.queue) {
            // Behind the ones already waiting, in the order they were typed.
            s.queue(text, crate::store::Holder::Radar);
            return;
        }
        if s.busy() && !caps.steer {
            s.queue(text.clone(), crate::store::Holder::Provider);
        }
        s.sending();
        self.send_now(radar, adapter, text);
    }

    fn send_now(&mut self, radar: RadarSessionId, adapter: Arc<dyn Adapter>, text: String) {
        let inbox = self.inbox.clone();
        self.spawn(async move {
            if let Err(e) = adapter.send(&radar, SendInput { text: text.clone() }).await {
                Self::push(&inbox, Inbound::SendFailed(radar, text, format!("could not send: {e}")));
            }
        });
    }

    /// Messages radar kept for a turn to end: the oldest of each session that is
    /// free now goes.
    fn send_due(&mut self) {
        let ids: Vec<RadarSessionId> = self.order.clone();
        for radar in ids {
            let Some(adapter) = self.store.get(&radar).and_then(|s| self.adapter(s.meta.provider.as_str())) else { continue };
            if let Some(text) = self.store.get_mut(&radar).and_then(|s| s.due()) {
                self.send_now(radar, adapter, text);
            }
        }
    }

    fn stop(&mut self) {
        let Some((radar, adapter)) = self.active_adapter() else { return };
        let todo = self.review.stopped(&radar, self.now);
        self.review_todo(todo);
        let inbox = self.inbox.clone();
        self.spawn(async move {
            if let Err(e) = adapter.stop(&radar).await {
                Self::push(&inbox, Inbound::Failed(Some(radar), format!("could not stop: {e}")));
            }
        });
    }

    /// The provider the pickers are about: the active session's, else the default.
    fn picker_provider(&self) -> String {
        self.active
            .as_ref()
            .and_then(|id| self.store.get(id))
            .map(|s| s.meta.provider.to_string())
            .or_else(|| self.default_provider())
            .unwrap_or_default()
    }

    /// What a new session of `provider` starts with: where it works, the model, and the setup.
    fn session_inputs(&self, provider: &str) -> (std::path::PathBuf, Option<String>, SessionSetup) {
        let cwd = self.settings.work_dir(std::env::current_dir().unwrap_or_else(|_| ".".into()));
        let model = self.chosen_model.clone().or_else(|| self.settings.defaults().model.clone());
        (cwd, model, self.setup(provider))
    }

    /// What `provider`'s CLI offers a session in the working folder. What was read before for
    /// this folder and setup is shown at once; the CLI is asked in the background unless that was
    /// read recently (`force` asks anyway), and its answer replaces it.
    fn read_catalog(&mut self, provider: &str, force: bool) {
        let Some(adapter) = self.adapter(provider) else { return };
        let caps = adapter.capabilities();
        if !caps.agents && !caps.skills {
            return;
        }
        let (cwd, _, setup) = self.session_inputs(provider);
        let ckey = warm::catalog_key(provider, &cwd, &setup);
        let shown = provider.strip_prefix("pty:").unwrap_or(provider).to_string();
        if let Some(cached) = self.cache.catalog(&ckey) {
            if self.settings.catalog(&shown) != Some(&cached.value) {
                self.settings.set_catalog(&shown, cached.value.clone());
            }
            if !force && cached.fresh(self.now, warm::CATALOG_FRESH) {
                return;
            }
        }
        let inbox = self.inbox.clone();
        self.spawn(async move {
            match adapter.catalog(cwd, setup).await {
                Ok(catalog) => Self::push(&inbox, Inbound::Catalog { provider: shown, key: ckey, catalog }),
                Err(e) => Self::push(&inbox, Inbound::CatalogFailed { provider: shown, key: ckey, why: e.to_string() }),
            }
        });
    }

    /// Read `provider`'s models unless the cache has them from the last hour (the picker shows
    /// whatever it has meanwhile).
    fn read_models(&mut self, provider: &str) {
        let Some(adapter) = self.adapter(provider) else { return };
        if !adapter.capabilities().list_models || self.cache.models(provider).is_some_and(|m| m.fresh(self.now, warm::MODELS_FRESH)) {
            return;
        }
        let (inbox, provider) = (self.inbox.clone(), provider.to_string());
        self.spawn(async move {
            if let Ok(models) = adapter.list_models().await {
                Self::push(&inbox, Inbound::Models(provider, models));
            }
        });
    }

    /// List the sessions `provider` holds, for the session picker.
    fn read_sessions(&mut self, provider: &str) {
        let Some(adapter) = self.adapter(provider) else { return };
        if !adapter.capabilities().list_sessions {
            return;
        }
        let (inbox, provider) = (self.inbox.clone(), provider.to_string());
        self.spawn(async move {
            if let Ok(list) = adapter.list_sessions().await {
                Self::push(&inbox, Inbound::Sessions(provider, list));
            }
        });
    }

    fn open_picker(&mut self, kind: PickerKind) {
        self.picker = Some(kind);
        let provider = self.picker_provider();
        // A picker about a CLI nothing has started yet starts it.
        self.warm_provider(&provider);
        match kind {
            PickerKind::Agent | PickerKind::Skill => self.read_catalog(&provider, false),
            PickerKind::Model => self.read_models(&provider),
            PickerKind::Session => {
                let providers: Vec<String> =
                    self.adapters.iter().filter(|a| a.capabilities().list_sessions).map(|a| a.id().to_string()).collect();
                for p in providers {
                    self.read_sessions(&p);
                }
            }
        }
    }

    // --- the session kept ready ----------------------------------------------

    /// Make a session for the next new tab, when the default provider can keep one for free (its
    /// CLI lists nothing for a session nobody has written to), the dock is open and one is not
    /// there already.
    fn ensure_spare(&mut self, provider: &str) {
        if self.spare.is_some() || self.spare_off {
            return;
        }
        let Some(adapter) = self.adapter(provider) else { return };
        if !adapter.idle_session_is_free() {
            return;
        }
        let (cwd, model, setup) = self.session_inputs(provider);
        let radar = self.next_id();
        self.spare = Some(Spare {
            radar: radar.clone(),
            provider: provider.to_string(),
            cwd: cwd.clone(),
            model: model.clone(),
            setup: setup.clone(),
            psid: None,
            events: Vec::new(),
            commands: None,
        });
        let inbox = self.inbox.clone();
        self.spawn(async move {
            match adapter.create(CreateSession { radar_session_id: radar.clone(), cwd, model, setup }).await {
                Ok(psid) => Self::push(&inbox, Inbound::Created(radar, psid)),
                Err(e) => Self::push(&inbox, Inbound::Failed(Some(radar), e.to_string())),
            }
        });
    }

    /// The spare is dropped when what a new tab would start with is not what it was made with
    /// (another model, agent, folder or folder list since), and made again by the next settle.
    fn check_spare(&mut self) {
        let Some(s) = &self.spare else { return };
        let (cwd, model, setup) = self.session_inputs(&s.provider);
        if !s.fits(&s.provider, &cwd, &model, &setup) {
            self.discard_spare();
        }
    }

    fn discard_spare(&mut self) {
        let Some(s) = self.spare.take() else { return };
        if let Some(adapter) = self.adapter(&s.provider) {
            self.spawn(async move {
                let _ = adapter.discard(&s.radar).await;
            });
        }
    }

    /// The spare, when it is exactly what a new tab with these inputs would be. A first message
    /// needs the provider's id for the session, so it waits for a spare that has one.
    fn take_spare(
        &mut self,
        provider: &str,
        cwd: &std::path::Path,
        model: &Option<String>,
        setup: &SessionSetup,
        needs_created: bool,
    ) -> Option<Spare> {
        let s = self.spare.as_ref()?;
        if !s.fits(provider, cwd, model, setup) || (needs_created && s.psid.is_none()) {
            return None;
        }
        self.persist_soon = true;
        self.spare.take()
    }

    /// The terminal tabs' screens, from the adapters that run a CLI's own screen.
    fn terminal_views(&self) -> BTreeMap<String, view::TerminalView> {
        let mut out = BTreeMap::new();
        for id in &self.order {
            let Some(s) = self.store.get(id) else { continue };
            let Some(t) = self.adapter(s.meta.provider.as_str()).and_then(|a| a.terminal(id)) else { continue };
            out.insert(
                id.to_string(),
                view::TerminalView {
                    id: id.to_string(),
                    socket: t.socket.display().to_string(),
                    columns: t.columns,
                    rows: t.rows,
                    state: match t.state {
                        TerminalStatus::Running => view::TerminalState::Running,
                        TerminalStatus::Exited(code) => view::TerminalState::Exited { code },
                        TerminalStatus::Failed => view::TerminalState::Failed,
                    },
                    command: t.command,
                    cwd: t.cwd,
                },
            );
        }
        out
    }

    /// The menu id of the question "stop it and close it?"; the session's id follows.
    const CLOSE_MENU: &'static str = "agents.close:";

    /// Close a tab. An idle session closes at once; one whose turn runs (or waits for an answer) is asked
    /// about first, because closing it stops the turn.
    fn close_tab(&mut self, id: &str) -> Vec<Effect> {
        let radar = RadarSessionId::new(id);
        let Some(s) = self.store.get(&radar) else { return vec![] };
        let on_terminal = self.adapter(s.meta.provider.as_str()).is_some_and(|a| a.terminal(&radar).is_some());
        if !on_terminal && matches!(s.state, crate::store::SessionState::Running | crate::store::SessionState::WaitingApproval) {
            let ask = radar_core::Menu::confirm(format!("{}{id}", Self::CLOSE_MENU), format!("close {}?", s.meta.title), "stop it and close", "keep it")
                .saying("its turn is still running; closing stops it. The session stays in the provider's list (alt-s opens it again).");
            return vec![Effect::Menu(ask)];
        }
        self.close_now(&radar, false)
    }

    /// Close the tab: stop the turn if `stop`, drop the session from radar and leave the next tab (else the
    /// previous) showing; with no tab left the dock hides and the page has its keys back. A terminal tab's program
    /// ends with its tab; a session stays with its provider, resumable from the session picker, and the provider is
    /// always told to stop whatever it still runs for it (an idle one has nothing; a turn that began unseen ends).
    fn close_now(&mut self, radar: &RadarSessionId, stop: bool) -> Vec<Effect> {
        let Some(s) = self.store.get(radar) else { return vec![] };
        let provider = s.meta.provider.clone();
        let psid = s.meta.provider_session_id.clone();
        if let Some(adapter) = self.adapter(provider.as_str()) {
            if adapter.terminal(radar).is_some() {
                self.spawn(async move {
                    let _ = adapter.delete_session(&psid).await;
                });
            } else {
                if stop {
                    let todo = self.review.stopped(radar, self.now);
                    self.review_todo(todo);
                }
                let radar = radar.clone();
                self.spawn(async move {
                    let _ = adapter.stop(&radar).await;
                });
            }
        }
        let at = self.order.iter().position(|x| x == radar);
        self.order.retain(|x| x != radar);
        self.store.remove(radar);
        self.clocks.remove(radar);
        self.unread.remove(radar);
        if self.active.as_ref() == Some(radar) {
            self.active = at.and_then(|i| self.order.get(i).or_else(|| self.order.last())).cloned();
            self.seen();
        }
        let mut fx = std::mem::take(&mut self.review_fx);
        fx.push(Effect::PersistCache);
        if self.order.is_empty() {
            fx.push(Effect::HideDock);
        }
        fx
    }
}

impl Drop for SessionsPlugin {
    fn drop(&mut self) {
        // Dropping a runtime inside another runtime's thread panics; hand it off.
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }
    }
}

impl Plugin for SessionsPlugin {
    fn id(&self) -> &'static str {
        "sessions"
    }

    fn name(&self) -> &'static str {
        "Agents"
    }

    fn configure(&mut self, table: Option<&toml::Value>) -> anyhow::Result<()> {
        self.settings.configure(table)?;
        // Reviews go where the workspace keeps notes; with no such part, into
        // the reviews folder set in Settings → Agents, else radar's state folder
        // (the part's own place, read back at start).
        if self.review.fallback_dir.is_none() {
            let state = std::env::var_os("XDG_STATE_HOME")
                .map(std::path::PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")));
            if let Some(dir) = self.settings.reviews_dir().or_else(|| state.map(|s| s.join("radar").join("reviews"))) {
                crate::review::driver::load(&dir, &mut self.review.reviews);
                self.review.fallback_dir = Some(dir);
            }
        }
        self.apply_settings();
        Ok(())
    }

    fn config(&self) -> Option<toml::Value> {
        self.settings.config()
    }

    fn restore(&mut self, cache: serde_json::Value) {
        if let Some(reviews) = cache.get("reviews").and_then(|v| serde_json::from_value::<crate::review::Reviews>(v.clone()).ok()) {
            self.review.reviews = reviews;
        }
        // What the pickers and the `/` menu showed last run: shown at once, read again in the background.
        if let Some(warm) = cache.get("warm").and_then(|v| serde_json::from_value::<Cache>(v.clone()).ok()) {
            for (provider, models) in warm.all_models() {
                self.models.insert(provider.clone(), models.clone());
            }
            self.cache = warm;
        }
        let mut store = Store::new();
        store.restore(cache.get("sessions").cloned().unwrap_or_default());
        self.restored = store.sessions().map(|s| s.meta.clone()).collect();
        for meta in &self.restored {
            self.store.insert(meta.clone());
        }
        self.order = cache
            .get("order")
            .and_then(|v| serde_json::from_value::<Vec<RadarSessionId>>(v.clone()).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|id| self.store.get(id).is_some())
            .collect();
        for meta in &self.restored {
            if !self.order.contains(&meta.radar_session_id) {
                self.order.push(meta.radar_session_id.clone());
            }
        }
        self.active = cache
            .get("active")
            .and_then(|v| serde_json::from_value::<RadarSessionId>(v.clone()).ok())
            .filter(|id| self.store.get(id).is_some())
            .or_else(|| self.order.last().cloned());
    }

    fn snapshot(&self) -> Option<serde_json::Value> {
        if self.store.sessions().next().is_none() && self.review.reviews.is_empty() && self.cache.is_empty() {
            return None;
        }
        Some(serde_json::json!({
            "sessions": self.store.snapshot(),
            "order": self.order,
            "active": self.active,
            "reviews": self.review.reviews,
            "warm": self.cache,
        }))
    }

    fn tick(&mut self, now: i64) -> Vec<Effect> {
        self.now = now;
        if !self.started && !self.adapters.is_empty() {
            self.start();
        }
        self.drain()
    }

    fn set_waker(&mut self, waker: radar_core::wake::Waker) {
        *self.inbox.waker.lock().expect("waker") = Some(waker);
    }

    /// A task pushed into the inbox: what it brought shows now, not at the next tick.
    fn woken(&mut self, now: i64) -> Vec<Effect> {
        self.now = now;
        self.drain()
    }

    fn sections(&self) -> Vec<SectionSpec> {
        settings::sections()
    }

    fn section_view(&self, section: &str, cx: &ViewCx<'_>) -> Option<SectionView> {
        // Settings → Agents is open: the CLIs not asked yet are asked on the next tick.
        self.settings_seen.store(true, Ordering::Relaxed);
        self.settings.section_view(section, cx)
    }

    fn intent(&mut self, scope: &str, _pane: Pane, intent: &ViewIntent) -> Outcome {
        let outcome = self.settings.intent(scope, intent);
        self.handle_requests();
        outcome
    }

    fn typing(&self, menu: &str, text: &str) -> Typing {
        self.settings.typing(menu, text)
    }

    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        if let Some(id) = menu.strip_prefix(Self::CLOSE_MENU) {
            return if value == "yes" { self.close_now(&RadarSessionId::new(id), true) } else { vec![] };
        }
        let fx = self.settings.on_menu(menu, value).unwrap_or_default();
        self.handle_requests();
        self.apply_settings();
        fx
    }

    fn dock(&self) -> Option<view::DockView> {
        if self.adapters.is_empty() && self.order.is_empty() {
            return None;
        }
        let active = self.active.as_ref().and_then(|id| self.store.get(id));
        let caps = active.and_then(|s| self.adapter(s.meta.provider.as_str())).map(|a| a.capabilities());
        let provider = active
            .map(|s| s.meta.provider.to_string())
            .or_else(|| self.default_provider())
            .unwrap_or_default();
        let models = self.models.get(&provider).cloned().unwrap_or_default();
        let terminals = self.terminal_views();
        Some(dock::dock_view(&self.inputs(caps.as_ref(), &models, &terminals, &provider)))
    }

    fn dock_tabs(&self) -> Option<(Vec<view::DockTab>, Option<String>)> {
        if self.adapters.is_empty() && self.order.is_empty() {
            return None;
        }
        let terminals = self.terminal_views();
        Some(dock::dock_tabs(&self.inputs(None, &[], &terminals, "")))
    }

    fn dock_visible(&mut self, visible: bool) {
        self.dock_visible = visible;
        self.seen();
        // The first time the dock shows in this run: what its first session and pickers need is made ready.
        if visible && !self.dock_open {
            self.dock_open = true;
            if let Some(provider) = self.default_provider() {
                self.warm_provider(&provider);
            }
            self.settle();
        }
    }

    fn agent_brief(&mut self, contexts: Vec<radar_core::agent_context::AgentContext>) -> Outcome {
        use crate::brief::{Brief, BriefRequest, Recipe};
        let (recipe, _, broken) = Recipe::load("ask", self.settings.recipes_dir().as_deref());
        let recipe = recipe.unwrap_or_else(Recipe::ask);
        let brief = Brief::build(&BriefRequest { recipe: &recipe, task: String::new(), contexts });
        // The prompt goes to a new session as its first message, at once.
        self.new_session_with(None, None, Some(brief.text), Some(recipe));
        Outcome::Handled(broken.map(|why| vec![Effect::Notify(format!("your ask recipe is not used: {why}"))]).unwrap_or_default())
    }

    fn review_start(
        &mut self,
        origin: Option<radar_core::agent_context::AgentContext>,
        change: radar_core::journal::SubjectKey,
    ) -> Outcome {
        let Some(provider) = self.default_provider() else {
            return Outcome::Handled(vec![Effect::Notify("no agent provider to review with".into())]);
        };
        let todo = self.review.start(change, origin, &provider, self.now);
        self.review_todo(todo);
        Outcome::Handled(std::mem::take(&mut self.review_fx))
    }

    fn adds(&self) -> Vec<radar_core::Adds> {
        vec![radar_core::Adds::new(
            "merge-request",
            "a chip when a local review exists, and when its head moved; opening the review",
        )]
    }

    /// The `x` menu of a reviewed merge request opens its local review, or
    /// copies where it is.
    fn act(&mut self, subject: &radar_core::Subject, action: &str) -> Vec<Effect> {
        let Some(path) = self.review.reviews.of_id(&subject.id).and_then(|r| r.path.clone()) else {
            return vec![];
        };
        match action {
            REVIEW_OPEN => vec![Effect::Open(format!("file://{path}"))],
            REVIEW_COPY => vec![Effect::Copy(path)],
            _ => vec![],
        }
    }

    /// A merge request that was reviewed locally says so on its row, with
    /// the sha reviewed; a head that moved since marks it stale. The head
    /// is the subject's `head_sha` field when its owner gives one.
    fn contribute(&self, point: radar_core::Point, subject: &radar_core::Subject) -> Option<radar_core::Contribution> {
        use radar_core::plugin::Tone;
        if subject.kind != "merge-request" {
            return None;
        }
        if point == radar_core::Point::Actions {
            self.review.reviews.of_id(&subject.id)?.path.as_ref()?;
            let row = |what: &str, id: &str| radar_core::Action { key: None, ..radar_core::Action::new('o', what, id) };
            return Some(radar_core::Contribution::Actions(vec![
                row("open the local review", REVIEW_OPEN),
                row("copy the local review's path", REVIEW_COPY),
            ]));
        }
        if point != radar_core::Point::Chips {
            return None;
        }
        let head = subject.fields.get("head_sha").map(String::as_str);
        let chip = self.review_chip(&subject.id, head)?;
        let tone = if chip.contains("stale") { Tone::Warn } else { Tone::Good };
        Some(radar_core::Contribution::Chips(vec![radar_core::Chip::new(chip, tone)]))
    }

    fn review_asks(&mut self) -> Vec<(u64, radar_core::plugin::ReviewAsk)> {
        self.review.take_asks()
    }

    fn review_answer(&mut self, run: u64, answer: radar_core::plugin::ReviewAnswer) -> Outcome {
        let todo = self.review.answer(run, answer, self.now);
        self.review_todo(todo);
        Outcome::Handled(std::mem::take(&mut self.review_fx))
    }

    fn dock_intent(&mut self, intent: &DockIntent) -> Outcome {
        match intent {
            // `/close` is the dock's own command (dock.rs lists it): the active tab closes like its close mark.
            DockIntent::Send { text } if text.trim() == "/close" && self.active.is_some() => {
                let id = self.active.clone().map(|g| g.to_string()).unwrap_or_default();
                return Outcome::Handled(self.close_tab(&id));
            }
            DockIntent::Send { text } => self.send(text.clone()),
            DockIntent::Stop => self.stop(),
            DockIntent::Approve { id } => self.answer(id, PermissionDecision::ApproveOnce),
            DockIntent::ApproveForSession { id } => self.answer(id, PermissionDecision::ApproveForSession),
            DockIntent::Deny { id } => self.answer(id, PermissionDecision::Deny { reason: Some("denied in radar".into()) }),
            DockIntent::Answer { id, question, choices, text } => self.reply(id, question, choices, text.as_deref()),
            DockIntent::Decline { id } => {
                if let Some(radar) = self.active.clone() {
                    let reason = Some("the operator declined to answer".to_string());
                    self.answer_question_for(&radar, id, QuestionDecision::Decline { reason });
                }
            }
            DockIntent::Fold { item, open } => {
                self.folds.insert(item.clone(), *open);
            }
            DockIntent::OpenPicker { kind } => self.open_picker(*kind),
            DockIntent::ClosePicker => self.picker = None,
            DockIntent::PickAgent { id } => {
                self.picker = None;
                let provider = self.picker_provider();
                self.chosen_agent = (!id.is_empty()).then(|| (provider, id.clone()));
                if let Some(s) = self.active.clone().and_then(|g| self.store.get_mut(&g)) {
                    s.note(if id.is_empty() { "new sessions start as the CLI's own agent".into() } else { format!("new sessions start as {id}") });
                }
            }
            // Ink puts the skill's request in the composer; the host only closes the picker.
            DockIntent::PickSkill { .. } => self.picker = None,
            DockIntent::PickModel { id } => {
                self.chosen_model = Some(id.clone());
                self.picker = None;
                if let Some(s) = self.active.clone().and_then(|g| self.store.get_mut(&g)) {
                    s.note(format!("model {id} applies to new sessions"));
                }
            }
            DockIntent::PickSession { id } => {
                self.picker = None;
                if let Some((provider, psid)) = id.split_once(':') {
                    self.new_session(Some(provider.to_string()), Some(ProviderSessionId::new(psid)));
                }
            }
            DockIntent::NewSession { provider } => self.new_session(provider.clone(), None),
            DockIntent::SwitchTab { id } => {
                let radar = RadarSessionId::new(id.clone());
                if self.store.get(&radar).is_some() {
                    self.active = Some(radar);
                    self.seen();
                }
            }
            DockIntent::CloseTab { id } => return Outcome::Handled(self.close_tab(id)),
            // Terminal tabs belong to the PTY adapter; host-owned intents never get here.
            DockIntent::RestartTerminal { id } => {
                let radar = RadarSessionId::new(id.clone());
                if let Some(adapter) = self.store.get(&radar).and_then(|s| self.adapter(s.meta.provider.as_str())) {
                    let inbox = self.inbox.clone();
                    self.spawn(async move {
                        if let Err(e) = adapter.restart_terminal(&radar).await {
                            Self::push(&inbox, Inbound::Failed(Some(radar), format!("could not restart: {e}")));
                        }
                    });
                }
            }
            DockIntent::Show
            | DockIntent::Hide
            | DockIntent::Toggle
            | DockIntent::Focus
            | DockIntent::Blur
            | DockIntent::Resize { .. }
            | DockIntent::ToggleFull => return Outcome::Ignored,
        }
        Outcome::done()
    }
}

#[cfg(test)]
mod run_in_tab_tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// A terminal adapter that records the command it was asked to run and
    /// reports a chosen exit.
    type Ran = Arc<StdMutex<Vec<(String, Vec<String>)>>>;

    struct Tab {
        ran: Ran,
        exit: Arc<StdMutex<TerminalStatus>>,
    }

    impl Adapter for Tab {
        fn id(&self) -> ProviderId {
            ProviderId::new("pty:codex")
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }
        fn status(&self) -> BoxFuture<'_, AdapterStatus> {
            Box::pin(async {
                AdapterStatus {
                    installed: true,
                    version: None,
                    protocol: None,
                    auth: AuthStatus::Unknown,
                    error: None,
                    capabilities: Capabilities::default(),
                }
            })
        }
        fn list_models(&self) -> BoxFuture<'_, anyhow::Result<Vec<ModelInfo>>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn subscribe(&self) -> BoxFuture<'_, anyhow::Result<Updates>> {
            Box::pin(async { Ok(tokio::sync::mpsc::unbounded_channel().1) })
        }
        fn create(&self, _: CreateSession) -> BoxFuture<'_, anyhow::Result<ProviderSessionId>> {
            Box::pin(async { Err(anyhow::anyhow!("unused")) })
        }
        fn send(&self, _: &RadarSessionId, _: SendInput) -> BoxFuture<'_, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
        fn stop(&self, _: &RadarSessionId) -> BoxFuture<'_, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
        fn resume(&self, _: ResumeSession) -> BoxFuture<'_, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
        fn list_sessions(&self) -> BoxFuture<'_, anyhow::Result<Vec<ProviderSessionInfo>>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn delete_session(&self, _: &ProviderSessionId) -> BoxFuture<'_, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
        fn answer_permission(&self, _: &RadarSessionId, _: PermissionAnswer) -> BoxFuture<'_, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
        fn open_command(&self, req: CreateSession, argv: Vec<String>) -> BoxFuture<'_, anyhow::Result<ProviderSessionId>> {
            self.ran.lock().unwrap().push((req.radar_session_id.to_string(), argv));
            Box::pin(async move { Ok(ProviderSessionId::new(req.radar_session_id.as_str())) })
        }
        fn terminal(&self, _: &RadarSessionId) -> Option<TerminalSurface> {
            Some(TerminalSurface {
                socket: "/tmp/none.sock".into(),
                command: "codex".into(),
                cwd: ".".into(),
                columns: 80,
                rows: 24,
                state: *self.exit.lock().unwrap(),
            })
        }
    }

    /// Press ⏎ on the Codex row of Settings → Agents, as the operator does.
    fn sign_in(p: &mut SessionsPlugin) {
        use radar_core::view::Target;
        p.settings.set_status("codex", AdapterStatus {
            installed: true,
            version: None,
            protocol: None,
            auth: AuthStatus::SignedOut,
            error: None,
            capabilities: Capabilities::default(),
        });
        p.settings.intent(settings::SECTION, &ViewIntent::Select { target: Target::new("cli:codex") });
        p.settings.intent(settings::SECTION, &ViewIntent::Activate { target: Target::new("cli:codex") });
    }

    fn spin(p: &mut SessionsPlugin, done: impl Fn(&SessionsPlugin) -> bool) {
        for n in 0..2_000_000i64 {
            p.tick(n);
            if done(p) {
                return;
            }
            std::thread::yield_now();
        }
        panic!("never reached");
    }

    #[test]
    fn run_in_tab_opens_a_tab_running_the_given_command_and_its_end_reads_the_status() {
        let ran = Arc::new(StdMutex::new(Vec::new()));
        let exit = Arc::new(StdMutex::new(TerminalStatus::Running));
        let mut p = SessionsPlugin::new();
        p.add_adapter(Box::new(Tab { ran: ran.clone(), exit: exit.clone() }));
        p.tick(0);
        let argv = vec!["codex".to_string(), "login".to_string()];
        sign_in(&mut p);
        assert!(p.tick(0).iter().any(|e| matches!(e, Effect::ShowDock)), "the dock shows the sign-in");
        spin(&mut p, |p| !p.store.get(&p.order[0]).unwrap().meta.provider_session_id.as_str().is_empty());
        assert_eq!(p.order.len(), 1);
        assert_eq!(ran.lock().unwrap()[0].1, argv);
        assert_eq!(p.active.as_ref(), p.order.first());
        assert!(p.terminal_views().contains_key(p.order[0].as_str()));
        assert!(matches!(p.settings.login_state(), Some((_, crate::login::Login::InTab { .. }))));

        *exit.lock().unwrap() = TerminalStatus::Exited(0);
        p.tick(1);
        assert!(p.login_tab.is_none());
        assert!(matches!(p.settings.login_state(), Some((_, crate::login::Login::Done))));
    }

    #[test]
    fn without_a_terminal_adapter_the_login_says_so() {
        let mut p = SessionsPlugin::new();
        sign_in(&mut p);
        p.tick(0);
        assert!(p.order.is_empty());
        assert!(matches!(p.settings.login_state(), Some((_, crate::login::Login::Failed(_)))));
    }
}
