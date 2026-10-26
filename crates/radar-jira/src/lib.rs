//! radar-jira: Jira tickets across sites, as a radar plugin.
//!
//! Contributes the `jira` screen (Shift+J): a nav pane of views in words │ a
//! tree of site → status → tickets │ a detail pane; `⏎` opens a ticket's page
//! (Overview, Comments, Links, and Code when a merge request is linked).
//! Changes, each asked first: `t` move, `c` comment, `n` new ticket, `i`
//! assign to me (`write.rs`). It owns `ticket` subjects and adds chips, a
//! section and actions to merge requests (`join.rs`). Settings sections
//! `connection`, `projects`, `views` and `look`; the `[jira]`
//! config table; the `jira.*` colour slots; a cache; `radar jira list` and
//! `radar jira show`.

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

mod palette;
mod source_navigation;
mod adf;
mod agent_context;
mod client;
mod code_history;
mod comments_history;
mod config;
mod health;
mod join;
mod model;
mod page;
mod page_history;
mod rate;
mod screen;
mod settings;
mod setup;
mod settings_checks;
mod settings_history;
mod start;
mod subject_scope;
mod sync_history;
mod view;
mod write;
mod write_context;
mod write_dialog;
mod write_recovery;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashSet};

use radar_core::design::help_bar::KeyHint;
use radar_core::plugin::{
    Binding, CliTask, Effect, MenuItem, Msg, Outcome, Pane, ScreenSpec, SectionSpec, SlotValue,
};
use radar_core::view::{Chord, Key};
use radar_core::design::theme::Color;
use radar_core::{Plugin, log, time};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use client::Client;
pub use config::Settings;
use config::Site;
use model::{Detail, Me, Ticket};

const SCREEN: ScreenSpec = ScreenSpec {
    id: "tickets",
    name: "jira",
    key: 'J',
};

const SECTIONS: [SectionSpec; 4] = [
    SectionSpec {
        // The sites: every part's first section is its connection.
        id: "connection",
        name: radar_core::ui::connection::SECTION,
        group: "jira",
    },
    SectionSpec {
        id: "projects",
        name: "projects",
        group: "jira",
    },
    SectionSpec {
        id: "views",
        name: "views",
        group: "jira",
    },
    SectionSpec {
        id: "look",
        name: "look",
        group: "jira",
    },
];

/// A section's id as it is now: the ids before sections were named alike
/// (`sites`, `jira-views`, `jira-look`) still find their section.
fn canon(id: &str) -> &str {
    match id {
        "sites" => "connection",
        "jira-views" => "views",
        "jira-look" => "look",
        other => other,
    }
}

/// Default site colours (SilkCircuit Neon, with 256 and 16 fallbacks),
/// overridable as `jira.<name>` in `[theme.slots]`: name, truecolor,
/// xterm-256 index, ANSI-16 colour.
type SiteSlot = (&'static str, (u8, u8, u8), u8, Color);

const SITE_SLOTS: [SiteSlot; 7] = [
    ("blue", (0x82, 0xaa, 0xff), 111, Color::Blue),
    ("cyan", (0x80, 0xff, 0xea), 122, Color::Cyan),
    ("green", (0x50, 0xfa, 0x7b), 84, Color::Green),
    ("magenta", (0xff, 0x6a, 0xc1), 205, Color::Magenta),
    ("purple", (0xe1, 0x35, 0xff), 171, Color::LightMagenta),
    ("yellow", (0xf1, 0xfa, 0x8c), 228, Color::Yellow),
    ("orange", (0xff, 0xb8, 0x6c), 215, Color::LightRed),
];

/// The key that jumps to a ticket by its key or words.
pub(crate) const GOTO_KEY: char = ':';
/// The key that picks whose tickets are shown.
pub(crate) const ASSIGNEE_KEY: char = 'a';
pub(crate) const ASSIGNEE_MENU: &str = "assignee";
/// The value the assignee menu gives for tickets nobody has.
pub(crate) const NOBODY: &str = "\u{0}nobody";

pub(crate) enum JiraMsg {
    /// A view's tickets from one site (and who the token is, when asked).
    Loaded {
        site: String,
        jql: String,
        action: radar_core::journal::ActionDescriptor,
        result: Result<sync_history::Synced, sync_history::Failed>,
    },
    /// A ticket page's data.
    Detail {
        site: String,
        key: String,
        action: radar_core::journal::ActionDescriptor,
        read: page::Read,
        result: Result<Box<client::DetailRead>, String>,
    },
    /// A ticket's `updated` alone, asked to see whether its open page is
    /// out of date (a ticket no view lists).
    Stamp {
        site: String,
        key: String,
        result: Result<String, String>,
    },
    /// A ticket's development links, read in the background.
    Code {
        site: String,
        key: String,
        updated: String,
        action: radar_core::journal::ActionDescriptor,
        result: Result<client::CodeRead, String>,
    },
    /// Older comments for a ticket page, from `start`.
    More {
        site: String,
        key: String,
        action: radar_core::journal::ActionDescriptor,
        read: page::More,
        result: Result<client::CommentsRead, String>,
    },
    /// A setting checked against Jira.
    Checked {
        target: settings_checks::Target,
        request: settings_checks::Request,
        action: radar_core::journal::ActionDescriptor,
        result: Result<client::CheckRead<String>, client::CheckError>,
    },
    /// Jira's verdict on JQL being typed.
    Jql {
        text: String,
        result: Result<Vec<String>, String>,
    },
    /// Fields that could hold a project's acceptance criteria, each with
    /// how many of its tickets fill it in.
    AcFields {
        site: String,
        project: String,
        result: Result<Vec<(String, String, u64)>, String>,
    },
    /// Projects the account looked at lately, to pick one to watch.
    Recent {
        site: String,
        result: Result<Vec<(String, String)>, String>,
    },
    /// Where a ticket can move now, to pick one.
    Moves {
        read: write_dialog::Read,
        site: String,
        key: String,
        result: Result<Vec<model::Transition>, String>,
    },
    /// The kinds of ticket a project takes, for a new one.
    Types {
        read: write_dialog::Read,
        site: String,
        project: String,
        result: Result<Vec<model::IssueType>, String>,
    },
    /// Who the token is, to assign a ticket to them.
    Who {
        read: write_dialog::Read,
        site: String,
        key: String,
        result: Result<Me, String>,
    },
    /// What start work on a ticket needed from Jira: who the token is, and
    /// where the ticket can move (each only when asked for).
    Started {
        site: String,
        key: String,
        me: Option<Result<Me, String>>,
        moves: Option<Result<Vec<model::Transition>, String>>,
    },
    /// What Jira said to a change.
    Wrote {
        dispatch: write_context::Dispatch,
        change: write::Change,
        result: Result<Value, String>,
    },
}

/// Where a check of a setting stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Check {
    Running,
    Passed(String),
    Failed(String),
}

/// What radar holds for one site: who the token is, the tickets, and which
/// tickets each query found (by its JQL, so renaming a view keeps them).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SiteData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub me: Option<Me>,
    #[serde(default)]
    pub tickets: BTreeMap<String, Ticket>,
    #[serde(default)]
    pub found: BTreeMap<String, Vec<String>>,
    /// What Jira's development panel linked to each ticket (by key), asked
    /// in the background for the tickets in view.
    #[serde(default)]
    pub code: BTreeMap<String, CodeEntry>,
    /// When each query (by its JQL) was last read, so the next refresh
    /// asks only what changed since. A cache from before has none: its
    /// first refresh reads each view whole.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub synced: BTreeMap<String, SyncPoint>,
    /// Ticket pages as last read (by key), so a page opens drawn after a
    /// restart and is read again only when its ticket's `updated` moved.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", deserialize_with = "lenient_details")]
    pub details: BTreeMap<String, DetailEntry>,
}

/// A ticket page as last read, and what it was read against.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetailEntry {
    /// The site's address when it was read: a site renamed to another
    /// address does not draw the old one's pages.
    pub url: String,
    /// When it was read, on this machine's clock.
    pub at: i64,
    /// The ticket's `updated` when it was read (the detail's own ticket).
    pub updated: String,
    pub detail: Detail,
}

/// Pages kept in the cache, the most recently read first to go last.
pub(crate) const MAX_STORED_PAGES: usize = 30;

/// Stored pages from the cache; one in a shape radar cannot read is let go, not
/// the rest of the site with it.
fn lenient_details<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<String, DetailEntry>, D::Error> {
    let held = BTreeMap::<String, Value>::deserialize(d)?;
    Ok(held
        .into_iter()
        .filter_map(|(k, v)| Some((k, serde_json::from_value(v).ok()?)))
        .collect())
}

