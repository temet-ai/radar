//! Read-only GitHub.com repositories and pull requests.

mod palette;
mod agent_context;
mod actions;
pub mod auth;
mod cli;
pub mod client;
pub mod config;
#[cfg(any(test, feature = "demo"))]
mod demo;
mod health;
mod history;
mod join;
#[cfg(test)]
mod live_tests;
pub mod model;
mod screen;
mod settings;
mod setup;
mod sync;
#[cfg(test)]
mod sync_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};

use radar_core::Plugin;
use radar_core::compose::{Offer, Owned, Setup, Showing, Subject};
use radar_core::design::help_bar::KeyHint;
use radar_core::journal::{ActionDescriptor, Outcome as ReadOutcome};
use radar_core::plugin::{Binding, Effect, Msg, Outcome, PageTabs, Pane, ScreenSpec, SectionSpec};
use radar_core::ui::{rich::Link, scroll::Scroll, status::Check};
use radar_core::view::{ActionSpec, Chord, Key};

use config::Settings;
use join::Target;
use model::{
    ApiError, CheckTags, Checks, Collection, Fresh, Marker, PullRequest, Repository, RepositoryId,
    Traffic, Validator,
};

#[derive(Default)]
pub struct GithubPlugin {
    via: client::Via,
    review_reads: std::collections::BTreeMap<(RepositoryId, u64), Option<Result<(), String>>>,
    pub(crate) cfg: Settings,
    actions: actions::State,
    repos: Vec<RepoState>,
    repo_at: usize,
    pull_at: usize,
    group_at: usize,
    tree_at: Option<screen::TreeItem>,
    folded_groups: std::collections::HashSet<usize>,
    folded_repos: std::collections::HashSet<usize>,
    pages: Vec<Page>,
    pull_detail: Option<PullRequest>,
    checks: Option<Checks>,
    page_error: Option<String>,
    checks_error: Option<String>,
    page_loading: bool,
    generation: u64,
    action_epoch: u64,
    next_request: u64,
    connection_request: u64,
    discovery_request: u64,
    discovery_loading: bool,
    available: Option<Collection<RepositoryId>>,
    discovery_owner: Option<String>,
    discovery_group: Option<usize>,
    page_request: u64,
    started: bool,
    now: i64,
    last_sync: Option<i64>,
    repo_scroll: Scroll,
    pull_scroll: Scroll,
    detail_scroll: Scroll,
    section_scroll: Scroll,
    connection_row: usize,
    /// The connection section was last drawn as a setup step, without the
    /// rows only Settings needs; `conn_rows` follows it.
    setup_card: Cell<bool>,
    repository_selected: Vec<String>,
    repository_folded: std::collections::HashSet<String>,
    editor: Option<settings::Editor>,
    connection_check: Option<Check>,
    /// Who the last check signed in as; `None` for public access or before.
    pub(crate) login: Option<String>,
    links: RefCell<Vec<Link>>,
    /// Rows the view had at its last frame: the Actions tab asks GitHub for
    /// about two screens of runs, not everything.
    view_rows: Cell<u16>,
    under_actions: RefCell<Vec<screen::UnderAction>>,
    link_menu: Option<(u64, u64, Target, Vec<Link>)>,
    remove_menu: Option<(u64, RepositoryId)>,
    settings_menu: Option<settings::MenuLease>,
    check_menu: Option<u64>,
    sync: sync::Sync,
    /// Pull request pages read before, by repository and number.
    seen: std::collections::BTreeMap<(RepositoryId, u64), sync::Seen>,
    /// When the open pull request page was last asked.
    page_polled: Option<i64>,
}

struct RepoState {
    id: RepositoryId,
    repository: Option<Repository>,
    pulls: Option<Collection<PullRequest>>,
    error: Option<String>,
    loading: bool,
    request: u64,
    sync: sync::RepoSync,
}

impl RepoState {
    fn new(id: RepositoryId) -> Self {
        Self {
            id,
            repository: None,
            pulls: None,
            error: None,
            loading: false,
            request: 0,
            sync: sync::RepoSync::default(),
        }
    }
}

struct Page {
    target: Target,
    tab: usize,
    back_repo: Option<RepositoryId>,
    back_pull: Option<u64>,
    held: Option<PullSnapshot>,
}

#[cfg(test)]
mod page_identity_tests {
    use super::*;

    #[test]
    fn page_identity_ignores_title_tab_and_selected_child() {
        let mut p = GithubPlugin::default();
        assert_eq!(p.page_identity("github"), None);
        let repository = RepositoryId::parse("ankh-morpork/watch").unwrap();
        p.pages.push(Page {
            target: Target::Pull(repository.clone(), 12),
            tab: 0,
            back_repo: None,
            back_pull: None,
            held: None,
        });
        p.pull_detail = Some(PullRequest {
            repository,
            number: 12,
            title: "Watch patrol".into(),
            body: None,
            author: "vimes".into(),
            state: "open".into(),
            draft: false,
            head_sha: "a".repeat(40),
            head_repository: None,
            head_branch: "patrol".into(),
            base_branch: "main".into(),
            updated_at: String::new(),
            merged: false,
        });
        let identity = p.page_identity("github");
        assert_eq!(identity.as_deref(), Some("https://github.com/ankh-morpork/watch/pull/12"));
        assert_eq!(p.page_identity("connection"), None);

        p.pull_detail.as_mut().unwrap().title = "Night watch patrol".into();
        p.pages.last_mut().unwrap().tab = 1;
        p.pull_at = 3;
        p.repo_at = 2;
        assert_eq!(p.page_identity("github"), identity);
        p.pages.last_mut().unwrap().target = Target::Pull(
            RepositoryId::parse("ankh-morpork/watch").unwrap(), 13,
        );
        assert_ne!(p.page_identity("github"), identity);
    }

    #[test]
    fn page_identity_distinguishes_repository_pull_run_and_attempt() {
        let repository = RepositoryId::parse("ankh-morpork/watch").unwrap();
        let other = RepositoryId::parse("lancre/watch").unwrap();
        let targets = [
            Target::Repository(repository.clone()),
            Target::Pull(repository.clone(), 12),
            Target::Pull(other.clone(), 12),
            Target::Run(repository.clone(), 12, 1),
            Target::Run(repository.clone(), 12, 2),
            Target::Run(repository, 13, 1),
            Target::Run(other, 12, 1),
        ];
        let keys: std::collections::BTreeSet<_> = targets.iter().map(Target::url).collect();
        assert_eq!(keys.len(), targets.len());
    }
}

