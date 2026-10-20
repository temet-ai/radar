//! radar-gitlab: merge requests across GitLab tenancies, as a radar plugin.
//!
//! Contributes the `repositories` screen (Shift+R), the `connection`,
//! `tenancies`, `views` and `look` settings sections, the `[gitlab]` config
//! table, the `tenant.*` theme slots, a cache, the `gitlab.selected` bus
//! topic and the `radar gitlab list` command.

mod nav;
mod palette;
mod source_navigation;
mod agent_context;
mod auth;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod backoff;
#[cfg(test)]
mod sync_backoff_tests;
#[cfg(test)]
mod confirmation_tests;
mod client;
mod comment_recovery;
mod config;
#[cfg(any(test, feature = "demo"))]
mod demo;
mod diff_history;
mod failure_history;
mod filter;
mod health;
mod goto;
mod joblog;
mod join;
mod list_actions;
mod log_history;
mod log_ownership;
mod model;
mod mr_actions;
mod page_history;
mod page_identity;
mod page_ownership;
mod pages;
mod pictures;
mod ready;
mod runs_history;
mod runs_ownership;
mod screen;
mod settings;
mod setup;
mod settings_checks;
mod settings_history;
#[cfg(test)]
mod stand_in;
mod sync;
mod sync_history;
mod tickets;
mod why;
mod write;
mod write_context;

use std::collections::BTreeMap;

use radar_core::design::help_bar::KeyHint;
use radar_core::plugin::{Binding, CliTask, Effect, MenuItem, Msg, Outcome, Pane, ScreenSpec, SectionSpec, SlotValue};
use radar_core::design::theme::Color;
use radar_core::view::{Chord, Key};

/// A key as the event a renderer sends for it, for the tests.
#[cfg(test)]
pub(crate) fn key_event(chord: Chord) -> radar_core::host::Event {
    radar_core::host::Event::Intent(radar_core::view::Intent::Key { chord })
}
use radar_core::{Plugin, log, time};
use serde_json::{Value, json};

pub use client::{Client, GroupData};
/// The sample with !263's page data and diffs held (the `demo` feature).
#[cfg(feature = "demo")]
pub use demo::sample_reviewed;
use client::{Synced, Want};
pub use config::Settings;
use config::Source;
use filter::{Context, Filter};
use model::{Mr, MrState, Role};

const SCREEN: ScreenSpec = ScreenSpec {
    id: "repos",
    name: "repositories",
    key: 'R',
};
const SECTIONS: [SectionSpec; 4] = [
    SectionSpec {
        id: "connection",
        name: "connection",
        group: "gitlab",
    },
    SectionSpec {
        id: "tenancies",
        name: "tenancies",
        group: "gitlab",
    },
    SectionSpec {
        id: "views",
        name: "views",
        group: "gitlab",
    },
    SectionSpec {
        id: "look",
        name: "look",
        group: "gitlab",
    },
];

/// Default tenancy colours (SilkCircuit Neon, with 256 and 16 fallbacks).
/// Overridable as `tenant.<name>` in `[theme.slots]`.
/// name, truecolor, xterm-256 index, ANSI-16 colour.
type TenantSlot = (&'static str, (u8, u8, u8), u8, Color);

const TENANT_SLOTS: [TenantSlot; 7] = [
    ("cyan", (0x80, 0xff, 0xea), 122, Color::Cyan),
    ("green", (0x50, 0xfa, 0x7b), 84, Color::Green),
    ("magenta", (0xff, 0x6a, 0xc1), 205, Color::Magenta),
    ("purple", (0xe1, 0x35, 0xff), 171, Color::LightMagenta),
    ("blue", (0x82, 0xaa, 0xff), 111, Color::Blue),
    ("yellow", (0xf1, 0xfa, 0x8c), 228, Color::Yellow),
    ("orange", (0xff, 0xb8, 0x6c), 215, Color::LightRed),
];

enum GlMsg {
    /// One tenancy's sync: what was asked for, and what came back.
    Loaded {
        group: String,
        want: Want,
        /// Validated provider/source identity captured before dispatch.
        action: radar_core::journal::ActionDescriptor,
        result: Result<Synced, String>,
        /// What the answer said about backing off.
        net: backoff::Net,
    },
    /// A setting was checked against GitLab.
    Checked {
        target: settings_checks::Target,
        request: settings_checks::Request,
        action: radar_core::journal::ActionDescriptor,
        result: Result<client::CheckRead, settings_history::CheckError>,
    },
    /// A merge request page's data, by `project!iid`.
    MrPage {
        key: String,
        read: page_ownership::Read,
        requested: page_identity::Requested,
        action: radar_core::journal::ActionDescriptor,
        result: Result<Box<client::MrDetailRead>, String>,
    },
    /// A repo page's pipelines on the exact requested ref, with frozen scope.
    Runs {
        request: runs_ownership::Request,
        action: radar_core::journal::ActionDescriptor,
        result: Result<client::RunsRead, String>,
    },
    /// A pipeline's failed jobs, by `project#pipeline id`, and the last part
    /// of the logs fetched with them.
    Failed {
        request: log_ownership::PipelineRead,
        action: radar_core::journal::ActionDescriptor,
        read: why::DiagnosisRead,
    },
    /// A page of a merge request's file diffs, asked for at `head`.
    Diffs {
        key: String,
        read: page_ownership::Read,
        head: String,
        page: usize,
        action: radar_core::journal::ActionDescriptor,
        result: Result<client::DiffRead, String>,
    },
    /// The required standalone job metadata and tail, with transient evidence.
    JobLog {
        request: log_ownership::LogRead,
        result: Result<log_history::JobLogRead, String>,
    },
    /// One required earlier chunk, retaining the actual transient trace evidence.
    EarlierLog {
        request: log_ownership::LogRead,
        result: Result<client::TraceRead, String>,
    },
    /// Payload-only reducer fixtures for the unchanged application path.
    #[cfg(test)]
    Log {
        request: log_ownership::LogRead,
        info: Option<model::JobInfo>,
        result: Result<model::TracePart, String>,
    },
}

/// Where a check of a setting stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Check {
    Running,
    Passed(String),
    Failed(String),
}

#[derive(Debug, Default)]
pub(crate) struct Sync {
    pub in_flight: usize,
    pub last_started: Option<i64>,
    pub last_ok: Option<i64>,
    pub errors: BTreeMap<String, String>,
    /// Tenancies that had their full sync this session; the rest are full.
    pub full_done: std::collections::BTreeSet<String>,
    /// When each tenancy's projects were last fetched.
    pub projects_at: BTreeMap<String, i64>,
    /// When each tenancy's last good sync started.
    pub synced_at: BTreeMap<String, i64>,
    /// Each tenancy's wait after failures GitLab is given time to get over,
    /// or while its rate limit is nearly spent.
    pub backoff: BTreeMap<String, backoff::Backoff>,
    /// The cache's sync state, until the first sync takes it up
    /// (`sync::adopt`).
    pub restored: Option<sync::Saved>,
}

/// The key that jumps the tree to what you type.
pub(crate) const GOTO_KEY: char = ':';
/// The key that picks whose merge requests are shown.
pub(crate) const AUTHOR_KEY: char = 'a';
/// The author menu's id.
pub(crate) const AUTHOR_MENU: &str = "author";

/// What is being typed, and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Conn(settings::Field),
    /// A tenancy's label (`code` in config).
    SourceCode(usize),
    /// Which group a tenancy tracks.
    SourceGroup(usize),
    SourceAdd,
    ViewFilter(usize),
    ViewName(usize),
    ViewAdd,
    /// Words to add to a view's filter as a condition.
    ViewWords(usize),
}

#[derive(Debug, Default)]
pub struct GitlabPlugin {
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) scripted_fixture: bool,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fixture: Option<test_support::HostedFixture>,
    pub(crate) cfg: Settings,
    /// Session-local connection identity; never persisted or put in the journal.
    pub(crate) connection: write_context::ConnectionStamp,
    pub(crate) me: String,
    pub(crate) data: BTreeMap<String, GroupData>,
    pub(crate) sync: Sync,
    pub(crate) now: i64,
    // merge requests screen
    pub(crate) view: usize,
    /// The `/` filter in force.
    pub(crate) filter: String,
    /// The `/` filter while it is being typed.
    pub(crate) filter_input: Option<radar_core::input::TextInput>,
    /// The repositories tree: what is selected and what is open.
    pub(crate) tree: std::cell::RefCell<radar_core::view::TreeNav>,
    /// Tree nodes already opened once by default, so closing one sticks.
    pub(crate) opened_once: std::cell::RefCell<std::collections::HashSet<Vec<String>>>,
    /// How far the detail pane is scrolled (PgUp/PgDn); back to the top when the
    /// selection moves.
    pub(crate) detail_scroll: radar_core::view::scroll::Scroll,
    /// Tenancies folded in the settings tree (by group).
    pub(crate) folded: std::collections::BTreeSet<String>,
    // settings
    pub(crate) section_rows: [usize; 4],
    /// The connection section was last drawn as a setup step, without the
    /// rows only Settings needs; `conn_rows` follows it.
    pub(crate) setup_card: std::cell::Cell<bool>,
    /// How far each section's rows are scrolled.
    pub(crate) section_scroll: [radar_core::view::scroll::Scroll; 4],
    pub(crate) edit: Option<(Target, radar_core::input::TextInput)>,
    /// Checks of settings against GitLab, by key: `auth`, `base`,
    /// `group:<full path>`. With when each finished.
    pub(crate) checks: BTreeMap<String, (Check, i64)>,
    pub(crate) check_owners: settings_checks::Owners,
    // item pages
    /// Pages opened from the tree, the shown one last; empty shows the tree.
    pub(crate) pages: Vec<pages::Page>,
    /// What merge request pages fetched, by `project!iid`.
    pub(crate) mr_pages: BTreeMap<String, pages::Fetched<(Mr, model::MrDetail)>>,
    /// Never reset, including connection changes and dedicated cache retirement.
    pub(crate) next_page_read: u64,
    pub(crate) mr_page_reads: BTreeMap<String, page_ownership::Read>,
    /// Merge requests whose detail a review asked for ([`agent_context::resolve`]),
    /// by `project!iid`, until the answer was given.
    pub(crate) resolving: std::collections::BTreeSet<String>,
    /// Merge requests' file diffs, by `project!iid`, asked for as files open.
    pub(crate) diffs: BTreeMap<String, pages::changes::Diffs>,
    /// What repo pages fetched: pipelines on the default branch, by path.
    pub(crate) runs: BTreeMap<String, pages::Fetched<Vec<model::PipelineRun>>>,
    pub(crate) runs_scopes: BTreeMap<String, runs_ownership::Scope>,
    /// Only this person's merge requests (display name), from the author menu.
    pub(crate) author: Option<String>,
    /// Go to: what is typed, and where the tree was before.
    pub(crate) goto: Option<goto::Goto>,
    /// The last rebase, merge, comment, resolve or approval asked for, by
    /// `project!iid`.
    pub(crate) acting: BTreeMap<String, mr_actions::Act>,
    /// A merge asked for, as its question showed it, by `project!iid`.
    pub(crate) merge_as: BTreeMap<String, Option<model::MergeSettings>>,
    /// A comment being typed.
    pub(crate) compose: Option<write::Compose>,
    /// A change shown and waiting for its `y`.
    pub(crate) change: Option<write::Change>,
    /// Frozen original text for the current confirmation; transient only.
    pub(crate) review: Option<comment_recovery::Review>,
    /// Independent kept occurrences and active reservations, never cached.
    pub(crate) recovery: comment_recovery::Recovery,
    // why a pipeline is red, and job logs
    /// A pipeline's failed jobs, by `project#pipeline id`.
    pub(crate) failed: BTreeMap<String, pages::Fetched<Vec<model::JobInfo>>>,
    pub(crate) failed_reads: BTreeMap<String, page_ownership::Read>,
    /// The diagnosis waiting for the selection to rest, and what held ones were made from.
    pub(crate) watch: why::Watch,
    /// Jobs and held logs are qualified by their requested project.
    pub(crate) jobs: BTreeMap<log_ownership::JobKey, model::JobInfo>,
    pub(crate) logs: BTreeMap<log_ownership::JobKey, pages::Fetched<joblog::JobLog>>,
    pub(crate) log_reads: BTreeMap<log_ownership::JobKey, log_ownership::JobReads>,
    /// Never reset, including configured connection changes.
    pub(crate) next_log_activity: u64,
    /// A search of a job's log being typed.
    pub(crate) log_search: Option<radar_core::input::TextInput>,
    /// The environment as names and values, when not the process's own
    /// (tests). Values are never shown.
    pub(crate) env: Option<Vec<(String, String)>>,
}

/// An MR as the screen shows it.
pub(crate) struct Item<'a> {
    pub mr: &'a Mr,
    pub role: Role,
    pub tenant: &'a Source,
}

impl Item<'_> {
    /// Its one word: what an open one needs from you (`review`), or how it
    /// ended (`merged`, `closed`). Also its look word.
    pub fn word(&self) -> &'static str {
        match self.mr.state {
            MrState::Opened => self.role.word(),
            ended => ended.word(),
        }
    }
}

impl GitlabPlugin {
    pub(crate) fn provider(&self) -> auth::Provider {
        auth::Provider {
            url: self.cfg.url.clone(),
            spec: self.cfg.token.clone(),
            #[cfg(any(test, feature = "test-support"))]
            scripted: self.scripted_fixture,
            #[cfg(any(test, feature = "test-support"))]
            fixture: self.fixture.clone(),
        }
    }