/// Where a query's list stands: when it was last read (asked, on this
/// machine's clock), when it was last read whole, and with which fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPoint {
    pub at: i64,
    pub swept: i64,
    /// The list fields asked for; other fields (a new acceptance-criteria
    /// field, a radar that shows more) read the view whole again.
    pub fields: String,
}

/// A changed-only read reaches this far before the last read: Jira's
/// search index catches up a little late, and its window is whole minutes.
pub(crate) const OVERLAP_SECS: i64 = 120;
/// The whole view's keys are swept every this many refresh intervals (and
/// on `r`): tickets that left the view without changing go then.
pub(crate) const SWEEP_REFRESHES: i64 = 5;
/// A view not read whole for this long is read in full again.
pub(crate) const WHOLE_AFTER_SECS: i64 = 7 * 24 * 3600;
/// The longest a site waits after failures before it is asked again.
pub(crate) const LONGEST_BACKOFF_SECS: i64 = 30 * 60;

/// The list fields a site's tickets are read with, as a [`SyncPoint`] keeps them.
pub(crate) fn ac_fields(site: &Site) -> Vec<String> {
    site.projects
        .iter()
        .map(|p| p.ac_field.clone())
        .filter(|f| !f.is_empty())
        .collect()
}

fn fields_of(site: &Site) -> String {
    model::list_fields(&ac_fields(site)).join(",")
}

/// A ticket's development links as last read, and what they were read
/// against, so they are asked again only when the ticket changed or they
/// have gone stale.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeEntry {
    pub at: i64,
    /// The ticket's `updated` when they were read.
    pub updated: String,
    pub links: Vec<model::CodeLink>,
}

/// How many development panels are asked for at once.
pub(crate) const CODE_AT_ONCE: usize = 4;
/// A merge request can be linked to a ticket without touching the ticket,
/// so the panel of an open ticket in view is read again this often (and
/// every ticket's on `r`).
pub(crate) const CODE_REREAD_SECS: i64 = 30 * 60;

#[derive(Debug, Default)]
pub(crate) struct Sync {
    /// `site\0key` whose development panel is being fetched.
    pub code_in_flight: BTreeSet<String>,
    /// `site\0key` whose panel `r` asked to read again; each stays until
    /// its read answers (or fails), so a few at a time reach them all.
    pub code_refresh: BTreeSet<String>,
    /// Until when a site's hold was last said in the log, so it is said once.
    pub held_said: BTreeMap<String, i64>,
    /// When each `site\0key` was last asked for (an answer or a failure).
    pub code_tried: BTreeMap<String, i64>,
    /// `site\0jql` being fetched.
    pub in_flight: BTreeSet<String>,
    /// When each `site\0jql` was last asked for.
    pub started: BTreeMap<String, i64>,
    /// How each `site\0jql` in flight was asked.
    pub asking: BTreeMap<String, client::Ask>,
    pub last_ok: Option<i64>,
    /// The last error per site.
    pub errors: BTreeMap<String, String>,
    /// Per site after failures: not asked again (but on `r`) before this
    /// time, and how many failed in a row.
    pub backoff: BTreeMap<String, (i64, u32)>,
    /// An answer since the last cache write changed what is kept.
    pub unsaved: bool,
}

/// Something fetched for a page: the last value, whether a fetch is under
/// way, and the last error. The value stays while a refresh runs.
#[derive(Debug)]
pub(crate) struct Fetched<T> {
    pub value: Option<T>,
    pub loading: bool,
    pub error: Option<String>,
    pub at: i64,
    /// Configured connection owning this entire dedicated entry.
    pub generation: u64,
    /// ID of the successfully installed full detail; failures leave it intact.
    pub revision: u64,
    pub request: Option<page::Read>,
    pub more: Option<page::More>,
    /// Its ticket's `updated` is being asked for alone.
    pub checking: bool,
}

impl<T> Default for Fetched<T> {
    fn default() -> Self {
        Self {
            value: None,
            checking: false,
            loading: false,
            error: None,
            at: 0,
            generation: 0,
            revision: 0,
            request: None,
            more: None,
        }
    }
}

/// What is being typed in settings, and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    SiteAdd,
    SiteName(usize),
    SiteUrl(usize),
    SiteEmail(usize),
    SiteToken(usize),
    Refresh,
    /// A project key to watch on a site.
    ProjectAdd(usize),
    ViewAdd,
    ViewName(usize),
    ViewJql(usize),
    /// Words for a "mentioning" condition.
    ViewWords(usize),
    /// A type name for a "of type" condition.
    ViewType(usize),
    /// A label for a "labelled" condition.
    ViewLabel(usize),
}

/// Go to (`:`): what is typed, and where the tree was.
#[derive(Debug)]
pub(crate) struct Goto {
    pub input: radar_core::input::TextInput,
    pub before: Vec<String>,
    pub found: Option<String>,
}

#[derive(Debug, Default)]
pub struct JiraPlugin {
    pub(crate) provider: client::Provider,
    pub(crate) cfg: Settings,
    pub(crate) data: BTreeMap<String, SiteData>,
    pub(crate) sync: Sync,
    /// What Jira last said about its rate limits, per site, from the reads
    /// the plugin starts.
    pub(crate) limits: rate::Limits,
    pub(crate) now: i64,
    // the screen
    pub(crate) view: usize,
    pub(crate) filter: String,
    pub(crate) filter_input: Option<radar_core::input::TextInput>,
    /// Selection when filtering began, before rendering may hide that ticket.
    pub(crate) filter_before: Option<Vec<String>>,
    /// Only this person's tickets (display name), or [`NOBODY`].
    pub(crate) assignee: Option<String>,
    /// The tree's selection and open groups.
    pub(crate) tree: RefCell<radar_core::view::TreeNav>,
    pub(crate) opened_once: RefCell<HashSet<String>>,
    pub(crate) goto: Option<Goto>,
    // ticket pages
    pub(crate) pages: Vec<page::Page>,
    /// What ticket pages fetched, by `site\0key`.
    pub(crate) details: BTreeMap<String, Fetched<Detail>>,
    /// One-use page-read IDs, never reset by connection/cache changes.
    pub(crate) next_page_read: u64,
    // settings
    pub(crate) section_rows: [usize; 4],
    /// Projects keeps stable site/project paths and the user's folds. The
    /// numeric row remains the adapter for the existing settings actions.
    pub(crate) project_tree: RefCell<radar_core::view::TreeNav>,
    pub(crate) project_row_index: std::cell::Cell<usize>,
    /// The connection section was last drawn as a setup step, without the
    /// rows only Settings needs; `site_rows` follows it.
    pub(crate) setup_card: std::cell::Cell<bool>,
    /// Each section's list, how far it is scrolled.
    pub(crate) section_scroll: [radar_core::ui::Scroll; 4],
    pub(crate) edit: Option<(Target, radar_core::input::TextInput)>,
    /// Checks against Jira, keyed by the exact transient site/project target.
    pub(crate) checks: BTreeMap<settings_checks::Target, (Check, i64)>,
    pub(crate) check_owners: settings_checks::Owners,
    /// Jira's verdict on JQL, by the text it was asked about.
    pub(crate) jql_checks: BTreeMap<String, Check>,
    /// When the JQL being typed last changed, to ask Jira once typing stops.
    pub(crate) typed_at: Option<std::time::Instant>,
    /// The environment the first-run offer looks in; `None` is this
    /// process's own (tests give their own).
    pub(crate) env: Option<Vec<(String, String)>>,
    /// Sites the `.mcp.json` files from radar's folder upwards describe, read
    /// once (tests set their own).
    pub(crate) mcp: std::sync::OnceLock<Vec<config::Offer>>,
    // changes to Jira
    pub(crate) dialogs: write_dialog::Dialogs,
    pub(crate) connections: write_context::Connections,
    pub(crate) next_write: u64,
    /// Text being typed: a comment, a new ticket's summary or description.
    pub(crate) compose: Option<write::Compose>,
    /// A change waiting for its `y`.
    pub(crate) change: Option<write::Change>,
    /// The moves offered for a ticket: site, key, moves.
    pub(crate) moving: Option<(String, String, Vec<model::Transition>)>,
    /// A new ticket whose type is being picked.
    pub(crate) new_ticket: Option<write::NewTicket>,
    /// Reads under way before a change can be asked (moves, types, who).
    pub(crate) asking: BTreeSet<String>,
    /// The last change sent per ticket (or project), and its answer.
    pub(crate) sent: BTreeMap<String, write::Sent>,
    /// Exact original text retained per occurrence, in memory only.
    pub(crate) recovery: write_recovery::Recovery,
    /// Start-on-Jira question waiting for its `y` (see `start`).
    pub(crate) starting: Option<start::Start>,
    /// The move that follows an assignment sent as part of it.
    pub(crate) start_next: Option<start::Pending>,
}

