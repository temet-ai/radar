//! radar-slack: Slack as a feed into radar. Reads only, except the review asks,
//! replies and reactions you confirm.
//!
//! Contributes the `slack` screen (Shift+S): a nav pane of For you, Discussions
//! and each watched channel with what is new │ the messages with their
//! threads folded under them │ a detail pane; `⏎` opens a thread's page. It
//! owns `thread`s (and a review ask being drafted), recognises its
//! permalinks, and adds to the other parts: a "Discussed in Slack" section
//! and a chip on merge requests, "ask for review…" on them, and a Slack tab
//! on tickets. Settings sections `connection`, `channels`, `review`,
//! `alerts` and `look`; the `[slack]` config table; the `slack.*` colour slots;
//! a cache (with the caught-up marks); `radar slack catchup`.

mod palette;
mod alert_view;
mod agent_context;
mod alerts;
mod ask;
mod ask_context;
mod cache;
mod client;
mod compact;
mod cli_history;
#[cfg(test)]
use cli_history::{catchup, list};
mod config;
mod health;
mod join;
mod model;
mod mrkdwn;
mod page;
mod read;
mod read_context;
mod reaction_context;
mod reply_context;
mod screen;
mod manifest;
mod settings;
mod settings_history;
mod setup;
mod sync_history;
mod view;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};

use radar_core::design::help_bar::KeyHint;
use radar_core::input::TextInput;
use radar_core::journal::{ActionDescriptor, Origin, Outcome as JournalOutcome, SubjectKey, SubjectRole, SubjectSnapshot};
use radar_core::ui::composer::Composer;
use radar_core::writes::{self, Service};
use radar_core::plugin::{Binding, CliTask, Effect, MenuItem, Msg, Outcome, Pane, ScreenSpec, SectionSpec, SlotValue};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::ratatui::style::Color;
use radar_core::ui::Scroll;
use radar_core::{Plugin, log, time};
use serde_json::Value;

use client::{ChannelInfo, Client, Me, Message};
pub use config::Settings;
use model::{Data, ts_key};

pub(crate) const SCREEN: ScreenSpec = ScreenSpec {
    id: "feed",
    name: "slack",
    key: 'S',
};

const SECTIONS: [SectionSpec; 5] = [
    SectionSpec {
        id: radar_core::ui::connection::SECTION,
        // The workspace: every part's first section is its connection.
        name: radar_core::ui::connection::SECTION,
        group: "slack",
    },
    SectionSpec {
        id: "channels",
        name: "channels",
        group: "slack",
    },
    SectionSpec {
        id: "review",
        name: "review",
        group: "slack",
    },
    SectionSpec {
        id: "alerts",
        name: "alerts",
        group: "slack",
    },
    SectionSpec {
        id: "look",
        name: "look",
        group: "slack",
    },
];

/// The icon beside a settings section in the nav.
fn section_icon(section: &str) -> Option<radar_core::ui::icon::Icon> {
    use radar_core::ui::icon::Icon;
    Some(match section {
        "connection" => Icon::Connection,
        "channels" => Icon::Channel,
        "review" => Icon::NeedsApproval,
        "alerts" => Icon::Alert,
        "look" => Icon::Look,
        _ => return None,
    })
}

/// Default channel colours, overridable as `slack.<name>` in
/// `[theme.slots]`: name, truecolor, xterm-256 index, ANSI-16 colour.
type ChannelSlot = (&'static str, (u8, u8, u8), u8, Color);

const CHANNEL_SLOTS: [ChannelSlot; 7] = [
    ("blue", (0x82, 0xaa, 0xff), 111, Color::Blue),
    ("cyan", (0x80, 0xff, 0xea), 122, Color::Cyan),
    ("green", (0x50, 0xfa, 0x7b), 84, Color::Green),
    ("magenta", (0xff, 0x6a, 0xc1), 205, Color::Magenta),
    ("purple", (0xe1, 0x35, 0xff), 171, Color::LightMagenta),
    ("yellow", (0xf1, 0xfa, 0x8c), 228, Color::Yellow),
    ("orange", (0xff, 0xb8, 0x6c), 215, Color::LightRed),
];

/// The key that refreshes: reads Slack now, or the open thread again.
pub(crate) const FETCH_KEY: char = 'r';
/// Slack, as writes name it. Every write to Slack adds (a post, a reply, a
/// reaction): write mode sends it, read only does not.
pub(crate) const SERVICE: Service = Service::new("Slack");

pub(crate) enum SlackMsg {
    /// One part of a read (see `read.rs`).
    Read {
        stamp: read_context::ReadStamp,
        read: Box<read::Read>,
    },
    /// A thread a page asked for.
    Thread {
        stamp: read_context::ReadStamp,
        channel: String,
        ts: String,
        action: ActionDescriptor,
        result: Result<client::MessagesRead, String>,
    },
    /// The settings auth check, with its own unscoped dispatch identity.
    WorkspaceChecked {
        stamp: read_context::CheckStamp,
        action: ActionDescriptor,
        result: Result<Me, String>,
    },
    /// A channel setting checked against Slack.
    Checked {
        stamp: read_context::CheckStamp,
        key: String,
        /// Actual direct-ID dispatch target; name lookups have no expected ID.
        /// Transient only, never copied into action history or the cache.
        requested_id: Option<String>,
        action: ActionDescriptor,
        evidence: client::ChannelEvidence,
        result: Result<Checked, String>,
    },
    /// Slack's user groups: to pick the review group from (`pick`), or to
    /// check a template's `{@handle}` against.
    Groups {
        stamp: read_context::CheckStamp,
        action: ActionDescriptor,
        result: Result<client::GroupsRead, String>,
        pick: bool,
    },
    /// Whether the Slack app runs here, asked before opening a message in
    /// it: its own link if so, else the web one.
    OpenIn { link: String, web: String, running: bool },
    /// A write's answer.
    Wrote {
        instance: String,
        what: Write,
        result: Result<client::Posted, String>,
    },
}

/// What a check found.
#[derive(Debug, Clone)]
pub(crate) enum Checked {
    Me(Me),
    Channel(ChannelInfo),
}

/// A write: what is sent, and what it was for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Write {
    Ask { operation: ask_context::AskOperation },
    Reply { operation: reply_context::ReplyOperation },
    React { operation: reaction_context::ReactionOperation },
}

/// Where Slack is reached: its API over HTTPS, unless a test put recorded
/// replies here.
#[derive(Default, Clone)]
pub(crate) struct Via(pub Option<std::sync::Arc<dyn client::Transport>>);

impl std::fmt::Debug for Via {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "Via(recorded)" } else { "Via(slack.com)" })
    }
}

/// Slack's Web API.
pub(crate) const SLACK_API: &str = config::DEFAULT_API;