    pub fn new() -> Self {
        // GitLab's token formats never reach the log; `token = "glab"`
        // names the CLI that holds one, and stays.
        for shape in ["glpat-", "gloas-", "glptt-", "glrt-", "gldt-"] {
            log::token_shape(shape);
        }
        log::not_secret("glab");
        let now = time::now_secs();
        Self {
            now,
            // Look opens on its first word, not its first heading.
            section_rows: [0, 0, 0, 1],
            // Tests never read the environment they run in.
            #[cfg(test)]
            env: Some(vec![]),
            ..Default::default()
        }
    }

    pub(crate) fn current_view(&self) -> Option<&config::View> {
        self.cfg.views.get(self.view)
    }

    /// Variables in the environment holding a GitLab token, by name, not
    /// in use yet.
    pub(crate) fn offers(&self) -> Vec<String> {
        // Signed in: the token works, and another one is only noise.
        if !self.me.is_empty() {
            return vec![];
        }
        let vars = match &self.env {
            Some(vars) => vars.clone(),
            None => std::env::vars().collect(),
        };
        config::token_offers(&vars, &self.cfg.token)
    }

    /// Every tracked, visible, open MR with its role and tenancy, unfiltered.
    pub(crate) fn all_items(&self) -> Vec<Item<'_>> {
        self.held_items()
            .into_iter()
            .filter(|it| it.mr.state == MrState::Opened)
            .collect()
    }

    /// Whether any view, or the `/` filter, could show a merged or closed
    /// merge request: only then are they fetched.
    pub(crate) fn wants_ended(&self) -> bool {
        std::iter::once(self.filter.as_str())
            .chain(self.cfg.views.iter().map(|v| v.filter.as_str()))
            .any(|f| Filter::parse(f).wants_ended())
    }

    /// Every tracked, visible MR held, in any state.
    fn held_items(&self) -> Vec<Item<'_>> {
        let mut items = Vec::new();
        for source in &self.cfg.sources {
            let Some(data) = self.data.get(&self.cfg.group_path(source)) else {
                continue;
            };
            for mr in &data.mrs {
                // A project under a nested source belongs to that source only.
                let owner = self.cfg.source_for(&mr.project_path);
                if owner.is_none_or(|(s, _)| s.group != source.group)
                    || self.cfg.is_hidden(&mr.project_path)
                {
                    continue;
                }
                items.push(Item {
                    mr,
                    role: mr.role(&self.me),
                    tenant: source,
                });
            }
        }
        items
    }

    /// The items all `filters` match. A merge request that is not open
    /// shows only when one of them names a state (`state:merged`).
    fn matching<'a>(&'a self, items: Vec<Item<'a>>, filters: &[Filter]) -> Vec<Item<'a>> {
        items
            .into_iter()
            .filter(|it| {
                let cx = Context {
                    me: &self.me,
                    role: it.role,
                    tenant: &it.tenant.code,
                };
                filter::state_default(filters, it.mr)
                    && filters.iter().all(|f| f.matches(it.mr, &cx))
            })
            .collect()
    }

    /// The current view's MRs: view filter, the `/` filter, then the author
    /// picked; open ones first, by what they need from you, then most
    /// recently updated.
    pub(crate) fn visible(&self) -> Vec<Item<'_>> {
        let mut filters = vec![Filter::parse(&self.filter)];
        if let Some(v) = self.current_view() {
            filters.push(Filter::parse(&v.filter));
        }
        let mut items = self.matching(self.held_items(), &filters);
        if let Some(name) = &self.author {
            items.retain(|it| author_name(it.mr) == *name);
        }
        items.sort_by(|a, b| {
            (a.mr.state != MrState::Opened)
                .cmp(&(b.mr.state != MrState::Opened))
                .then(a.role.cmp(&b.role))
                .then_with(|| b.mr.updated_at.cmp(&a.mr.updated_at))
        });
        items
    }

    /// How many tracked, visible MRs a filter matches now.
    pub(crate) fn count(&self, filter: &str) -> usize {
        self.matching(self.held_items(), &[Filter::parse(filter)])
            .len()
    }

    /// Everyone who has opened a tracked MR: (username, name), you first.
    pub(crate) fn people(&self) -> Vec<(String, String)> {
        let mut people: Vec<(String, String)> = self
            .all_items()
            .iter()
            .map(|it| (it.mr.author.username.clone(), it.mr.author.name.clone()))
            .collect();
        people.sort_by(|a, b| (a.0 != self.me).cmp(&(b.0 != self.me)).then(a.1.cmp(&b.1)));
        people.dedup();
        people
    }

    /// The authors of the merge requests held now, by display name, once
    /// each (two accounts with one name are one person here): you first,
    /// then by name. With whether it is you.
    pub(crate) fn authors(&self) -> Vec<(String, bool)> {
        let mut names: Vec<(String, bool)> = self
            .held_items()
            .iter()
            .map(|it| (author_name(it.mr), it.mr.author.username == self.me))
            .collect();
        names.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
        });
        names.dedup_by(|a, b| a.0 == b.0);
        names
    }

    /// The author menu: everyone, then each author seen.
    pub(crate) fn author_menu(&self) -> radar_core::Menu {
        let mut items = vec![MenuItem::new(
            'e',
            "everyone",
            if self.author.is_none() {
                "shown now"
            } else {
                "every author again"
            },
            "",
        )];
        items.extend(self.authors().into_iter().map(|(name, me)| {
            let hint = match (me, self.author.as_deref() == Some(name.as_str())) {
                (_, true) => "shown now",
                (true, false) => "you",
                _ => "",
            };
            MenuItem::pick(name.clone(), hint, name)
        }));
        radar_core::Menu::new(AUTHOR_MENU, "show merge requests opened by", items)
    }

    /// The author menu chose `name` (empty: everyone).
    fn pick_author(&mut self, name: &str) -> Vec<Effect> {
        let before = self.author.clone();
        self.author = (!name.is_empty()).then(|| name.to_string());
        log::debug(|| format!("gitlab author {before:?} → {:?}", self.author));
        let n = self.visible().len();
        vec![Effect::Notify(match &self.author {
            Some(a) => format!("only {a}'s merge requests: {n}"),
            None => "everyone's merge requests".into(),
        })]
    }

    /// Repo names that have open MRs.
    pub(crate) fn repo_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .all_items()
            .iter()
            .map(|it| it.mr.project_name.clone())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Every label on a merge request held, once each, by name.
    pub(crate) fn labels(&self) -> Vec<String> {
        let mut labels: Vec<String> = self
            .held_items()
            .iter()
            .flat_map(|it| it.mr.labels.iter().cloned())
            .collect();
        labels.sort_by_key(|l| l.to_lowercase());
        labels.dedup();
        labels
    }

    /// The role of every tracked, visible MR, whatever the view.
    pub(crate) fn all_roles(&self) -> Vec<Role> {
        self.all_items().iter().map(|it| it.role).collect()
    }

    /// Open, visible MRs per tenancy, in source order.
    pub(crate) fn tenancy_counts(&self) -> Vec<usize> {
        let items = self.all_items();
        self.cfg
            .sources
            .iter()
            .map(|s| items.iter().filter(|it| it.tenant.group == s.group).count())
            .collect()
    }

    pub(crate) fn view_counts(&self) -> Vec<usize> {
        self.cfg
            .views
            .iter()
            .map(|v| {
                self.matching(self.held_items(), &[Filter::parse(&v.filter)])
                    .len()
            })
            .collect()
    }

    /// Fetch everything again (`r`, or after a setting changed).
    /// The person asked, so a tenancy that is backing off is asked too, this
    /// once; its count of failures stays until a sync succeeds.
    pub(crate) fn refresh(&mut self) -> Vec<Effect> {
        self.sync_now(true, true)
    }

    /// Whether a tenancy is left alone for now (it failed, or GitLab's rate
    /// limit is nearly spent).
    pub(crate) fn backing_off(&self, group: &str) -> bool {
        self.sync
            .backoff
            .get(group)
            .is_some_and(|b| b.waiting(self.now))
    }

    /// A sync failure's words, and when the tenancy will be tried again.
    pub(crate) fn sync_failure(&self, group: &str, why: &str) -> String {
        let why = why.lines().next().unwrap_or_default();
        match self.sync.backoff.get(group).filter(|b| b.waiting(self.now)) {
            Some(b) => format!(
                "{why} · trying again in {}",
                backoff::in_words(b.left(self.now))
            ),
            None => why.to_string(),
        }
    }

    /// Sync every tenancy: in full, or only what changed (`sync::plan`).
    /// A tenancy that is backing off is left out unless `force`.
    fn sync_now(&mut self, full: bool, force: bool) -> Vec<Effect> {
        if self.sync.in_flight > 0 || self.cfg.sources.is_empty() {
            return vec![];
        }
        sync::adopt(self);
        let mut plans = sync::plan(self, full);
        if !force {
            plans.retain(|(group, _)| !self.backing_off(group));
        }
        if plans.is_empty() {
            return vec![];
        }
        self.sync.in_flight = plans.len();
        self.sync.last_started = Some(self.now);
        plans
            .into_iter()
            .map(|(group, want)| {
                let url = self.cfg.url.clone();
                let provider = self.provider();
                let instance = url.clone();
                sync_history::spawn(&instance, group.clone(), want.clone(), async move {
                    let how = if want.changed_since.is_some() {
                        "changes"
                    } else {
                        "everything"
                    };
                    log::info(format!("gitlab {group}: {how}"));
                    let started = std::time::Instant::now();
                    let result = provider.run(|c| {
                        let (group, want) = (group.clone(), want.clone());
                        async move {
                            let synced = c.sync(&group, &want).await?;
                            Ok((synced, c.pacing()))
                        }
                    })
                    .await;
                    let (result, net) = match result {
                        Ok((synced, pacing)) => (
                            Ok(synced),
                            backoff::Net {
                                transient: false,
                                pacing,
                            },
                        ),
                        Err(e) => {
                            let trouble = backoff::trouble_of(&e);
                            let net = backoff::Net {
                                transient: trouble.is_some(),
                                pacing: trouble.map(|t| t.pacing).unwrap_or_default(),
                            };
                            (Err(format!("{e:#}")), net)
                        }
                    };
                    match &result {
                        Ok(s) => log::info(format!(
                            "gitlab {group}: {} merge requests{} in {:.1}s",
                            s.mrs.len(),
                            if s.repo { " (a repo)" } else { "" },
                            started.elapsed().as_secs_f32()
                        )),
                        Err(e) => log::error(format!("gitlab {group} failed: {e}")),
                    }
                    (result, net)
                })
            })
            .collect()
    }

    /// The merge request selected in the tree, for the bus.
    pub(crate) fn selected_payload(&self) -> Option<Value> {
        let items = self.visible();
        let it = screen::selected_mr(self, &items)?;
        Some(json!({
            "repo": it.mr.project_path, "iid": it.mr.iid, "title": it.mr.title,
            "branch": it.mr.source_branch, "url": it.mr.web_url, "state": it.word(),
        }))
    }

    pub(crate) fn clamp(&mut self) {
        self.view = self.view.min(self.cfg.views.len().saturating_sub(1));
    }

    /// Keys the tree, the nav pane and pages share: go to and authors.
    /// `None` when the key is not one of them.
    fn shared_key(&mut self, pane: Pane, key: Chord) -> Option<Outcome> {
        if !key.plain() {
            return None;
        }
        let tree = self.pages.is_empty() && matches!(pane, Pane::Nav | Pane::Middle);
        match key.key {
            Key::Char(AUTHOR_KEY) if tree => {
                Some(Outcome::Handled(vec![Effect::Menu(self.author_menu())]))
            }
            Key::Char(GOTO_KEY) if tree && !self.cfg.sources.is_empty() => {
                goto::start(self);
                Some(Outcome::Handled(vec![Effect::Focus(Pane::Middle)]))
            }
            _ => None,
        }
    }

    /// The `x` rows for [`Self::shared_key`], said for now.
    fn shared_keys(&self, pane: Pane) -> Vec<Row> {
        let mut b = Vec::new();
        let tree = self.pages.is_empty() && matches!(pane, Pane::Nav | Pane::Middle);
        if tree && pane == Pane::Middle && !self.cfg.sources.is_empty() {
            b.push(act(
                "gitlab.repos.goto",
                Binding::new(
                    GOTO_KEY.to_string(),
                    "go to a tenancy, repo or merge request…",
                ),
            ));
        }
        if tree {
            b.push(act(
                "gitlab.repos.authors",
                Binding::new(
                    AUTHOR_KEY.to_string(),
                    match &self.author {
                        Some(a) => format!("only {a}'s now: pick another author…"),
                        None => "show one author's merge requests…".into(),
                    },
                ),
            ));
        }
        b
    }

    /// An invoked action id runs the action the view lists under it: look
    /// the id up among the actions offered here and answer the intent for the
    /// key that runs it. An id nobody lists is left as it is (and ignored).
    fn resolve_action(
        &self,
        scope: &str,
        pane: Pane,
        intent: &radar_core::view::Intent,
    ) -> Option<radar_core::view::Intent> {
        let radar_core::view::Intent::Invoke { action } = intent else {
            return None;
        };
        let offered = if scope == SCREEN.id {
            actions(&self.rows(pane))
        } else {
            settings::action_specs(self, scope)
        };
        let spec = offered.into_iter().find(|a| a.id == *action)?;
        spec.key.map(|chord| radar_core::view::Intent::Key { chord })
    }

    /// The `x` rows of `pane` with their ids.
    pub(crate) fn rows(&self, pane: Pane) -> Vec<Row> {
        let mut b = if self.pages.is_empty() {
            screen::keys(self, pane)
        } else {
            pages::keys(self, pane)
        };
        // Before a trailing "back" row, so that stays last.
        let at = b
            .iter()
            .rposition(|(_, r)| r.keys == "esc")
            .filter(|i| i + 1 == b.len())
            .unwrap_or(b.len());
        let shared = self.shared_keys(if self.pages.is_empty() {
            pane
        } else {
            Pane::Middle
        });
        b.splice(at..at, shared);
        // Like the shared keys: on a page every pane asks about the page's MR.
        b.splice(at..at, agent_context::rows(self, if self.pages.is_empty() { pane } else { Pane::Middle }));
        b.push(act(
            if self.pages.is_empty() { "gitlab.repos.kept_comments" } else { "gitlab.page.kept_comments" },
            Binding::new("ctrl-r", "kept comments…"),
        ));
        b
    }
}