/// A ticket as the screen shows it: with its site.
pub(crate) struct Item<'a> {
    pub site: &'a Site,
    pub t: &'a Ticket,
}

pub(crate) fn job(site: &str, jql: &str) -> String {
    format!("{site}\0{jql}")
}

pub(crate) fn detail_key(site: &str, key: &str) -> String {
    format!("{site}\0{key}")
}

impl JiraPlugin {
    pub fn new() -> Self {
        let now = time::now_secs();
        Self {
            now,
            // Projects and look open on their first row, not a heading.
            section_rows: [0, 1, 0, 1],
            // Tests never read the folders they run in.
            #[cfg(test)]
            mcp: std::sync::OnceLock::from(vec![]),
            ..Default::default()
        }
    }

    /// The environment, as names and values (never shown).
    pub(crate) fn vars(&self) -> Vec<(String, String)> {
        #[cfg(any(test, feature = "test-support"))]
        if self.provider.fixture.is_some() {
            return Vec::new();
        }
        match &self.env {
            Some(vars) => vars.clone(),
            None => std::env::vars().collect(),
        }
    }

    /// Sites radar can add for you, from `.mcp.json` files and the
    /// environment, that no site uses yet.
    pub(crate) fn offers(&self) -> Vec<config::Offer> {
        #[cfg(any(test, feature = "test-support"))]
        if self.provider.fixture.is_some() {
            return Vec::new();
        }
        let mcp = self.mcp.get_or_init(|| {
            std::env::current_dir()
                .map(|d| config::mcp_found(&d))
                .unwrap_or_default()
        });
        config::offers(&self.cfg.sites, &self.vars(), mcp)
    }

    pub(crate) fn current_view(&self) -> Option<&config::View> {
        self.cfg.views.get(self.view)
    }

    /// The JQL view `v` sends to site `i`, if any.
    pub(crate) fn jql_for(&self, i: usize, v: &config::View) -> Option<String> {
        let site = self.cfg.sites.get(i)?;
        let others = |k: &str| {
            self.cfg
                .sites
                .iter()
                .enumerate()
                .any(|(j, s)| j != i && s.watches(k))
        };
        view::jql(site, &others, v)
    }

    /// The current view's tickets on one site, in Jira's order, unfiltered;
    /// `None` when the view asks nothing of it or it has not been fetched.
    pub(crate) fn site_tickets(&self, i: usize) -> Option<Vec<&Ticket>> {
        let v = self.current_view()?;
        let jql = self.jql_for(i, v)?;
        let site = &self.cfg.sites[i];
        let data = self.data.get(&site.name)?;
        let keys = data.found.get(&jql)?;
        Some(keys.iter().filter_map(|k| data.tickets.get(k)).collect())
    }

    /// A ticket the current view's list holds on `site`, kept current by
    /// its syncs.
    pub(crate) fn tracked(&self, site: &str, key: &str) -> Option<&Ticket> {
        let i = self.cfg.sites.iter().position(|s| s.name == site)?;
        let jql = self.jql_for(i, self.current_view()?)?;
        let data = self.data.get(site)?;
        data.found.get(&jql)?.iter().any(|k| k == key).then(|| data.tickets.get(key))?
    }

    /// Whether `t` passes the `/` filter and the assignee picked.
    fn shown(&self, t: &Ticket) -> bool {
        let words = self.filter.trim().to_lowercase();
        let text_ok = words.split_whitespace().all(|w| {
            [&t.key, &t.summary, &t.assignee, &t.status, &t.kind]
                .iter()
                .any(|f| f.to_lowercase().contains(w))
                || t.labels.iter().any(|l| l.to_lowercase().contains(w))
        });
        let who_ok = match self.assignee.as_deref() {
            None => true,
            Some(NOBODY) => t.assignee.is_empty(),
            Some(name) => t.assignee == name,
        };
        text_ok && who_ok
    }