/// Where a check of a setting stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Check {
    Running,
    Passed(String),
    Failed(String),
}

/// Read failures have their own scopes, separate from user-chosen aliases.
/// Transient only: neither these keys nor the sync state enter the cache.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SyncErrorKey {
    Bootstrap,
    Channel(String),
    ForYou,
    Followed,
}

#[derive(Debug, Default)]
pub(crate) struct Sync {
    /// Read tasks under way.
    pub reading: usize,
    pub started: Option<i64>,
    pub last_ok: Option<i64>,
    /// The last error per read scope; channel scopes retain their aliases.
    pub errors: BTreeMap<SyncErrorKey, String>,
    /// When the user groups were last read.
    pub groups_at: i64,
    /// Each channel read under way: from where it asked (`None`: its whole
    /// window) and when.
    pub channel_reads: BTreeMap<String, (Option<String>, i64)>,
    /// The For you read under way: whether it asks only for what is newer
    /// than what is held, and when it was asked.
    pub for_you_read: Option<(bool, i64)>,
    /// `r` asked: the next read takes every channel's whole window again.
    pub full_now: bool,
    /// radar started from the saved cache: the first read leaves out what it
    /// holds that was read less than a refresh ago.
    pub resumed: bool,
    /// Nothing was asked, all of it read at this time: what the read that
    /// asked nothing reports as its sync.
    pub held_at: Option<i64>,
}

/// Something fetched for a page: the last value, whether a fetch is under
/// way, and the last error.
#[derive(Debug)]
pub(crate) struct Fetched<T> {
    pub value: Option<T>,
    pub loading: bool,
    pub error: Option<String>,
}

impl<T> Default for Fetched<T> {
    fn default() -> Self {
        Self {
            value: None,
            loading: false,
            error: None,
        }
    }
}

/// What is being typed in settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Token,
    /// `[slack] api`.
    Api,
    Refresh,
    Days,
    ChannelAdd,
    Alias(usize),
    Purpose(usize),
    Template,
    Link,
    Reask,
    /// How many hours resolved alerts stay shown.
    Hours,
    /// A mapping rule's field, by the rule's place.
    RulePrefix(usize),
    RuleLabel(usize),
    RuleRepo(usize),
    RuleService(usize),
    RuleEnvironment(usize),
    /// A new rule: the start of the alert's name.
    RuleAdd,
    /// Where an alert no rule places belongs: typed against its name.
    MapUnmapped(String),
}

/// A reply being written in a thread.
#[derive(Debug)]
pub(crate) struct Replying {
    pub channel: String,
    pub thread: String,
    pub composer: Composer,
    pub stamp: ask_context::DraftStamp,
    pub read: read_context::ReadStamp,
    pub team: String,
    pub status: reply_context::ReplyStatus,
}

#[derive(Debug, Default)]
pub struct SlackPlugin {
    pub(crate) cfg: Settings,
    pub(crate) data: Data,
    pub(crate) account_cache: cache::AccountCache,
    /// Transient: never restored or serialized as authority.
    pub(crate) session_owner: Option<cache::AccountOwner>,
    /// The print of the token this session signed in with, saved beside
    /// its data (see `cache::token_print`).
    pub(crate) session_token: Option<String>,
    /// What is drawn is the saved data of the account last signed in with
    /// this token, not yet confirmed by Slack: shown as cached, never
    /// signed in.
    pub(crate) previewing: bool,
    pub(crate) sync: Sync,
    pub(crate) read_context: read_context::ReadContext,
    pub(crate) now: i64,
    // the screen
    pub(crate) nav_at: usize,
    pub(crate) tree: RefCell<radar_core::ui::tree::TreeState<String>>,
    pub(crate) opened_once: RefCell<HashSet<Vec<String>>>,
    /// What each row of the last frame's tree is, by its path.
    pub(crate) shown: RefCell<BTreeMap<Vec<String>, screen::Item>>,
    pub(crate) detail_scroll: Scroll,
    // pages
    pub(crate) pages: Vec<page::Page>,
    pub(crate) page_scroll: Scroll,
    /// Threads fetched for pages, by `channel\0ts`.
    pub(crate) threads: BTreeMap<String, Fetched<Vec<Message>>>,
    pub(crate) draft: Option<ask::Draft>,
    pub(crate) ask_context: ask_context::AskContext,
    /// A reply being written, in the kit's composer; it stays open while
    /// its question is asked.
    pub(crate) replying: Option<Replying>,
    pub(crate) reply_context: reply_context::ReplyContext,
    pub(crate) reaction_context: reaction_context::ReactionContext,
    /// The words `/` filters the lists by, and the input while they are typed.
    pub(crate) filter: String,
    pub(crate) filter_input: Option<TextInput>,
    /// What the thread page's Links tab lists, as of the last frame.
    pub(crate) page_links: RefCell<Vec<radar_core::Subject>>,
    /// The connection and thread those rendered links belong to.
    pub(crate) page_links_scope: RefCell<Option<page::LinkScope>>,
    // settings
    pub(crate) section_rows: [usize; 5],
    /// The connection section was last drawn as a setup step, without the
    /// rows only Settings needs; `ws_rows` follows it.
    pub(crate) setup_card: std::cell::Cell<bool>,
    pub(crate) section_scroll: Scroll,
    pub(crate) edit: Option<(Target, TextInput)>,
    /// Checks against Slack, by key: `workspace`, `channel:<id>`.
    pub(crate) checks: BTreeMap<String, (Check, i64)>,
    /// The environment the first-run offer looks in; `None` is this
    /// process's own (tests give their own).
    pub(crate) env: Option<Vec<(String, String)>>,
    /// This process's environment as [`SlackPlugin::vars`] first read it.
    pub(crate) own_env: std::sync::OnceLock<Vec<(String, String)>>,
    pub(crate) via: Via,
    /// What tasks that run together share: the calls in flight, a lookup
    /// each, a wait out of a rate limit (see `client::Pace`).
    pub(crate) pace: std::sync::Arc<client::Pace>,
    #[cfg(feature = "test-support")]
    pub(crate) offline_fixture: bool,
    /// Explicit public fixture constructors only; ordinary recorded unit-test
    /// transports retain their normal config/cache behavior.
    #[cfg(feature = "test-support")]
    pub(crate) scripted_fixture: bool,
    /// What each held thread links to and says (see `join::index`).
    pub(crate) index: RefCell<Option<Vec<join::Indexed>>>,
    /// What the alerts channels' held messages make (see `alerts::collect`),
    /// worked out when next asked after [`SlackPlugin::changed`].
    pub(crate) collected: RefCell<Option<alerts::Collected>>,
    /// Messages an alert reader refused that the log has said so of already
    /// (this run only).
    pub(crate) logged_refusals: HashSet<(String, String)>,
    /// An alert being mapped to a repo or service: what is typed.
    pub(crate) mapping: Option<Mapping>,
    /// Whether the Slack app runs here, when a test says; `None` asks the
    /// machine each time.
    pub(crate) app_running: Option<bool>,
}