#[derive(Clone)]
struct PullSnapshot {
    pull: PullRequest,
    checks: Option<Checks>,
    page_error: Option<String>,
    checks_error: Option<String>,
}

enum Answer {
    ReviewRead { generation: u64, id: (RepositoryId, u64), action: ActionDescriptor, result: Box<Result<(PullRequest, Option<Checks>), String>> },
    Actions(Box<actions::Answer>),
    Connection {
        generation: u64,
        request: u64,
        action: ActionDescriptor,
        /// Who the token signs in as (none for public access), and its
        /// OAuth scopes when GitHub said them.
        result: Result<(Option<String>, Option<Vec<String>>), String>,
    },
    Discovery {
        generation: u64,
        request: u64,
        action: ActionDescriptor,
        owner: Option<String>,
        result: Result<Collection<RepositoryId>, String>,
    },
    /// What moved: each repository's open pull request marker.
    Probe {
        generation: u64,
        request: u64,
        action: ActionDescriptor,
        result: Result<Vec<model::Probed>, ApiError>,
        traffic: Traffic,
    },
    Repository {
        generation: u64,
        request: u64,
        id: RepositoryId,
        action: ActionDescriptor,
        /// `None` when the description was not asked this time.
        repository: Option<Result<Fresh<Repository, Option<Validator>>, String>>,
        pulls: Result<Fresh<Collection<PullRequest>>, String>,
        reply: sync::Reply,
    },
    Pull {
        generation: u64,
        request: u64,
        target: Target,
        action: ActionDescriptor,
        pull: Box<Result<Fresh<PullRequest, Option<Validator>>, String>>,
        checks: Result<Fresh<Checks, CheckTags>, String>,
        reply: sync::Reply,
    },
}

/// The chord a test key stands for.
#[cfg(test)]
pub(crate) fn chord(code: radar_core::ratatui::crossterm::event::KeyCode) -> Chord {
    use radar_core::ratatui::crossterm::event::KeyCode;
    Chord::new(match code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Esc => Key::Esc,
        KeyCode::Enter => Key::Enter,
        other => panic!("no chord for {other:?}"),
    })
}

/// A menu row with its stable id (`github.<surface>.<verb>`), written where
/// the row is built and never made from the row's words, so rewording a
/// label keeps its id. A heading keeps an empty id.
pub(crate) type Act = (String, Binding);

/// A runnable row: the surface (`browse`, `page`, `actions` on the screen;
/// `connection`, `repositories` in settings) and the verb name its id.
pub(crate) fn act(surface: &str, verb: &str, b: Binding) -> Act {
    (format!("github.{surface}.{verb}"), b)
}

/// A heading over the rows that follow (not runnable).
pub(crate) fn head(b: Binding) -> Act {
    (String::new(), b)
}

/// The rows as plain bindings, for the host's key hints.
pub(crate) fn bindings(rows: Vec<Act>) -> Vec<Binding> {
    rows.into_iter().map(|(_, b)| b).collect()
}

/// A menu row as view data under its stable id.
pub(crate) fn action_spec((id, b): &Act) -> ActionSpec {
    ActionSpec::named(id.clone(), b)
}

/// The chord an intent stands for: a key's own, or the key of the listed
/// action whose id it names.
pub(crate) fn chord_for(rows: &[Act], intent: &radar_core::view::Intent) -> Option<Chord> {
    match intent {
        radar_core::view::Intent::Invoke { action } => rows
            .iter()
            .find(|(id, b)| !b.is_heading() && id == action)
            .and_then(|(_, b)| Chord::parse(&b.keys))
            .or_else(|| intent.chord()),
        _ => intent.chord(),
    }
}

impl GithubPlugin {
    /// A chord on the GitHub screen. Anything handled starts a new lease
    /// epoch, so a pointer hit made before it cannot act on what it loads.
    fn press(&mut self, pane: Pane, key: Chord) -> Outcome {
        let out = screen::key(self, pane, key);
        if matches!(out, Outcome::Handled(_)) {
            self.action_epoch = self.action_epoch.wrapping_add(1);
        }
        out
    }

    /// A target chosen on the screen (`scope == "github"`) or in a settings
    /// section. Contributed `under:` targets keep their own epoch lease and
    /// never start a new one.
    fn point(&mut self, scope: &str, id: &str) -> Outcome {
        let out = if scope == "github" {
            screen::click(self, id)
        } else {
            settings::click(self, scope, id)
        };
        if !id.starts_with("under:") && matches!(out, Outcome::Handled(_)) {
            self.action_epoch = self.action_epoch.wrapping_add(1);
        }
        out
    }

    #[cfg(test)]
    fn key(&mut self, _: &str, pane: Pane, key: Chord) -> Outcome {
        self.press(pane, key)
    }

    #[cfg(test)]
    fn context_select(&mut self, scope: &str, id: &str) -> Outcome {
        if id == radar_core::view_tui::PANE_HIT || id.starts_with("tab:") || id.starts_with("under:") {
            Outcome::Ignored
        } else {
            self.point(scope, id)
        }
    }