    /// Every ticket the screen shows now, site by site.
    pub(crate) fn visible(&self) -> Vec<Item<'_>> {
        let mut out = Vec::new();
        for (i, site) in self.cfg.sites.iter().enumerate() {
            for t in self.site_tickets(i).unwrap_or_default() {
                if self.shown(t) {
                    out.push(Item { site, t });
                }
            }
        }
        out
    }

    /// Everyone tickets in this view are assigned to, by name, once each.
    pub(crate) fn assignees(&self) -> Vec<String> {
        let mut names: Vec<String> = (0..self.cfg.sites.len())
            .flat_map(|i| self.site_tickets(i).unwrap_or_default())
            .map(|t| t.assignee.clone())
            .filter(|n| !n.is_empty())
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names
    }

    pub(crate) fn assignee_menu(&self) -> radar_core::Menu {
        let now = |v: Option<&str>| self.assignee.as_deref() == v;
        let mut items = vec![
            MenuItem::new(
                'e',
                "everyone",
                if now(None) { "shown now" } else { "every assignee again" },
                "",
            ),
            MenuItem::new(
                'n',
                "nobody",
                if now(Some(NOBODY)) { "shown now" } else { "tickets nobody is assigned" },
                NOBODY,
            ),
        ];
        items.extend(self.assignees().into_iter().map(|name| {
            let hint = if now(Some(&name)) { "shown now" } else { "" };
            MenuItem::pick(name.clone(), hint, name)
        }));
        radar_core::Menu::new(ASSIGNEE_MENU, "show tickets assigned to", items)
    }

    fn pick_assignee(&mut self, value: &str) -> Vec<Effect> {
        self.assignee = (!value.is_empty()).then(|| value.to_string());
        let n = self.visible().len();
        vec![Effect::Notify(match self.assignee.as_deref() {
            None => "everyone's tickets".into(),
            Some(NOBODY) => format!("tickets nobody is assigned: {n}"),
            Some(a) => format!("only {a}'s tickets: {n}"),
        })]
    }

    /// Fetch the current view from every site again (`r`), and the
    /// development panel of every ticket in it.
    pub(crate) fn refresh(&mut self) -> Vec<Effect> {
        let mut fx = self.sync_view(true);
        fx.extend(self.sync_code_with(true));
        fx
    }

    /// Until when Jira's rate limits (or a `Retry-After` it sent) have the
    /// background reads of `site` wait, if they do.
    pub(crate) fn held(&self, site: &str) -> Option<i64> {
        self.limits.hold(site, self.now)
    }

    /// Date the `Retry-After`s heard, and say in the log once when a site
    /// is let alone and why.
    fn note_holds(&mut self) {
        self.limits.stamp(self.now);
        for site in self.cfg.sites.iter().map(|s| s.name.clone()).collect::<Vec<_>>() {
            let Some(until) = self.held(&site) else {
                self.sync.held_said.remove(&site);
                continue;
            };
            if self.sync.held_said.insert(site.clone(), until) == Some(until) {
                continue;
            }
            let said = self.limits.rate(&site).unwrap_or_default();
            let count = match (said.remaining, said.limit) {
                (Some(r), Some(l)) => format!(", {r} of {l} left"),
                _ => String::new(),
            };
            let why = said.reason.map(|r| format!(" ({r})")).unwrap_or_default();
            log::info(format!(
                "jira {site}: Jira's rate limit{why}{count}: background reads wait until {}; `r` asks anyway",
                time::to_rfc3339(until)
            ));
        }
    }

    /// Fetch the current view from each site it asks something of, unless
    /// it is fresh, already coming, or its site is backing off after
    /// failures (`force`, the `r` key, skips the freshness and the backoff).
    pub(crate) fn sync_view(&mut self, force: bool) -> Vec<Effect> {
        let Some(v) = self.current_view().cloned() else {
            return vec![];
        };
        let mut fx = Vec::new();
        for i in 0..self.cfg.sites.len() {
            let Some(jql) = self.jql_for(i, &v) else {
                continue;
            };
            let site = self.cfg.sites[i].clone();
            let Some(ask) = self.plan(&site, &jql, force) else {
                continue;
            };
            let id = job(&site.name, &jql);
            self.sync.in_flight.insert(id.clone());
            self.sync.started.insert(id.clone(), self.now);
            self.sync.asking.insert(id, ask);
            let data = self.data.get(&site.name);
            // The accountId rides with the first sync (or the first after a
            // cache that held only a name).
            let ask_me = data.is_none_or(|d| d.me.as_ref().is_none_or(|m| m.account_id.is_empty()));
            let held = data
                .map(|d| d.tickets.iter().map(|(k, t)| (k.clone(), t.updated.clone())).collect())
                .unwrap_or_default();
            fx.push(load(site, jql, ask_me, ask, held, self.limits.clone(), self.provider.clone()));
        }
        fx
    }

    /// How to read one query on one site now, or `None` to leave it. What
    /// is held is the reference: a query read before asks only what changed
    /// since (`updated` within the time since its last read, and a little
    /// more), and now and then sweeps the whole view's keys. Only a query
    /// never read, read with other fields, or not read whole for a week is
    /// read whole.
    pub(crate) fn plan(&self, site: &Site, jql: &str, force: bool) -> Option<client::Ask> {
        let id = job(&site.name, jql);
        if self.sync.in_flight.contains(&id) {
            return None;
        }
        let every = self.cfg.refresh_secs as i64;
        let point = self
            .data
            .get(&site.name)
            .filter(|d| d.found.contains_key(jql))
            .and_then(|d| d.synced.get(jql))
            .filter(|pt| {
                pt.fields == fields_of(site)
                    && pt.swept <= pt.at
                    && pt.at <= self.now
                    && self.now - pt.swept < WHOLE_AFTER_SECS
            });
        if !force {
            let last = self.sync.started.get(&id).copied().max(point.map(|pt| pt.at));
            if last.is_some_and(|t| self.now - t < every) {
                return None;
            }
            if self.sync.backoff.get(&site.name).is_some_and(|(until, _)| self.now < *until) {
                return None;
            }
            // Jira's own word: nearly out of allowance, or told to wait.
            if self.held(&site.name).is_some() {
                return None;
            }
        }
        let Some(pt) = point else {
            return Some(client::Ask::Full);
        };
        Some(client::Ask::Changed {
            minutes: (self.now - pt.at + OVERLAP_SECS + 59) / 60,
            sweep: force || self.now - pt.swept >= every * SWEEP_REFRESHES,
        })
    }

    /// Start reading the development panels of the tickets in view that
    /// have none held, or whose ticket changed (its `updated`), or, for a
    /// ticket still open, were read half an hour ago ([`CODE_REREAD_SECS`]):
    /// a merge request can be linked to a ticket without touching it. A few
    /// at a time, the rest as those finish. The first view never waits for
    /// them; a site Jira has asked radar to leave alone is left. A ticket's
    /// page reads its panel whenever it is read.
    pub(crate) fn sync_code(&mut self) -> Vec<Effect> {
        self.sync_code_with(false)
    }

    /// [`Self::sync_code`]; with `force` (`r`) every ticket in view whose
    /// panel was not read within the refresh interval is read again too,
    /// whatever it is.
    pub(crate) fn sync_code_with(&mut self, force: bool) -> Vec<Effect> {
        let Some(v) = self.current_view().cloned() else {
            return vec![];
        };
        let every = self.cfg.refresh_secs as i64;
        let mut want: Vec<(Site, Ticket)> = Vec::new();
        let mut asked: Vec<String> = Vec::new();
        for i in 0..self.cfg.sites.len() {
            let Some(jql) = self.jql_for(i, &v) else { continue };
            let site = &self.cfg.sites[i];
            let Some(data) = self.data.get(&site.name) else { continue };
            let Some(keys) = data.found.get(&jql) else { continue };
            for t in keys.iter().filter_map(|k| data.tickets.get(k)) {
                if t.id.is_empty() {
                    continue;
                }
                let id = detail_key(&site.name, &t.key);
                let held = data.code.get(&t.key);
                if force && held.is_none_or(|e| self.now - e.at >= every) {
                    asked.push(id.clone());
                }
                if self.sync.code_in_flight.contains(&id) || self.held(&site.name).is_some() {
                    continue;
                }
                let again = self.sync.code_refresh.contains(&id) || asked.contains(&id);
                let stale = t.category != model::Category::Done
                    &&held.is_some_and(|e| self.now - e.at >= CODE_REREAD_SECS);
                let due = held.is_none_or(|e| e.updated != t.updated) || stale || again;
                let tried = !again && self.sync.code_tried.get(&id).is_some_and(|at| self.now - at < every);
                if due && !tried {
                    want.push((site.clone(), t.clone()));
                }
            }
        }
        self.sync.code_refresh.extend(asked);
        // What is still open first: its merge requests are the live ones.
        want.sort_by_key(|(_, t)| t.category == model::Category::Done);
        let room = CODE_AT_ONCE.saturating_sub(self.sync.code_in_flight.len());
        let mut fx = Vec::new();
        for (site, t) in want.into_iter().take(room) {
            let id = detail_key(&site.name, &t.key);
            self.sync.code_in_flight.insert(id.clone());
            self.sync.code_tried.insert(id, self.now);
            fx.push(load_code(site, t, self.limits.clone(), self.provider.clone()));
        }
        fx
    }

    fn on_code(&mut self, site: String, key: String, updated: String, result: Result<Vec<model::CodeLink>, String>) {
        self.sync.code_in_flight.remove(&detail_key(&site, &key));
        self.sync.code_refresh.remove(&detail_key(&site, &key));
        match result {
            Ok(links) => {
                if let Some(data) = self.data.get_mut(&site)
                    && data.tickets.contains_key(&key)
                {
                    data.code.insert(
                        key,
                        CodeEntry {
                            at: self.now,
                            updated,
                            links,
                        },
                    );
                }
            }
            Err(e) => log::warn(format!("jira development panel of {key} on {site}: {e}")),
        }
    }

    /// The `x` rows for the keys the middle pane, nav pane and pages share.
    fn shared_keys(&self, pane: Pane) -> Vec<Binding> {
        let mut b = Vec::new();
        let tree = self.pages.is_empty() && matches!(pane, Pane::Nav | Pane::Middle);
        if tree && pane == Pane::Middle && !self.cfg.sites.is_empty() {
            b.push(Binding::new(GOTO_KEY.to_string(), "go to a ticket by its key or words…").pane());
        }
        if agent_context::can_send(self) && pane != Pane::Nav {
            b.push(
                Binding::new(agent_context::KEY.to_string(), "ask an agent about it…")
                    .group(radar_core::view::ActionGroup::Ask),
            );
        }
        if tree {
            b.push(
                Binding::new(
                    ASSIGNEE_KEY.to_string(),
                    match self.assignee.as_deref() {
                        Some(NOBODY) => "only unassigned now: pick an assignee…".to_string(),
                        Some(a) => format!("only {a}'s now: pick an assignee…"),
                        None => "show one assignee's tickets…".into(),
                    },
                )
                .pane(),
            );
        }
        b
    }

    fn shared_key(&mut self, pane: Pane, key: Chord) -> Option<Outcome> {
        if !key.plain() {
            return None;
        }
        let tree = self.pages.is_empty() && matches!(pane, Pane::Nav | Pane::Middle);
        match key.key {
            Key::Char(agent_context::KEY) if pane != Pane::Nav && agent_context::can_send(self) => {
                Some(Outcome::Handled(agent_context::send_selected(self)))
            }
            Key::Char(ASSIGNEE_KEY) if tree => {
                Some(Outcome::Handled(vec![Effect::Menu(self.assignee_menu())]))
            }
            Key::Char(GOTO_KEY) if tree && !self.cfg.sites.is_empty() => {
                screen::goto_start(self);
                Some(Outcome::Handled(vec![Effect::Focus(Pane::Middle)]))
            }
            _ => None,
        }
    }

    /// Whether `t`, a ticket of `site`, is assigned to the account that
    /// site's `myself` named. Until that is known, nobody is you.
    pub(crate) fn is_mine(&self, site: &str, t: &Ticket) -> bool {
        !t.assignee_id.is_empty()
            && self
                .data
                .get(site)
                .and_then(|d| d.me.as_ref())
                .is_some_and(|m| m.account_id == t.assignee_id)
    }

    /// A key on the screen the frontend did not use for moving: the kept
    /// text, the composer, go to, the shared keys, the changes, then the
    /// page's or the tree's own.
    fn screen_key(&mut self, pane: Pane, key: Chord) -> Outcome {
        if key.ctrl && key.key == Key::Char('r') {
            return Outcome::Handled(write_recovery::list(self, None));
        }
        if self.compose.is_some() {
            return write::compose_key(self, key);
        }
        if self.goto.is_some() {
            return screen::goto_key(self, key);
        }
        if self.filter_input.is_none()
            && let Some(out) = self.shared_key(pane, key)
        {
            return out;
        }
        if self.filter_input.is_none()
            && let Some(out) = write::key(self, pane, key)
        {
            return out;
        }
        if !self.pages.is_empty() {
            return page::key(self, key);
        }
        let out = screen::key(self, pane, key);
        if matches!(out, Outcome::Ignored)
            && key.key == Key::Esc
            && pane == Pane::Middle
            && self.assignee.is_some()
        {
            return Outcome::Handled(self.pick_assignee(""));
        }
        self.clamp();
        out
    }

    /// The `x` rows of the screen (the page's while one is open), each with
    /// its stable id `jira.tickets.<verb>`. The verb is chosen here by the
    /// key the row lists, never made from its label; a heading has no id.
    fn screen_rows(&self, pane: Pane) -> Vec<(String, Binding)> {
        self.bindings(pane)
            .into_iter()
            .map(|b| {
                let verb = match b.keys.as_str() {
                    ":" => "goto",
                    "a" => "assignee",
                    "t" => "move",
                    "c" => "comment",
                    "i" => "assign_me",
                    "n" => "new_ticket",
                    "⏎" => "open",
                    "space" => "fold",
                    "/" => "filter",
                    "r" => "refresh",
                    "o" => "open_in_jira",
                    "y" => "copy_link",
                    "Y" => "copy_key",
                    "pgdn" => "page_down",
                    "pgup" => "page_up",
                    "esc" => "back",
                    "[" => "previous_tab",
                    "]" => "next_tab",
                    "ctrl-r" => "kept_text",
                    "A" => "send_to_agent",
                    _ => "",
                };
                let id = if b.is_heading() || verb.is_empty() {
                    String::new()
                } else if b.keys == agent_context::KEY.to_string() {
                    // the one id every part's rows use for this action
                    radar_core::agent_context::SEND_TO_AGENT_ACTION.to_string()
                } else {
                    format!("jira.tickets.{verb}")
                };
                (id, b)
            })
            .collect()
    }

    /// An `Invoke` names an action by its stable id. The row that has the id
    /// runs by the key it lists, so the key routing stays the one way an
    /// action runs. Any other intent passes unchanged.
    fn by_action_id(&self, scope: &str, pane: Pane, intent: &radar_core::view::Intent) -> Option<radar_core::view::Intent> {
        let radar_core::view::Intent::Invoke { action } = intent else { return None };
        let rows = if matches!(canon(scope), "look" | "views" | "connection" | "projects") {
            settings::rows(self, canon(scope))
                .into_iter()
                .chain(std::iter::once((
                    format!("jira.{}.kept_text", canon(scope)),
                    Binding::new("ctrl-r", "kept Jira text…").pane(),
                )))
                .collect::<Vec<_>>()
        } else if scope == SCREEN.id {
            self.screen_rows(if self.pages.is_empty() { pane } else { Pane::Middle })
        } else {
            return None;
        };
        rows.iter()
            .find(|(id, _)| !id.is_empty() && id == action)
            .map(|(_, b)| radar_core::view::Intent::Invoke { action: b.keys.clone() })
    }

    /// The actions menu's rows for `pane` (the page's while one is open).
    pub(crate) fn bindings(&self, pane: Pane) -> Vec<Binding> {
        let mut b = if self.pages.is_empty() {
            screen::keys(self, pane)
        } else {
            page::keys(self)
        };
        // The changes come after what the menu's heading names, before the
        // rest (a page's tabs, the way back).
        let changes = write::bindings(self, if self.pages.is_empty() { pane } else { Pane::Middle });
        let at = b
            .iter()
            .skip(1)
            .position(Binding::is_heading)
            .map_or(b.len(), |i| i + 1);
        let at = b.iter().rposition(|r| r.keys == "esc").filter(|i| i + 1 == b.len() && *i < at).unwrap_or(at);
        b.splice(at..at, changes);
        let at = b
            .iter()
            .rposition(|r| r.keys == "esc")
            .filter(|i| i + 1 == b.len())
            .unwrap_or(b.len());
        let shared = self.shared_keys(if self.pages.is_empty() {
            pane
        } else {
            Pane::Middle
        });
        b.splice(at..at, shared);
        b.push(Binding::new("ctrl-r", "kept Jira text…").pane());
        b
    }

    pub(crate) fn clamp(&mut self) {
        self.view = self.view.min(self.cfg.views.len().saturating_sub(1));
    }

    /// Fold a whole view's answer for one site and query into what is held.
    fn on_loaded(&mut self, site: String, jql: String, got: (Option<Me>, Vec<Ticket>)) {
        self.fold(site, jql, got.0, got.1, false, None);
    }

    /// Fold an answer for one site and query into what is held: the whole
    /// view, or (`changed_only`) the tickets that changed, put first (most
    /// recently updated) before the rest held, and, with `swept`, the view's
    /// keys in Jira's order. Tickets no query of a configured view found
    /// any more are let go.
    fn fold(
        &mut self,
        site: String,
        jql: String,
        me: Option<Me>,
        tickets: Vec<Ticket>,
        changed_only: bool,
        swept: Option<Vec<String>>,
    ) {
        let data = self.data.entry(site.clone()).or_default();
        if let Some(me) = me {
            data.me = Some(me);
        }
        let read: Vec<String> = tickets.iter().map(|t| t.key.clone()).collect();
        let found = match (changed_only, swept) {
            (false, _) => read,
            (true, Some(keys)) => keys,
            (true, None) => {
                let first: HashSet<&String> = read.iter().collect();
                let rest = data.found.get(&jql).into_iter().flatten().filter(|k| !first.contains(k));
                let mut keys: Vec<String> = read.iter().chain(rest).cloned().collect();
                keys.truncate(client::MAX_TICKETS);
                keys
            }
        };
        data.found.insert(jql, found);
        for t in tickets {
            data.tickets.insert(t.key.clone(), t);
        }
        // Only what a configured view on this site asks for is kept.
        let Some(i) = self.cfg.sites.iter().position(|s| s.name == site) else {
            return;
        };
        let wanted: BTreeSet<String> = self
            .cfg
            .views
            .iter()
            .filter_map(|v| self.jql_for(i, v))
            .collect();
        let data = self.data.get_mut(&site).expect("inserted above");
        data.found.retain(|q, _| wanted.contains(q));
        data.synced.retain(|q, _| wanted.contains(q));
        let keep: BTreeSet<String> = data.found.values().flatten().cloned().collect();
        data.tickets.retain(|k, _| keep.contains(k));
        data.code.retain(|k, _| keep.contains(k));
    }
}