/// An alert being mapped: its name, and what is typed (a repo's path, or a
/// service's name).
#[derive(Debug)]
pub(crate) struct Mapping {
    pub name: String,
    pub input: TextInput,
}

/// Whether the Slack desktop app is running on this machine (never over
/// SSH: the app that matters is on the other machine). Blocks: call it in a
/// task.
pub(crate) fn slack_app_running() -> bool {
    use std::process::{Command, Stdio};
    if radar_core::system::Env::detect().ssh {
        return false;
    }
    if cfg!(windows) {
        return Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq slack.exe", "/NH"])
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("slack.exe"));
    }
    // `slack` on Linux, `Slack` on the Mac.
    Command::new("pgrep")
        .args(["-i", "-x", "slack"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The Slack app's own link to a message: `slack://channel?team=T…&id=C…`,
/// with the message (and its thread, for a reply).
pub(crate) fn app_link(team: &str, channel: &str, ts: &str, thread: Option<&str>) -> String {
    let mut link = format!("slack://channel?team={team}&id={channel}&message={ts}");
    if let Some(t) = thread.filter(|t| *t != ts) {
        link.push_str(&format!("&thread_ts={t}"));
    }
    link
}

pub(crate) fn thread_key(channel: &str, ts: &str) -> String {
    format!("{channel}\0{ts}")
}

impl SlackPlugin {
    pub fn new() -> Self {
        let now = time::now_secs();
        Self {
            now,
            // The look opens on its first word, not a heading.
            section_rows: [0, 0, 0, 0, 1],
            ..Default::default()
        }
    }

    /// A client for the workspace: built here (no I/O), used in a task.
    pub(crate) fn client(&self) -> Result<Client, String> {
        #[cfg(feature = "test-support")]
        if self.offline_fixture { return Err("offline test fixture cannot contact Slack".into()); }
        if let Some(t) = &self.via.0 {
            let wait: client::Wait = std::sync::Arc::new(|_| Box::pin(async {}));
            return Ok(Client::new(t.clone()).with_wait(wait).with_pace(self.pace.clone()));
        }
        let token = self.cfg.resolved_token().map_err(|e| format!("{e:#}"))?;
        let api = self.cfg.resolved_api().map_err(|e| format!("{e:#}"))?;
        Client::http(&api, &token)
            .map(|c| c.with_pace(self.pace.clone()))
            .map_err(|e| format!("{e:#}"))
    }

    /// Injected fixture clients never inspect real credentials, including the
    /// cache-preview and keyboard paths which do not construct a client.
    pub(crate) fn session_credential(&self) -> anyhow::Result<String> {
        #[cfg(feature = "test-support")]
        if self.scripted_fixture || self.offline_fixture {
            return Ok("xoxp-offline-synthetic-fixture".into());
        }
        self.cfg.resolved_token()
    }

    /// The environment, as names and values (values never shown): this
    /// process's is read once, and only the names that could hold a Slack
    /// token are kept. The host asks whether the part is set up on every
    /// frame, and that asks here: reading and upper-casing the whole
    /// environment each time was nearly half of what a frame cost.
    pub(crate) fn vars(&self) -> &[(String, String)] {
        #[cfg(feature = "test-support")]
        if self.scripted_fixture || self.offline_fixture { return &[]; }
        match &self.env {
            Some(vars) => vars,
            None => self.own_env.get_or_init(|| {
                std::env::vars()
                    .filter(|(name, _)| {
                        let up = name.to_uppercase();
                        up.contains("SLACK") && up.contains("TOKEN")
                    })
                    .collect()
            }),
        }
    }

    /// User tokens in the environment, by name, not in use yet.
    pub(crate) fn offers(&self) -> Vec<config::Offer> {
        // Signed in: the token works, and another one is only noise.
        if self.data.me.is_some() {
            return vec![];
        }
        config::offers(self.vars(), &self.cfg.token)
    }

    /// The workspace's address, as Slack gave it: `https://acme.slack.com/`.
    pub(crate) fn workspace(&self) -> Option<&str> {
        self.data.me.as_ref().map(|m| m.url.as_str()).filter(|u| !u.is_empty())
    }

    /// How the kit fetches a picture in a message: with the token, which
    /// goes only to Slack's own hosts. Resolved as asked, never logged.
    pub(crate) fn pictures(&self) -> radar_core::ui::image::Fetch {
        #[cfg(feature = "test-support")]
        if self.offline_fixture || self.scripted_fixture {
            let via = self.via.clone();
            return radar_core::ui::image::fetch(move |_| {
                let via = via.clone();
                async move {
                    if let Some(transport) = via.0 { transport.blocked_external_io("picture fetch"); }
                    anyhow::bail!("offline test fixture cannot fetch pictures")
                }
            });
        }
        let token = self.cfg.resolved_token().ok();
        let api = self.cfg.resolved_api().unwrap_or_else(|_| SLACK_API.into());
        radar_core::ui::image::fetch(move |src: String| {
            let token = token.clone();
            let api = api.clone();
            async move { client::picture(&src, token.as_deref(), &api).await }
        })
    }

    /// A message's link in Slack, when the workspace is known.
    pub(crate) fn permalink(&self, channel: &str, ts: &str, thread: Option<&str>) -> Option<String> {
        Some(model::permalink(self.workspace()?, channel, ts, thread))
    }

    /// Open a message in Slack: in the desktop app when it is running here
    /// (its own `slack://` link, the web link should the app not take it),
    /// else its web link.
    pub(crate) fn open_in_slack(&self, channel: &str, ts: &str, thread: Option<&str>, web: String) -> Vec<Effect> {
        let team = self.data.me.as_ref().map(|m| m.team_id.clone()).unwrap_or_default();
        if team.is_empty() {
            return vec![Effect::Open(web)];
        }
        let link = app_link(&team, channel, ts, thread);
        #[cfg(feature = "test-support")]
        if self.offline_fixture || self.scripted_fixture {
            // Produce the normal host-owned effect without process discovery.
            return self.opened_in(link, web, self.app_running.unwrap_or(false));
        }
        match self.app_running {
            Some(running) => self.opened_in(link, web, running),
            None => vec![Effect::Spawn(Box::pin(async move {
                let running = tokio::task::spawn_blocking(slack_app_running).await.unwrap_or(false);
                Box::new(SlackMsg::OpenIn { link, web, running }) as Msg
            }))],
        }
    }

    fn opened_in(&self, link: String, web: String, running: bool) -> Vec<Effect> {
        if running {
            vec![Effect::OpenApp { link, web }]
        } else {
            vec![Effect::Open(web)]
        }
    }

    /// The alias of a channel by id, else its name, else the id.
    pub(crate) fn alias(&self, channel: &str) -> String {
        if let Some(c) = self.cfg.by_id(channel) {
            return c.alias.clone();
        }
        self.data
            .channels
            .get(channel)
            .and_then(|c| c.info.as_ref())
            .map(|i| i.name.clone())
            .unwrap_or_else(|| channel.to_string())
    }

    /// The colour slot of a channel.
    pub(crate) fn colour(&self, channel: &str) -> String {
        format!(
            "slack.{}",
            self.cfg.by_id(channel).map_or("blue", |c| c.color.as_str())
        )
    }

    /// Read now (`r`), unless a read is under way.
    pub(crate) fn fetch_now(&mut self) -> Vec<Effect> {
        if self.sync.reading > 0 {
            return vec![Effect::Notify("already reading Slack".into())];
        }
        self.sync.full_now = true;
        self.read()
    }

    /// Move a channel's caught-up mark to `ts` (never back past it unless
    /// asked). Local only: nothing is sent to Slack.
    pub(crate) fn catch_up(&mut self, channel: &str, ts: &str) -> Vec<Effect> {
        if !self.authenticated() { return vec![Effect::Notify("check the Slack connection before moving caught-up marks".into())]; }
        self.data.marks.insert(channel.to_string(), ts.to_string());
        vec![
            Effect::PersistCache,
            Effect::Notify(format!(
                "#{} caught up to {}",
                self.alias(channel),
                view::when(ts)
            )),
        ]
    }

    /// The newest ts held in a channel, replies included.
    pub(crate) fn newest(&self, channel: &str) -> Option<String> {
        self.data
            .threads(channel)
            .iter()
            .map(|t| t.last().to_string())
            .max_by_key(|t| ts_key(t))
    }

    /// Whose thread it is, by its first message's author.
    pub(crate) fn thread_author(&self, channel: &str, thread: &str) -> Option<String> {
        if let Some(th) = self.data.thread(channel, thread) {
            return Some(self.data.author(th.parent));
        }
        self.threads
            .get(&thread_key(channel, thread))
            .and_then(|f| f.value.as_ref())
            .and_then(|msgs| msgs.iter().find(|m| m.ts == thread))
            .or_else(|| {
                // A message found for you that starts the thread.
                self.data
                    .for_you
                    .iter()
                    .find(|f| f.channel_id == channel && f.message.ts == thread)
                    .map(|f| &f.message)
            })
            .map(|m| self.data.author(m))
    }

    /// Where a reply goes, in words: `#team, in Bob's thread`.
    pub(crate) fn reply_place(&self, channel: &str, thread: &str) -> String {
        let alias = self.alias(channel);
        match self.thread_author(channel, thread) {
            Some(who) => format!("#{alias}, in {who}'s thread"),
            None => format!("#{alias}, in the thread"),
        }
    }

    /// What a write does, in words: `post a reply in #team, in Bob's thread`.
    pub(crate) fn doing(&self, what: &Write) -> String {
        match what {
            Write::Ask { operation } => format!("post the review ask in #{}", self.alias(&operation.payload.channel)),
            Write::Reply { operation } => format!("post a reply in {}", self.reply_place(&operation.payload.channel, &operation.payload.thread)),
            Write::React { operation } => format!("react :{}: in #{}", operation.payload.name, self.alias(&operation.payload.channel)),
        }
    }

    /// Stable operation identifiers only; no rendered post, reaction text or URLs.
    pub(crate) fn journal_write(&self, operation: &'static str, channel: &str, thread: Option<&str>) -> Effect {
        let mut action = ActionDescriptor::new(operation, Origin::Plugin("slack".into()));
        let instance = self.data.me.as_ref().map(|me| me.team_id.as_str()).unwrap_or_default();
        let id = thread.map_or_else(|| channel.to_string(), |ts| format!("{channel}/{ts}"));
        let kind = if thread.is_some() { "message" } else { "channel" };
        if let Ok(key) = SubjectKey::new("slack", instance, kind, &id) {
            action = action.with_subject(SubjectSnapshot::new(key, SubjectRole::Primary));
        } else {
            action.metadata.redacted = true;
            action.metadata.omitted_subjects = 1;
        }
        let root = match (operation, thread) {
            ("slack.reply.post", Some(root)) => Some(root),
            ("slack.reaction.add", Some(ts)) => self.known_thread_of(channel, ts),
            _ => None,
        };
        if let Some(root) = root {
            if let Ok(key) = SubjectKey::new("slack", instance, "thread", &format!("{channel}/{root}")) {
                action = action.with_subject(SubjectSnapshot::new(key, SubjectRole::Context));
            } else {
                action.metadata.redacted = true;
                action.metadata.omitted_subjects += 1;
            }
        }
        Effect::Journal(action)
    }

    /// Send a write, once. With writes switched off it says so and sends
    /// nothing.
    pub(crate) fn write(&mut self, what: Write) -> Vec<Effect> {
        let mut fx = vec![Effect::Report(JournalOutcome::Confirmed)];
        if let Write::Ask { operation } = &what
            && !self.claim_ask(operation)
        {
            return vec![Effect::Report(JournalOutcome::Blocked), Effect::Notify("that review ask is no longer ready to send; preview it again".into())];
        }
        if let Write::Reply { operation } = &what
            && !self.claim_reply(operation)
        {
            return vec![Effect::Report(JournalOutcome::Blocked), Effect::Notify("that reply is no longer ready to send; preview it again".into())];
        }
        if let Write::React { operation } = &what
            && !self.claim_reaction(operation)
        {
            return vec![Effect::Report(JournalOutcome::Blocked), Effect::Notify("that reaction is no longer ready to send; choose it again".into())];
        }
        let c = match self.client() {
            Ok(c) => c,
            Err(e) => {
                fx.extend(self.finish_write(what, Err(e), JournalOutcome::Failed));
                return fx;
            }
        };
        if let Some(why) = writes::not_sent(writes::Change::Add) {
            fx.extend(self.finish_write(what, Err(why), JournalOutcome::Blocked));
            return fx;
        }
        let instance = self.data.me.as_ref().map(|me| me.team_id.clone()).unwrap_or_default();
        fx.extend([
            Effect::Report(JournalOutcome::Started),
            Effect::Notify(SERVICE.asking(&self.doing(&what))),
            Effect::Spawn(Box::pin(async move {
                let result = send_write(&c, &what).await;
                match &result {
                    Ok(p) => log::info(format!("slack write {}: ok {}", p.channel, p.ts)),
                    Err(e) => log::warn(format!("slack write failed: {e}")),
                }
                Box::new(SlackMsg::Wrote { instance, what, result }) as Msg
            })),
        ]);
        fx
    }

    fn on_wrote(&mut self, what: Write, result: Result<client::Posted, String>, instance: &str) -> Vec<Effect> {
        // A timeout or lost response can follow a successful post. Never claim failure
        // from an unclassified string, and never journal the response or message text.
        let outcome = if result.is_ok() { JournalOutcome::Succeeded } else { JournalOutcome::Unknown };
        if let Write::Ask { operation } = &what
            && self.duplicate_ask(operation)
        {
            return self.duplicate_ask_effects(outcome);
        }
        if let Write::Reply { operation } = &what
            && self.duplicate_reply(operation)
        {
            return self.duplicate_reply_effects(outcome);
        }
        let posted_channel = match &what {
            Write::Ask { operation } => Some(&operation.payload.channel),
            Write::Reply { operation } => Some(&operation.payload.channel),
            Write::React { .. } => None,
        };
        let posted = match (posted_channel, &result) {
            (Some(channel), Ok(posted)) => {
                let mut action = ActionDescriptor::new("slack.message.post.result", Origin::Plugin("slack".into()));
                let channel = if posted.channel.is_empty() { channel } else { &posted.channel };
                let id = format!("{channel}/{}", posted.ts);
                if !posted.ts.is_empty()
                    && let Ok(key) = SubjectKey::new("slack", instance, "message", &id)
                {
                    action = action.with_subject(SubjectSnapshot::new(key, SubjectRole::Result));
                } else {
                    action.metadata.redacted = true;
                    action.metadata.omitted_subjects = 1;
                }
                let root = match &what {
                    Write::Reply { operation } => operation.payload.thread.as_str(),
                    _ => posted.ts.as_str(), // A review ask creates a top-level message.
                };
                if !root.is_empty()
                    && let Ok(key) = SubjectKey::new("slack", instance, "thread", &format!("{channel}/{root}"))
                {
                    action = action.with_subject(SubjectSnapshot::new(key, SubjectRole::Context));
                } else {
                    action.metadata.redacted = true;
                    action.metadata.omitted_subjects += 1;
                }
                Some(action)
            }
            _ => None,
        };
        let mut fx = self.finish_write(what, result, outcome);
        if let Some(action) = posted {
            fx.splice(1..1, [Effect::Journal(action), Effect::Report(JournalOutcome::Succeeded)]);
        }
        fx
    }

    fn finish_write(&mut self, what: Write, result: Result<client::Posted, String>, outcome: JournalOutcome) -> Vec<Effect> {
        if let Write::Ask { operation } = what {
            return self.finish_ask(operation, result, outcome);
        }
        if let Write::Reply { operation } = what {
            return self.finish_reply(operation, result, outcome);
        }
        let Write::React { operation } = what else { unreachable!("post completion handled above") };
        self.finish_reaction(operation, result, outcome)
    }

    /// The thread a message is in (its own ts when it starts one).
    pub(crate) fn thread_of(&self, channel: &str, ts: &str) -> String {
        self.known_thread_of(channel, ts).unwrap_or(ts).to_string()
    }

    /// Unlike the UI fallback, a journal association requires an observed root.
    fn known_thread_of<'a>(&'a self, channel: &str, ts: &'a str) -> Option<&'a str> {
        if let Some(c) = self.data.channels.get(channel) {
            if c.messages.iter().any(|m| m.ts == ts) {
                return Some(ts);
            }
            for (t, replies) in &c.replies {
                if replies.iter().any(|m| m.ts == ts) {
                    return Some(t);
                }
            }
        }
        for (k, f) in &self.threads {
            if let Some((ch, t)) = k.split_once('\0')
                && ch == channel
                && f.value.as_ref().is_some_and(|v| v.iter().any(|m| m.ts == ts))
            {
                return Some(t);
            }
        }
        None
    }

    /// The `x` rows for keys every place on the screen shares.
    fn shared_keys(&self) -> Vec<Row> {
        let surface = surface(self);
        let mut b = vec![(format!("slack.{surface}.kept_replies"), Binding::new("ctrl-r", "kept replies…"))];
        if self.session_credential().is_ok() {
            b.push((format!("slack.{surface}.refresh"), Binding::new(
                FETCH_KEY.to_string(),
                if self.pages.is_empty() { "refresh" } else { "refresh this page" },
            )));
            if self.pages.is_empty() {
                b.push(("slack.feed.filter".into(), Binding::new("/", "filter the list by words")));
            }
        }
        b
    }

    fn shared_key(&mut self, key: KeyEvent) -> Option<Outcome> {
        if key.modifiers != KeyModifiers::NONE || self.session_credential().is_err() {
            return None;
        }
        match key.code {
            KeyCode::Char(FETCH_KEY) => Some(Outcome::Handled(match self.pages.last().cloned() {
                // On a thread page `r` reads that thread again.
                Some(page::Page::Thread { channel, ts, .. }) => page::fetch_thread(self, &channel, &ts),
                _ => self.fetch_now(),
            })),
            _ => None,
        }
    }

    /// What is held or which channels are watched changed: the index of
    /// threads is built again when next asked.
    pub(crate) fn changed(&self) {
        self.index.replace(None);
        self.collected.replace(None);
    }

    /// The alerts read from the alerts channels, and what no reader could
    /// read; built on first use after [`SlackPlugin::changed`].
    pub(crate) fn alerts(&self) -> std::cell::Ref<'_, alerts::Collected> {
        if self.collected.borrow().is_none() {
            *self.collected.borrow_mut() = Some(alerts::collect(&self.data, &self.cfg));
        }
        std::cell::Ref::map(self.collected.borrow(), |c| c.as_ref().expect("built above"))
    }

    /// Say in the log, once a run, of each bot message an alert reader
    /// refused: which, by what reader, and why. It never guesses.
    pub(crate) fn log_refusals(&mut self) {
        let fresh: Vec<(String, String, String)> = self
            .alerts()
            .refused
            .iter()
            .filter(|(c, ts, _)| !self.logged_refusals.contains(&(c.clone(), ts.clone())))
            .cloned()
            .collect();
        for (channel, ts, why) in fresh {
            let reader = self.cfg.by_id(&channel).and_then(|c| c.reader).map_or("", config::Reader::word);
            log::warn(format!(
                "slack #{}: message {ts} not read as an alert by the {reader} reader: {why}",
                self.alias(&channel)
            ));
            self.logged_refusals.insert((channel, ts));
        }
    }

    /// After anything that may change what is held or watched.
    pub(crate) fn clamp(&mut self) {
        let n = screen::nav_rows(self).len();
        self.nav_at = self.nav_at.min(n.saturating_sub(1));
        self.changed();
    }
}

/// Send one write through Slack's API, once.
pub(crate) async fn send_write(c: &Client, what: &Write) -> Result<client::Posted, String> {
    match what {
        Write::Ask { operation } => c.post(&operation.payload.channel, &operation.payload.text, None).await,
        Write::Reply { operation } => c.post(&operation.payload.channel, &operation.payload.text, Some(&operation.payload.thread)).await,
        Write::React { operation } => c.react(&operation.payload.channel, &operation.payload.ts, &operation.payload.name).await.map(|()| client::Posted {
            channel: operation.payload.channel.clone(),
            ts: operation.payload.ts.clone(),
        }),
    }
    .map_err(|e| format!("{e:#}"))
}

/// The surface a screen's or page's actions belong to: `feed`, a thread
/// `page`, the review-ask `draft`, or a stand-in's own screen.
pub(crate) fn surface(p: &SlackPlugin) -> &'static str {
    match p.pages.last() {
        None => "feed",
        Some(page::Page::Draft) => "draft",
        Some(_) => "page",
    }
}