    fn apply_config(&mut self, cfg: Settings) {
        self.review_reads.clear();
        self.action_epoch = self.action_epoch.wrapping_add(1);
        let cfg = cfg.clone().validated().unwrap_or(cfg);
        self.discovery_loading = false;
        // Validators, limits and held data belong to one credential at one API.
        let same_token = cfg.token == self.cfg.token && cfg.api == self.cfg.api;
        if !same_token {
            self.available = None;
            self.discovery_owner = None;
            self.discovery_group = None;
            // Validators, limits and held data belong to the old credential.
            self.sync = sync::Sync::default();
            self.seen.clear();
        }
        self.sync.probing = false;
        // Queued reads name repositories by index, and the list is rebuilt.
        self.sync.queue.clear();
        self.generation = self.generation.wrapping_add(1);
        self.page_request = self.page_request.wrapping_add(1);
        self.cfg = cfg;
        settings::reconcile_editor(self);
        let held = std::mem::take(&mut self.actions);
        self.actions = if same_token {
            held.keep(&self.cfg.ids())
        } else {
            actions::State::default()
        };
        // Repositories still configured keep what was read for them.
        let mut held = std::mem::take(&mut self.repos);
        self.repos = self
            .cfg
            .ids()
            .into_iter()
            .map(|id| match held.iter().position(|r| r.id == id) {
                Some(i) if same_token => {
                    let mut state = held.swap_remove(i);
                    state.loading = false;
                    state.error = None;
                    state
                }
                _ => RepoState::new(id),
            })
            .collect();
        let ids = self.cfg.ids();
        self.seen.retain(|(id, _), _| ids.contains(id));
        let kept = self.repos.iter().any(|r| r.pulls.is_some());
        self.repo_at = 0;
        self.pull_at = 0;
        self.group_at = 0;
        self.tree_at = None;
        self.folded_groups.clear();
        self.folded_repos.clear();
        self.pages.clear();
        self.clear_page();
        self.started = false;
        if !kept {
            self.last_sync = None;
        }
        self.connection_check = None;
        self.links.borrow_mut().clear();
        self.under_actions.borrow_mut().clear();
        self.link_menu = None;
        self.remove_menu = None;
        self.settings_menu = None;
        self.check_menu = None;
    }

    fn clear_page(&mut self) {
        actions::leave(self);
        self.pull_detail = None;
        self.checks = None;
        self.page_error = None;
        self.checks_error = None;
        self.page_loading = false;
        self.detail_scroll.top();
        self.links.borrow_mut().clear();
        self.under_actions.borrow_mut().clear();
    }

    fn hold_page(&mut self) {
        if let (Some(page), Some(pull)) = (self.pages.last_mut(), self.pull_detail.as_ref())
            && page.target == Target::Pull(pull.repository.clone(), pull.number)
        {
            page.held = Some(PullSnapshot {
                pull: pull.clone(),
                checks: self.checks.clone(),
                page_error: self.page_error.clone(),
                checks_error: self.checks_error.clone(),
            });
        }
    }

    fn restore_page(&mut self) {
        if let Some(page) = self.pages.last()
            && let Some(held) = page.held.as_ref()
            && page.target == Target::Pull(held.pull.repository.clone(), held.pull.number)
        {
            self.pull_detail = Some(held.pull.clone());
            self.checks = held.checks.clone();
            self.page_error = held.page_error.clone();
            self.checks_error = held.checks_error.clone();
        }
    }

    fn repository(&self) -> Option<&RepoState> {
        self.repos.get(self.repo_at)
    }

    fn selected_pull(&self) -> Option<&PullRequest> {
        self.repository()?.pulls.as_ref()?.items.get(self.pull_at)
    }

    fn target(&self, pane: Pane) -> Option<Target> {
        if actions::active(self) {
            return actions::selected_target(self)
                .or_else(|| self.pages.last().map(|p| p.target.clone()));
        }
        if let Some(page) = self.pages.last() {
            if page.tab == 1 && matches!(page.target, Target::Repository(_)) {
                return self
                    .selected_pull()
                    .map(|p| Target::Pull(p.repository.clone(), p.number));
            }
            return Some(page.target.clone());
        }
        if pane == Pane::Nav {
            return None;
        }
        match screen::selected_item(self)? {
            screen::TreeItem::Group(_) => None,
            screen::TreeItem::Repo(i) => {
                self.repos.get(i).map(|r| Target::Repository(r.id.clone()))
            }
            screen::TreeItem::Pull(i, j) => self
                .repos
                .get(i)?
                .pulls
                .as_ref()?
                .items
                .get(j)
                .map(|pull| Target::Pull(pull.repository.clone(), pull.number)),
        }
    }

    fn target_url(target: &Target) -> String {
        target.url()
    }

    fn subject_for(&self, target: &Target) -> Subject {
        let subject = target.subject();
        let Target::Pull(repository, number) = target else {
            return subject;
        };
        // A loaded page owns its detail snapshot. A list may independently hold
        // a newer snapshot, but it cannot rewrite the page's source identity.
        let on_page = self.pages.last().is_some_and(|page| &page.target == target);
        let pull = if on_page {
            self.pull_detail.as_ref()
        } else {
            self.repos
                .iter()
                .find(|state| &state.id == repository)
                .and_then(|state| state.pulls.as_ref())
                .and_then(|pulls| pulls.items.iter().find(|pull| pull.number == *number))
        };
        let Some(pull) =
            pull.filter(|pull| &pull.repository == repository && pull.number == *number)
        else {
            return subject;
        };
        let subject = subject
            .with("title", &pull.title)
            .with("branch", &pull.head_branch)
            .with("head_sha", &pull.head_sha);
        match &pull.head_repository {
            Some(source) => subject.with("source_repository_url", source.web_url()),
            None => subject,
        }
    }

    pub(crate) fn open_target(&mut self, target: Target) -> Vec<Effect> {
        let id = target.repository();
        let Some(index) = self.repos.iter().position(|r| &r.id == id) else {
            return vec![];
        };
        self.hold_page();
        let back_repo = self.repository().map(|r| r.id.clone());
        let back_pull = self.selected_pull().map(|p| p.number);
        self.repo_at = index;
        if let Target::Pull(_, number) = &target {
            self.pull_at = self.repos[index]
                .pulls
                .as_ref()
                .and_then(|list| list.items.iter().position(|p| p.number == *number))
                .unwrap_or(0);
        } else {
            self.pull_at = 0;
        }
        self.pages.push(Page {
            target: target.clone(),
            tab: 0,
            back_repo,
            back_pull,
            held: None,
        });
        self.page_request = self.page_request.wrapping_add(1);
        self.clear_page();
        if let Target::Pull(repository, number) = &target
            && let Some(seen) = self.seen.get(&(repository.clone(), *number))
        {
            // Drawn at once from what was read before; the read below asks
            // GitHub only whether it changed.
            self.pull_detail = Some(seen.pull.clone());
            self.checks = seen.checks.clone();
        }
        let mut effects = vec![history::opened(&target), Effect::Focus(Pane::Middle)];
        match target {
            Target::Pull(_, _) => effects.extend(self.refresh_page()),
            Target::Run(..) => effects.extend(actions::ensure(self)),
            Target::Repository(_)
                if self.repos[index].repository.is_none() && !self.repos[index].loading =>
            {
                effects.extend(self.refresh_repository(index, None, true))
            }
            _ => {}
        }
        effects
    }