/// The task that reads one ticket's development panel. Reads are retried
/// by the client; a failure is only logged, the ticket just has no chip.
fn load_code(site: Site, t: Ticket, limits: rate::Limits, provider: client::Provider) -> Effect {
    let action = code_history::action(&site, &t.key);
    code_history::spawn(action, site.name.clone(), t.key, t.updated, async move {
        async { provider.tracked(&site, &limits)?.code_observed(&t.id).await }
            .await
            .map_err(|e| format!("{e:#}"))
    })
}

/// The task that fetches one site's tickets for one query, as `ask` says;
/// `held` is each held ticket's `updated`, for a sweep to compare.
fn load(site: Site, jql: String, ask_me: bool, ask: client::Ask, held: BTreeMap<String, String>, limits: rate::Limits, provider: client::Provider) -> Effect {
    let action = sync_history::action(&site);
    sync_history::spawn(action, site.name.clone(), jql.clone(), async move {
        let started = std::time::Instant::now();
        let how = match ask {
            client::Ask::Full => "whole".to_string(),
            client::Ask::Changed { minutes, sweep } => {
                format!("changed in {minutes}m{}", if sweep { ", keys swept" } else { "" })
            }
        };
        log::info(format!("jira {} ({how}): {jql}", site.name));
        let result = async {
            let c = provider.tracked(&site, &limits)?;
            let me = if ask_me { Some(c.myself().await?) } else { None };
            let ac = |p: &str| site.ac_field(p).map(str::to_string);
            let read = c.view_list(&jql, &ask, &held, &ac, &ac_fields(&site)).await?;
            anyhow::Ok(sync_history::Synced {
                me,
                list: read.list,
                changed_only: read.changed_only,
                swept: read.swept,
            })
        }
        .await
        .map_err(|e| sync_history::Failed::of(&e));
        match &result {
            Ok(got) => log::info(format!(
                "jira {}: {} tickets read in {:.1}s",
                site.name,
                got.list.tickets.len(),
                started.elapsed().as_secs_f32()
            )),
            Err(e) => log::error(format!("jira {} failed: {}", site.name, e.text)),
        }
        result
    })
}