/// A row of the actions menu with the id of the action it runs. The ids are
/// written out where the row is made, never made from its key or its words.
pub(crate) type Row = (String, Binding);

pub(crate) fn act(id: &str, b: Binding) -> Row {
    (id.to_string(), b)
}

/// A heading row: it runs nothing, so it has no id.
pub(crate) fn head(b: Binding) -> Row {
    (String::new(), b)
}

pub(crate) fn actions(rows: &[Row]) -> Vec<radar_core::view::ActionSpec> {
    rows.iter().map(|(id, b)| radar_core::view::ActionSpec::named(id.as_str(), b)).collect()
}

impl SlackPlugin {
    /// The actions menu rows for a pane, each with its id.
    pub(crate) fn rows(&self, pane: Pane) -> Vec<Row> {
        let mut b = if self.pages.is_empty() {
            screen::rows(self, pane)
        } else {
            page::rows(self)
        };
        let at = b
            .iter()
            .rposition(|(_, r)| r.keys == "esc")
            .filter(|i| i + 1 == b.len())
            .unwrap_or(b.len());
        let mut shared = self.shared_keys();
        // On a list the keys of the whole screen are their own group, apart
        // from the writes above them; a page's sit under its `page` heading.
        if self.pages.is_empty() && !shared.is_empty() {
            shared.insert(0, head(Binding::heading("list")));
        }
        b.splice(at..at, shared);
        b.splice(at..at, agent_context::rows(self, pane));
        b
    }