    /// Read one repository, conditionally: its open pull requests with the
    /// validators of the list held (all 304 is free and parses nothing), and
    /// its description only when asked for, missing or a day old. `marker`
    /// is what the probe saw, kept once the read is complete.
    fn refresh_repository(
        &mut self,
        index: usize,
        marker: Option<Marker>,
        force_meta: bool,
    ) -> Vec<Effect> {
        let now = self.now;
        let Some(repo) = self.repos.get_mut(index) else {
            return vec![];
        };
        if repo.loading {
            return vec![];
        }
        self.next_request = self.next_request.wrapping_add(1);
        repo.request = self.next_request;
        repo.loading = true;
        repo.error = None;
        let meta = force_meta
            || repo.repository.is_none()
            || repo
                .sync
                .repository_at
                .is_none_or(|t| now - t >= sync::META_EVERY);
        let repository_tag = repo
            .repository
            .as_ref()
            .and(repo.sync.repository_tag.clone());
        let pulls_tags = if repo.pulls.as_ref().is_some_and(|p| p.completion.complete) {
            repo.sync.pulls_tags.clone()
        } else {
            vec![]
        };
        let (generation, request, id) = (self.generation, repo.request, repo.id.clone());
        let ids = self.cfg.ids();
        let token = self.cfg.token.clone();
        let api = self.cfg.api.clone();
        let via = self.via.clone();
        let action = history::action("github.repository.read", &Target::Repository(id.clone()));
        let scope = action.clone();
        vec![Effect::SpawnAction {
            action,
            task: Box::pin(async move {
                let (repository, pulls, reply) = match via.client(&ids, &token, &api) {
                    Ok(client) => {
                        let repository = if meta {
                            Some(client.repository_if(&id, repository_tag.as_ref()).await)
                        } else {
                            None
                        };
                        let pulls = match &repository {
                            Some(Err(_)) => None,
                            _ => Some(client.open_pulls_if(&id, &pulls_tags).await),
                        };
                        let errors = [
                            repository.as_ref().and_then(|r| r.as_ref().err()),
                            pulls.as_ref().and_then(|r| r.as_ref().err()),
                        ];
                        let reply = sync::Reply::of(&client, marker, errors.into_iter().flatten());
                        (
                            repository.map(|r| r.map_err(|e| e.to_string())),
                            pulls.map_or_else(
                                || Err("pull requests were not read because the repository could not be loaded".into()),
                                |r| r.map_err(|e| e.to_string()),
                            ),
                            reply,
                        )
                    }
                    Err(error) => (
                        Some(Err(error.to_string())),
                        Err(error.to_string()),
                        sync::Reply::default(),
                    ),
                };
                Box::new(Answer::Repository {
                    generation,
                    request,
                    id,
                    action: scope,
                    repository,
                    pulls,
                    reply,
                }) as Msg
            }),
        }]
    }

    /// `r` on the list, or a settings save: a round now, asking only what moved.
    fn refresh_all(&mut self) -> Vec<Effect> {
        sync::start(self, true)
    }

    /// The round ends when nothing is reading: count it synced when every
    /// repository is current, say so when asked or when that changed, and
    /// write the cache when held data changed.
    fn finish_round(&mut self) -> Vec<Effect> {
        if self.sync.probing || !self.sync.queue.is_empty() || self.repos.iter().any(|r| r.loading)
        {
            return vec![];
        }
        let complete = !self.sync.refused
            && self.repos.iter().all(|r| {
                r.error.is_none()
                    && r.repository.is_some()
                    && r.pulls.as_ref().is_some_and(|p| p.completion.complete)
            });
        if complete {
            self.last_sync = Some(self.now);
        }
        let mut effects = vec![];
        if self.sync.manual || self.sync.said_complete != Some(complete) {
            let said = if complete {
                "configured repositories are readable".into()
            } else if let Some(waiting) = sync::waiting(self) {
                waiting
            } else {
                self.repos
                    .iter()
                    .find_map(|r| {
                        r.error
                            .clone()
                            .or_else(|| r.pulls.as_ref()?.completion.reason.clone())
                    })
                    .unwrap_or_else(|| {
                        "repository reads failed or are incomplete; see their details".into()
                    })
            };
            effects.push(if complete {
                Effect::notify(Some(radar_core::ui::Mark::Done.icon()), said)
            } else {
                Effect::error(Some(radar_core::ui::Mark::Failed.icon()), said)
            });
        }
        self.sync.said_complete = Some(complete);
        self.sync.manual = false;
        if std::mem::take(&mut self.sync.dirty) {
            effects.push(Effect::PersistCache);
        }
        effects
    }

    fn check_connection(&mut self) -> Vec<Effect> {
        self.next_request = self.next_request.wrapping_add(1);
        self.connection_request = self.next_request;
        self.connection_check = Some(Check::Running);
        let (generation, request) = (self.generation, self.connection_request);
        let token = self.cfg.token.clone();
        let api = self.cfg.api.clone();
        let via = self.via.clone();
        let action = history::connection_action();
        let scope = action.clone();
        vec![Effect::SpawnAction {
            action,
            task: Box::pin(async move {
                let result = match via.client(&[], &token, &api) {
                    Ok(client) => client.check_connection_scopes().await.map_err(|e| e.to_string()),
                    Err(error) => Err(error.to_string()),
                };
                Box::new(Answer::Connection {
                    generation,
                    request,
                    action: scope,
                    result,
                }) as Msg
            }),
        }]
    }