impl Plugin for JiraPlugin {
    fn id(&self) -> &'static str {
        "jira"
    }
    #[cfg(feature = "demo")]
    fn demo(&mut self) {
        let provider = self.provider.clone();
        *self = demo::sample();
        self.provider = provider;
    }
    fn name(&self) -> &'static str {
        "Jira"
    }
    fn icon(&self, _screen: &str) -> Option<radar_core::ui::icon::Icon> {
        Some(radar_core::ui::icon::Icon::Issue)
    }
    fn section_icon(&self, section: &str) -> Option<radar_core::ui::icon::Icon> {
        use radar_core::ui::icon::Icon;
        Some(match section {
            "connection" => Icon::Connection,
            "projects" => Icon::Folder,
            "views" => Icon::Views,
            "look" => Icon::Look,
            _ => return None,
        })
    }
    fn owns(&self) -> Vec<radar_core::Owned> {
        join::OWNS.to_vec()
    }
    fn subject_from_journal(&self, key: &radar_core::journal::SubjectKey) -> Option<radar_core::Subject> {
        source_navigation::from_journal(self, key)
    }

    fn journal_subject(&self, subject: &radar_core::Subject) -> Option<radar_core::journal::SubjectSnapshot> {
        use radar_core::journal::{SafeUrl, SubjectKey, SubjectRole, SubjectSnapshot};
        if !subject.is(join::TICKET) || !config::is_ticket_key(&subject.id) {
            return None;
        }
        // Do not use site_for's first-site fallback: duplicate ticket keys are valid.
        let site = self.cfg.site(subject.field("site").filter(|alias| !alias.is_empty())?)?;
        if let Some(given) = &subject.url
            && !subject_scope::url_matches(site, &subject.id, given)
        {
            return None;
        }
        let instance = site.base_url().ok()?;
        let url = SafeUrl::parse(&site.browse(&subject.id)?).ok()?;
        if let Some(given) = &subject.url
            && SafeUrl::parse(given).ok()? != url
        {
            return None;
        }
        let key = SubjectKey::new("jira", &instance, join::TICKET, &subject.id).ok()?;
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
        Some(self.pages.last()?.identity())
    }
    fn tabs(&self, _: &str) -> Option<radar_core::plugin::PageTabs> {
        page::page_tabs(self)
    }
    fn show_tab(&mut self, _: &str, i: usize) -> Vec<Effect> {
        page::show_tab(self, i)
    }
    fn detail_identity(&self, scope: &str, pane: Pane) -> Option<radar_core::plugin::DetailIdentity> {
        if scope != SCREEN.id || pane != Pane::Detail || !self.pages.is_empty() {
            return None;
        }
        let screen::Selected::Ticket(item) = screen::selected(self)? else {
            return None;
        };
        if item.site.name.trim().is_empty() || !config::is_ticket_key(&item.t.key) || item.t.id.trim().is_empty() {
            return None;
        }
        Some(radar_core::plugin::DetailIdentity {
            connection: self.connections.generation(&item.site.name)?,
            subject: vec![item.site.name.clone(), item.t.key.clone(), item.t.id.clone()],
        })
    }
    fn subject(&self, _: &str, pane: Pane) -> Option<radar_core::Subject> {
        join::subject(self, pane)
    }
    fn offers(&self) -> Vec<radar_core::Offer> {
        join::OFFERS.to_vec()
    }
    fn adds(&self) -> Vec<radar_core::Adds> {
        join::ADDS.to_vec()
    }
    fn related(&self, subject: &radar_core::Subject) -> Vec<radar_core::Subject> {
        join::related(self, subject)
    }
    fn contribute(&self, point: radar_core::Point, subject: &radar_core::Subject) -> Option<radar_core::Contribution> {
        join::contribute(self, point, subject)
    }
    fn act(&mut self, subject: &radar_core::Subject, action: &str) -> Vec<Effect> {
        join::act(self, subject, action)
    }

    fn set_up(&self) -> radar_core::Setup {
        if self.cfg.sites.is_empty() {
            // Name what was found to start from (never its values).
            let mut text = "Add a site: the Jira address, your email and an API token.".to_string();
            if let Some(f) = JiraPlugin::offers(self).first() {
                text.push_str(&format!(" radar found {} {}; connection offers it.", f.label, f.source));
            }
            radar_core::Setup::Needed(text)
        } else {
            radar_core::Setup::Ready
        }
    }

    fn configure(&mut self, table: Option<&toml::Value>) -> anyhow::Result<()> {
        let cfg: Settings = match table {
            Some(v) => v.clone().try_into()?,
            None => Settings::default(),
        };
        // URL-derived navigation and journal scope also resolve environment
        // references. Hosted scenarios require literal synthetic origins so
        // those pure-looking paths cannot accidentally read the real process.
        #[cfg(any(test, feature = "test-support"))]
        if let Some(fixture) = &self.provider.fixture
            && cfg.sites.iter().any(|site| site.url.trim().starts_with("env:"))
        {
            fixture.blocked("environment-backed site URL");
            anyhow::bail!("Jira hosted fixture requires literal site URLs");
        }
        self.cfg = cfg;
        write_context::reconcile(self);
        self.clamp();
        Ok(())
    }

    fn config(&self) -> Option<toml::Value> {
        toml::Value::try_from(&self.cfg).ok()
    }

    fn first_run(&mut self) -> Option<String> {
        let found = self.offers();
        Some(match found.first() {
            Some(f) => format!("Jira: found {} {}; use it under connection", f.label, f.source),
            None => "Jira: add a site under connection".into(),
        })
    }

    fn slots(&self) -> Vec<(String, SlotValue)> {
        SITE_SLOTS
            .iter()
            .map(|&(name, rgb, i256, c16)| (format!("jira.{name}"), SlotValue::new(rgb, i256, c16)))
            .collect()
    }

    /// What was held last time is drawn at once and is the reference the
    /// first refresh asks from. A site whose part cannot be read (a cache
    /// from another radar) is let go once: its views are read whole again.
    fn restore(&mut self, cache: Value) {
        let Some(sites) = cache.get("data").and_then(Value::as_object) else {
            log::warn("jira cache holds no sites: every view is read whole");
            return;
        };
        let mut data = BTreeMap::new();
        for (name, held) in sites {
            match SiteData::deserialize(held) {
                Ok(d) => {
                    data.insert(name.clone(), d);
                }
                Err(e) => log::warn(format!("jira cache for {name} let go, read again from Jira: {e}")),
            }
        }
        self.data = data;
        self.sync.last_ok = self.data.values().flat_map(|d| d.synced.values().map(|pt| pt.at)).max();
    }

    fn snapshot(&self) -> Option<Value> {
        Some(json!({ "data": self.data }))
    }

    fn tick(&mut self, now: i64) -> Vec<Effect> {
        self.now = now;
        self.note_holds();
        let mut fx = settings::tick(self);
        // An open ticket page follows the part's cycle (fetch keeps it fresh
        // within `refresh_secs`), unless Jira has the site left alone.
        if let Some(page) = self.pages.last() {
            let (site, key) = (page.site.clone(), page.key.clone());
            if self.held(&site).is_none() {
                fx.extend(page::fetch(self, &site, &key, false));
            }
        }
        // A page newly kept is written at the next tick once no list is
        // being read (one that is writes it with its own answer).
        if self.sync.unsaved && self.sync.in_flight.is_empty() && !page::loading(self) {
            self.sync.unsaved = false;
            fx.push(Effect::PersistCache);
        }
        fx.extend(self.sync_view(false));
        fx.extend(self.sync_code());
        fx
    }

    fn prepare_agent_context(&mut self, subject: &radar_core::journal::SubjectKey) -> (bool, Vec<Effect>) {
        if subject.provider != "jira" || subject.kind != "ticket" { return (false, vec![]); }
        let Some(site) = self.cfg.sites.iter().find(|site| agent_context::key(site, &subject.id).as_ref() == Some(subject)).map(|s| s.name.clone()) else { return (false, vec![]); };
        if let Some(f) = self.details.get(&detail_key(&site, &subject.id)) {
            if f.loading || f.checking || f.more.is_some() { return (true, vec![]); }
            if f.error.is_some() { return (false, vec![]); }
            if let Some(d) = &f.value && self.now - f.at < 60 {
                if d.comments.len() < d.comments_total && d.comments.len() < 500 {
                    let fx = page::fetch_more(self, &site, &subject.id);
                    return (!fx.is_empty(), fx);
                }
                return (false, vec![]);
            }
        }
        let fx = page::fetch(self, &site, &subject.id, true);
        (!fx.is_empty(), fx)
    }
    fn agent_identity(&self, subject: &radar_core::journal::SubjectKey) -> Option<String> {
        let site = self.cfg.sites.iter().find(|site| agent_context::key(site, &subject.id).as_ref() == Some(subject))?;
        self.data.get(&site.name)?.me.as_ref().map(|m| m.account_id.clone())
    }
    fn agent_document(&mut self, request: &radar_core::agent_context::AgentContextRequest) -> Option<radar_core::context::Document> {
        agent_context::document(self, request)
    }
    fn agent_context(
        &mut self,
        request: &radar_core::agent_context::AgentContextRequest,
    ) -> Option<radar_core::agent_context::AgentContext> {
        agent_context::agent_context(self, request)
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        let Ok(msg) = msg.downcast::<JiraMsg>() else {
            return vec![];
        };
        match *msg {
            JiraMsg::Loaded { site, jql, action, result } => {
                let id = job(&site, &jql);
                self.sync.in_flight.remove(&id);
                self.sync.asking.remove(&id);
                let asked_at = self.sync.started.get(&id).copied().unwrap_or(self.now).min(self.now);
                let mut fx = sync_history::observed(&action, &result);
                let mut changed = None;
                match result {
                    Ok(got) => {
                        let before = self.data.get(&site).cloned().unwrap_or_default();
                        let whole = !got.changed_only || got.swept.is_some();
                        let complete = got.list.evidence.incomplete_pages == 0;
                        self.fold(site.clone(), jql.clone(), got.me, got.list.tickets, got.changed_only, got.swept);
                        // Where the query stands now, for the next read to ask
                        // from: never past an answer with pages missing.
                        let fields = self.cfg.site(&site).map(fields_of);
                        if let (Some(fields), true) = (fields, complete)
                            && let Some(data) = self.data.get_mut(&site)
                            && data.found.contains_key(&jql)
                        {
                            let swept = match data.synced.get(&jql) {
                                Some(pt) if !whole && pt.fields == fields => pt.swept,
                                _ => asked_at,
                            };
                            data.synced.insert(jql.clone(), SyncPoint { at: asked_at, swept, fields });
                        }
                        let after = &self.data[&site];
                        // What the answer changed, for the rows to glow (motion).
                        let changes = radar_core::motion::Changes::between(
                            before.tickets.iter().map(|(k, t)| (join::motion_key(k), t)),
                            after.tickets.iter().map(|(k, t)| (join::motion_key(k), t)),
                        );
                        // Written when something shown changed, or the view
                        // was read whole: a quiet changed-only read is not
                        // worth a write (the next start asks a little more).
                        self.sync.unsaved |=
                            whole || !changes.is_empty() || before.found != after.found || before.me != after.me;
                        if !changes.is_empty() {
                            changed = Some(Effect::Changed(changes));
                        }
                        self.sync.errors.remove(&site);
                        self.sync.backoff.remove(&site);
                        self.sync.last_ok = Some(self.now);
                    }
                    Err(e) => {
                        // Each failure in a row waits twice as long (from one
                        // refresh interval), and at least as long as Jira asked.
                        let every = self.cfg.refresh_secs as i64;
                        let n = self.sync.backoff.get(&site).map_or(0, |b| b.1) + 1;
                        let wait = every
                            .saturating_mul(1 << (n - 1).min(16))
                            .min(LONGEST_BACKOFF_SECS)
                            .max(e.retry_after.map_or(0, |s| s as i64));
                        self.sync.backoff.insert(site.clone(), (self.now + wait, n));
                        self.sync.errors.insert(site, e.text);
                    }
                }
                fx.extend(changed);
                fx.extend(self.sync_code());
                // Once every site has answered; one site failing keeps the
                // others' answers.
                if self.sync.unsaved && self.sync.in_flight.is_empty() {
                    self.sync.unsaved = false;
                    fx.push(Effect::PersistCache);
                }
                fx
            }
            JiraMsg::Code { site, key, updated, action, result } => {
                let mut fx = code_history::observed(&action, &result);
                self.on_code(site, key, updated, result.map(|got| got.links));
                fx.extend(self.sync_code());
                if self.sync.code_in_flight.is_empty() {
                    fx.push(Effect::PersistCache);
                }
                fx
            }
            JiraMsg::Detail { site, key, action, read, result } => {
                let mut fx = page_history::observed(&action, &result);
                if !page::on_detail(self, &site, &key, read, result.map(|got| *got)) {
                    fx.push(page_history::skipped(&action));
                }
                fx
            }
            JiraMsg::Stamp { site, key, result } => page::on_stamp(self, &site, &key, result),
            JiraMsg::More { site, key, action, read, result } => {
                let mut fx = comments_history::observed(&action, &result);
                if !page::on_more(self, &site, &key, read, result.map(|got| (got.comments, got.total))) {
                    fx.push(comments_history::skipped(&action));
                }
                fx
            }
            JiraMsg::Checked {
                target,
                request,
                action,
                result,
            } => {
                let mut fx = settings_history::observed(target.kind(), &action, &result);
                if settings_checks::claim(self, &target, request) {
                    fx.extend(settings::on_checked(
                        self,
                        target,
                        result
                            .map(|got| got.value)
                            .map_err(|e| format!("{:#}", e.error())),
                    ));
                } else {
                    fx.push(settings_history::skipped(target.kind(), &action));
                }
                fx
            }
            JiraMsg::Jql { text, result } => {
                self.jql_checks.insert(
                    text,
                    match result {
                        Ok(errors) if errors.is_empty() => Check::Passed("Jira reads it".into()),
                        Ok(errors) => Check::Failed(errors.join("; ")),
                        Err(e) => Check::Failed(e),
                    },
                );
                vec![]
            }
            JiraMsg::AcFields {
                site,
                project,
                result,
            } => settings::on_ac_fields(self, &site, &project, result),
            JiraMsg::Recent { site, result } => settings::on_recent(self, &site, result),
            JiraMsg::Moves { read, site, key, result } => write::on_moves(self, read, &site, &key, result),
            JiraMsg::Types { read, site, project, result } => write::on_types(self, read, &site, &project, result),
            JiraMsg::Who { read, site, key, result } => write::on_who(self, read, &site, &key, result),
            JiraMsg::Started { site, key, me, moves } => start::on_started(self, &site, &key, me, moves),
            JiraMsg::Wrote { dispatch, change, result } => {
                let ok = result.is_ok();
                let again = change.clone();
                let mut fx = write::on_wrote(self, change, result, &dispatch);
                fx.extend(start::after_wrote(self, &again, ok, &dispatch));
                fx
            }
        }
    }

    fn on_bus(&mut self, topic: &str, payload: &Value) -> Vec<Effect> {
        start::on_bus(self, topic, payload)
    }

    fn input(&mut self, _: i64) {
        self.dialogs.input();
    }

    fn capturing(&self) -> bool {
        self.filter_input.is_some() || self.edit.is_some() || self.goto.is_some() || self.compose.is_some()
    }

    fn paste(&mut self, text: &str) -> Vec<Effect> {
        if self.compose.is_some() {
            write::compose_paste(self, text);
        } else if self.goto.is_some() {
            screen::goto_paste(self, text);
        } else if let Some(input) = self.filter_input.as_mut() {
            input.paste(text);
            self.filter = input.value();
        } else if let Some((_, input)) = self.edit.as_mut() {
            input.paste(text);
            self.typed_at = Some(std::time::Instant::now());
        }
        vec![]
    }

    fn screens(&self) -> Vec<ScreenSpec> {
        vec![SCREEN]
    }
    fn top_right(&self, _: &str) -> Option<String> {
        screen::top_right(self)
    }
    fn busy(&self, _: &str) -> bool {
        !self.sync.in_flight.is_empty() || page::loading(self) || write::busy(self)
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
        if screen != SCREEN.id {
            return None;
        }
        let mut view = page::view(self, cx).unwrap_or_else(|| screen::view(self, cx));
        let pane = if self.pages.is_empty() { cx.focus.into() } else { Pane::Middle };
        view.actions =
            self.screen_rows(pane).iter().map(|(id, b)| radar_core::view::ActionSpec::named(id.clone(), b)).collect();
        view.overlay = write::overlay(self).map(Box::new);
        Some(view)
    }
    fn section_view(&self, section: &str, cx: &radar_core::view::ViewCx<'_>) -> Option<radar_core::view::SectionView> {
        if canon(section) == "connection" {
            self.setup_card.set(cx.setup);
        }
        match canon(section) {
            "look" => Some(settings::look_view(self)),
            "views" => Some(settings::views_view(self, cx.width)),
            "connection" => Some(settings::connection_view(self, cx.width, cx.now)),
            "projects" => Some(settings::projects_view(self, cx.width, cx.now)),
            _ => None,
        }
    }
    fn intent(&mut self, scope: &str, pane: Pane, intent: &radar_core::view::Intent) -> Outcome {
        use radar_core::view::Intent;
        let by_id = self.by_action_id(scope, pane, intent);
        let intent = by_id.as_ref().unwrap_or(intent);
        if matches!(canon(scope), "look" | "views" | "connection" | "projects") {
            if intent.chord().is_some_and(|chord| chord.ctrl && chord.key == Key::Char('r')) {
                return Outcome::Handled(write_recovery::list(self, None));
            }
            let out = match canon(scope) {
                "look" => settings::look_intent(self, intent),
                "views" => settings::views_intent(self, intent),
                "connection" => settings::connection_intent(self, intent),
                _ => settings::projects_intent(self, intent),
            };
            self.clamp();
            return out;
        }
        if scope != SCREEN.id {
            return Outcome::Ignored;
        }
        if let Intent::Key { .. } | Intent::Invoke { .. } = intent {
            let Some(chord) = intent.chord() else {
                return Outcome::Ignored;
            };
            return self.screen_key(pane, chord);
        }
        if self.pages.is_empty() {
            let out = screen::intent(self, pane, intent);
            self.clamp();
            out
        } else {
            page::intent(self, intent)
        }
    }
    fn pictures(&self, source: &str) -> Option<radar_core::ui::image::Fetch> {
        self.cfg.site(source).map(|_| page::pictures(self, source))
    }
    fn panes(&self, _: &str) -> Vec<Pane> {
        if self.pages.is_empty() {
            vec![Pane::Nav, Pane::Middle, Pane::Detail]
        } else {
            vec![Pane::Middle]
        }
    }
    fn hints(&self, _: &str, _: Pane) -> Vec<KeyHint<'static>> {
        write::hints(self).unwrap_or_else(|| screen::hints(self))
    }
    /// `+`/`-` fold the tree of sites, statuses and tickets.
    fn folds(&self, scope: &str, pane: Pane) -> bool {
        pane == Pane::Middle && (canon(scope) == "projects" || (scope == SCREEN.id && self.pages.is_empty()))
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
    fn section_hints(&self, _: &str) -> Vec<KeyHint<'static>> {
        let mut hints = settings::hints(self);
        if !self.recovery.records.is_empty() {
            hints.push(KeyHint::new("ctrl-r", "kept Jira text"));
        }
        hints
    }
    fn bulk(&self, scope: &str) -> Vec<MenuItem> {
        if scope == SCREEN.id && !self.pages.is_empty() {
            vec![]
        } else if scope == SCREEN.id {
            screen::bulk(self)
        } else {
            settings::bulk(self, canon(scope))
        }
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        if menu == ASSIGNEE_MENU {
            return self.pick_assignee(value);
        }
        if let Some(fx) = write_recovery::on_menu(self, menu, value) {
            return fx;
        }
        if let Some(fx) = start::on_menu(self, menu, value) {
            return fx;
        }
        if let Some(fx) = write::on_menu(self, menu, value) {
            return fx;
        }
        if menu == radar_core::host::BULK_MENU {
            let fx = match screen::on_bulk(self, value) {
                Some(fx) => fx,
                None => settings::on_bulk(self, value),
            };
            self.clamp();
            return fx;
        }
        let fx = settings::on_menu(self, menu, value);
        self.clamp();
        fx
    }

    fn cli(&self) -> Option<clap::Command> {
        let json = || {
            clap::Arg::new("json")
                .long("json")
                .action(clap::ArgAction::SetTrue)
                .help("JSON output")
        };
        let site = || {
            clap::Arg::new("site")
                .long("site")
                .help("Only this site (its name in settings)")
        };
        Some(
            clap::Command::new("jira")
                .about("Jira tickets")
                .subcommand_required(true)
                .subcommand(
                    clap::Command::new("list")
                        .about("Print a view's tickets")
                        .arg(
                            clap::Arg::new("view")
                                .long("view")
                                .help("View name (default: the first)"),
                        )
                        .arg(
                            clap::Arg::new("filter")
                                .long("filter")
                                .help("Only tickets mentioning these words (the screen's /)"),
                        )
                        .arg(site())
                        .arg(json()),
                )
                .subcommand(
                    clap::Command::new("show")
                        .about("Print one ticket: details, description, acceptance criteria, links, comments")
                        .arg(clap::Arg::new("key").required(true).help("The ticket's key, e.g. ABC-12"))
                        .arg(site())
                        .arg(json()),
                ),
        )
    }

    fn run_cli(&self, matches: &clap::ArgMatches) -> CliTask {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(fixture) = &self.provider.fixture {
            fixture.blocked("CLI");
            return Box::pin(async { anyhow::bail!("Jira hosted fixture does not support CLI") });
        }
        let cfg = self.cfg.clone();
        let m = matches.clone();
        Box::pin(async move {
            match m.subcommand() {
                Some(("list", m)) => cli_list(cfg, m).await,
                Some(("show", m)) => cli_show(cfg, m).await,
                _ => anyhow::bail!("unknown jira command"),
            }
        })
    }
}