/// A row of the actions menu with its stable id, `gitlab.<surface>.<verb>`,
/// written where the row is made: not taken from its key or its words, which
/// can change without the action being another.
pub(crate) type Row = (String, Binding);

/// A row with the id `id`.
pub(crate) fn act(id: &str, binding: Binding) -> Row {
    (id.to_string(), binding)
}

/// A heading row: it has no id.
pub(crate) fn head(binding: Binding) -> Row {
    (String::new(), binding)
}

/// The bindings of rows, as the menu shows them.
pub(crate) fn bindings_of(rows: Vec<Row>) -> Vec<Binding> {
    rows.into_iter().map(|(_, b)| b).collect()
}

/// The actions a view offers, from the rows it shows.
pub(crate) fn actions(rows: &[Row]) -> Vec<radar_core::view::ActionSpec> {
    rows.iter()
        .map(|(id, b)| radar_core::view::ActionSpec::named(id.as_str(), b))
        .collect()
}

/// How an author is shown and picked: the display name, else the username.
pub(crate) fn author_name(mr: &Mr) -> String {
    if mr.author.name.is_empty() {
        mr.author.username.clone()
    } else {
        mr.author.name.clone()
    }
}

impl GitlabPlugin {
    fn on_key(&mut self, pane: Pane, key: Chord) -> Outcome {
        if key.ctrl && key.key == Key::Char('r') {
            return Outcome::Handled(comment_recovery::list(self, None));
        }
        if self.compose.is_some() {
            return write::compose_key(self, key);
        }
        if self.goto.is_some() {
            return goto::key(self, key);
        }
        if self.filter_input.is_none()
            && self.log_search.is_none()
            && let Some(out) = self.shared_key(pane, key)
        {
            return out;
        }
        if self.filter_input.is_none()
            && self.log_search.is_none()
            && let Some(out) = agent_context::key_press(self, if self.pages.is_empty() { pane } else { Pane::Middle }, key)
        {
            return out;
        }
        if !self.pages.is_empty() {
            return pages::key(self, pane, key);
        }
        let out = screen::key(self, pane, key);
        // esc with no filter left clears the author picked.
        if matches!(out, Outcome::Ignored)
            && key.key == Key::Esc
            && pane == Pane::Middle
            && self.author.is_some()
        {
            return Outcome::Handled(self.pick_author(""));
        }
        self.clamp();
        out
    }
    /// Test shim: a pointer hit on the target `id` of `scope`, as the host
    /// sends it.
    #[cfg(test)]
    fn click(&mut self, scope: &str, id: &str) -> Outcome {
        let target = radar_core::view::Target::new(id);
        Plugin::intent(self, scope, Pane::Middle, &radar_core::view::Intent::Select { target })
    }
    /// Test shim: a key as the host sends it: routed over the screen's view
    /// (movement along rows and trees is the frontend's, selecting through
    /// the part), the rest handed to the part as an intent.
    #[cfg(test)]
    fn key(&mut self, scope: &str, pane: Pane, key: Chord) -> Outcome {
        use radar_core::route::Routed;
        let chord = key;
        let vcx = radar_core::view::ViewCx {
            now: self.now,
            focus: pane.into(),
            width: 160,
            height: 40,
            parts: radar_core::Parts::none(),
            icon_lead: 0,
            setup: false,
        };
        let Some(view) = Plugin::view(self, scope, &vcx) else {
            return self.on_key(pane, key);
        };
        let frontend = radar_core::route::Frontend::default();
        frontend.set_room(160, 40);
        match radar_core::route::key(&frontend, &view, pane.into(), chord) {
            Routed::Intent(intent) => Plugin::intent(self, scope, pane, &intent),
            Routed::Done => Outcome::done(),
            Routed::Ignored => Outcome::Ignored,
        }
    }
}