    /// An invoked action id is the key that runs it: look the id up in the
    /// actions the view offers here and answer the intent for that key.
    fn resolve_action(&self, scope: &str, pane: Pane, intent: &radar_core::view::Intent) -> Option<radar_core::view::Intent> {
        use radar_core::view as v;
        let v::Intent::Invoke { action } = intent else { return None };
        let offered = if scope == SCREEN.id {
            actions(&self.rows(pane))
        } else {
            settings::action_specs(self, scope)
        };
        let spec = offered.into_iter().find(|a| a.id == *action)?;
        spec.key.map(|chord| v::Intent::Key { chord })
    }

    fn handle_key(&mut self, pane: Pane, key: KeyEvent) -> Outcome {
        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('r') {
            return Outcome::Handled(self.kept_replies());
        }
        if self.replying.is_some() {
            return page::reply_key(self, key);
        }
        if self.filter_input.is_some() {
            return screen::filter_key(self, key);
        }
        if self.mapping.is_some() {
            let out = alert_view::map_key(self, key);
            // A new rule changes where alerts belong.
            self.clamp();
            return out;
        }
        if !self.capturing()
            && let Some(out) = agent_context::key_press(self, pane, key)
        {
            return out;
        }
        if !self.capturing()
            && let Some(out) = self.shared_key(key)
        {
            return out;
        }
        let out = if self.pages.is_empty() {
            screen::key(self, pane, key)
        } else {
            page::key(self, key)
        };
        self.clamp();
        out
    }
    #[cfg(test)]
    fn key(&mut self, _: &str, pane: Pane, key: KeyEvent) -> Outcome {
        self.handle_key(pane, key)
    }
}