/// The sites a command acts on: all, or the one named.
fn cli_sites(cfg: &Settings, m: &clap::ArgMatches) -> anyhow::Result<Vec<usize>> {
    if cfg.sites.is_empty() {
        anyhow::bail!("no Jira site set: run radar, press , for settings, then add one under Jira, connection");
    }
    match m.get_one::<String>("site") {
        Some(name) => cfg
            .sites
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name))
            .map(|i| vec![i])
            .ok_or_else(|| anyhow::anyhow!("no site named {name}")),
        None => Ok((0..cfg.sites.len()).collect()),
    }
}

async fn cli_list(cfg: Settings, m: &clap::ArgMatches) -> anyhow::Result<String> {
    let sites = cli_sites(&cfg, m)?;
    let mut p = JiraPlugin {
        cfg,
        ..JiraPlugin::new()
    };
    if let Some(name) = m.get_one::<String>("view") {
        p.view = p
            .cfg
            .views
            .iter()
            .position(|v| v.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| anyhow::anyhow!("no view named {name}"))?;
    }
    let v = p.current_view().cloned().unwrap_or_else(|| config::default_views()[0].clone());
    for i in sites {
        let Some(jql) = p.jql_for(i, &v) else { continue };
        let site = p.cfg.sites[i].clone();
        let c = Client::for_site(&site)?;
        let ac_fields: Vec<String> = site.projects.iter().map(|p| p.ac_field.clone()).filter(|f| !f.is_empty()).collect();
        let ac = |k: &str| site.ac_field(k).map(str::to_string);
        let tickets = c
            .tickets(&jql, &ac, &ac_fields)
            .await
            .map_err(|e| anyhow::anyhow!("{}: {e:#}", site.name))?;
        p.on_loaded(site.name.clone(), jql, (None, tickets));
    }
    if let Some(words) = m.get_one::<String>("filter") {
        p.filter = words.clone();
    }
    let items = p.visible();
    if m.get_flag("json") {
        let rows: Vec<Value> = items
            .iter()
            .map(|it| {
                json!({
                    "site": it.site.name, "key": it.t.key, "summary": it.t.summary,
                    "type": it.t.kind, "status": it.t.status, "category": it.t.category.word(),
                    "assignee": screen::person(&it.t.assignee, &it.t.assignee_id),
                    "updated": screen::rfc3339(&it.t.updated),
                    "parent": it.t.parent.as_ref().map(|p| &p.key),
                    "url": it.site.browse(&it.t.key),
                })
            })
            .collect();
        return Ok(serde_json::to_string_pretty(&rows)? + "\n");
    }
    Ok(items
        .iter()
        .map(|it| {
            format!(
                "{:<8} {:<12} {:<14} {:<10} {}\n",
                it.site.name,
                it.t.key,
                it.t.status,
                it.t.kind,
                it.t.summary
            )
        })
        .collect())
}