impl Plugin for GitlabPlugin {
    fn id(&self) -> &'static str {
        "gitlab"
    }
    #[cfg(feature = "demo")]
    fn demo(&mut self) {
        // With !263's page data and diffs held, so a demo run reaches its
        // Changes tab and can write a comment on it.
        #[cfg(any(test, feature = "test-support"))]
        let fixture = (self.scripted_fixture, self.fixture.clone());
        *self = demo::sample_reviewed(false);
        #[cfg(any(test, feature = "test-support"))]
        {
            self.scripted_fixture = fixture.0;
            self.fixture = fixture.1;
            if self.scripted_fixture { self.env = Some(vec![]); }
        }
    }
    fn name(&self) -> &'static str {
        "GitLab"
    }

    fn set_up(&self) -> radar_core::Setup {
        if self.cfg.sources.is_empty() {
            radar_core::Setup::Needed(
                "Add a tenancy: a GitLab group or repo whose merge requests to follow.".into(),
            )
        } else {
            radar_core::Setup::Ready
        }
    }
    fn owns(&self) -> Vec<radar_core::Owned> {
        join::OWNS.to_vec()
    }
    fn agent_context(
        &mut self,
        request: &radar_core::agent_context::AgentContextRequest,
    ) -> Option<radar_core::agent_context::AgentContext> {
        agent_context::agent_context(self, request)
    }
    fn prepare_agent_context(&mut self, subject: &radar_core::journal::SubjectKey) -> (bool, Vec<Effect>) {
        if subject.provider != "gitlab" || subject.kind != "merge-request" { return (false, vec![]); }
        let Some((project, iid)) = agent_context::split_id(&subject.id) else { return (false, vec![]); };
        if self.mr_pages.get(&pages::mr_key(project, iid)).is_some_and(|f| f.error.is_some()) { return (false, vec![]); }
        agent_context::resolve(self, &radar_core::agent_context::ChangeResolveRequest { subject: subject.clone() })
            .map_or((false, vec![]), |(state, fx)| (state == radar_core::agent_context::Resolve::Pending, fx))
    }
    fn agent_document(&mut self, request: &radar_core::agent_context::AgentContextRequest) -> Option<radar_core::context::Document> {
        agent_context::document(self, request)
    }
    fn agent_related(&self, subject: &radar_core::Subject) -> Vec<radar_core::context::Related> {
        use radar_core::context::Related;
        if subject.is("ticket") {
            return tickets::carrying(self, &subject.id).into_iter().map(|m| Related::new(join::mr(self, &m.project_path, m.iid), "implements", "inferred", "ticket key in MR title or source branch")).collect();
        }
        if let Some(snapshot) = self.journal_subject(subject)
            && snapshot.key.kind == "merge-request"
            && let Some((project, _)) = agent_context::split_id(&snapshot.key.id) {
            return vec![Related::new(join::repo(self, project), "repository", "provider", "MR project")];
        }
        vec![]
    }
    fn resolve_change(
        &mut self,
        request: &radar_core::agent_context::ChangeResolveRequest,
    ) -> Option<(radar_core::agent_context::Resolve, Vec<radar_core::plugin::Effect>)> {
        agent_context::resolve(self, request)
    }
    fn subject_from_journal(&self, key: &radar_core::journal::SubjectKey) -> Option<radar_core::Subject> {
        source_navigation::from_journal(self, key)
    }

    fn journal_subject(
        &self,
        subject: &radar_core::Subject,
    ) -> Option<radar_core::journal::SubjectSnapshot> {
        use radar_core::journal::{SafeUrl, SubjectKey, SubjectRole, SubjectSnapshot};
        join::Target::consistent(&self.cfg.url, subject)?;
        let base = SafeUrl::parse(&self.cfg.url).ok()?;
        let root = base.as_str().trim_end_matches('/');
        let (project, id, url) = match subject.kind.as_str() {
            join::REPO => (
                subject.id.as_str(),
                subject.id.clone(),
                format!("{root}/{}", subject.id),
            ),
            join::MERGE_REQUEST => {
                let (project, number) = subject.id.rsplit_once('!')?;
                let number: u64 = number.parse().ok()?;
                (
                    project,
                    format!("{project}!{number}"),
                    format!("{root}/{project}/-/merge_requests/{number}"),
                )
            }
            join::PIPELINE | join::JOB => {
                let (project, number) = subject.id.rsplit_once('#')?;
                let number: u64 = number.parse().ok()?;
                let path = if subject.kind == join::JOB {
                    "jobs"
                } else {
                    "pipelines"
                };
                // A slash retains the numeric identity without a URL-fragment marker.
                (
                    project,
                    format!("{project}/{number}"),
                    format!("{root}/{project}/-/{path}/{number}"),
                )
            }
            _ => return None,
        };
        if config::group_path_problem(project).is_some() {
            return None;
        }
        let url = SafeUrl::parse(&url).ok()?;
        // A familiar kind alone does not establish ownership of another server's item.
        if SafeUrl::parse(subject.url.as_deref()?).ok()? != url {
            return None;
        }
        let key = SubjectKey::new("gitlab", base.as_str(), &subject.kind, &id).ok()?;
        Some(SubjectSnapshot::new(key, SubjectRole::Primary).with_url(url))
    }
    fn recognise(&self, text: &str) -> Vec<(std::ops::Range<usize>, radar_core::Subject)> {
        join::recognise(self, text)
    }
    fn palette_targets(&self) -> Vec<radar_core::palette::PaletteTarget> {
        palette::targets(self)
    }

    fn show(&mut self, subject: &radar_core::Subject) -> radar_core::Showing {
        join::show(self, subject)
    }
    fn depth(&self, _: &str) -> usize {
        self.pages.len()
    }
    fn page_identity(&self, screen: &str) -> Option<String> {
        if screen != SCREEN.id {
            return None;
        }
        Some(self.pages.last()?.kind.identity())
    }
    fn detail_identity(&self, scope: &str, pane: Pane) -> Option<radar_core::plugin::DetailIdentity> {
        if scope != SCREEN.id || pane != Pane::Detail || !self.pages.is_empty() {
            return None;
        }
        let items = self.visible();
        let item = screen::selected_mr(self, &items)?;
        let mr = item.mr;
        let group = self.cfg.group_path(item.tenant);
        let usable_path = |path: &str| config::group_path_problem(path).is_none()
            && !path.split('/').any(|part| matches!(part, "." | ".."));
        if mr.iid == 0 || !usable_path(&mr.project_path) || !usable_path(&group) {
            return None;
        }
        Some(radar_core::plugin::DetailIdentity {
            connection: self.connection_stamp().generation(),
            subject: vec![
                group,
                mr.project_path.clone(),
                mr.iid.to_string(),
                mr.web_url.clone(),
                mr.pipeline_url.clone().unwrap_or_default(),
                mr.source_branch.clone(),
                mr.target_branch.clone(),
                mr.head_sha.clone(),
                self.me.clone(),
            ],
        })
    }
    fn subject(&self, _: &str, pane: Pane) -> Option<radar_core::Subject> {
        join::subject(self, pane)
    }
    fn offers(&self) -> Vec<radar_core::Offer> {
        join::OFFERS.to_vec()
    }
    fn adds(&self) -> Vec<radar_core::Adds> {
        tickets::ADDS.to_vec()
    }
    fn related(&self, subject: &radar_core::Subject) -> Vec<radar_core::Subject> {
        tickets::related(self, subject)
    }
    fn contribute(
        &self,
        point: radar_core::Point,
        subject: &radar_core::Subject,
    ) -> Option<radar_core::Contribution> {
        tickets::contribute(self, point, subject)
    }

    fn configure(&mut self, table: Option<&toml::Value>) -> anyhow::Result<()> {
        let cfg: Settings = match table {
            Some(v) => v.clone().try_into()?,
            None => Settings::default(),
        };
        self.replace_connection(cfg.url.clone(), cfg.token.clone());
        self.cfg = cfg;
        settings_checks::reconcile(self);
        self.clamp();
        Ok(())
    }

    fn config(&self) -> Option<toml::Value> {
        toml::Value::try_from(&self.cfg).ok()
    }

    fn first_run(&mut self) -> Option<String> {
        Some(match self.offers().first() {
            Some(var) => {
                format!("GitLab: found {var} in your environment; use it under connection")
            }
            None => "GitLab: set the server and token under connection, then add a tenancy".into(),
        })
    }

    fn slots(&self) -> Vec<(String, SlotValue)> {
        TENANT_SLOTS
            .iter()
            .map(|&(name, rgb, i256, c16)| {
                (format!("tenant.{name}"), SlotValue::new(rgb, i256, c16))
            })
            .collect()
    }

    fn restore(&mut self, cache: Value) {
        if let Some(me) = cache["me"].as_str() {
            self.me = me.to_string();
        }
        if let Ok(data) = serde_json::from_value(cache["data"].clone()) {
            self.data = data;
            runs_ownership::reconcile(self);
            sync::restore(self, &cache);
        }
    }

    fn snapshot(&self) -> Option<Value> {
        Some(json!({ "me": self.me, "data": self.data, "sync": sync::saved(self) }))
    }

    fn tick(&mut self, now: i64) -> Vec<Effect> {
        self.now = now;
        let rested = why::rested(self);
        if !rested.is_empty() {
            return rested;
        }
        // Paused (`z`, or idle) is the host's: it does not tick a part then.
        let due = self
            .sync
            .last_started
            .is_none_or(|t| now - t >= self.cfg.refresh_secs as i64);
        // A view that shows merged ones needs them now, not next minute; not
        // from a tenancy that is backing off, or a failed sync would be sent
        // again on every tick.
        let missing = self.wants_ended()
            && self
                .data
                .iter()
                .any(|(group, d)| d.ended_since.is_none() && !self.backing_off(group));
        if due || missing {
            self.sync_now(false, false)
        } else {
            vec![]
        }
    }

    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        let msg = match msg.downcast::<GlMsg>() {
            Ok(msg) => msg,
            // Local checkouts, rebase and merge.
            Err(other) => return pages::act::on_msg(self, other),
        };
        let (group, want, action, result, net) = match *msg {
            GlMsg::Loaded {
                group,
                want,
                action,
                result,
                net,
            } => (group, want, action, result, net),
            GlMsg::Checked {
                target,
                request,
                action,
                result,
            } => {
                let mut effects = settings_history::observed(target.kind(), &action, &result);
                if !settings_checks::claim(self, &target, request) {
                    effects.push(settings_history::skipped(target.kind(), &action));
                    return effects;
                }
                let check = match result {
                    Ok(got) => Check::Passed(got.value),
                    Err(error) => Check::Failed(error.message),
                };
                let say = match &check {
                    Check::Passed(m) => Effect::Notify(radar_core::writes::done_line(m)),
                    Check::Failed(e) => Effect::error(None, radar_core::writes::failed_line(e)),
                    Check::Running => Effect::Notify(String::new()),
                };
                self.checks.insert(target.key(), (check, self.now));
                effects.push(say);
                return effects;
            }
            GlMsg::MrPage {
                key,
                read,
                requested,
                action,
                result,
            } => {
                let mut effects = page_history::observed(&action, &result);
                if !page_ownership::detail(self, &key, read) {
                    effects.push(page_history::skipped(&action));
                    return effects;
                }
                if result
                    .as_ref()
                    .is_ok_and(|got| !requested.accepts(&key, got))
                {
                    page_identity::reject(self, &key);
                    effects.push(page_history::identity_rejected(&action));
                    return effects;
                }
                effects.extend(pages::on_mr_page(
                    self,
                    key,
                    result.map(|got| (got.mr, got.detail)),
                ));
                return effects;
            }
            GlMsg::Runs {
                request,
                action,
                result,
            } => {
                let mut effects = runs_history::observed(&action, &result);
                if !runs_ownership::claim(self, &request) {
                    effects.push(runs_history::skipped(&action));
                    return effects;
                }
                pages::on_runs(self, request.path, result.map(|got| got.runs));
                return effects;
            }
            GlMsg::Failed {
                request,
                action,
                read,
            } => {
                // Original read evidence survives every application rejection.
                let mut effects = failure_history::observed(&action, &read);
                effects.extend(why::on_failed(
                    self,
                    request,
                    &action,
                    read.result.map(|got| got.jobs),
                    read.logs.unwrap_or_default(),
                ));
                return effects;
            }
            GlMsg::Diffs {
                key,
                read,
                head,
                page,
                action,
                result,
            } => {
                // Original read evidence survives application ownership rejection.
                let mut effects = diff_history::observed(&action, &result);
                if !page_ownership::diff(self, &key, &head, page, read) {
                    effects.push(diff_history::stale(&action));
                    return effects;
                }
                effects.extend(pages::changes::on_diffs(
                    self,
                    key,
                    head,
                    page,
                    result.map(|got| got.files),
                ));
                return effects;
            }
            GlMsg::JobLog { request, result } => {
                // Original combined-read evidence survives application rejection.
                let mut effects = log_history::observed(&request.action, &result);
                let (info, result) = match result {
                    Ok(got) => (Some(got.job.info), Ok(got.trace.part)),
                    Err(error) => (None, Err(error)),
                };
                effects.extend(why::on_log(self, request, info, result));
                return effects;
            }
            GlMsg::EarlierLog { request, result } => {
                // Observe the actual read even if its held base can no longer apply.
                let mut effects = log_history::earlier_observed(&request.action, &result);
                effects.extend(why::on_log(self, request, None, result.map(|got| got.part)));
                return effects;
            }
            #[cfg(test)]
            GlMsg::Log {
                request,
                info,
                result,
            } => {
                return why::on_log(self, request, info, result);
            }
        };
        let selection = settings::StableSelection::capture(self);
        self.sync.in_flight = self.sync.in_flight.saturating_sub(1);
        let mut fx = sync_history::observed(&action, &result);
        let mut changes = radar_core::motion::Changes::default();
        match result {
            Ok(got) => {
                if !got.me.is_empty() {
                    self.me = got.me.clone();
                }
                if want.projects {
                    self.sync.projects_at.insert(group.clone(), self.now);
                }
                if want.changed_since.is_none() {
                    self.sync.full_done.insert(group.clone());
                }
                if let Some(at) = self.sync.last_started {
                    self.sync.synced_at.insert(group.clone(), at);
                }
                let data = sync::merge(
                    self.data.get(&group),
                    &want,
                    got,
                    self.now,
                    self.cfg.history_days,
                );
                let held = self.data.insert(group.clone(), data);
                runs_ownership::reconcile(self);
                // What the sync changed, for the rows to glow (motion).
                changes = radar_core::motion::Changes::between(
                    held.iter()
                        .flat_map(|d| &d.mrs)
                        .map(|m| (join::motion_key(m), m)),
                    self.data[&group]
                        .mrs
                        .iter()
                        .map(|m| (join::motion_key(m), m)),
                );
                self.sync.errors.remove(&group);
                self.sync.last_ok = Some(self.now);
                let wait = backoff::Backoff::succeeded(self.now, &net.pacing);
                if wait.waiting(self.now) {
                    log::info(format!(
                        "gitlab {group}: rate limit nearly spent, next sync in {}",
                        backoff::in_words(wait.left(self.now))
                    ));
                }
                self.sync.backoff.insert(group, wait);
            }
            Err(e) => {
                if net.transient {
                    let held = self.sync.backoff.get(&group).copied().unwrap_or_default();
                    let wait = held.failed(&group, self.now, &net.pacing);
                    log::info(format!(
                        "gitlab {group}: failure {} in a row, next try in {}",
                        wait.failures,
                        backoff::in_words(wait.left(self.now))
                    ));
                    self.sync.backoff.insert(group.clone(), wait);
                }
                self.sync.errors.insert(group, e);
            }
        }
        selection.restore(self);
        self.clamp();
        if !changes.is_empty() {
            fx.push(Effect::Changed(changes));
        }
        if self.sync.in_flight == 0 && self.sync.errors.is_empty() {
            fx.push(Effect::PersistCache);
        }
        fx
    }

    fn capturing(&self) -> bool {
        self.filter_input.is_some()
            || self.edit.is_some()
            || self.goto.is_some()
            || self.log_search.is_some()
            || self.compose.is_some()
    }

    fn paste(&mut self, text: &str) -> Vec<Effect> {
        if self.compose.is_some() {
            write::compose_paste(self, text);
        } else if self.goto.is_some() {
            goto::paste(self, text);
        } else if let Some(input) = self.filter_input.as_mut() {
            input.paste(text);
            self.filter = input.value();
        } else if let Some((_, input)) = self.edit.as_mut() {
            input.paste(text);
        } else if let Some(input) = self.log_search.as_mut() {
            input.paste(text);
        }
        vec![]
    }

    fn screens(&self) -> Vec<ScreenSpec> {
        vec![SCREEN]
    }
    fn screen_group(&self, _: &str) -> Option<radar_core::ScreenGroup> {
        Some(radar_core::ScreenGroup {
            id: "repositories",
            name: "repositories",
            key: 'R',
            source: "GitLab",
        })
    }
    fn icon(&self, screen: &str) -> Option<radar_core::ui::icon::Icon> {
        use radar_core::ui::icon::Icon;
        (screen == SCREEN.id).then_some(Icon::Repo)
    }
    /// What waits on you: the merge requests to review, hidden at 0.
    fn badge(&self, _: &str) -> Option<(String, radar_core::Tone)> {
        let n = screen::to_review(self);
        (n > 0).then(|| (n.to_string(), radar_core::Tone::Warn))
    }
    fn section_icon(&self, section: &str) -> Option<radar_core::ui::icon::Icon> {
        use radar_core::ui::icon::Icon;
        Some(match section {
            "connection" => Icon::Connection,
            "tenancies" => Icon::Group,
            "views" => Icon::Views,
            "look" => Icon::Look,
            _ => return None,
        })
    }
    fn top_right(&self, _: &str) -> Option<String> {
        screen::top_right(self)
    }
    fn busy(&self, _: &str) -> bool {
        self.sync.in_flight > 0 || pages::loading(self) || mr_actions::running(self)
    }
    fn status(&self, _: &str) -> Vec<(String, radar_core::Tone)> {
        screen::status(self)
    }
    fn health(&self) -> Option<radar_core::health::Health> {
        health::health(self)
    }
    fn refresh_now(&mut self) -> Vec<Effect> {
        self.refresh()
    }
    fn view(&self, screen: &str, cx: &radar_core::view::ViewCx<'_>) -> Option<radar_core::view::ScreenView> {
        if screen != SCREEN.id { return None; }
        let mut view = if self.pages.is_empty() { Some(screen::neutral::view(self, cx)) }
            else { pages::neutral::view(self, cx) }?;
        view.overlay = if self.pages.last().is_some_and(|page| pages::review::inline(self, page)) {
            None
        } else {
            write::overlay(self).map(Box::new)
        };
        Some(view)
    }
    fn section_view(&self, section: &str, cx: &radar_core::view::ViewCx<'_>) -> Option<radar_core::view::SectionView> {
        if section == "connection" {
            self.setup_card.set(cx.setup);
        }
        match section {
            "look" => Some(settings::look_view(self)),
            "views" => Some(settings::views_view(self, cx.width)),
            "connection" => Some(settings::connection_view(self, cx.width, cx.now)),
            "tenancies" => Some(settings::tenancies_view(self, cx.width, cx.now)),
            _ => None,
        }
    }
    fn intent(&mut self, scope: &str, pane: Pane, intent: &radar_core::view::Intent) -> Outcome {
        let resolved = self.resolve_action(scope, pane, intent);
        let intent = resolved.as_ref().unwrap_or(intent);
        if scope == SCREEN.id && self.pages.is_empty() {
            let out = screen::neutral::intent(self, pane, intent);
            self.clamp();
            return out;
        }
        if scope == SCREEN.id && self.pages.last().is_some_and(|page| (matches!(page.kind, pages::Kind::Tenancy(_)) && page.tab == 0)
            || (matches!(page.kind, pages::Kind::Repo(_)) && matches!(page.tab, 0 | 1))
            || (matches!(page.kind, pages::Kind::Mr { .. }) && matches!(page.tab, 0..=4))
            || (matches!(page.kind, pages::Kind::Job { .. }) && page.tab == 0)
            || page.tab >= pages::own_tabs(&page.kind)) {
            let out = pages::neutral::intent(self, pane, intent);
            self.clamp();
            return out;
        }
        if !matches!(scope, "look" | "views" | "connection" | "tenancies") { return Outcome::Ignored; }
        if intent.chord().is_some_and(|chord| chord.ctrl && chord.key == radar_core::view::Key::Char('r')) {
            return Outcome::Handled(comment_recovery::list(self, None));
        }
        let out = if scope == "tenancies" { settings::tenancies_intent(self, intent) }
            else { settings::list_intent(self, scope, intent) };
        self.clamp();
        out
    }
    fn pictures(&self, _source: &str) -> Option<radar_core::fetch::Fetch> {
        #[cfg(any(test, feature = "test-support"))]
        if self.scripted_fixture {
            let fixture = self.fixture.clone();
            return Some(radar_core::fetch::fetch(move |_src: String| {
                let fixture = fixture.clone();
                async move {
                    if let Some(fixture) = fixture { fixture.blocked("picture fetch"); }
                    anyhow::bail!("scripted GitLab fixture does not support pictures")
                }
            }));
        }
        Some(pictures::fetch(self.cfg.url.clone(), self.cfg.token.clone()))
    }
    // The preview pages from any pane (PgDn) and offers its page keys by
    // how far the renderer laid it out; a page keeps its own scroll.
    fn scroll(&self, screen: &str, pane: Pane) -> Option<&radar_core::view::scroll::Scroll> {
        (screen == SCREEN.id && pane == Pane::Detail && self.pages.is_empty()).then_some(&self.detail_scroll)
    }
    // An open page takes the whole body, its keys, clicks and actions.
    fn panes(&self, _: &str) -> Vec<Pane> {
        if self.pages.is_empty() {
            vec![Pane::Nav, Pane::Middle, Pane::Detail]
        } else {
            // A page is one pane, but for the Changes tab's files beside
            // its diff; the host moves between them and, past the edge,
            // across the page's tabs.
            pages::panes(self)
        }
    }
    fn tabs(&self, _: &str) -> Option<radar_core::plugin::PageTabs> {
        pages::tabs(self)
    }
    fn show_tab(&mut self, _: &str, i: usize) -> Vec<Effect> {
        let mut fx = pages::show_tab(self, i);
        fx.extend(pages::changes::want(self));
        fx
    }
    fn hints(&self, _: &str, _: Pane) -> Vec<KeyHint<'static>> {
        if let Some(h) = write::hints(self) {
            return h;
        }
        if self.goto.is_some() {
            return goto::hints();
        }
        if self.log_search.is_some() {
            return vec![
                KeyHint::new("⏎", "search the log"),
                KeyHint::new("esc", "stop"),
            ];
        }
        screen::hints(self)
    }
    /// `+`/`-` fold: the tree of tenancies, repos and merge requests, the
    /// Changes tab's files (its first pane), a job's log sections.
    fn folds(&self, _: &str, pane: Pane) -> bool {
        match self.pages.last() {
            None => pane == Pane::Middle,
            Some(page) => {
                (pages::changes::on_changes(page).is_some()
                    && pane == Pane::Nav
                    && page.tree_shown.get())
                    || matches!(page.kind, pages::Kind::Job { .. })
            }
        }
    }
    fn fold_keys(&self, screen: &str, pane: Pane) -> Vec<Binding> {
        if self.pages.is_empty() && pane == Pane::Middle {
            return bindings_of(screen::fold_keys(self));
        }
        if self.folds(screen, pane) {
            vec![
                Binding::new("+", "expand the group"),
                Binding::new("-", "collapse the group, or the one this row is in"),
            ]
        } else {
            Vec::new()
        }
    }
    fn section_folds(&self, section: &str) -> bool {
        section == "tenancies"
    }
    fn row_hints(&self, _: &str, pane: Pane) -> Vec<KeyHint<'static>> {
        list_actions::hints(self, pane)
    }
    fn keys(&self, _: &str, pane: Pane) -> Vec<Binding> {
        bindings_of(self.rows(pane))
    }

    fn sections(&self) -> Vec<SectionSpec> {
        SECTIONS.to_vec()
    }
    fn setup_step(&self) -> Option<radar_core::plugin::SetupStep> {
        Some(setup::step(self))
    }
    fn setup_finds(&self) -> radar_core::plugin::SetupFinds {
        setup::finds()
    }
    fn section_hints(&self, section: &str) -> Vec<KeyHint<'static>> {
        let mut hints = settings::hints(self, section);
        if !self.recovery.records.is_empty() {
            hints.push(KeyHint::new("ctrl-r", "kept comments"));
        }
        hints
    }
    fn bulk(&self, scope: &str) -> Vec<MenuItem> {
        if scope == SCREEN.id && !self.pages.is_empty() {
            // The tree's bulk commands act on the tree, not on a page.
            vec![]
        } else if scope == SCREEN.id {
            screen::bulk(self)
        } else {
            settings::bulk(self, scope)
        }
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        if menu == AUTHOR_MENU {
            return self.pick_author(value);
        }
        if menu == radar_core::host::BULK_MENU {
            // One bulk menu id for both: the values do not overlap.
            let fx = match value {
                "open" | "close" | "browse" | "copy" | "refresh" => screen::on_bulk(self, value),
                _ => settings::on_bulk(self, value),
            };
            self.clamp();
            return fx;
        }
        if let Some(fx) = comment_recovery::on_menu(self, menu, value)
            .or_else(|| write::on_menu(self, menu, value))
            .or_else(|| pages::act::on_menu(self, menu, value))
        {
            return fx;
        }
        let fx = settings::on_menu(self, menu, value);
        self.clamp();
        fx
    }

    fn cli(&self) -> Option<clap::Command> {
        Some(
            clap::Command::new("gitlab")
                .about("Merge requests")
                .subcommand_required(true)
                .subcommand(
                    clap::Command::new("list")
                        .about("Print a view's merge requests")
                        .arg(
                            clap::Arg::new("view")
                                .long("view")
                                .help("View name (default: the first)"),
                        )
                        .arg(
                            clap::Arg::new("filter")
                                .long("filter")
                                .help("Extra filter, same grammar as / in the TUI"),
                        )
                        .arg(
                            clap::Arg::new("json")
                                .long("json")
                                .action(clap::ArgAction::SetTrue)
                                .help("JSON output"),
                        ),
                )
                .subcommand(
                    clap::Command::new("show")
                        .about("Print one merge request: !263, or repo!263 when several repos have one")
                        .arg(clap::Arg::new("id").required(true).help("!iid, or repo!iid"))
                        .arg(
                            clap::Arg::new("json")
                                .long("json")
                                .action(clap::ArgAction::SetTrue)
                                .help("JSON output"),
                        ),
                ),
        )
    }

    fn run_cli(&self, matches: &clap::ArgMatches) -> CliTask {
        let cfg = self.cfg.clone();
        let provider = self.provider();
        let m = matches
            .subcommand()
            .map(|(name, m)| (name.to_string(), m.clone()));
        Box::pin(async move {
            let (name, m) = m.ok_or_else(|| anyhow::anyhow!("unknown gitlab command"))?;
            let mut p = GitlabPlugin {
                cfg,
                ..GitlabPlugin::new()
            };
            let show = name == "show";
            if show {
                // Merged and closed ones too: it is asked for by number.
                p.filter = "state:all".into();
            } else {
                if let Some(view) = m.get_one::<String>("view") {
                    p.view = p
                        .cfg
                        .views
                        .iter()
                        .position(|v| v.name.eq_ignore_ascii_case(view))
                        .ok_or_else(|| anyhow::anyhow!("no view named {view}"))?;
                }
                p.filter = m.get_one::<String>("filter").cloned().unwrap_or_default();
            }
            for (group, want) in sync::plan(&p, true) {
                let got = provider.run(|c| {
                    let (group, want) = (group.clone(), want.clone());
                    async move { c.sync(&group, &want).await }
                })
                .await?;
                p.me = got.me.clone();
                let data = sync::merge(None, &want, got, p.now, p.cfg.history_days);
                p.data.insert(group, data);
            }
            let items = p.visible();
            if show {
                let id = m.get_one::<String>("id").cloned().unwrap_or_default();
                let found = cli_find(&items, &id)?;
                return Ok(if m.get_flag("json") {
                    serde_json::to_string_pretty(&cli_json(found))? + "\n"
                } else {
                    cli_text(found)
                });
            }
            if m.get_flag("json") {
                let rows: Vec<Value> = items.iter().map(cli_json).collect();
                return Ok(serde_json::to_string_pretty(&rows)? + "\n");
            }
            Ok(items
                .iter()
                .map(|it| {
                    format!(
                        "{:<4} {:<26} !{:<5} {:<8} {}\n",
                        it.tenant.code,
                        it.mr.project_name,
                        it.mr.iid,
                        it.word(),
                        it.mr.title
                    )
                })
                .collect())
        })
    }
}