    fn discover_repositories(&mut self, owner: Option<String>) -> Vec<Effect> {
        if self.cfg.token.is_empty() {
            return vec![Effect::Notify(
                "add a GitHub token to browse accessible repositories".into(),
            )];
        }
        self.next_request = self.next_request.wrapping_add(1);
        self.discovery_request = self.next_request;
        self.discovery_loading = true;
        self.available = None;
        let (generation, request) = (self.generation, self.discovery_request);
        let token = self.cfg.token.clone();
        let api = self.cfg.api.clone();
        let via = self.via.clone();
        let action = history::discovery_action();
        let scope = action.clone();
        vec![Effect::SpawnAction {
            action,
            task: Box::pin(async move {
                let result = match via.client(&[], &token, &api) {
                    Ok(client) => client
                        .accessible_repositories()
                        .await
                        .map_err(|e| e.to_string()),
                    Err(error) => Err(error.to_string()),
                };
                Box::new(Answer::Discovery {
                    generation,
                    request,
                    action: scope,
                    owner,
                    result,
                }) as Msg
            }),
        }]
    }

    fn refresh_page(&mut self) -> Vec<Effect> {
        if actions::active(self) {
            return actions::refresh(self);
        }
        let Some(target) = self.pages.last().map(|p| p.target.clone()) else {
            return self.refresh_all();
        };
        let Target::Pull(id, number) = &target else {
            let effects = self.refresh_repository(self.repo_at, None, true);
            self.sync.manual |= !effects.is_empty();
            return effects;
        };
        if self.page_loading {
            return vec![Effect::Notify(
                "this pull request is already loading".into(),
            )];
        }
        if let Some(said) = sync::waiting(self) {
            return vec![Effect::Notify(said)];
        }
        actions::leave(self);
        self.page_request = self.page_request.wrapping_add(1);
        self.page_loading = true;
        self.page_polled = Some(self.now);
        self.page_error = None;
        self.checks_error = None;
        let (generation, request, id, number) =
            (self.generation, self.page_request, id.clone(), *number);
        // What was read before, to ask GitHub only whether it changed.
        let known = self.seen.get(&(id.clone(), number)).cloned();
        let ids = self.cfg.ids();
        let token = self.cfg.token.clone();
        let api = self.cfg.api.clone();
        let via = self.via.clone();
        let action = history::action("github.pull-request.read", &target);
        let scope = action.clone();
        vec![Effect::SpawnAction {
            action,
            task: Box::pin(async move {
                let held = known
                    .as_ref()
                    .and_then(|seen| Some((&seen.pull, seen.tag.as_ref()?)));
                let (pull, checks, reply) = match via.client(&ids, &token, &api) {
                    Ok(client) => match client.pull_if(&id, number, held).await {
                        Ok(fresh) => {
                            // A 304 answers the held copy, which is the one to check.
                            let pull = match &fresh {
                                Fresh::New(pull, _) => Some(pull),
                                Fresh::Same => held.map(|(pull, _)| pull),
                            };
                            let known = known
                                .as_ref()
                                .and_then(|seen| Some((seen.checks.as_ref()?, &seen.check_tags)));
                            let checks = match pull {
                                Some(pull) => client.checks_if(pull, known).await,
                                None => Err(ApiError::new(model::ApiErrorKind::InvalidResponse)),
                            };
                            let reply = sync::Reply::of(&client, None, checks.as_ref().err());
                            (Ok(fresh), checks.map_err(|e| e.to_string()), reply)
                        }
                        Err(error) => (
                            Err(error.to_string()),
                            Err(
                                "checks were not read because the pull request could not be loaded"
                                    .into(),
                            ),
                            sync::Reply::of(&client, None, [&error]),
                        ),
                    },
                    Err(error) => (
                        Err(error.to_string()),
                        Err("checks were not read because the connection could not be used".into()),
                        sync::Reply::default(),
                    ),
                };
                Box::new(Answer::Pull {
                    generation,
                    request,
                    target,
                    action: scope,
                    pull: Box::new(pull),
                    checks,
                    reply,
                }) as Msg
            }),
        }]
    }