async fn cli_show(cfg: Settings, m: &clap::ArgMatches) -> anyhow::Result<String> {
    let key = m
        .get_one::<String>("key")
        .map(|k| k.trim().to_ascii_uppercase())
        .unwrap_or_default();
    if !config::is_ticket_key(&key) {
        anyhow::bail!("\"{key}\" is not a ticket key like ABC-12");
    }
    let project = key.split('-').next().unwrap_or_default().to_string();
    let mut sites = cli_sites(&cfg, m)?;
    // The site that watches the project first.
    sites.sort_by_key(|i| !cfg.sites[*i].watches(&project));
    let mut last = None;
    for i in sites {
        let site = &cfg.sites[i];
        let c = Client::for_site(site)?;
        match c.detail(&key, site.ac_field(&project).map(str::to_string)).await {
            Ok(d) => return Ok(page::as_text(site, &d, m.get_flag("json"))),
            Err(e) => last = Some(anyhow::anyhow!("{}: {e:#}", site.name)),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no site to ask")))
}

/// A key as the event a test sends the host.
#[cfg(test)]
pub(crate) fn key_event(chord: Chord) -> radar_core::host::Event {
    radar_core::host::Event::Intent(radar_core::view::Intent::Key { chord })
}

#[cfg(any(test, feature = "demo"))]
mod demo;
#[cfg(test)]
mod tests;