/// The merge request `id` names: `263`, `!263`, or `repo!263` (the repo's
/// name or its path's tail) when several repos have a !263.
fn cli_find<'a>(items: &'a [Item<'a>], id: &str) -> anyhow::Result<&'a Item<'a>> {
    let (repo, iid) = id.rsplit_once('!').unwrap_or(("", id));
    let iid: u64 = iid
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("not a merge request: {id} (try !263 or repo!263)"))?;
    let found: Vec<&Item<'_>> = items
        .iter()
        .filter(|it| it.mr.iid == iid)
        .filter(|it| {
            repo.is_empty() || it.mr.project_name == repo || it.mr.project_path.ends_with(repo)
        })
        .collect();
    match found.as_slice() {
        [one] => Ok(one),
        [] => anyhow::bail!("no merge request {id} in the tracked repos"),
        many => anyhow::bail!(
            "several repos have !{iid}: {}; say which, e.g. {}!{iid}",
            many.iter()
                .map(|it| it.mr.project_name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            many[0].mr.project_name
        ),
    }
}

/// A person in the output of every part: `{name, handle}`.
fn cli_person(p: &model::Person) -> Value {
    json!({ "name": p.name, "handle": p.username })
}

/// One merge request as JSON, the shape every part's `--json` has: the time
/// under `updated` (RFC 3339), people as `{name, handle}`, and its `url`.
fn cli_json(it: &Item<'_>) -> Value {
    let updated = time::parse_rfc3339(&it.mr.updated_at)
        .map(time::to_rfc3339)
        .unwrap_or_else(|| it.mr.updated_at.clone());
    json!({
        "tenant": it.tenant.code, "repo": it.mr.project_name, "iid": it.mr.iid,
        "state": it.word(), "title": it.mr.title, "author": cli_person(&it.mr.author), "labels": it.mr.labels,
        "pipeline": it.mr.pipeline.map(|p| p.word()), "approvals": it.mr.approvals_words(),
        "merge": it.mr.merge_words(),
        "ready": ready::blocks(it.mr).is_empty() && it.mr.state == model::MrState::Opened,
        "in_the_way": ready::blocks(it.mr).iter().map(ready::Block::words).collect::<Vec<_>>(),
        "updated": updated, "url": it.mr.web_url,
    })
}