    fn receive(&mut self, answer: Answer) -> Vec<Effect> {
        self.action_epoch = self.action_epoch.wrapping_add(1);
        match answer {
            Answer::ReviewRead { generation, id, action, result } => {
                if generation != self.generation || !self.repos.iter().any(|r| r.id == id.0) { return vec![]; }
                let outcome = if result.is_ok() { ReadOutcome::Succeeded } else { ReadOutcome::Failed };
                match *result {
                    Ok((pull, checks)) => {
                        self.seen.insert(id.clone(), sync::Seen { pull, tag: None, checks, check_tags: CheckTags::default(), at: self.now });
                        self.review_reads.insert(id, Some(Ok(())));
                    }
                    Err(error) => { self.review_reads.insert(id, Some(Err(error))); }
                }
                history::finished(&action, outcome, None, false)
            }
            Answer::Actions(answer) => actions::receive(self, *answer),
            Answer::Connection {
                generation,
                request,
                action,
                result,
            } => {
                let mut effects = history::finished(
                    &action,
                    if result.is_ok() {
                        ReadOutcome::Succeeded
                    } else {
                        ReadOutcome::Failed
                    },
                    None,
                    false,
                );
                if generation != self.generation || request != self.connection_request {
                    effects.push(history::skipped(&action));
                    return effects;
                }
                let passed = result.is_ok();
                if let Ok((login, _)) = &result {
                    self.login = login.clone();
                }
                let said = match result {
                    Ok((Some(login), scopes)) => format!(
                        "GitHub token works · signed in as @{login}{}",
                        setup::scope_note(self.cfg.access, scopes.as_deref()).unwrap_or_default()
                    ),
                    Ok((None, _)) => "GitHub public API access works".into(),
                    Err(error) => error,
                };
                self.connection_check = Some(if passed {
                    Check::Passed {
                        said: said.clone(),
                        ago: None,
                    }
                } else {
                    Check::Failed {
                        said: said.clone(),
                        ago: None,
                    }
                });
                effects.push(if passed {
                    Effect::notify(Some(radar_core::ui::Mark::Done.icon()), said)
                } else {
                    Effect::error(Some(radar_core::ui::Mark::Failed.icon()), said)
                });
                effects
            }
            Answer::Discovery {
                generation,
                request,
                action,
                owner,
                result,
            } => {
                let outcome = match &result {
                    Ok(repos) if repos.completion.complete => ReadOutcome::Succeeded,
                    Ok(_) => ReadOutcome::Partial,
                    Err(_) => ReadOutcome::Failed,
                };
                let mut effects = history::finished(
                    &action,
                    outcome,
                    result.as_ref().ok().map(|repos| repos.items.len()),
                    outcome == ReadOutcome::Partial,
                );
                if generation != self.generation || request != self.discovery_request {
                    effects.push(history::skipped(&action));
                    return effects;
                }
                self.discovery_loading = false;
                match result {
                    Ok(repos) => {
                        self.discovery_owner = owner;
                        self.available = Some(repos);
                        if let Some(menu) = settings::discovery_menu(self) {
                            effects.push(Effect::Menu(menu));
                        } else {
                            effects.push(Effect::Notify(
                                "no untracked repositories in this selection".into(),
                            ));
                        }
                    }
                    Err(error) => effects.push(Effect::error(None, error)),
                }
                effects
            }
            Answer::Probe {
                generation,
                request,
                action,
                result,
                traffic,
            } => sync::probed(self, generation, request, action, result, traffic),
            Answer::Repository {
                generation,
                request,
                id,
                action,
                repository,
                pulls,
                reply,
            } => {
                let held = self
                    .repos
                    .iter()
                    .find(|r| r.id == id)
                    .and_then(|r| r.pulls.as_ref());
                let (count, complete) = match &pulls {
                    Ok(Fresh::New(p, _)) => (Some(p.items.len()), p.completion.complete),
                    Ok(Fresh::Same) => (
                        held.map(|p| p.items.len()),
                        held.is_some_and(|p| p.completion.complete),
                    ),
                    Err(_) => (None, false),
                };
                let repository_failed = matches!(repository, Some(Err(_)));
                let outcome = if !repository_failed && complete {
                    ReadOutcome::Succeeded
                } else if repository_failed && pulls.is_err() {
                    ReadOutcome::Failed
                } else {
                    ReadOutcome::Partial
                };
                let mut effects =
                    history::finished(&action, outcome, count, !complete && pulls.is_ok());
                let index = self
                    .repos
                    .iter()
                    .position(|r| r.id == id && r.request == request);
                let Some(index) = index.filter(|_| generation == self.generation) else {
                    effects.push(history::skipped(&action));
                    return effects;
                };
                let sync::Reply {
                    marker,
                    stop,
                    traffic,
                } = reply;
                self.sync.take(traffic, stop, self.now);
                let moved = matches!(repository, Some(Ok(Fresh::New(..))))
                    || matches!(pulls, Ok(Fresh::New(..)));
                if moved {
                    // New rows: links and menus made from the old ones go.
                    self.links.borrow_mut().clear();
                    self.under_actions.borrow_mut().clear();
                    self.link_menu = None;
                }
                let selected = self.selected_pull().map(|p| p.number);
                let now = self.now;
                let repo = &mut self.repos[index];
                repo.loading = false;
                let mut errors = Vec::new();
                // The cache is written for new data, and for what saves a
                // read at the next start: a new marker, a day's description.
                let mut dirty = moved;
                match repository {
                    None => {}
                    Some(Ok(Fresh::New(value, tag))) => {
                        repo.repository = Some(value);
                        repo.sync.repository_tag = tag;
                        repo.sync.repository_at = Some(now);
                    }
                    Some(Ok(Fresh::Same)) => {
                        repo.sync.repository_at = Some(now);
                        dirty = true;
                    }
                    Some(Err(error)) => errors.push(error),
                }
                match pulls {
                    Ok(Fresh::New(value, tags)) => {
                        repo.sync.marker = marker.filter(|_| value.completion.complete);
                        repo.sync.pulls_tags = tags;
                        repo.pulls = Some(value);
                    }
                    Ok(Fresh::Same) if marker.is_some() && repo.sync.marker != marker => {
                        repo.sync.marker = marker;
                        dirty = true;
                    }
                    Ok(Fresh::Same) => {}
                    Err(error) => errors.push(error),
                }
                repo.error = (!errors.is_empty()).then(|| errors.join("; "));
                self.sync.dirty |= dirty;
                if self.repo_at == index {
                    self.pull_at = selected
                        .and_then(|n| {
                            repo.pulls
                                .as_ref()?
                                .items
                                .iter()
                                .position(|p| p.number == n)
                        })
                        .unwrap_or(0);
                    if matches!(self.tree_at, Some(screen::TreeItem::Pull(repo, _)) if repo == index)
                    {
                        self.tree_at = Some(screen::TreeItem::Pull(index, self.pull_at));
                    }
                }
                // A read finished: the next queued one may start.
                effects.extend(sync::pump(self));
                effects.extend(self.finish_round());
                effects
            }
            Answer::Pull {
                generation,
                request,
                target,
                action,
                pull,
                checks,
                reply,
            } => {
                let (count, complete) = match &checks {
                    Ok(Fresh::New(c, _)) => (
                        Some(c.check_runs.items.len() + c.statuses.items.len()),
                        c.errors.is_empty()
                            && c.check_runs.completion.complete
                            && c.statuses.completion.complete,
                    ),
                    // Only a complete read is asked conditionally.
                    Ok(Fresh::Same) => (
                        self.checks
                            .as_ref()
                            .map(|c| c.check_runs.items.len() + c.statuses.items.len()),
                        true,
                    ),
                    Err(_) => (None, false),
                };
                let outcome = if pull.is_err() {
                    ReadOutcome::Failed
                } else if complete {
                    ReadOutcome::Succeeded
                } else {
                    ReadOutcome::Partial
                };
                let mut effects =
                    history::finished(&action, outcome, count, checks.is_ok() && !complete);
                if generation != self.generation
                    || request != self.page_request
                    || self.pages.last().is_none_or(|p| p.target != target)
                {
                    effects.push(history::skipped(&action));
                    return effects;
                }
                self.sync.take(reply.traffic, reply.stop, self.now);
                self.page_loading = false;
                let Target::Pull(repository, number) = &target else {
                    return effects;
                };
                let key = (repository.clone(), *number);
                let mut seen = self.seen.get(&key).cloned();
                let moved =
                    matches!(*pull, Ok(Fresh::New(..))) || matches!(checks, Ok(Fresh::New(..)));
                if moved {
                    self.links.borrow_mut().clear();
                    self.under_actions.borrow_mut().clear();
                    self.link_menu = None;
                }
                match *pull {
                    Ok(Fresh::New(value, tag)) => {
                        let head_moved = |old: &PullRequest| {
                            old.head_sha != value.head_sha
                                || old.head_repository != value.head_repository
                        };
                        if self.pull_detail.as_ref().is_some_and(head_moved) {
                            self.checks = None;
                        }
                        seen = Some(match seen {
                            Some(mut held) => {
                                if head_moved(&held.pull) {
                                    held.checks = None;
                                    held.check_tags.clear();
                                }
                                held.pull = value.clone();
                                held.tag = tag;
                                held
                            }
                            None => sync::Seen {
                                pull: value.clone(),
                                tag,
                                checks: None,
                                check_tags: CheckTags::new(),
                                at: self.now,
                            },
                        });
                        self.pull_detail = Some(value);
                    }
                    Ok(Fresh::Same) => {
                        if self.pull_detail.is_none() {
                            self.pull_detail = seen.as_ref().map(|s| s.pull.clone());
                        }
                    }
                    Err(error) => self.page_error = Some(error),
                }
                match checks {
                    Ok(Fresh::New(value, tags)) => {
                        if let Some(held) = &mut seen {
                            held.checks = Some(value.clone());
                            held.check_tags = tags;
                        }
                        self.checks = Some(value);
                    }
                    Ok(Fresh::Same) => {
                        if self.checks.is_none() {
                            self.checks = seen.as_ref().and_then(|s| s.checks.clone());
                        }
                    }
                    Err(error) => self.checks_error = Some(error),
                }
                if let Some(mut held) = seen {
                    held.at = self.now;
                    sync::remember(self, held);
                    if moved {
                        effects.push(Effect::PersistCache);
                    }
                }
                effects
            }
        }
    }
}