impl Plugin for SlackPlugin {
    fn id(&self) -> &'static str {
        "slack"
    }
    #[cfg(feature = "demo")]
    fn demo(&mut self) {
        #[cfg(feature = "test-support")]
        let (offline, scripted, via) = (self.offline_fixture, self.scripted_fixture, self.via.clone());
        *self = demo::sample();
        #[cfg(feature = "test-support")]
        { self.offline_fixture = offline; self.scripted_fixture = scripted; self.via = via; }
    }
    fn name(&self) -> &'static str {
        "Slack"
    }
    fn icon(&self, _screen: &str) -> Option<radar_core::ui::icon::Icon> {
        Some(radar_core::ui::icon::Icon::Chat)
    }
    fn section_icon(&self, section: &str) -> Option<radar_core::ui::icon::Icon> {
        section_icon(section)
    }
    fn set_up(&self) -> radar_core::Setup {
        if self.cfg.token.trim().is_empty() {
            let mut text = "Add your workspace token: a user token (xoxp-…), as env:NAME or typed in.".to_string();
            if let Some(o) = self.offers().first() {
                text.push_str(&format!(" radar found {} holding a user token; workspace offers it.", o.var));
            }
            radar_core::Setup::Needed(text)
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
    fn journal_subject(&self, subject: &radar_core::Subject) -> Option<SubjectSnapshot> {
        use radar_core::journal::SafeUrl;
        if !subject.is(join::THREAD) {
            return None;
        }
        let (channel, ts) = join::thread_target(self, subject)?;
        let me = self.data.me.as_ref()?;
        let url = SafeUrl::parse(&self.permalink(channel, ts, None)?).ok()?;
        if SafeUrl::parse(subject.url.as_deref()?).ok()? != url {
            return None;
        }
        let key = SubjectKey::new("slack", &me.team_id, join::THREAD, &subject.id).ok()?;
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
        self.pages.last()?.identity(self.draft.as_ref())
    }
    fn tabs(&self, _: &str) -> Option<radar_core::plugin::PageTabs> {
        page::page_tabs(self)
    }
    fn show_tab(&mut self, _: &str, i: usize) -> Vec<Effect> {
        page::show_tab(self, i)
    }
    fn subject(&self, _: &str, pane: Pane) -> Option<radar_core::Subject> {
        join::subject(self, pane)
    }
    fn adds(&self) -> Vec<radar_core::Adds> {
        join::ADDS.to_vec()
    }
    fn contribute(&self, point: radar_core::Point, subject: &radar_core::Subject) -> Option<radar_core::Contribution> {
        join::contribute(self, point, subject)
    }
    fn act(&mut self, subject: &radar_core::Subject, action: &str) -> Vec<Effect> {
        join::act(self, subject, action)
    }

    fn configure(&mut self, table: Option<&toml::Value>) -> anyhow::Result<()> {
        let cfg: Settings = match table {
            Some(v) => v.clone().try_into()?,
            None => Settings::default(),
        };
        self.replace_token(cfg.token.clone());
        self.cfg = cfg;
        self.clamp();
        self.draw_cached();
        Ok(())
    }

    fn config(&self) -> Option<toml::Value> {
        toml::Value::try_from(&self.cfg).ok()
    }

    fn first_run(&mut self) -> Option<String> {
        Some(match self.offers().first() {
            Some(o) => format!("Slack: found {} holding a user token; use it under connection", o.var),
            None => "Slack: add your workspace token under connection".into(),
        })
    }

    fn slots(&self) -> Vec<(String, SlotValue)> {
        CHANNEL_SLOTS
            .iter()
            .map(|&(name, rgb, i256, c16)| (format!("slack.{name}"), SlotValue::new(rgb, i256, c16)))
            .collect()
    }

    fn restore(&mut self, cache: Value) {
        self.revoke_session();
        self.account_cache = cache::AccountCache::restore(cache);
        self.sync.resumed = true;
        self.draw_cached();
    }

    fn snapshot(&self) -> Option<Value> {
        // Synthetic test data must never mint a verified production partition.
        #[cfg(feature = "test-support")]
        if self.offline_fixture { return Some(serde_json::json!({"data": self.data})); }
        Some(self.cache_snapshot().snapshot())
    }

    fn tick(&mut self, now: i64) -> Vec<Effect> {
        self.now = now;
        // The host pauses every part (`z`, `pause if idle`) by not ticking.
        let due = self
            .sync
            .started
            .is_none_or(|t| now - t >= self.cfg.refresh_secs as i64);
        if self.sync.reading == 0 && due {
            let mut fx = self.read();
            // An open thread page is read again with the cycle.
            if let Some(page::Page::Thread { channel, ts, .. }) = self.pages.last().cloned() {
                fx.extend(page::fetch_thread(self, &channel, &ts));
            }
            return fx;
        }
        vec![]
    }

    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        let Ok(msg) = msg.downcast::<SlackMsg>() else {
            return vec![];
        };
        let fx = match *msg {
            SlackMsg::Read { stamp, read } => {
                if !self.is_current_read(stamp) {
                    return read.stale_effects();
                }
                let held = self.threads();
                let mut fx = self.on_read(*read);
                // What the read changed, for the rows to glow (motion).
                let changes = radar_core::motion::Changes::between(held, self.threads());
                if !changes.is_empty() {
                    fx.push(Effect::Changed(changes));
                }
                fx
            }
            SlackMsg::Thread {
                stamp,
                channel,
                ts,
                action,
                result,
            } => {
                let mut fx = sync_history::page_thread(&action, &result);
                if !self.is_current_read(stamp) {
                    fx.push(read_context::skipped(Some(&action)));
                    return fx;
                }
                page::on_thread(self, &channel, &ts, result.map(|got| got.messages));
                fx.push(Effect::PersistCache);
                fx
            }
            SlackMsg::WorkspaceChecked { stamp, action, result } => {
                let mut fx = settings_history::workspace(&action, &result);
                if !self.complete_check("workspace", stamp) {
                    let scope = settings_history::workspace_scope(&action, &result);
                    fx.push(read_context::skipped(Some(&scope)));
                    return fx;
                }
                fx.extend(settings::on_workspace_checked(self, stamp, result));
                fx
            }
            SlackMsg::Checked {
                stamp,
                key,
                requested_id,
                action,
                evidence,
                result,
            } => {
                let mut fx = settings_history::channel(&action, &evidence);
                if !self.complete_check(&key, stamp) {
                    fx.push(read_context::skipped(Some(&action)));
                    return fx;
                }
                fx.extend(settings::on_channel_checked(
                    self,
                    key,
                    requested_id.as_deref(),
                    &action,
                    result,
                ));
                fx
            }
            SlackMsg::Groups {
                stamp,
                action,
                result,
                pick,
            } => {
                let mut fx = settings_history::groups(&action, &result);
                if !self.complete_check("groups", stamp) {
                    fx.push(read_context::skipped(Some(&action)));
                    return fx;
                }
                fx.extend(settings::on_groups(self, result.map(|got| got.groups), pick));
                fx
            }
            SlackMsg::Wrote { instance, what, result } => self.on_wrote(what, result, &instance),
            SlackMsg::OpenIn { link, web, running } => self.opened_in(link, web, running),
        };
        // Accepted answers may change the text or links. Until the next frame
        // derives them again, no action may use the previous rendered rows.
        page::invalidate_links(self);
        self.clamp();
        fx
    }

    fn capturing(&self) -> bool {
        self.edit.is_some()
            || self.replying.is_some()
            || self.filter_input.is_some()
            || self.mapping.is_some()
            || self.draft.as_ref().is_some_and(|d| d.edit.is_some())
    }

    fn paste(&mut self, text: &str) -> Vec<Effect> {
        if let Some(m) = self.mapping.as_mut() {
            m.input.paste(text);
        } else if let Some(input) = self.filter_input.as_mut() {
            input.paste(text);
            self.filter = input.value();
        } else if let Some(r) = self.replying.as_mut() {
            let before = r.composer.text();
            r.composer.paste(text);
            if r.composer.text() != before { r.stamp.changed(); }
        } else if let Some(draft) = self.draft.as_mut().filter(|d| d.edit.is_some()) {
            let input = draft.edit.as_mut().expect("editing draft");
            let before = input.value();
            input.paste(text);
            if input.value() != before { draft.changed(); }
        } else if let Some((_, input)) = self.edit.as_mut() {
            input.paste(text);
        }
        vec![]
    }

    fn screens(&self) -> Vec<ScreenSpec> {
        vec![SCREEN]
    }
    fn badge(&self, _: &str) -> Option<(String, radar_core::Tone)> {
        let n = screen::for_you_count(self);
        (n > 0).then(|| (n.to_string(), radar_core::Tone::Warn))
    }
    fn top_right(&self, _: &str) -> Option<String> {
        screen::top_right(self)
    }
    fn busy(&self, _: &str) -> bool {
        self.sync.reading > 0 || page::loading(self) || self.draft.as_ref().is_some_and(|d| d.sending.is_some())
    }
    fn status(&self, _: &str) -> Vec<(String, radar_core::Tone)> {
        screen::status(self)
    }
    fn health(&self) -> Option<radar_core::health::Health> {
        health::health(self)
    }
    fn refresh_now(&mut self) -> Vec<Effect> {
        self.fetch_now()
    }
    fn view(&self, screen: &str, cx: &radar_core::view::ViewCx<'_>) -> Option<radar_core::view::ScreenView> {
        if screen != SCREEN.id { return None; }
        if self.pages.is_empty() { Some(screen::view(self, cx)) } else { page::view(self, cx) }
    }
    fn pictures(&self, _source: &str) -> Option<radar_core::ui::image::Fetch> {
        Some(self.pictures())
    }
    fn section_view(&self, section: &str, cx: &radar_core::view::ViewCx<'_>) -> Option<radar_core::view::SectionView> {
        if section == "connection" {
            self.setup_card.set(cx.setup);
        }
        match section {
            "look" => Some(settings::look_view(self)),
            "connection" => Some(settings::connection_view(self, cx.width, cx.now)),
            "channels" => Some(settings::channels_view(self, cx.width, cx.now)),
            "alerts" => Some(settings::alerts_view(self, cx.width)),
            "review" => Some(settings::review_view(self)),
            _ => None,
        }
    }
    fn intent(&mut self, scope: &str, pane: Pane, intent: &radar_core::view::Intent) -> Outcome {
        let resolved = self.resolve_action(scope, pane, intent);
        let intent = resolved.as_ref().unwrap_or(intent);
        if scope == SCREEN.id && self.pages.is_empty() {
            return screen::intent(self, pane, intent);
        }
        if scope == SCREEN.id { return page::intent(self, pane, intent); }
        if !matches!(scope, "look" | "connection" | "channels" | "alerts" | "review") { return Outcome::Ignored; }
        if intent.chord().is_some_and(|chord| chord.ctrl && chord.key == radar_core::view::Key::Char('r')) {
            return Outcome::Handled(self.kept_replies_in_settings());
        }
        let out = settings::list_intent(self, scope, intent);
        self.clamp();
        out
    }
    fn panes(&self, _: &str) -> Vec<Pane> {
        if self.pages.is_empty() {
            vec![Pane::Nav, Pane::Middle, Pane::Detail]
        } else {
            vec![Pane::Middle]
        }
    }
    fn hints(&self, _: &str, _: Pane) -> Vec<KeyHint<'static>> {
        if let Some(r) = &self.replying {
            let mut hints = r.composer.hints();
            hints.push(KeyHint::new("ctrl-r", "kept replies"));
            return hints;
        }
        if self.mapping.is_some() {
            return vec![KeyHint::new("⏎", "map it"), KeyHint::new("esc", "cancel")];
        }
        if self.filter_input.is_some() {
            return vec![KeyHint::new("⏎", "keep the words"), KeyHint::new("esc", "clear them")];
        }
        if self.draft.as_ref().is_some_and(|d| d.edit.is_some()) {
            return vec![KeyHint::new("⏎", "keep the words"), KeyHint::new("esc", "cancel")];
        }
        vec![]
    }
    /// `+`/`-` fold the feed's groups and threads.
    fn folds(&self, _: &str, pane: Pane) -> bool {
        self.pages.is_empty() && pane == Pane::Middle
    }
    fn keys(&self, _: &str, pane: Pane) -> Vec<Binding> {
        self.rows(pane).into_iter().map(|(_, b)| b).collect()
    }

    fn menu_key_context(&self, scope: &str, _: Pane, action: &str) -> Option<String> {
        (scope == SCREEN.id)
            .then(|| page::menu_key_context(self, action))
            .flatten()
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
        let mut hints = if self.edit.is_some() {
            vec![KeyHint::new("⏎", "keep"), KeyHint::new("esc", "cancel")]
        } else { vec![] };
        if self.replying.is_some() || self.reply_context.usage().0 > 0 {
            hints.push(KeyHint::new("ctrl-r", "kept replies"));
        }
        hints
    }
    fn bulk(&self, scope: &str) -> Vec<MenuItem> {
        if scope == SCREEN.id && !self.pages.is_empty() {
            vec![]
        } else if scope == SCREEN.id {
            screen::bulk(self)
        } else {
            settings::bulk(self, scope)
        }
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        let fx = if menu == radar_core::host::BULK_MENU {
            match screen::on_bulk(self, value) {
                Some(fx) => fx,
                None => settings::on_bulk(self, value),
            }
        } else if let Some(fx) = page::on_menu(self, menu, value) {
            fx
        } else {
            settings::on_menu(self, menu, value)
        };
        self.clamp();
        fx
    }

    fn cli(&self) -> Option<clap::Command> {
        Some(
            clap::Command::new("slack")
                .about("Slack: what is new in the channels radar reads")
                .subcommand_required(true)
                .subcommand(clap::Command::new("cache-recovery").about("Export quarantined Slack cache JSON locally; never adopts or sends it"))
                .subcommand(
                    clap::Command::new("list")
                        .about("Print the threads of the watched channels, newest first (read only)")
                        .arg(
                            clap::Arg::new("view")
                                .long("view")
                                .help("Only this channel: its alias or its name in Slack"),
                        )
                        .arg(
                            clap::Arg::new("filter")
                                .long("filter")
                                .help("Words every row must contain (the TUI's / words)"),
                        )
                        .arg(
                            clap::Arg::new("json")
                                .long("json")
                                .action(clap::ArgAction::SetTrue)
                                .help("JSON output"),
                        ),
                )
                .subcommand(
                    clap::Command::new("catchup")
                        .about("Print what is new since you caught up, per watched channel (read only; marks are not moved)")
                        .arg(
                            clap::Arg::new("channel")
                                .help("Only this channel: its alias or its name in Slack"),
                        )
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
        let task = cli_history::run(self, matches);
        Box::pin(async move { task.await.result })
    }

    fn run_cli_observed(&self, matches: &clap::ArgMatches) -> radar_core::plugin::ObservedCliTask {
        cli_history::run(self, matches)
    }
}

#[cfg(any(test, feature = "demo", feature = "test-support"))]
mod demo;
#[cfg(test)]
mod alert_tests;
#[cfg(test)]
mod compact_tests;
#[cfg(test)]
mod stand_in;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod read_context_tests;

#[cfg(test)]
mod ask_context_tests;

#[cfg(test)]
mod reply_context_tests;

#[cfg(test)]
mod reaction_context_tests;

#[cfg(test)]
mod account_boundary_controls;

#[cfg(feature = "test-support")]
pub mod test_support;

#[cfg(test)]
mod cli_output_tests;
#[cfg(test)]
mod live_tests;