/// One merge request, for `radar gitlab show`.
fn cli_text(it: &Item<'_>) -> String {
    let mr = it.mr;
    let name = if mr.author.name.is_empty() {
        &mr.author.username
    } else {
        &mr.author.name
    };
    let mut out = format!("{} !{}  {}\n", mr.project_path, mr.iid, mr.title);
    out += &format!(
        "{} ({}) · {} → {}\n",
        name, mr.author.username, mr.source_branch, mr.target_branch
    );
    out += &format!(
        "{} · pipeline {} · approvals {} · {}\n",
        it.word(),
        mr.pipeline.map_or("none", |p| p.word()),
        mr.approvals_words(),
        mr.merge_words()
    );
    for b in ready::blocks(mr) {
        out += &format!("  in the way: {}\n", b.words());
    }
    if !mr.labels.is_empty() {
        out += &format!("labels: {}\n", mr.labels.join(", "));
    }
    out += &format!("updated {}\n{}\n", mr.updated_at, mr.web_url);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use model::fixtures::mr;

    pub use crate::demo::sample;

    /// What shows, as view text (`radar_core::headless::text`): the screen's
    /// panes and rows, the open menu and the status line. No painted frame.
    pub(crate) fn shown(host: &radar_core::Host, width: u16, height: u16) -> String {
        radar_core::headless::text_of(host, width, height)
    }

    /// The part's words on the right of the top bar: what the text form
    /// leaves out of the screen.
    pub(crate) fn top_bar_words(host: &radar_core::Host) -> Option<String> {
        host.chrome_view(160, 40).top_bar.words.map(|badge| badge.text)
    }

    /// The targets of the state cells the tree offers: the cells a press does
    /// something on.
    pub(crate) fn state_targets(p: &GitlabPlugin) -> Vec<String> {
        use radar_core::view::{Node, PaneId, TreeRow};
        fn walk(rows: &[TreeRow], out: &mut Vec<String>) {
            for row in rows {
                out.extend(row.cells.iter().filter_map(|c| c.target.as_ref()).map(|t| t.as_str().to_string()).filter(|t| t.starts_with("state:")));
                walk(&row.children, out);
            }
        }
        let vcx = radar_core::view::ViewCx {
            now: p.now,
            focus: PaneId::Middle,
            width: 160,
            height: 40,
            parts: radar_core::Parts::none(),
            icon_lead: 0,
            setup: false,
        };
        let view = Plugin::view(p, "repos", &vcx).expect("the screen has a view");
        let Node::Tree(tree) = &view.pane(PaneId::Middle).expect("the middle pane").body else { panic!("the tree") };
        let mut out = Vec::new();
        walk(&tree.rows, &mut out);
        out
    }

    /// The tone (`Ink`, as the view names it) of the first run of text in
    /// the repositories screen's view that holds `words`: what the text
    /// form leaves out.
    pub(crate) fn ink_of(host: &radar_core::Host, words: &str) -> Option<String> {
        fn find(value: &serde_json::Value, words: &str) -> Option<String> {
            match value {
                serde_json::Value::Object(map) => {
                    if map.get("text").and_then(|t| t.as_str()).is_some_and(|t| t.contains(words)) {
                        return Some(map.get("ink").and_then(|i| i.as_str()).unwrap_or("plain").to_string());
                    }
                    map.values().find_map(|v| find(v, words))
                }
                serde_json::Value::Array(items) => items.iter().find_map(|v| find(v, words)),
                _ => None,
            }
        }
        let view = host.screen_view(0, SCREEN.id)?;
        find(&serde_json::to_value(view).ok()?, words)
    }

    /// The screen `screen` of the part alone (no host), as view text, with
    /// the middle pane focused.
    pub(crate) fn view_text(p: &GitlabPlugin, screen: &str, width: u16, height: u16) -> String {
        view_text_in(p, screen, Pane::Middle, width, height)
    }

    /// [`view_text`] with `pane` focused.
    pub(crate) fn view_text_in(p: &GitlabPlugin, screen: &str, pane: Pane, width: u16, height: u16) -> String {
        let vcx = radar_core::view::ViewCx {
            now: p.now,
            focus: pane.into(),
            width,
            height,
            parts: radar_core::Parts::none(),
            icon_lead: 0,
            setup: false,
        };
        let view = Plugin::view(p, screen, &vcx).expect("the screen has a view");
        radar_core::headless::text::screen(&view, Some(pane.into()))
    }

    #[test]
    fn journal_subjects_keep_owned_server_identity_without_labels_or_fields() {
        let p = sample();
        for (kind, id, path, stored_id) in [
            ("repo", "acme/api", "acme/api", "acme/api"),
            (
                "merge-request",
                "acme/api!12",
                "acme/api/-/merge_requests/12",
                "acme/api!12",
            ),
            (
                "pipeline",
                "acme/api#34",
                "acme/api/-/pipelines/34",
                "acme/api/34",
            ),
            ("job", "acme/api#56", "acme/api/-/jobs/56", "acme/api/56"),
        ] {
            let subject = radar_core::Subject::new(kind, id, "private title")
                .with("description", "private content")
                .with_url(format!("https://gitlab.com/{path}?token=secret#secret"));
            let snapshot = p.journal_subject(&subject).unwrap();
            assert_eq!(snapshot.key.instance, "https://gitlab.com/");
            assert_eq!(snapshot.key.id, stored_id);
            let stored = format!("{snapshot:?}");
            assert!(!stored.contains("private") && !stored.contains("secret"));
            let foreign = subject.with_url(format!("https://other.example/{path}"));
            assert!(p.journal_subject(&foreign).is_none());
        }
        assert!(
            p.journal_subject(&radar_core::Subject::new("ticket", "ABC-12", "x"))
                .is_none()
        );
        assert!(
            p.journal_subject(&radar_core::Subject::new("repo", "acme/api", "x"))
                .is_none()
        );
    }

    pub fn iids(p: &GitlabPlugin) -> Vec<u64> {
        p.visible().iter().map(|i| i.mr.iid).collect()
    }

    #[test]
    fn json_has_the_shape_every_part_has_and_show_finds_a_merge_request_by_number() {
        let p = sample();
        let items = p.visible();
        let one = cli_find(&items, "!263").unwrap();
        let j = cli_json(one);
        assert!(j["author"]["handle"].is_string() && j["author"]["name"].is_string());
        assert!(j["url"].is_string());
        assert!(
            time::parse_rfc3339(j["updated"].as_str().unwrap()).is_some(),
            "RFC 3339: {}",
            j["updated"]
        );
        assert!(j.get("updated_at").is_none());
        assert_eq!(cli_find(&items, "263").unwrap().mr.iid, 263);
        assert_eq!(cli_find(&items, "api!263").unwrap().mr.iid, 263);
        assert!(cli_find(&items, "web!263").is_err(), "not in that repo");
        assert!(cli_find(&items, "nope").is_err());
        assert!(cli_text(one).contains("!263"));
    }

    #[test]
    fn the_rail_counts_only_what_waits_on_you_and_the_tab_badges_it() {
        let p = sample();
        assert!(screen::to_review(&p) > 0);
        assert_eq!(
            radar_core::Plugin::badge(&p, "repos").map(|b| b.0),
            Some(screen::to_review(&p).to_string())
        );
        let text = crate::tests::shown(&hosted(), 160, 40);
        // A nav row is its view's words and count, then its target.
        let row = |target: &str| {
            let line = text.lines().find(|l| l.ends_with(target)).unwrap();
            line.strip_suffix(target).unwrap().trim_end().to_string()
        };
        assert!(row("{view:1}").ends_with("Needs my action  2"), "{}", row("{view:1}"));
        assert!(row("{view:3}").ends_with("Opened by teammates"), "{}", row("{view:3}"));
        assert!(row("{view:0}").ends_with("Everything"), "{}", row("{view:0}"));
    }

    #[test]
    fn needs_my_action_shows_review_then_ready_and_hides_hidden_repos_and_bots() {
        let mut p = sample();
        p.view = p
            .cfg
            .views
            .iter()
            .position(|v| v.name == "Needs my action")
            .unwrap();
        assert_eq!(iids(&p), vec![263, 187]);
        // Everything, Needs my action, Opened by me, by teammates, by bots.
        assert_eq!(p.view_counts(), vec![4, 2, 2, 1, 1]);
    }

    #[test]
    fn tick_spawns_one_task_per_tenancy_and_waits_for_them() {
        let mut p = sample();
        let fx = p.tick(p.now);
        assert_eq!(fx.len(), 2);
        assert!(fx.iter().all(|e| matches!(e, Effect::SpawnAction { .. })));
        assert!(
            p.tick(p.now + 1000).is_empty(),
            "nothing new while in flight"
        );
        p.on_msg(Box::new(GlMsg::Loaded {
            group: "acme/team-b".into(),
            want: Want::default(),
            action: sync_history::action(&p.cfg.url, "acme/team-b", &Want::default()),
            result: Err("boom".into()),
            net: Default::default(),
        }));
        let fx = p.on_msg(Box::new(GlMsg::Loaded {
            group: "acme/team-a".into(),
            action: sync_history::action(&p.cfg.url, "acme/team-a", &Want::default()),
            want: Want {
                projects: true,
                ..Default::default()
            },
            result: Ok(Synced {
                me: "me".into(),
                ..Default::default()
            }),
            net: Default::default(),
        }));
        assert_eq!(p.sync.in_flight, 0);
        assert!(
            !fx.iter().any(|e| matches!(e, Effect::PersistCache)),
            "no cache write while a tenancy failed"
        );
        assert_eq!(p.sync.errors.len(), 1);
        assert!(p.sync.full_done.contains("acme/team-a"));
        assert!(
            !p.sync.full_done.contains("acme/team-b"),
            "failed: full again next time"
        );
        assert_eq!(p.sync.projects_at.get("acme/team-a"), Some(&p.now));
    }

    #[test]
    fn a_sync_says_which_merge_requests_changed_and_the_tree_marks_their_rows() {
        let a = "acme/team-a";
        let key = |iid| format!("merge-request:{a}/api!{iid}");
        let mut p = sample();
        let mut retitled = p.data[a].mrs.iter().find(|m| m.iid == 263).unwrap().clone();
        retitled.title = "ABC-12: A stats endpoint, now paged".into();
        let unchanged = p.data[a].mrs.iter().find(|m| m.iid == 187).unwrap().clone();
        let new = mr(400, "bob", &format!("{a}/api"));
        let since = || Want {
            changed_since: Some("2026-10-06T00:00:00Z".into()),
            ..Default::default()
        };
        let sync = |p: &mut GitlabPlugin, mrs: Vec<Mr>| {
            p.sync.in_flight = 1;
            p.on_msg(Box::new(GlMsg::Loaded {
                group: a.into(),
                want: since(),
                action: sync_history::action(&p.cfg.url, a, &since()),
                result: Ok(Synced {
                    me: "me".into(),
                    mrs,
                    ..Default::default()
                }),
                net: Default::default(),
            }))
        };
        let fx = sync(&mut p, vec![retitled.clone(), unchanged, new.clone()]);
        let changes = fx.iter().find_map(|e| match e {
            Effect::Changed(c) => Some(c.clone()),
            _ => None,
        });
        let changes = changes.expect("the sync says what changed");
        assert_eq!(changes.changed.iter().collect::<Vec<_>>(), [&key(263)]);
        assert_eq!(changes.added.iter().collect::<Vec<_>>(), [&key(400)]);
        let fx = sync(&mut p, vec![retitled, new]);
        assert!(
            !fx.iter().any(|e| matches!(e, Effect::Changed(_))),
            "the same again changes nothing"
        );

        // The changed row is in the view; where its mark is drawn is the renderer's.
        let host = radar_core::Host::new(vec![Box::new(p)], 1_791_271_000).unwrap();
        let text = crate::tests::shown(&host, 160, 40);
        assert!(text.lines().any(|line| line.contains("!263 ABC-12")), "{text}");
    }

    #[test]
    fn a_view_of_merged_ones_shows_them_with_their_word_and_labels_filter() {
        let mut p = sample();
        assert!(!iids(&p).contains(&150), "open only by default");
        p.filter = "state:merged".into();
        assert_eq!(iids(&p), vec![150]);
        assert_eq!(p.visible()[0].word(), "merged");
        p.filter = "state:all label:backend".into();
        assert_eq!(iids(&p), vec![263, 150], "open ones first");
        p.filter = "label:backend".into();
        assert_eq!(iids(&p), vec![263]);
        assert!(p.labels().contains(&"performance".to_string()));
        assert!(!p.wants_ended());
        p.cfg.views[1].filter = "state:merged".into();
        assert!(p.wants_ended());
        assert_eq!(p.view_counts()[1], 1, "the nav pane counts merged ones too");
    }

    #[test]
    fn the_author_menu_lists_names_once_and_everyone_clears_it() {
        let mut p = sample();
        let menu = p.author_menu();
        let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels[0], "everyone");
        assert_eq!(labels[1], "me", "you first");
        assert!(labels.contains(&"Bob Example") && labels.contains(&"alice"));
        assert_eq!(
            labels.iter().filter(|l| **l == "me").count(),
            1,
            "deduped by name"
        );
        p.on_menu(AUTHOR_MENU, "alice");
        assert_eq!(iids(&p), vec![263]);
        let text = {
            let mut host = radar_core::Host::new(vec![Box::new(sample())], p.now).unwrap();
            host.load_config(Some("")).unwrap();
            let mut s = sample();
            s.author = Some("alice".into());
            host.plugins[0] = Box::new(s);
            crate::tests::shown(&host, 160, 40)
        };
        assert!(text.contains("OPENED BY :person: alice"), "{text}");
        p.on_menu(AUTHOR_MENU, "");
        assert_eq!(p.author, None);
        assert_eq!(iids(&p).len(), 4);
    }

    #[test]
    fn go_to_through_the_host_types_jumps_and_keeps() {
        use radar_core::host::Event;
        use radar_core::view::{Chord, Key};
        let mut host = hosted();
        let key = |c| crate::key_event(Chord::new(c));
        let _ = crate::tests::shown(&host, 160, 40);
        host.update(crate::key_event(Chord::char(':')));
        assert!(host.plugins[0].capturing(), "go to takes the keys");
        for c in "stats".chars() {
            host.update(key(Key::Char(c)));
        }
        let text = crate::tests::shown(&host, 160, 40);
        assert!(text.contains(":  [stats|]"), "{text}");
        assert!(text.contains("at api !263 ABC-12"), "{text}");
        host.update(Event::Paste(" endpoint".into()));
        host.update(key(Key::Enter));
        assert!(!host.plugins[0].capturing());
        let text = crate::tests::shown(&host, 160, 40);
        // The detail pane shows what the tree landed on.
        assert!(text.contains("api !263"), "{text}");
    }

    #[test]
    fn choosing_the_authors_heading_opens_the_author_menu() {
        use radar_core::host::Event;
        use radar_core::view::{Intent, Target};
        let mut host = hosted();
        // The tree's header line carries the target that opens the menu.
        let text = crate::tests::shown(&host, 160, 40);
        let line = text.lines().find(|l| l.contains("OPENED BY")).expect(&text);
        assert!(line.ends_with("{authors}"), "{line}");
        host.update(Event::Intent(Intent::Select { target: Target::new("authors") }));
        let open = host.menu.as_ref().expect("the author menu");
        assert_eq!(open.menu.id, AUTHOR_MENU);
        assert_eq!(open.menu.title, "show merge requests opened by");
    }

    #[test]
    fn config_round_trips_through_the_plugin() {
        let mut p = sample();
        let table = p.config().unwrap();
        p.cfg = Settings::default();
        p.configure(Some(&table)).unwrap();
        assert_eq!(p.cfg.sources.len(), 2);
        assert_eq!(p.cfg.sources[1].hide, vec!["tools/renovate"]);
    }

    #[test]
    fn declares_a_slot_per_tenancy_colour() {
        let names: Vec<String> = GitlabPlugin::new()
            .slots()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names.len(), config::TENANT_COLORS.len());
        assert!(names.contains(&"tenant.cyan".to_string()));
    }

    // --- the screen through the real host, as view text ---

    /// A context press selects the shown row for its menu; the row's target
    /// is in the view, once.
    #[test]
    fn tree_context_selection_requires_a_shown_row() {
        use radar_core::view::{Intent, Node, PaneId, Target, TreeRow};
        use radar_core::{Outcome, Pane, Plugin};

        let mut p = sample();
        let host = hosted();
        let text = crate::tests::shown(&host, 160, 40);
        let view = host.screen_view(0, SCREEN.id).expect("repositories screen");
        let Node::Tree(tree) = &view.pane(PaneId::Middle).expect("repositories pane").body else {
            panic!("repositories tree")
        };
        let tenancy = tree.rows.iter().find(|row| !row.children.is_empty()).expect("a tenancy with repos");
        let repo = &tenancy.children[0];
        let path = vec![tenancy.id.clone(), repo.id.clone()];
        assert!(text.contains(&format!("{} ", radar_core::headless::text::text(&repo.name))), "{text}");
        let target = TreeRow::target_of(&path);
        for _ in 0..2 {
            assert!(matches!(
                p.intent(SCREEN.id, Pane::Middle, &Intent::Context { target: target.clone() }),
                Outcome::Handled(_)
            ));
            assert_eq!(p.tree.borrow().selected(), path);
        }
        // A press on the pane's blank space is the host's own `pane:middle`
        // target: the pane is focused, no menu opens, the selection stays.
        let mut host = hosted();
        host.focus = Pane::Nav;
        let before = host.plugins[0].subject(SCREEN.id, Pane::Middle);
        host.update(radar_core::host::Event::Intent(Intent::Select { target: Target::new("pane:middle") }));
        assert_eq!(host.focus, Pane::Middle);
        assert!(host.menu.is_none(), "a blank press opens no menu");
        assert_eq!(host.plugins[0].subject(SCREEN.id, Pane::Middle), before);
    }

    fn hosted() -> radar_core::Host {
        let mut host = radar_core::Host::new(vec![Box::new(sample())], 1_791_271_000).unwrap();
        host.load_config(Some("")).unwrap();
        // load_config re-configures the plugin from the (empty) file; put the
        // sample back the way a loaded config would.
        host.plugins[0] = Box::new(sample());
        host
    }

    /// ↑ on the first row of each pane reaches the top bar; → changes the
    /// screen, ↓ comes back to that pane; inside a page ↑ stays.
    #[test]
    fn up_from_the_top_of_each_pane_reaches_the_bar_and_down_comes_back() {
        use radar_core::host::View;
        use radar_core::view::{Chord, Key};
        let press = |host: &mut radar_core::Host, code| {
            let _ = crate::tests::shown(host, 160, 40);
            host.update(crate::key_event(Chord::new(code)));
        };
        for pane in [Pane::Nav, Pane::Middle, Pane::Detail] {
            let mut host = hosted();
            host.focus = pane;
            press(&mut host, Key::Char('g'));
            press(&mut host, Key::Up);
            assert!(host.bar, "{pane:?}: ↑ at the top reaches the bar");
            press(&mut host, Key::Right);
            assert_eq!(host.view, View::Settings, "→ is the next screen");
            press(&mut host, Key::Left);
            press(&mut host, Key::Down);
            assert_eq!(
                (host.view, host.bar, host.focus),
                (View::Screen(0), false, pane),
                "back in {pane:?}"
            );
        }
        // Not from the second row: ↑ moves up a row.
        let mut host = hosted();
        host.focus = Pane::Middle;
        press(&mut host, Key::Char('j'));
        press(&mut host, Key::Up);
        assert!(!host.bar);
        // Inside a page ↑ at the top does nothing.
        press(&mut host, Key::Enter);
        assert_eq!(radar_core::Plugin::depth(host.plugins[0].as_ref(), "repos"), 1);
        press(&mut host, Key::Char('g'));
        press(&mut host, Key::Up);
        assert!(!host.bar, "inside a page");
    }

    #[test]
    fn choosing_out_of_and_back_into_a_pane_keeps_its_cursor() {
        use radar_core::host::Event;
        use radar_core::view::Chord;
        use radar_core::view::{Intent, Node, PaneId, Target, TreeRow};
        let mut host = hosted();
        let select = |host: &mut radar_core::Host, target: Target| {
            let _ = crate::tests::shown(host, 160, 40);
            host.update(Event::Intent(Intent::Select { target }));
        };
        let tree = |host: &radar_core::Host| {
            let view = host.screen_view(0, SCREEN.id).expect("repositories screen");
            let Node::Tree(tree) = &view.pane(PaneId::Middle).expect("repositories pane").body else {
                panic!("repositories tree")
            };
            tree.clone()
        };
        let _ = crate::tests::shown(&host, 160, 40);
        for _ in 0..2 {
            host.update(crate::key_event(Chord::char('j')));
        }
        let _ = crate::tests::shown(&host, 160, 40);
        let before = tree(&host).selected;
        assert!(before.len() > 1, "moved into the tree: {before:?}");
        select(&mut host, Target::new("pane:nav"));
        assert_eq!(host.focus, radar_core::Pane::Nav);
        select(&mut host, Target::new("pane:middle")); // back into the tree, below its rows
        assert_eq!(host.focus, radar_core::Pane::Middle);
        assert_eq!(
            tree(&host).selected,
            before,
            "empty space only focuses: the cursor stays"
        );
        select(&mut host, Target::new("pane:nav"));
        let first = TreeRow::target_of(&[tree(&host).rows[0].id.clone()]); // a different row, from another pane
        select(&mut host, first);
        assert_eq!(host.focus, radar_core::Pane::Middle);
        assert_ne!(
            tree(&host).selected,
            before,
            "a press on a row selects it at once"
        );
    }

    /// The first row of the tree, as the view holds it.
    fn first_row_target(host: &radar_core::Host) -> radar_core::view::Target {
        use radar_core::view::{Node, PaneId, TreeRow};
        let view = host.screen_view(0, SCREEN.id).expect("repositories screen");
        let Node::Tree(tree) = &view.pane(PaneId::Middle).expect("repositories pane").body else {
            panic!("repositories tree")
        };
        TreeRow::target_of(&[tree.rows[0].id.clone()])
    }

    #[test]
    fn context_on_a_tree_row_offers_that_rows_actions() {
        use radar_core::host::Event;
        use radar_core::view::Chord;
        use radar_core::view::Intent;

        let mut expected = hosted();
        let _ = crate::tests::shown(&expected, 160, 40);
        let target = first_row_target(&expected);
        expected.update(Event::Intent(Intent::Select { target: target.clone() }));
        expected.update(crate::key_event(Chord::char('x')));
        let expected_menu = expected
            .menu
            .as_ref()
            .expect("left selection and x menu")
            .menu
            .clone();

        let mut actual = hosted();
        let _ = crate::tests::shown(&actual, 160, 40);
        actual.update(Event::Intent(Intent::Context { target }));
        let menu = actual.menu.as_ref().expect("context menu");
        assert_eq!(menu.menu, expected_menu);
    }

    #[test]
    fn a_press_below_the_last_tree_row_only_focuses_and_a_row_still_opens_its_actions() {
        use radar_core::host::Event;
        use radar_core::view::{Intent, Target};

        let mut host = hosted();
        let _ = crate::tests::shown(&host, 160, 40);
        let row = first_row_target(&host);
        let before = host.plugins[0].keys(SCREEN.id, Pane::Middle);

        // Below the final row, inside the middle pane: the pane's own target.
        host.focus = Pane::Nav;
        for _ in 0..2 {
            host.update(Event::Intent(Intent::Select { target: Target::new("pane:middle") }));
            assert_eq!(host.focus, Pane::Middle);
            assert!(host.menu.is_none(), "empty space must not borrow a row's actions");
            assert_eq!(host.plugins[0].keys(SCREEN.id, Pane::Middle), before);
        }

        // It has not disabled subsequent real-row context presses.
        host.update(Event::Intent(Intent::Context { target: row.clone() }));
        assert!(host.menu.is_some());
        let menu = host.menu.as_ref().unwrap().menu.clone();
        let _ = crate::tests::shown(&host, 160, 40);
        host.update(Event::Intent(Intent::Context { target: row }));
        assert_eq!(host.menu.as_ref().unwrap().menu, menu);
    }

    #[test]
    fn a_right_press_below_the_last_row_offers_nothing() {
        use radar_core::host::Event;
        use radar_core::view::{Intent, Node, PaneId, Target};

        let selected = |host: &radar_core::Host| {
            let view = host.screen_view(0, SCREEN.id).expect("repositories screen");
            let Node::Tree(tree) = &view.pane(PaneId::Middle).expect("repositories pane").body else {
                panic!("repositories tree")
            };
            tree.selected.clone()
        };
        for focus in [Pane::Nav, Pane::Middle] {
            let mut host = hosted();
            let _ = crate::tests::shown(&host, 160, 40);
            let row = first_row_target(&host);
            host.update(Event::Intent(Intent::Select { target: row.clone() }));
            let _ = crate::tests::shown(&host, 160, 40);
            let (path, subject) = (selected(&host), host.plugins[0].subject(SCREEN.id, Pane::Middle));
            assert_eq!(path, vec![row.as_str().to_string()], "a real row is selected");
            host.focus = focus;

            // Below the final row, inside the middle pane: the pane's own target.
            for intent in [
                Intent::Context { target: Target::new("pane:middle") },
                Intent::Activate { target: Target::new("pane:middle") },
            ] {
                host.update(Event::Intent(intent));
                let _ = crate::tests::shown(&host, 160, 40);
                assert_eq!(host.focus, Pane::Middle);
                assert!(host.menu.is_none(), "empty space offers no actions; focus={focus:?}");
                assert_eq!(selected(&host), path, "the selection stays; focus={focus:?}");
                assert_eq!(host.plugins[0].subject(SCREEN.id, Pane::Middle), subject);
                host.focus = focus;
            }
        }
    }

    #[test]
    fn mr_actions_menu_folds_its_parent_without_expanding_or_acting_on_the_mr() {
        use radar_core::view::{Chord, Key};
        use radar_core::view::{Node, PaneId};

        let press = |host: &mut radar_core::Host, code| {
            let _ = crate::tests::shown(host, 160, 40);
            host.update(crate::key_event(Chord::new(code)))
        };
        let tree = |host: &radar_core::Host| {
            let view = host.screen_view(0, SCREEN.id).expect("repositories screen");
            let Node::Tree(tree) = &view.pane(PaneId::Middle).expect("repositories pane").body else {
                panic!("repositories tree")
            };
            tree.clone()
        };
        let mut host = hosted();
        for _ in 0..5 {
            press(&mut host, Key::Down);
        }
        let before = tree(&host);
        assert_eq!(before.selected.len(), 3, "a merge request under its repo and tenancy");
        assert!(before.row(&before.selected).unwrap().children.is_empty(), "an MR is a leaf");
        let subject = host.plugins[0].subject(SCREEN.id, Pane::Middle).expect("selected MR");
        assert!(subject.is("merge-request"));

        press(&mut host, Key::Char('x'));
        let menu = &host.menu.as_ref().expect("MR actions menu").menu;
        for key in ['+', '-', '='] {
            assert_eq!(menu.items.iter().filter(|item| item.key == Some(key)).count(), 1,
                "the shared tree action {key} appears once");
        }
        // The shared keys apply to this tree, but '+' must not invent MR children
        // or silently retarget the operation to its parent.
        assert!(press(&mut host, Key::Char('+')).is_empty());
        assert!(host.menu.is_none());
        assert_eq!(tree(&host), before);
        assert_eq!(host.plugins[0].subject(SCREEN.id, Pane::Middle), Some(subject));
        assert_eq!(host.plugins[0].depth(SCREEN.id), 0, "no MR page was opened");

        press(&mut host, Key::Char('x'));
        assert!(press(&mut host, Key::Char('-')).is_empty(), "folding sends no write or request");
        assert!(host.menu.is_none());
        let after = tree(&host);
        let parent = before.selected[..2].to_vec();
        assert_eq!(after.selected, parent);
        assert!(!after.open.contains(&parent));
        assert_eq!(after.rows, before.rows, "the held rows and their action targets are unchanged");
        assert_eq!(host.plugins[0].depth(SCREEN.id), 0);
    }

    fn section(host: &mut radar_core::Host, name: &str) {
        host.view = radar_core::host::View::Settings;
        host.section = host
            .sections
            .iter()
            .position(|(_, s)| s.name == name)
            .unwrap();
    }

    #[test]
    fn merge_requests() {
        let host = hosted();
        // The sync words are only on the top bar, which the text form leaves out.
        assert_eq!(top_bar_words(&host).as_deref(), Some("not synced yet"));
        // So is the part's chip at the right of the status line.
        let chips: Vec<String> = host.chrome_view(160, 40).status.health.into_iter().map(|c| c.part).collect();
        assert_eq!(chips, ["gitlab"]);
        insta::assert_snapshot!(crate::tests::shown(&host, 160, 40));
    }

    #[test]
    fn settings_tenancies() {
        let mut host = hosted();
        section(&mut host, "tenancies");
        insta::assert_snapshot!(crate::tests::shown(&host, 160, 40));
    }

    #[test]
    fn settings_connection() {
        let mut host = hosted();
        section(&mut host, "connection");
        insta::assert_snapshot!(crate::tests::shown(&host, 160, 40));
    }

    /// With a token found in the environment: asked about first; its
    /// detail says what was found and what radar keeps of it.
    #[test]
    fn settings_connection_found() {
        let mut host = radar_core::Host::new(vec![Box::new(sample())], 1_791_271_000).unwrap();
        host.load_config(Some("")).unwrap();
        let mut p = sample();
        p.cfg.token = "glab".into();
        p.env = Some(vec![(
            "GITLAB_TOKEN".into(),
            "glpat-not.a.real.token.1234".into(),
        )]);
        p.me.clear();
        host.plugins[0] = Box::new(p);
        section(&mut host, "connection");
        insta::assert_snapshot!(crate::tests::shown(&host, 160, 40));
    }

    #[test]
    fn settings_look() {
        let mut host = hosted();
        section(&mut host, "look");
        insta::assert_snapshot!(crate::tests::shown(&host, 160, 40));
    }

    #[test]
    fn look_and_views_settings_are_neutral_and_route_selection() {
        let mut p = sample();
        let look = settings::look_view(&p);
        let radar_core::view::Node::List { rows, selected, .. } = &look.body else { panic!("look is a list") };
        assert!(rows[0].heading);
        assert_eq!(*selected, Some(1));
        let down = radar_core::route::section_key(&look, radar_core::view::Chord::new(radar_core::view::Key::Down));
        settings::list_intent(&mut p, "look", &down);
        assert_eq!(p.section_rows[3], 2);
        let views = settings::views_view(&p, 100);
        assert_eq!(views.scope, "views");
        assert!(serde_json::to_string(&views).unwrap().contains("gitlab.views.kept_comments"));
        settings::list_intent(&mut p, "views", &radar_core::view::Intent::Activate { target: radar_core::view::Target::new("row:0") });
        let radar_core::view::Node::List { editing: Some(editing), .. } = settings::views_view(&p, 100).body else { panic!("editor in view") };
        assert_eq!(editing.row, 0);
    }

    #[test]
    fn connection_and_tenancies_settings_use_neutral_views() {
        let mut p = sample();
        let connection = settings::connection_view(&p, 100, 1_791_271_000);
        let radar_core::view::Node::List { rows, .. } = &connection.body else { panic!("connection is a list") };
        assert_eq!(rows.len(), settings::conn_rows(&p).len());
        assert!(!connection.detail.is_empty());
        let tenancies = settings::tenancies_view(&p, 100, 1_791_271_000);
        let radar_core::view::Node::Tree(tree) = &tenancies.body else { panic!("tenancies is a tree") };
        assert_eq!(tree.rows[0].id, "add");
        assert!(tree.rows.iter().any(|row| !row.children.is_empty()));
        assert_eq!(tree.look.as_ref().unwrap().heading, "bg.overlay");
        let source = p.cfg.sources[0].group.clone();
        settings::tenancies_intent(&mut p, &radar_core::view::Intent::Fold {
            target: radar_core::view::TreeRow::target_of(&[tree.rows[1].id.clone()]), open: false,
        });
        assert!(p.folded.contains(&source));
    }

    #[test]
    fn settings_colours_lists_this_plugins_slots() {
        let mut host = hosted();
        section(&mut host, "colours");
        let last = host.colours.rows().len() - 1;
        host.colours.row = last;
        let text = crate::tests::shown(&host, 160, 40);
        assert!(text.contains("tenant"), "{text}");
    }

    #[test]
    fn keys_menu() {
        use radar_core::view::Chord;
        let mut host = hosted();
        // As the app does: a frame first, which selects the first row.
        let _ = crate::tests::shown(&host, 160, 40);
        host.update(crate::key_event(Chord::char('x')));
        insta::assert_snapshot!(crate::tests::shown(&host, 160, 40));
    }

    /// The text of the row of the shown settings section that starts with
    /// `row`, as its view holds it (a list's rows, or a tree's names).
    fn section_row_text(host: &radar_core::Host, row: &str) -> Option<String> {
        use radar_core::view::{Node, TreeRow};
        let view = radar_core::headless::section_of(host, 160, 40)?;
        let words = |t: &radar_core::view::Text| radar_core::headless::text::text(t);
        match &view.body {
            Node::List { rows, .. } => {
                rows.iter().map(|r| words(&r.text)).find(|t| t.trim_start().starts_with(row))
            }
            Node::Tree(tree) => {
                fn names(rows: &[TreeRow], out: &mut Vec<String>) {
                    for r in rows {
                        out.push(radar_core::headless::text::text(&r.name));
                        names(&r.children, out);
                    }
                }
                let mut all = Vec::new();
                names(&tree.rows, &mut all);
                all.into_iter().find(|t| t.trim_start().starts_with(row))
            }
            _ => None,
        }
    }

    /// Open a settings field's input from its row, empty it and type `w`:
    /// the view's input starts at the cell where the value was, and holds
    /// only the `w`. (A tenancy's label once kept its first letter in front
    /// of the input: drawn, not typed, so it could not be deleted and ⏎
    /// dropped it.) `row` picks the line, `value` the text on it where the
    /// input starts. The old value is not drawn under the input: that is
    /// ink/test/nodes/list-editing-at.test.tsx.
    fn input_sits_on_its_value(
        host: &mut radar_core::Host,
        name: &str,
        downs: usize,
        open: &[Key],
        row: &str,
        value: &str,
    ) {
        use radar_core::view::{Chord, Key};
        use radar_core::view::Node;
        let press = |host: &mut radar_core::Host, chord: Chord| {
            let _ = crate::tests::shown(host, 160, 40);
            host.update(crate::key_event(chord))
        };
        section(host, name);
        host.focus = radar_core::Pane::Middle;
        for _ in 0..20 {
            press(host, Chord::new(Key::Up));
        }
        for _ in 0..downs {
            press(host, Chord::new(Key::Down));
        }
        let before = crate::tests::shown(host, 160, 40);
        let text = section_row_text(host, row)
            .unwrap_or_else(|| panic!("no row {row:?} in {name}:\n{before}"));
        let start = text.find(row).unwrap();
        let c = match text[start..].find(value) {
            Some(at) => radar_core::ui::width(&text[..start + at]),
            None => panic!("no {value:?} on {text:?}"),
        };
        for code in open {
            press(host, Chord::new(*code));
        }
        assert!(
            host.plugins[0].capturing(),
            "{name}: {open:?} on {row:?} opened no input"
        );
        press(host, Chord::ctrl('e'));
        press(host, Chord::ctrl('u'));
        // w is text here, not the save key.
        let fx = press(host, Chord::char('w'));
        assert!(
            !fx.iter()
                .any(|e| matches!(e, radar_core::host::HostEffect::Save(_))),
            "{name}: w saved instead of being typed"
        );
        let _ = crate::tests::shown(host, 160, 40);
        let view = radar_core::headless::section_of(host, 160, 40).expect("the section");
        let (typed, at, mark) = match &view.body {
            // A list row's mark takes one cell before its text.
            Node::List { editing: Some(e), .. } => (e.text.clone(), usize::from(e.at), 1),
            Node::Tree(tree) => match tree.editing.as_deref() {
                Some(e) => (e.text.clone(), usize::from(e.at), 0),
                None => panic!("{name}, {row:?}: the view has no input"),
            },
            _ => panic!("{name}, {row:?}: the view has no input"),
        };
        assert_eq!(
            (typed.as_str(), at),
            ("w", c + mark),
            "{name}, {row:?}: the input does not start where {value:?} was in {text:?}"
        );
        press(host, Chord::new(Key::Esc));
        assert!(!host.plugins[0].capturing());
    }

    #[test]
    fn every_settings_input_starts_where_its_value_was_drawn() {
        use Key::{Char, Enter};
        let mut host = hosted();
        // connection: server, base group, token, refresh, history
        // (signed in as and check have no input).
        // The first few letters after a row's name: where its value starts.
        let after = |name: &str, row: &str| {
            let mut h = hosted();
            section(&mut h, name);
            let t = section_row_text(&h, row).unwrap();
            let rest = &t[t.find(row).unwrap() + row.len()..];
            rest.trim_start().chars().take(3).collect::<String>()
        };
        for (n, label) in [
            (0, "server"),
            (1, "base group"),
            (2, "token"),
            (5, "refresh"),
            (6, "merged, closed"),
        ] {
            let row = format!("{label} ");
            let value = after("connection", &row);
            input_sits_on_its_value(&mut host, "connection", n, &[Char('e')], &row, &value);
        }
        // tenancies: a tenancy's label and its path; the add row.
        input_sits_on_its_value(&mut host, "tenancies", 1, &[Char('t')], "b ", "b ");
        input_sits_on_its_value(&mut host, "tenancies", 1, &[Char('e')], "b ", "team-b");
        input_sits_on_its_value(
            &mut host,
            "tenancies",
            0,
            &[Enter],
            "+ add a tenancy",
            "add a tenancy",
        );
        // views: its name, its filter as text, words to add.
        input_sits_on_its_value(
            &mut host,
            "views",
            1,
            &[Char('r')],
            "Everything",
            "Everything",
        );
        let shows = after("views", "Needs my action");
        input_sits_on_its_value(
            &mut host,
            "views",
            2,
            &[Char('t')],
            "Needs my action",
            &shows,
        );
        let shows = after("views", "Everything");
        input_sits_on_its_value(
            &mut host,
            "views",
            1,
            &[Char('a'), Char('w')],
            "Everything",
            &shows,
        );
        input_sits_on_its_value(&mut host, "views", 0, &[Enter], "+ new view", "new view");
    }

    /// Every `x` row GitLab offers is GitLab's to take and it takes it, and
    /// every click target it draws acts (radar_core::check, on the plugin
    /// alone): the nav pane's rows, the middle pane's tenancies, repos and merge
    /// requests and their detail pane, each page's tabs and the pages they
    /// open (a failed job, a repo's pipelines), and each settings section's
    /// first rows. Rows other parts add are checked in the part that adds
    /// them; the host's routing of them in radar-core.
    #[test]
    fn every_row_and_target_is_taken() {
        use radar_core::check::{Check, Place};
        use radar_core::view::Chord;
        let tree = || Place::screen("repos", Pane::Middle);
        let mut places = Vec::new();
        for down in 0..6 {
            places.push(Place::screen("repos", Pane::Nav).down(down));
            places.push(tree().down(down));
            places.push(tree().down(down).focus(Pane::Detail));
            for t in 0..5 {
                places.push(tree().down(down).enter().tab(t));
            }
            // The Changes tab's files pane, beside the diff.
            places.push(tree().down(down).enter().tab(1).focus(Pane::Nav));
            places.push(tree().down(down).enter().enter());
            places.push(tree().down(down).enter().enter().tab(1));
        }
        // !342's failed job, and a search in its log; web's pipelines.
        let job = tree().down(2).enter().tab(3).enter();
        places.push(job.clone());
        places.push(job.press(Chord::char('/'), 1).typed("test").enter());
        places.push(tree().down(6).enter().tab(1));
        places.push(tree().down(6).enter().tab(1).enter());
        let sections: Vec<&'static str> = sample().sections().iter().map(|s| s.id).collect();
        for id in sections {
            for row in 0..7 {
                places.push(Place::section(id).down(row));
            }
        }
        let make = || Box::new(sample()) as Box<dyn Plugin>;
        let mut check = Check::new(&make);
        let gaps = check.places(&places);
        assert!(gaps.is_empty(), "{} gaps:\n{}", gaps.len(), gaps.join("\n"));
        assert!(check.checked > 400, "only {} checked", check.checked);
    }

    /// The sample with !263's page data and its files' diffs already held,
    /// as after they arrived: its review rows have something to act on.
    pub(crate) fn sample_reviewed() -> GitlabPlugin {
        let mut p = sample();
        let mr = p
            .data
            .values()
            .flat_map(|d| d.mrs.iter())
            .find(|m| m.iid == 263)
            .unwrap()
            .clone();
        let key = pages::mr_key("acme/team-a/api", 263);
        let detail = model::fixtures::detail();
        let head = detail.refs.as_ref().unwrap().head.clone();
        p.mr_pages.insert(
            key.clone(),
            pages::Fetched {
                value: Some((mr, detail)),
                at: p.now,
                ..Default::default()
            },
        );
        p.diffs.insert(
            key,
            pages::changes::Diffs {
                head,
                files: model::fixtures::diffs()
                    .into_iter()
                    .map(|f| (f.new_path.clone(), f))
                    .collect(),
                asked: [1].into(),
                ended: Some(1),
                ..Default::default()
            },
        );
        p
    }

    /// The review's places on !263's page: each tab, a line of the diff
    /// with a thread under it and one without, the Threads tab's comment
    /// row and a thread.
    #[test]
    fn every_row_and_target_of_the_review_is_taken() {
        use radar_core::check::{Check, Place};
        use radar_core::view::Chord;
        let page = || Place::screen("repos", Pane::Middle).down(5).enter();
        let mut places: Vec<Place> = (0..5).map(|t| page().tab(t)).collect();
        places.push(page().tab(1).down(5));
        places.push(page().tab(1).down(21));
        places.push(page().tab(1).press(Chord::char('n'), 1));
        places.push(page().tab(2).down(1));
        places.push(page().tab(2).down(2));
        let make = || Box::new(sample_reviewed()) as Box<dyn Plugin>;
        let mut check = Check::new(&make);
        let gaps = check.places(&places);
        assert!(gaps.is_empty(), "{} gaps:\n{}", gaps.len(), gaps.join("\n"));
        assert!(check.checked > 35, "only {} checked", check.checked);
    }

    #[test]
    fn choosing_a_row_in_the_keys_menu_presses_its_key() {
        use radar_core::view::{Chord, Key};
        let mut host = hosted();
        let press = |host: &mut radar_core::Host, c| {
            host.update(crate::key_event(Chord::new(c)));
        };
        press(&mut host, Key::Char('x'));
        // `/` in the menu runs "filter this view": the filter input opens.
        press(&mut host, Key::Char('/'));
        assert!(host.menu.is_none());
        assert!(host.plugins[0].capturing());
    }

    #[test]
    fn h_and_l_move_between_panes_and_esc_comes_back() {
        use radar_core::Pane;
        use radar_core::view::{Chord, Key};
        let mut host = hosted();
        let press = |host: &mut radar_core::Host, c| {
            host.update(crate::key_event(Chord::new(c)));
        };
        assert_eq!(host.focus, Pane::Middle);
        press(&mut host, Key::Char('l'));
        assert_eq!(host.focus, Pane::Detail);
        // Past the right edge: the next screen (settings), and back.
        press(&mut host, Key::Char('l'));
        assert_eq!(
            (host.view, host.focus),
            (radar_core::host::View::Settings, Pane::Nav)
        );
        press(&mut host, Key::Char('h'));
        assert_eq!(host.focus, Pane::Detail);
        press(&mut host, Key::Left);
        press(&mut host, Key::Char('h'));
        assert_eq!(host.focus, Pane::Nav);
        press(&mut host, Key::Char('3'));
        assert_eq!(host.focus, Pane::Detail);
        press(&mut host, Key::Esc);
        assert_eq!(host.focus, Pane::Middle);
    }
}