impl Plugin for GithubPlugin {
    fn id(&self) -> &'static str {
        "github"
    }
    fn name(&self) -> &'static str {
        "GitHub"
    }
    fn configure(&mut self, table: Option<&toml::Value>) -> anyhow::Result<()> {
        for prefix in ["ghp_", "github_pat_", "gho_", "ghu_", "ghs_", "ghr_"] {
            radar_core::log::token_shape(prefix);
        }
        let cfg = table
            .cloned()
            .map(toml::Value::try_into)
            .transpose()?
            .unwrap_or_default();
        self.apply_config(Settings::validated(cfg)?);
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        toml::Value::try_from(&self.cfg).ok()
    }
    fn set_up(&self) -> Setup {
        if self.cfg.groups.is_empty() {
            Setup::Needed("Create a GitHub group under repositories in settings. Public repositories need no token.".into())
        } else {
            Setup::Ready
        }
    }
    fn owns(&self) -> Vec<Owned> {
        join::OWNS.to_vec()
    }
    fn offers(&self) -> Vec<Offer> {
        join::OFFERS.to_vec()
    }
    fn recognise(&self, text: &str) -> Vec<(std::ops::Range<usize>, Subject)> {
        join::recognise(text)
    }
    fn agent_context(&mut self, request: &radar_core::agent_context::AgentContextRequest) -> Option<radar_core::agent_context::AgentContext> {
        agent_context::document(self, request).map(|d| d.context)
    }
    fn agent_document(&mut self, request: &radar_core::agent_context::AgentContextRequest) -> Option<radar_core::context::Document> { agent_context::document(self, request) }
    fn resolve_change(&mut self, request: &radar_core::agent_context::ChangeResolveRequest) -> Option<(radar_core::agent_context::Resolve, Vec<Effect>)> { agent_context::resolve(self, request) }
    fn subject_from_journal(&self, key: &radar_core::journal::SubjectKey) -> Option<Subject> {
        history::from_journal(self, key)
    }
    fn related(&self, subject: &Subject) -> Vec<Subject> {
        history::related(self, subject)
    }
    fn journal_subject(&self, subject: &Subject) -> Option<radar_core::journal::SubjectSnapshot> {
        history::subject(subject)
    }
    fn palette_targets(&self) -> Vec<radar_core::palette::PaletteTarget> {
        palette::targets(self)
    }

    fn show(&mut self, subject: &Subject) -> Showing {
        let Some(target) = join::target(subject) else {
            return Showing::Cannot;
        };
        let id = target.repository();
        if !self.repos.iter().any(|r| &r.id == id) {
            return Showing::Cannot;
        }
        Showing::Page {
            screen: "github",
            effects: self.open_target(target),
        }
    }
    fn subject(&self, _: &str, pane: Pane) -> Option<Subject> {
        self.target(pane)
            .as_ref()
            .map(|target| self.subject_for(target))
    }
    fn depth(&self, _: &str) -> usize {
        self.pages.len()
    }
    fn page_identity(&self, screen: &str) -> Option<String> {
        if screen != "github" {
            return None;
        }
        // The requested page target, not the selected PR or workflow job.
        Some(self.pages.last()?.target.url())
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "github",
            name: "GitHub",
            key: 'G',
        }]
    }
    fn screen_group(&self, _: &str) -> Option<radar_core::ScreenGroup> {
        Some(radar_core::ScreenGroup {
            id: "repositories",
            name: "repositories",
            key: 'R',
            source: "GitHub",
        })
    }
    fn restore(&mut self, cache: serde_json::Value) {
        sync::restore(self, cache);
    }
    fn snapshot(&self) -> Option<serde_json::Value> {
        sync::snapshot(self)
    }
    fn tick(&mut self, now: i64) -> Vec<Effect> {
        self.now = now;
        let mut effects = if sync::due(self, now) {
            sync::start(self, false)
        } else {
            vec![]
        };
        effects.extend(sync::poll_page(self, now));
        effects
    }
    fn health(&self) -> Option<radar_core::health::Health> {
        health::health(self)
    }
    fn refresh_now(&mut self) -> Vec<Effect> {
        self.refresh_all()
    }
    fn busy(&self, _: &str) -> bool {
        self.page_loading
            || self.sync.probing
            || actions::busy(self)
            || self.repos.iter().any(|r| r.loading)
    }
    fn top_right(&self, _: &str) -> Option<String> {
        if actions::active(self) {
            return Some(
                if actions::busy(self) {
                    "loading Actions…"
                } else if actions::incomplete(self) {
                    "Actions read incomplete"
                } else {
                    "Actions · observed snapshot"
                }
                .into(),
            );
        }
        if !self.page_loading
            && self
                .pages
                .last()
                .is_some_and(|p| matches!(p.target, Target::Pull(_, _)))
            && (self.page_error.is_some()
                || self.checks_error.is_some()
                || self.checks.as_ref().is_some_and(|c| {
                    !c.errors.is_empty()
                        || !c.check_runs.completion.complete
                        || !c.statuses.completion.complete
                }))
        {
            Some("read incomplete".into())
        } else if let Some(waiting) = sync::waiting(self) {
            Some(waiting)
        } else if !self.busy("github")
            && matches!(self.connection_check, Some(Check::Failed { .. }))
        {
            Some("GitHub connection check failed".into())
        } else if !self.busy("github")
            && self.repos.iter().any(|repo| {
                repo.error.is_some()
                    || repo
                        .pulls
                        .as_ref()
                        .is_some_and(|pulls| !pulls.completion.complete)
            })
        {
            Some("repository read incomplete".into())
        } else {
            Some(radar_core::ui::status::sync_words(
                "GitHub",
                self.busy("github"),
                self.last_sync,
                self.now,
                None,
            ))
        }
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        msg.downcast::<Answer>()
            .map(|a| self.receive(*a))
            .unwrap_or_default()
    }
    fn panes(&self, _: &str) -> Vec<Pane> {
        if self.pages.is_empty() {
            vec![Pane::Nav, Pane::Middle, Pane::Detail]
        } else {
            vec![Pane::Middle]
        }
    }
    fn tabs(&self, _: &str) -> Option<PageTabs> {
        self.pages.last().map(|p| PageTabs {
            names: screen::tabs(&p.target),
            on: p.tab,
        })
    }
    fn show_tab(&mut self, _: &str, tab: usize) -> Vec<Effect> {
        self.action_epoch = self.action_epoch.wrapping_add(1);
        screen::show_tab(self, tab)
    }
    fn view(
        &self,
        screen: &str,
        cx: &radar_core::view::ViewCx<'_>,
    ) -> Option<radar_core::view::ScreenView> {
        (screen == "github").then(|| screen::neutral::view(self, cx))
    }
    fn section_view(
        &self,
        section: &str,
        cx: &radar_core::view::ViewCx<'_>,
    ) -> Option<radar_core::view::SectionView> {
        settings::neutral::view(self, section, cx)
    }
    fn intent(&mut self, scope: &str, pane: Pane, intent: &radar_core::view::Intent) -> Outcome {
        if scope == "github" {
            screen::neutral::intent(self, pane, intent)
        } else {
            settings::neutral::intent(self, scope, intent)
        }
    }
    fn keys(&self, _: &str, pane: Pane) -> Vec<Binding> {
        bindings(screen::keys(self, pane))
    }
    fn folds(&self, screen: &str, pane: Pane) -> bool {
        screen == "github" && pane == Pane::Middle && self.pages.is_empty()
    }
    fn section_folds(&self, section: &str) -> bool {
        section == "repositories"
    }
    fn subject_menu_context(&self, scope: &str, pane: Pane) -> Option<String> {
        (scope == "github")
            .then(|| self.target(pane))
            .flatten()
            .map(|target| format!("{}:{}", self.generation, target.url()))
    }
    fn menu_key_context(&self, scope: &str, pane: Pane, action: &str) -> Option<String> {
        let key = Chord::parse(action)?;
        if scope == "github"
            && actions::active(self)
            && matches!(key.key, Key::Char('o' | 'y' | 'r' | 'a') | Key::Enter)
        {
            return actions::context(self);
        }
        if scope == "github" && matches!(key.key, Key::Char('o' | 'y' | 'r') | Key::Enter) {
            self.target(pane)
                .map(|t| format!("{}:{}:{}", self.generation, scope, Self::target_url(&t)))
        } else if scope != "github" {
            settings::context(self, scope, key)
        } else {
            None
        }
    }
    fn sections(&self) -> Vec<SectionSpec> {
        vec![
            SectionSpec {
                id: "connection",
                name: "connection",
                group: "github",
            },
            SectionSpec {
                id: "repositories",
                name: "repositories",
                group: "github",
            },
        ]
    }
    fn section_keys(&self, section: &str) -> Vec<Binding> {
        bindings(settings::keys(self, section))
    }
    fn setup_step(&self) -> Option<radar_core::plugin::SetupStep> {
        Some(setup::step(self))
    }
    fn setup_finds(&self) -> radar_core::plugin::SetupFinds {
        #[cfg(any(test, feature = "test-support"))]
        if self.via.fixture.is_some() {
            return Default::default();
        }
        setup::finds()
    }
    fn capturing(&self) -> bool {
        self.editor.is_some()
    }
    fn section_hints(&self, _: &str) -> Vec<KeyHint<'static>> {
        vec![KeyHint::new("⏎", "keep"), KeyHint::new("esc", "cancel")]
    }
    fn paste(&mut self, text: &str) -> Vec<Effect> {
        if let Some(editor) = &mut self.editor {
            editor.input.paste(text);
        }
        vec![]
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        if menu == "github-attempts" {
            actions::choose_attempt(self, value)
        } else if menu == "links" {
            screen::follow_link(self, value)
        } else {
            settings::on_menu(self, menu, value)
        }
    }
    fn demo(&mut self) {
        #[cfg(any(test, feature = "demo"))]
        {
            let via = self.via.clone();
            *self = demo::sample();
            self.via = via;
        }
    }
    fn cli(&self) -> Option<clap::Command> {
        Some(cli::command())
    }
    fn run_cli(&self, matches: &clap::ArgMatches) -> radar_core::plugin::CliTask {
        cli::run(&self.cfg, matches, self.via.clone())
    }
}
