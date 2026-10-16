//! The Slack settings sections: `connection` (id `workspace`: a token found
//! in the environment, the workspace, the token, who it signs in as, the
//! check, how often to read, and the permissions it has against what each
//! part of radar-slack needs), `channels` (the registry: alias, purpose, kind, colour,
//! watched), `review` (where asks go, the group, the template with a live
//! preview, the test channel, when to ask again) and `message look`. Every
//! row has an options menu (`⏎`) whose keys also work straight from the
//! row; the detail panel says what the row is, what it accepts and whether
//! it was checked.

use radar_core::design::theme::Slot;
use radar_core::input::{Edit, TextInput};
use radar_core::plugin::{Binding, Effect, Menu, MenuItem, Msg, Outcome, key_text};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent};
use radar_core::theme::slot_info;
use radar_core::ui::connection;
use radar_core::ui::rows::{Move, select};
use radar_core::view as v;
use radar_core::{log, time};

use crate::client::{ChannelEnd, ChannelEvidence, ChannelKind, UserGroup};
use crate::config::{self, CHANNEL_COLORS, Channel, Kind, Reader, Rule, default_styles};
use crate::view;
use crate::{Check, Checked, SlackMsg, SlackPlugin, Target, ask};

// --- what each part of radar-slack needs -----------------------------------------------

/// What radar-slack does, and the permissions (scopes) each needs. The app
/// manifest setup offers (`crate::manifest`) asks for exactly these.
pub(crate) const NEEDS: [(&str, &[&str], &str); 8] = [
    (
        "public channels",
        &["channels:history", "channels:read"],
        "reading the public channels you watch",
    ),
    (
        "private channels",
        &["groups:history", "groups:read"],
        "reading the private channels you watch",
    ),
    (
        "for you",
        &["search:read"],
        "finding mentions of you and your groups, and messages to you",
    ),
    ("names", &["users:read"], "showing who wrote what"),
    (
        "user groups",
        &["usergroups:read"],
        "your groups' mentions, and picking the review group",
    ),
    (
        "asking and replying",
        &["chat:write"],
        "posting review asks and replies as you, after you say yes",
    ),
    (
        "reacting",
        &["reactions:write"],
        "adding reactions as you, after you say yes",
    ),
    (
        "pictures",
        &["files:read"],
        "showing the pictures posted in messages",
    ),
];

fn missing(p: &SlackPlugin, need: &[&str]) -> Option<Vec<String>> {
    let me = p.data.me.as_ref()?;
    Some(
        need.iter()
            .filter(|s| !me.scopes.iter().any(|h| h == *s))
            .map(|s| s.to_string())
            .collect(),
    )
}

// --- rows --------------------------------------------------------------------------

/// A row of the connection section (id `workspace`), in the order every
/// part has (`radar_core::ui::connection`): what was found, where, the token,
/// who it signs in as, the check, refreshing; then Slack's own (how far
/// back to read, the permissions the token has).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WsRow {
    /// A variable in the environment holding a user token.
    Offer(usize),
    /// The workspace the token is for, as the check found it.
    Workspace,
    /// Where Slack's Web API is (`[slack] api`).
    Api,
    Token,
    SignedIn,
    Check,
    Refresh,
    Days,
    Need(usize),
    /// The gap before the channels: not selectable.
    Gap,
    /// `CHANNELS`: not selectable.
    ChannelsHeading,
    /// `+ add a channel`: the channels section's own add field.
    ChannelAdd,
    /// A channel in the registry.
    Channel(usize),
}

impl WsRow {
    /// Whether the cursor stops on it.
    pub(crate) const fn selectable(self) -> bool {
        !matches!(self, Self::Gap | Self::ChannelsHeading)
    }
}

pub(crate) fn ws_rows(p: &SlackPlugin) -> Vec<WsRow> {
    let mut rows: Vec<WsRow> = (0..p.offers().len()).map(WsRow::Offer).collect();
    // As a setup step the card leaves out api, refresh and read back, which
    // only Settings needs; the plugin remembers how the card was last drawn,
    // so keys and clicks walk what is drawn.
    let setup = p.setup_card.get();
    rows.push(WsRow::Workspace);
    if !setup {
        rows.push(WsRow::Api);
    }
    rows.extend([WsRow::Token, WsRow::SignedIn, WsRow::Check]);
    if !setup {
        rows.extend([WsRow::Refresh, WsRow::Days]);
    }
    rows.extend((0..NEEDS.len()).map(WsRow::Need));
    rows.extend([WsRow::Gap, WsRow::ChannelsHeading, WsRow::ChannelAdd]);
    rows.extend((0..p.cfg.channels.len()).map(WsRow::Channel));
    rows
}

/// A token variable found, as the question the connection asks.
fn found(var: &str) -> connection::Found {
    connection::Found {
        what: "a Slack user token".into(),
        names: vec![(var.to_string(), "the token (xoxp-…)".into())],
        file: None,
        does: format!("sets the token to env:{var} and checks it with Slack"),
        note: None,
    }
}

/// Where the token's check stands, as the kit shows a check.
fn token_check(p: &SlackPlugin, now: i64) -> radar_core::ui::status::Check {
    use radar_core::ui::status::Check as Shown;
    let ago = |at: &i64| Some(time::ago(*at, now));
    match p.checks.get("workspace") {
        None => Shown::NotYet,
        Some((Check::Running, _)) => Shown::Running,
        Some((Check::Passed(m), at)) => Shown::Passed {
            said: m.clone(),
            ago: ago(at),
        },
        Some((Check::Failed(e), at)) => Shown::Failed {
            said: e.clone(),
            ago: ago(at),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChField {
    Alias,
    Purpose,
    Kind,
    /// How an alerts channel's messages are read (alerts channels only).
    Reader,
    Colour,
    Watched,
}

impl ChField {
    /// A channel's fields: the reader only for one that raises alerts.
    fn of(kind: Kind) -> Vec<Self> {
        let mut out = vec![Self::Alias, Self::Purpose, Self::Kind];
        if kind == Kind::Alerts {
            out.push(Self::Reader);
        }
        out.extend([Self::Colour, Self::Watched]);
        out
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Alias => "alias",
            Self::Purpose => "purpose",
            Self::Kind => "kind",
            Self::Reader => "reader",
            Self::Colour => "colour",
            Self::Watched => "watched",
        }
    }

    const fn about(self) -> &'static str {
        match self {
            Self::Alias => {
                "What radar calls the channel: in the nav pane, in breadcrumbs and in `radar slack catchup`."
            }
            Self::Purpose => "What the channel is for, in your words; shown under it in the nav pane.",
            Self::Kind => {
                "How its messages are read: a team channel's threads are discussions; a service channel is what others say about something the team runs; a test channel is where review asks can be tried first; an alerts channel is a bot raising alerts, read into the Alerts view by its reader."
            }
            Self::Reader => {
                "How a bot's messages in this alerts channel become alerts: Alertmanager's attachments ([FIRING] and [RESOLVED] with their fields), a job that failed (a header saying so, details in a context), or any bot message with a status word. A message the reader cannot read is left out and the log says why."
            }
            Self::Colour => "The channel's colour: its stripe in the nav and middle panes.",
            Self::Watched => {
                "A watched channel is read on every refresh and listed in the nav pane; one not watched is kept in the registry (for review asks) but not read."
            }
        }
    }

    const fn accepts(self) -> &'static str {
        match self {
            Self::Alias => config::ALIAS_ACCEPTS,
            Self::Purpose => "a few words",
            Self::Kind => "team, service, test or alerts",
            Self::Reader => "alertmanager, job-failed or status",
            Self::Colour => "one of the channel colours",
            Self::Watched => "yes or no",
        }
    }

    const fn key(self) -> &'static str {
        match self {
            Self::Alias => "alias",
            Self::Purpose => "purpose",
            Self::Kind => "kind",
            Self::Reader => "reader",
            Self::Colour => "color",
            Self::Watched => "watched",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChRow {
    Add,
    Channel(usize),
    Field(usize, ChField),
}

pub(crate) fn ch_rows(p: &SlackPlugin) -> Vec<ChRow> {
    let mut rows = vec![ChRow::Add];
    for i in 0..p.cfg.channels.len() {
        rows.push(ChRow::Channel(i));
        rows.extend(
            ChField::of(p.cfg.channels[i].kind)
                .into_iter()
                .map(|f| ChRow::Field(i, f)),
        );
    }
    rows
}

// --- the alerts section: what alerts belong to --------------------------------------------

/// A field of a mapping rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleField {
    Prefix,
    Label,
    Repo,
    Service,
    Environment,
}

impl RuleField {
    const ALL: [Self; 5] = [
        Self::Prefix,
        Self::Label,
        Self::Repo,
        Self::Service,
        Self::Environment,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Prefix => "name starts",
            Self::Label => "carries label",
            Self::Repo => "repo",
            Self::Service => "service",
            Self::Environment => "environment",
        }
    }

    const fn about(self) -> &'static str {
        match self {
            Self::Prefix => {
                "The start of the alert's name, in any case: api- takes api-HighErrorRate. A rule with a label too needs both."
            }
            Self::Label => {
                "A label the alert carries, as key=value: service=billing. Labels come from the alert's title, fields and label lines."
            }
            Self::Repo => {
                "The repo these alerts belong to: its path (acme/team-a/api) or the last part of it (api), or a link to it. Repos show a firing marker and an Alerts tab."
            }
            Self::Service => {
                "The service's name as you call it: alerts are grouped by it. A repo's own name when empty."
            }
            Self::Environment => "The environment, for alerts that do not say: prod, staging.",
        }
    }

    const fn accepts(self) -> &'static str {
        match self {
            Self::Prefix => "a few characters of the alert's name",
            Self::Label => "key=value",
            Self::Repo => "a repo's path, the last part of it, or a link",
            Self::Service => "a name, no spaces",
            Self::Environment => "a name, no spaces",
        }
    }

    const fn key(self) -> &'static str {
        match self {
            Self::Prefix => "prefix",
            Self::Label => "label",
            Self::Repo => "repo",
            Self::Service => "service",
            Self::Environment => "environment",
        }
    }

    fn of(self, r: &Rule) -> &str {
        match self {
            Self::Prefix => &r.prefix,
            Self::Label => &r.label,
            Self::Repo => &r.repo,
            Self::Service => &r.service,
            Self::Environment => &r.environment,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AlRow {
    /// How long resolved alerts stay.
    Hours,
    Add,
    Rule(usize),
    Field(usize, RuleField),
    /// An alert name no rule places: ⏎ maps it.
    Unmapped(String),
}

/// The alert names no rule places, each once, A to Z.
pub(crate) fn unmapped_names(p: &SlackPlugin) -> Vec<String> {
    let mut names: Vec<String> = p
        .alerts()
        .alerts
        .iter()
        .filter(|a| !a.place.mapped())
        .map(|a| a.name.clone())
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

pub(crate) fn al_rows(p: &SlackPlugin) -> Vec<AlRow> {
    let mut rows = vec![AlRow::Add, AlRow::Hours];
    for i in 0..p.cfg.alerts.rules.len() {
        rows.push(AlRow::Rule(i));
        rows.extend(RuleField::ALL.iter().map(|f| AlRow::Field(i, *f)));
    }
    rows.extend(unmapped_names(p).into_iter().map(AlRow::Unmapped));
    rows
}

/// A rule in words: `api-… → repo acme/team-a/api · service api`.
fn rule_words(r: &Rule) -> String {
    let mut when = Vec::new();
    if !r.prefix.is_empty() {
        when.push(format!("{}…", r.prefix));
    }
    if !r.label.is_empty() {
        when.push(r.label.clone());
    }
    let mut to = Vec::new();
    if !r.repo.is_empty() {
        to.push(format!("repo {}", r.repo));
    }
    if !r.service.is_empty() {
        to.push(format!("service {}", r.service));
    }
    if !r.environment.is_empty() {
        to.push(format!("in {}", r.environment));
    }
    format!(
        "{} → {}",
        when.join(" and "),
        if to.is_empty() {
            "nothing yet".into()
        } else {
            to.join(" · ")
        }
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RvRow {
    Channel,
    Group,
    Template,
    Link,
    Test,
    Reask,
}

const RV_ROWS: [RvRow; 6] = [
    RvRow::Channel,
    RvRow::Group,
    RvRow::Template,
    RvRow::Link,
    RvRow::Test,
    RvRow::Reask,
];

impl RvRow {
    const fn label(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Group => "user group",
            Self::Template => "template",
            Self::Link => "link text",
            Self::Test => "test channel",
            Self::Reask => "ask again",
        }
    }
}

const LOOK: [(&str, &[(&str, &str)]); 3] = [
    (
        "messages",
        &[
            ("author", "who wrote a message"),
            ("you", "the reader, in place of their own name"),
            ("pinged", "a mention of you or your groups"),
            ("mention", "a mention of someone else, a group or a channel"),
            ("link", "a link"),
            ("emoji", "a custom emoji, shown as its :name:"),
            ("code", "code, inline or in a block"),
            ("quote", "a quoted line"),
            ("new", "what is new since you caught up"),
            ("time", "when"),
            ("channel", "a channel's name"),
        ],
    ),
    (
        "messages tree",
        &[
            ("band", "the shade behind every other thread"),
            ("heading", "the shade behind a day's or a group's heading"),
            ("lines", "the tree lines joining threads and replies"),
        ],
    ),
    (
        "alerts",
        &[
            ("firing", "an alert that is firing"),
            ("failed", "a job that failed"),
            ("resolved", "an alert that resolved"),
            ("priority", "an alert's priority"),
            ("unmapped", "an alert no rule places, and its group"),
        ],
    ),
];

fn is_background(word: &str) -> bool {
    matches!(word, "band" | "heading")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LookRow {
    Header(&'static str),
    /// How a conversation is laid out: compact or cozy.
    Density,
    Word(&'static str, &'static str),
}

fn look_rows() -> Vec<LookRow> {
    let mut rows = Vec::new();
    for (group, words) in LOOK {
        rows.push(LookRow::Header(group));
        if group == "messages" {
            rows.push(LookRow::Density);
        }
        rows.extend(words.iter().map(|(w, d)| LookRow::Word(w, d)));
    }
    rows
}

const fn index(section: &str) -> usize {
    match section.as_bytes() {
        b"connection" => 0,
        b"channels" => 1,
        b"review" => 2,
        b"alerts" => 3,
        _ => 4,
    }
}

fn row_count(p: &SlackPlugin, section: &str) -> usize {
    match section {
        "connection" => ws_rows(p).len(),
        "channels" => ch_rows(p).len(),
        "review" => RV_ROWS.len(),
        "alerts" => al_rows(p).len(),
        _ => look_rows().len(),
    }
}

fn al_row(p: &SlackPlugin) -> AlRow {
    al_rows(p)
        .get(p.section_rows[3])
        .cloned()
        .unwrap_or(AlRow::Add)
}

fn ws_row(p: &SlackPlugin) -> WsRow {
    ws_rows(p)
        .get(p.section_rows[0])
        .copied()
        .unwrap_or(WsRow::Token)
}

fn ch_row(p: &SlackPlugin) -> ChRow {
    ch_rows(p)
        .get(p.section_rows[1])
        .copied()
        .unwrap_or(ChRow::Add)
}

fn rv_row(p: &SlackPlugin) -> RvRow {
    RV_ROWS
        .get(p.section_rows[2])
        .copied()
        .unwrap_or(RvRow::Channel)
}

fn selected_look(p: &SlackPlugin) -> Option<(&'static str, &'static str)> {
    match look_rows().get(p.section_rows[4]) {
        Some(LookRow::Word(w, d)) => Some((w, d)),
        _ => None,
    }
}

/// A token typed in is never shown in full.
fn masked(token: &str) -> String {
    connection::masked(token, &[])
}

/// A connection row's label.
fn ws_label(row: WsRow) -> &'static str {
    match row {
        WsRow::Offer(_) => "found",
        WsRow::Workspace => "workspace",
        WsRow::Api => "api",
        WsRow::Token => connection::TOKEN,
        WsRow::SignedIn => connection::SIGNED_IN,
        WsRow::Check => connection::CHECK,
        WsRow::Refresh => connection::REFRESH,
        WsRow::Days => "read back",
        WsRow::Need(k) => NEEDS[k].0,
        WsRow::Gap | WsRow::ChannelsHeading | WsRow::ChannelAdd => "add a channel",
        WsRow::Channel(_) => "channel",
    }
}

/// A connection row's value: the workspace, who the token is.
fn ws_value(p: &SlackPlugin, row: WsRow) -> String {
    match (row, &p.data.me) {
        (WsRow::Workspace, Some(m)) if m.url.is_empty() => m.team.clone(),
        (WsRow::Workspace, Some(m)) => format!("{} · {}", m.team, m.url.trim_end_matches('/')),
        (WsRow::SignedIn, Some(m)) => m.user.clone(),
        (WsRow::Workspace | WsRow::SignedIn, None) => "— the check finds it".into(),
        (WsRow::Api, _) => p.cfg.api.trim().to_string(),
        (WsRow::Token, _) => masked(&p.cfg.token),
        (WsRow::Refresh, _) => connection::every(p.cfg.refresh_secs),
        (WsRow::Days, _) => days(p.cfg.days),
        _ => String::new(),
    }
}

fn days(n: u64) -> String {
    if n == 1 {
        "1 day".into()
    } else {
        format!("{n} days")
    }
}

fn field_value(p: &SlackPlugin, i: usize, f: ChField) -> String {
    let c = &p.cfg.channels[i];
    match f {
        ChField::Alias => c.alias.clone(),
        ChField::Purpose if c.purpose.is_empty() => "—".into(),
        ChField::Purpose => c.purpose.clone(),
        ChField::Kind => c.kind.word().into(),
        ChField::Reader => c
            .reader
            .map_or("— none: nothing is read as alerts".into(), |r| {
                r.word().into()
            }),
        ChField::Colour => c.color.clone(),
        ChField::Watched => if c.watched { "yes" } else { "no" }.into(),
    }
}

fn rv_value(p: &SlackPlugin, r: RvRow) -> String {
    let rv = &p.cfg.review;
    let or_dash = |s: &str| {
        if s.trim().is_empty() {
            "—".to_string()
        } else {
            s.to_string()
        }
    };
    match r {
        RvRow::Channel => or_dash(&rv.channel),
        RvRow::Group if rv.group.is_empty() => "—".into(),
        RvRow::Group => format!("@{}", crate::page::review_handle(p)),
        RvRow::Template => rv.template.clone(),
        RvRow::Link => rv.link.clone(),
        RvRow::Test => or_dash(&rv.test_channel),
        RvRow::Reask => rv.reask().words(),
    }
}

// --- checks ---------------------------------------------------------------------------

pub(crate) fn check_token(p: &mut SlackPlugin) -> Vec<Effect> {
    check_token_then(p, Vec::new())
}

fn check_token_then(p: &mut SlackPlugin, ids: Vec<String>) -> Vec<Effect> {
    if !p.authenticated() {
        p.revoke_session();
    }
    let stamp = p.begin_check("workspace");
    let action = crate::settings_history::workspace_action();
    let c = match p.client() {
        Ok(c) => c,
        Err(e) => {
            p.complete_check("workspace", stamp);
            p.revoke_session();
            p.checks
                .insert("workspace".into(), (Check::Failed(e.clone()), p.now));
            return vec![
                crate::settings_history::workspace_unavailable(),
                crate::view::failed(&e.to_string()),
            ];
        }
    };
    p.defer_channel_checks(stamp, ids);
    p.checks.insert("workspace".into(), (Check::Running, p.now));
    let captured = action.clone();
    vec![Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            let result = c.auth_test().await.map_err(|e| format!("{e:#}"));
            match &result {
                Ok(_) => log::info("slack workspace check: auth returned"),
                Err(_) => log::warn("slack workspace check: auth failed"),
            }
            Box::new(SlackMsg::WorkspaceChecked {
                stamp,
                action: captured,
                result,
            }) as Msg
        }),
    }]
}

/// Look a channel up in Slack: by id, or by name. `key` says what for:
/// `add` adds it, `channel:<id>` checks one held.
pub(crate) fn check_channel(
    p: &mut SlackPlugin,
    key: String,
    what: Result<String, String>,
) -> Vec<Effect> {
    let stamp = p.begin_check(&key);
    let action = crate::settings_history::channel_action();
    let c = match p.client() {
        Ok(c) => c,
        Err(e) => {
            p.complete_check(&key, stamp);
            p.checks.insert(key, (Check::Failed(e.clone()), p.now));
            return vec![
                crate::settings_history::channel_unavailable(),
                crate::view::failed(&format!("{e}: set the token under connection first")),
            ];
        }
    };
    if !p.authenticated() {
        p.complete_check(&key, stamp);
        let error = "check the Slack connection first, then try again";
        p.checks.insert(key, (Check::Failed(error.into()), p.now));
        return vec![
            crate::settings_history::channel_unavailable(),
            crate::view::failed(error),
        ];
    }
    p.checks.insert(key.clone(), (Check::Running, p.now));
    let looking = match &what {
        Err(name) => vec![Effect::Notify(format!(
            "looking for #{name} in joined and public channel lists…"
        ))],
        Ok(_) => vec![],
    };
    let captured = action.clone();
    looking
        .into_iter()
        .chain([Effect::SpawnAction {
            action,
            task: Box::pin(async move {
                let requested_id = what.as_ref().ok().cloned();
                let (result, evidence) = match what {
                    Ok(id) => {
                        let got = c.channel_info_observed(&id).await;
                        (got.result, got.evidence)
                    }
                    Err(name) => {
                        let got = c.find_channel_observed(&name).await;
                        let result = match got.result {
                            Ok(found) => found.found.ok_or_else(|| {
                                anyhow::anyhow!("{}", channel_lookup_failure(&name, &got.evidence))
                            }),
                            Err(_) if got.evidence.end() == Some(ChannelEnd::LocalCap) => Err(
                                anyhow::anyhow!("{}", channel_lookup_failure(&name, &got.evidence)),
                            ),
                            Err(error) => Err(error),
                        };
                        (result, got.evidence)
                    }
                };
                let result = result.map(Checked::Channel).map_err(|e| format!("{e:#}"));
                Box::new(SlackMsg::Checked {
                    stamp,
                    key,
                    requested_id,
                    action: captured,
                    evidence,
                    result,
                }) as Msg
            }),
        }])
        .collect()
}

/// Transient feedback for a negative lookup or local cap. Only completed,
/// gap-free list traversal supports a bounded no-match statement; neither the
/// cached account nor these lists prove workspace inventory or permissions.
fn channel_lookup_failure(name: &str, evidence: &ChannelEvidence) -> String {
    if let [Some(yours), Some(public)] = evidence.phases
        && yours.kind == ChannelKind::Yours
        && public.kind == ChannelKind::Public
        && yours.end == ChannelEnd::NoMatch
        && public.end == ChannelEnd::NoMatch
        && yours.requests > 0
        && public.requests > 0
        && !evidence.has_gaps()
    {
        return format!(
            "No match for #{name} in the returned lists: {} joined-channel entries and {} public-channel entries (duplicates included). Check the exact name and channel access, or retry with a channel ID or link.",
            yours.returned, public.returned
        );
    }
    let reason = if evidence.end() == Some(ChannelEnd::LocalCap) {
        " because radar reached its page limit"
    } else {
        ""
    };
    format!(
        "Search for #{name} was incomplete{reason}; a match could not be established. Retry with a channel ID or link, and check the exact name and channel access."
    )
}

pub(crate) fn fetch_groups(p: &mut SlackPlugin, pick: bool) -> Vec<Effect> {
    let stamp = p.begin_check("groups");
    let action = crate::settings_history::groups_action();
    let c = match p.client() {
        Ok(c) => c,
        Err(e) => {
            p.complete_check("groups", stamp);
            p.checks
                .insert("groups".into(), (Check::Failed(e.clone()), p.now));
            return vec![
                crate::settings_history::groups_unavailable(),
                crate::view::failed(&format!("{e}: set the token under connection first")),
            ];
        }
    };
    if !p.authenticated() {
        p.complete_check("groups", stamp);
        let error = "check the Slack connection first, then try again";
        p.checks
            .insert("groups".into(), (Check::Failed(error.into()), p.now));
        return vec![
            crate::settings_history::groups_unavailable(),
            crate::view::failed(error),
        ];
    }
    p.checks.insert("groups".into(), (Check::Running, p.now));
    let captured = action.clone();
    vec![
        Effect::Notify("asking Slack for its user groups…".into()),
        Effect::SpawnAction {
            action,
            task: Box::pin(async move {
                let result = c.usergroups_observed().await.map_err(|e| format!("{e:#}"));
                Box::new(SlackMsg::Groups {
                    stamp,
                    action: captured,
                    result,
                    pick,
                }) as Msg
            }),
        },
    ]
}

/// Slack's user groups came back: held, and offered to pick from when
/// that is what they were read for.
pub(crate) fn on_groups(
    p: &mut SlackPlugin,
    result: Result<Vec<UserGroup>, String>,
    pick: bool,
) -> Vec<Effect> {
    match result {
        Ok(g) => {
            let n = g.len();
            p.data.groups = g;
            p.changed();
            p.checks.insert(
                "groups".into(),
                (Check::Passed(format!("{n} groups")), p.now),
            );
            if pick {
                vec![group_menu(p)]
            } else {
                vec![Effect::Notify(format!("Slack has {n} user groups"))]
            }
        }
        Err(e) => {
            p.checks
                .insert("groups".into(), (Check::Failed(e.clone()), p.now));
            vec![crate::view::failed(&e.to_string())]
        }
    }
}

/// Apply only a usable identity, after the caller has recorded the lookup and
/// accepted its ownership stamp. This is radar's supported ID grammar, not a claim
/// that the permissive parser validated Slack's complete response schema.
pub(crate) fn on_channel_checked(
    p: &mut SlackPlugin,
    key: String,
    requested_id: Option<&str>,
    action: &radar_core::journal::ActionDescriptor,
    result: Result<Checked, String>,
) -> Vec<Effect> {
    if let Ok(Checked::Channel(info)) = &result
        && (!config::is_channel_id(&info.id) || requested_id.is_some_and(|id| info.id != id))
    {
        let error = "Slack returned an unusable channel identity; no channel settings were changed";
        let mut fx = vec![crate::settings_history::channel_application_blocked(action)];
        fx.extend(on_checked(p, key, Err(error.into())));
        return fx;
    }
    on_checked(p, key, result)
}

/// The caller has accepted this exact workspace request. Take its bounded intent
/// before installing the identity invalidates unauthenticated read generations.
pub(crate) fn on_workspace_checked(
    p: &mut SlackPlugin,
    stamp: crate::read_context::CheckStamp,
    result: Result<crate::client::Me, String>,
) -> Vec<Effect> {
    let ids = p.take_channel_checks(stamp);
    let succeeded = result.is_ok();
    let mut effects = on_checked(p, "workspace".into(), result.map(Checked::Me));
    if succeeded && p.authenticated() {
        for id in ids {
            if p.cfg.channels.iter().any(|channel| channel.id == id) {
                effects.extend(check_channel(p, format!("channel:{id}"), Ok(id)));
            }
        }
    }
    effects
}

pub(crate) fn on_checked(
    p: &mut SlackPlugin,
    key: String,
    result: Result<Checked, String>,
) -> Vec<Effect> {
    match (key.as_str(), result) {
        ("workspace", Ok(Checked::Me(mut me))) => {
            let say = format!("signed in as {} in {}", me.user, me.team);
            let old = p
                .data
                .me
                .as_ref()
                .filter(|m| m.team_id == me.team_id && m.user_id == me.user_id);
            if let Some(old) = old {
                me.tz_offset = me.tz_offset.or(old.tz_offset);
            }
            let fresh = !p.authenticated() || old.is_none();
            if !p.install_me(me) {
                return on_checked(
                    p,
                    key,
                    Err(
                        "Slack returned an incomplete account identity; check the connection again"
                            .into(),
                    ),
                );
            }
            p.checks.insert(key, (Check::Passed(say.clone()), p.now));
            let mut fx = vec![Effect::Notify(format!("✓ {say}"))];
            if fresh {
                fx.extend(p.read());
            }
            fx
        }
        ("add", Ok(Checked::Channel(info))) => {
            if let Some(i) = p.cfg.channels.iter().position(|c| c.id == info.id) {
                p.checks.remove("add");
                p.section_rows[1] = row_of(p, ChRow::Channel(i));
                return vec![Effect::Notify(format!(
                    "#{} is in the registry already",
                    info.name
                ))];
            }
            let taken: Vec<&str> = p.cfg.channels.iter().map(|c| c.alias.as_str()).collect();
            let alias = config::alias_for(&info.name, &taken);
            let mut purpose = info.purpose.clone();
            if purpose.chars().count() > 60 {
                purpose = radar_core::ui::fit(&purpose, 60).trim_end().to_string();
            }
            let color = p.cfg.next_color();
            p.cfg.channels.push(Channel {
                alias: alias.clone(),
                id: info.id.clone(),
                name: info.name.clone(),
                purpose,
                kind: Kind::Team,
                color,
                watched: true,
                reader: None,
            });
            let i = p.cfg.channels.len() - 1;
            p.checks.remove("add");
            p.checks.insert(
                format!("channel:{}", info.id),
                (Check::Passed(channel_verdict(&info)), p.now),
            );
            p.data.channels.entry(info.id.clone()).or_default().info = Some(info.clone());
            p.section_rows[1] = row_of(p, ChRow::Channel(i));
            let mut fx = vec![Effect::Notify(if alias == info.name {
                format!("added #{}", info.name)
            } else {
                format!("added #{} as {alias}", info.name)
            })];
            fx.extend(p.fetch_now());
            fx
        }
        (k, Ok(Checked::Channel(info))) => {
            if let Some(c) = p.cfg.channels.iter_mut().find(|c| c.id == info.id) {
                c.name = info.name.clone();
            }
            let verdict = channel_verdict(&info);
            let id = info.id.clone();
            p.data.channels.entry(id).or_default().info = Some(info);
            p.checks
                .insert(k.to_string(), (Check::Passed(verdict.clone()), p.now));
            vec![Effect::Notify(format!("✓ {verdict}"))]
        }
        (_, Ok(Checked::Me(_))) => vec![],
        (k, Err(e)) => {
            if k == "workspace" {
                p.revoke_session();
            }
            p.checks
                .insert(k.to_string(), (Check::Failed(e.clone()), p.now));
            vec![crate::view::failed(&e.to_string())]
        }
    }
}

fn channel_verdict(info: &crate::client::ChannelInfo) -> String {
    let private = if info.is_private { "private" } else { "public" };
    let member = if info.is_member {
        "you are in it"
    } else {
        "you are not in it: only public ones can be read without joining"
    };
    format!("#{} · {private} · {member}", info.name)
}

fn ask_to_check(key: &str, what: &str) -> Effect {
    Effect::Menu(Menu::confirm(
        format!("ask-check:{key}"),
        format!("check {what} now?"),
        "check it",
        "leave it unchecked",
    ))
}

// --- validation as you type ---------------------------------------------------------

fn env_ok(spec: &str) -> Option<Result<String, String>> {
    let var = spec.strip_prefix("env:")?;
    Some(match std::env::var(var) {
        Ok(v) if !v.trim().is_empty() => {
            Ok(format!("{var} is set: {}", config::token_kind(v.trim())))
        }
        _ => Err(format!("{var} is not set in this shell")),
    })
}

fn validate(p: &SlackPlugin, target: &Target, text: &str) -> Result<String, String> {
    let text = text.trim();
    match target {
        Target::Token => match env_ok(text) {
            Some(Ok(m)) if m.contains("a user token") => Ok(m),
            Some(Ok(m)) => Err(format!(
                "{m}; radar reads and asks as you, so it needs a user token (xoxp-…)"
            )),
            Some(Err(e)) => Err(e),
            None if text.starts_with("xoxp-") && text.len() > 20 => {
                Ok("a user token typed in; it is stored in config.toml".into())
            }
            None if text.starts_with("xox") => Err(format!(
                "{}: radar needs a user token (xoxp-…)",
                config::token_kind(text)
            )),
            None => Err("env:NAME, or a user token (xoxp-…)".into()),
        },
        Target::Api => match config::api_problem(text) {
            _ if text.is_empty() => Ok("Slack's own, https://slack.com".into()),
            None => Ok(format!(
                "Slack's API at {}",
                text.trim_end_matches('/')
            )),
            Some(why) => Err(why),
        },
        Target::Refresh => match text.trim_end_matches('s').parse::<u64>() {
            Ok(n) if n >= 30 => Ok(format!("every {}", time::ago(0, n as i64))),
            Ok(_) => Err("at least 30 seconds: Slack limits how often it is asked".into()),
            Err(_) => Err("whole seconds, e.g. 120".into()),
        },
        Target::Days => match text.trim_end_matches('d').trim().parse::<u64>() {
            Ok(n @ 1..=30) => Ok(format!(
                "reads the last {} of a channel with no mark",
                days(n)
            )),
            _ => Err("whole days, 1 to 30".into()),
        },
        Target::ChannelAdd => {
            if text.is_empty() {
                return Err("a link to the channel or a message in it, its id, or #name".into());
            }
            match config::channel_ref(text) {
                Ok(id) if p.cfg.channels.iter().any(|c| c.id == id) => {
                    Err("that channel is in the registry already".into())
                }
                Ok(id) => Ok(format!("the channel {id}; Slack is asked for its name")),
                Err(name) if config::is_channel_name(&name) => {
                    Ok(format!("#{name}; Slack is asked which channel it is"))
                }
                Err(_) => Err("a channel name: lowercase letters, digits, - and _".into()),
            }
        }
        Target::Alias(i) => match config::alias_problem(text) {
            Some(why) => Err(why.into()),
            None if p
                .cfg
                .channels
                .iter()
                .enumerate()
                .any(|(j, c)| j != *i && c.alias == text) =>
            {
                Err("another channel has that alias".into())
            }
            None => Ok(format!("#{text} in the nav pane")),
        },
        Target::Purpose(_) => {
            if text.chars().count() > 80 {
                Err("at most 80 characters".into())
            } else {
                Ok(if text.is_empty() {
                    "no purpose".into()
                } else {
                    "shown under it in the nav pane".into()
                })
            }
        }
        Target::Template => {
            let mentions = ask::check_template(text, &p.data.groups)?;
            Ok(format!(
                "{mentions}; posts: {}",
                ask::text(
                    &with_template(p, text),
                    &p.data.groups,
                    &p.cfg.review.channel,
                    &ask::sample()
                )
                .replace('\n', " ⏎ ")
            ))
        }
        Target::Link => {
            if text.is_empty() || text.chars().count() > 30 {
                Err("1 to 30 characters".into())
            } else {
                Ok(format!("one merge request is linked as \"{text}\""))
            }
        }
        Target::Reask => config::Reask::parse(text).map(|r| match r {
            config::Reask::Never => "never offers to ask again".into(),
            r => format!("offers to ask again {}", r.words()),
        }),
        Target::Hours => match text.trim_end_matches('h').trim().parse::<u64>() {
            Ok(n @ 1..=720) => Ok(format!(
                "resolved alerts and failed jobs stay shown {}",
                crate::alert_view::hours(n)
            )),
            _ => Err("whole hours, 1 to 720".into()),
        },
        Target::RuleAdd => match text {
            "" => Err("the start of the alert's name, e.g. api-".into()),
            t if t.contains(char::is_whitespace) => {
                Err("no spaces: the start of the alert's name".into())
            }
            t => Ok(format!(
                "alerts named {t}… ; where they belong is asked next"
            )),
        },
        Target::RulePrefix(i) => {
            if text.is_empty()
                && p.cfg
                    .alerts
                    .rules
                    .get(*i)
                    .is_none_or(|r| r.label.is_empty())
            {
                Err("a rule needs a name prefix or a label".into())
            } else if text.contains(char::is_whitespace) {
                Err("no spaces: the start of the alert's name".into())
            } else {
                Ok(if text.is_empty() {
                    "any name".into()
                } else {
                    format!("alerts named {text}…")
                })
            }
        }
        Target::RuleLabel(i) => match text.split_once('=') {
            None if text.is_empty()
                && p.cfg
                    .alerts
                    .rules
                    .get(*i)
                    .is_none_or(|r| r.prefix.is_empty()) =>
            {
                Err("a rule needs a name prefix or a label".into())
            }
            None if text.is_empty() => Ok("no label".into()),
            Some((k, v)) if !k.trim().is_empty() && !v.trim().is_empty() => {
                Ok(format!("alerts carrying {}={}", k.trim(), v.trim()))
            }
            _ => Err("key=value, e.g. service=billing".into()),
        },
        Target::RuleRepo(_) if text.is_empty() => Ok("no repo".into()),
        Target::RuleRepo(_) => crate::alert_view::repo_path(text).map(|r| format!("repo {r}")),
        Target::RuleService(_) | Target::RuleEnvironment(_) => {
            if text.contains(char::is_whitespace) || text.chars().count() > 40 {
                Err("a name of up to 40 characters, no spaces".into())
            } else {
                Ok(if text.is_empty() {
                    "none".into()
                } else {
                    text.to_string()
                })
            }
        }
        Target::MapUnmapped(name) => {
            crate::alert_view::rule_for(name, text).map(|r| match (&r.repo, &r.service) {
                (repo, _) if !repo.is_empty() => format!("{name}… belongs to repo {repo}"),
                (_, s) => format!("{name}… belongs to service {s}"),
            })
        }
    }
}

/// The settings with another template, for its live preview.
fn with_template(p: &SlackPlugin, template: &str) -> config::Settings {
    let mut cfg = p.cfg.clone();
    cfg.review.template = template.to_string();
    cfg
}

fn look_summary(spec: &str) -> String {
    let mut parts = spec.split_whitespace();
    let colour = parts.next().map(|s| slot_info(s).label).unwrap_or_default();
    let mods: Vec<&str> = parts.collect();
    if mods.is_empty() {
        colour
    } else {
        format!("{colour} · {}", mods.join(" "))
    }
}

// Where an input starts on its row, counted from the row's text (after the
// margin `RowList` leaves): exactly where the value it replaces is drawn.
/// The width a `token` style row's label is padded to.
const KV_W: usize = 17;
/// A channel's field label is padded to this, after `  ├─ `.
const FIELD_LABEL_W: usize = 16;
const CH_FIELD_AT: u16 = 2 + 3 + FIELD_LABEL_W as u16;
/// The `+ ` before an add row's words.
const ADD_AT: u16 = 2;

/// A `├─ label  value` field row under its parent.
pub fn look_view(p: &SlackPlugin) -> v::SectionView {
    let rows = look_rows().into_iter().enumerate().map(|(i, row)| match row {
        LookRow::Header(group) => v::ListRow::heading(format!("row:{i}"), v::kit::heading(group)),
        LookRow::Density => {
            let mut row = v::ListRow::new(format!("row:{i}"), v::Text::new()
                .push(v::Seg::new("  ", v::Ink::Plain))
                .push(v::Seg::new(format!("{:<9}", "density"), v::Ink::Primary)));
            row.detail = Some(v::Text::of("how a thread's messages are laid out", v::Ink::Muted));
            row.right = Some(v::Text::of(p.cfg.density.word(), v::Ink::Secondary));
            row
        }
        LookRow::Word(word, about) => {
            let spec = p.cfg.style_for(word);
            let mut row = v::ListRow::new(format!("row:{i}"), v::Text::new()
                .push(v::Seg::new("  ", v::Ink::Plain))
                .push(v::Seg::styled(format!("{word:<9}"), spec.clone())));
            row.detail = Some(v::Text::of(about, v::Ink::Muted));
            row.right = Some(v::Text::of(look_summary(&spec), v::Ink::Secondary));
            row
        }
    }).collect();
    let mut detail = Vec::new();
    if on_density(p) {
        let mark = |density| if p.cfg.density == density { "in use" } else { "" };
        detail.extend([v::Text::plain("How the messages of a thread, the detail pane and a direct message are laid out."),
            v::Text::new(), v::Text::new().push(v::Seg::new("compact  ", v::Ink::Emphasis).strong())
                .push(v::Seg::new(mark(config::Density::Compact), v::Ink::Good)),
            v::Text::plain("One line a message: the time, the author in a column, then the text. The author is left blank for the same person within five minutes. A thin line marks a new day, and a thread's replies sit under one divider. In a narrow panel the author and time take a line and the text sits under it."),
            v::Text::new(), v::Text::new().push(v::Seg::new("cozy     ", v::Ink::Emphasis).strong())
                .push(v::Seg::new(mark(config::Density::Cozy), v::Ink::Good)),
            v::Text::plain("The roomier layout: the author, place and date on a line, the text under it, the reactions on a line of their own, a blank line between messages, and replies two cells in."),
            v::Text::new(), v::Text::of("config: [slack] density = \"compact\" or \"cozy\"", v::Ink::Muted)]);
    } else if let Some((word, about)) = selected_look(p) {
        let spec = p.cfg.style_for(word);
        let colour = spec.split_whitespace().next().unwrap_or("text.primary");
        let info = slot_info(colour);
        detail.extend([v::Text::plain(format!("How {about} is drawn.")), v::Text::new(),
            v::Text::new().push(v::Seg::new("  sample  ", v::Ink::Muted)).push(v::Seg::styled(word, spec.clone())),
            v::Text::new(), v::kit::labelled("colour", 10, v::Text::new()
                .push(v::Seg::styled("██ ", colour)).push(v::Seg::new(info.label.clone(), v::Ink::Primary))),
            v::Text::of(format!("          the theme's {} colour, used for {}", info.label, info.meaning), v::Ink::Muted)]);
        if !is_background(word) {
            let mods: Vec<_> = spec.split_whitespace().skip(1).collect();
            detail.push(v::kit::labelled("emphasis", 10, v::Text::plain(if mods.is_empty() { "none".into() } else { mods.join(", ") })));
        }
        if default_styles().get(word).is_some_and(|default| *default != spec) {
            detail.push(v::kit::labelled("default", 10, v::Text::of(look_summary(&default_styles()[word]), v::Ink::Muted)));
        }
        detail.extend([v::Text::new(), v::Text::of("What each colour looks like is set in colours, under general.", v::Ink::Muted),
            v::Text::of(format!("config: [slack.style] \"{word}\""), v::Ink::Muted)]);
    }
    v::SectionView {
        version: v::VERSION, scope: "look".into(),
        body: v::Node::List { rows, selected: Some(p.section_rows[4]), editing: None },
        detail_title: detail_title(p, "look"), detail,
        detail_blocks: vec![],
        actions: action_specs(p, "look"),
    }
}

fn check_text(p: &SlackPlugin, key: &str, now: i64) -> v::Text {
    match p.checks.get(key) {
        None => v::Text::of("not checked yet", v::Ink::Muted),
        Some((Check::Running, _)) => v::Text::of("checking with Slack…", v::Ink::Info),
        Some((Check::Passed(message), at)) => v::Text::new()
            .push(v::Seg::new(format!("✓ {message}"), v::Ink::Good))
            .push(v::Seg::new(format!("  {} ago", time::ago(*at, now)), v::Ink::Muted)),
        Some((Check::Failed(message), at)) => v::Text::new()
            .push(v::Seg::new(format!("✗ {message}"), v::Ink::Bad))
            .push(v::Seg::new(format!("  {} ago", time::ago(*at, now)), v::Ink::Muted)),
    }
}

fn check_mark_text(p: &SlackPlugin, key: &str) -> Option<v::Text> {
    match p.checks.get(key) {
        None => None,
        Some((Check::Running, _)) => Some(v::Text::of("checking", v::Ink::Info)),
        Some((Check::Passed(_), _)) => Some(v::Text::of("✓", v::Ink::Good)),
        Some((Check::Failed(_), _)) => Some(v::Text::of("✗", v::Ink::Bad)),
    }
}

pub fn connection_view(p: &SlackPlugin, width: u16, now: i64) -> v::SectionView {
    let offers = p.offers();
    let mut editing_at = None;
    let rows = ws_rows(p).into_iter().enumerate().map(|(i, row)| {
        let target = match row { WsRow::Token => Some(Target::Token), WsRow::Api => Some(Target::Api), WsRow::Refresh => Some(Target::Refresh),
            WsRow::Days => Some(Target::Days), _ => None };
        if target.as_ref().is_some_and(|target| p.edit.as_ref().is_some_and(|(edit, _)| edit == target)) {
            editing_at = Some((i, 1 + connection::value_x(1)));
        }
        let mut list = match row {
            WsRow::Offer(k) => v::ListRow::new(format!("row:{i}"), found(&offers[k].var).text(width)),
            WsRow::Gap => v::ListRow::heading(format!("row:{i}"), v::Text::new()),
            WsRow::ChannelsHeading => v::ListRow::heading(format!("row:{i}"), v::Text::plain(" ")
                .push(v::Seg::new("CHANNELS", v::Ink::Muted).strong())
                .push(v::Seg::new("  the registry; watched ones are read", v::Ink::Muted))),
            WsRow::ChannelAdd => {
                if matches!(p.edit, Some((Target::ChannelAdd, _))) {
                    editing_at = Some((i, ADD_AT + 2));
                }
                // Set in by a cell, as the card's other rows and GitLab's follow list are.
                v::ListRow::new(format!("row:{i}"), v::kit::add_line("add a channel", "paste its link, a message link, or its id", width))
            }
            WsRow::Channel(k) => {
                let channel = &p.cfg.channels[k];
                let mut row = v::ListRow::new(format!("row:{i}"), v::Text::plain("   ")
                    .push(v::Seg::icon(radar_core::ui::icon::Icon::Channel))
                    .push(v::Seg::styled(format!("#{}", channel.alias), p.cfg.style_for("channel")))
                    .push(v::Seg::new(format!("   {} · {}", channel.kind.word(), if channel.watched { "watched" } else { "not watched" }), v::Ink::Muted)));
                row.right = check_mark_text(p, &format!("channel:{}", channel.id));
                row
            }
            WsRow::Workspace | WsRow::SignedIn => {
                let label = if row == WsRow::Workspace { "workspace" } else { connection::SIGNED_IN };
                let value = match (row, &p.data.me) {
                    (WsRow::SignedIn, Some(me)) => connection::signed_in_text(&p.data.users.get(&me.user_id).cloned().unwrap_or_default(), &me.user),
                    _ => v::Text::of(ws_value(p, row), v::Ink::Secondary),
                };
                v::ListRow::new(format!("row:{i}"), connection::field_text(v::Text::plain(" "), label, value))
            }
            WsRow::Api | WsRow::Token | WsRow::Refresh | WsRow::Days => {
                let label = match row { WsRow::Api => "api", WsRow::Token => connection::TOKEN, WsRow::Refresh => connection::REFRESH, _ => "read back" };
                v::ListRow::new(format!("row:{i}"), connection::field_text(v::Text::plain(" "), label,
                    v::Text::of(ws_value(p, row), v::Ink::Primary)))
            }
            WsRow::Check => v::ListRow::new(format!("row:{i}"), connection::check_field_text(
                v::Text::plain(" "), &token_check(p, now), "Slack")),
            WsRow::Need(k) => {
                let (what, need, _) = NEEDS[k];
                let lead = v::Text::of(format!(" {:<14}", if k == 0 { "permissions" } else { "" }), v::Ink::Muted);
                let branch = v::Text::new().push(v::Seg::styled(if k + 1 == NEEDS.len() { "└─ " } else { "├─ " }, p.cfg.style_for("lines")));
                let mut value = lead;
                value.extend(branch);
                value.extend(v::kit::labelled(what, FIELD_LABEL_W, v::Text::new()));
                let mut list = v::ListRow::new(format!("row:{i}"), value);
                match missing(p, need) {
                    None => list.right = Some(v::Text::of("not checked", v::Ink::Muted)),
                    Some(missing) if missing.is_empty() => {
                        list.right = Some(v::Text::new().push(v::Seg::icon_or(radar_core::ui::icon::Icon::Done, "has it", v::Ink::Good)));
                    }
                    Some(missing) => {
                        list.right = Some(v::Text::new().push(v::Seg::icon_or(radar_core::ui::icon::Icon::Error, "missing", v::Ink::Warn)));
                        list.detail = Some(v::Text::of(format!("needs {}", missing.join(", ")), v::Ink::Warn));
                    }
                }
                list
            }
        };
        list.target = v::Target::new(format!("row:{i}"));
        list
    }).collect();
    let editing = editing_at.and_then(|(row, at)| p.edit.as_ref().map(|(_, input)| v::Editing {
        row, at, text: input.value(), caret: input.cursor(),
    }));
    let mut detail = Vec::new();
    if p.has_quarantined_cache() { detail.extend([v::kit::heading("older cache kept separately"),
        v::Text::of("Older or unverified Slack cache is kept separately and is not used automatically. Run radar slack cache-recovery to export it locally; no token is needed.", v::Ink::Secondary), v::Text::new()]); }
    if p.draft.is_some() { detail.extend([v::Text::of("A review ask is kept in this session. Press y to copy its authored text, even without a token.", v::Ink::Secondary), v::Text::new()]); }
    if p.replying.is_some() || p.reply_context.usage().0 > 0 { detail.extend([v::Text::of("Reply text is kept in this session. Press ctrl-r to view, copy or drop it, even without a token.", v::Ink::Secondary), v::Text::new()]); }
    match ws_row(p) {
        WsRow::Offer(k) => { if let Some(offer) = offers.get(k) { detail.extend(found(&offer.var).detail_text()); } }
        WsRow::Need(k) => {
            let (what, need, about) = NEEDS[k];
            detail.push(v::kit::heading("the token has"));
            detail.push(match missing(p, need) {
                None => v::Text::of("not known until the token is checked", v::Ink::Muted),
                Some(missing) if missing.is_empty() => v::Text::new()
                    .push(v::Seg::icon_or_lead(radar_core::ui::icon::Icon::Done, "", v::Ink::Good))
                    .push(v::Seg::new("all of them", v::Ink::Good)),
                Some(missing) => v::Text::new()
                    .push(v::Seg::icon_or_lead(radar_core::ui::icon::Icon::Error, "", v::Ink::Warn))
                    .push(v::Seg::new(format!("not {}: the Slack app the token comes from needs it as a user scope; radar's manifest asks for it", missing.join(", ")), v::Ink::Warn)),
            });
            detail.extend([v::Text::new(), v::Text::plain(format!("{what}: {about}.")), v::Text::new(),
                v::kit::heading("needs"), v::Text::of(need.join(", "), v::Ink::Secondary)]);
        }
        WsRow::Gap | WsRow::ChannelsHeading | WsRow::ChannelAdd => {
            typing_text(p, &mut detail);
            detail.push(v::Text::plain("Watch another channel: Slack is asked which it is, and it is added as a team channel, watched."));
        }
        WsRow::Channel(k) => {
            let channel = &p.cfg.channels[k];
            detail.extend([v::kit::heading("checked"), check_text(p, &format!("channel:{}", channel.id), now), v::Text::new(),
                v::Text::plain(format!("#{}: {}.", channel.alias, channel.kind.about())), v::Text::new(),
                v::kit::labelled("in Slack", 10, v::Text::plain(if channel.name.is_empty() { "—".into() } else { format!("#{}", channel.name) })),
                v::kit::labelled("id", 10, v::Text::plain(channel.id.clone()))]);
        }
        row => {
            let mut d = match row {
                WsRow::Workspace => connection::DetailText::new("The Slack workspace the token is for: a user token belongs to one. The check finds it."),
                WsRow::Api => {
                    let mut d = connection::DetailText::new("The address of Slack's Web API: every call goes to it, with the token. Change it for GovSlack (https://slack-gov.com) or a stand-in on this machine; the token is sent to this host and its subdomains, nowhere else.");
                    d.accepts = Some("https://slack.com, https://slack-gov.com, or http://127.0.0.1:PORT for this machine".into()); d.config = Some("[slack] api".into()); d
                }
                WsRow::Token => {
                    let mut d = connection::DetailText::new("How radar proves who you are to Slack: a user token (xoxp-…), so radar reads what you can read and asks for review as you. It only reads, unless you say yes to a post, reply or reaction.");
                    d.accepts = Some(connection::TOKEN_ACCEPTS.into()); d.config = Some("[slack] token".into()); d
                }
                WsRow::SignedIn => connection::DetailText::new(format!("{} \"For you\" means this person.", connection::about_signed_in("Slack"))),
                WsRow::Check => connection::DetailText::new(format!("{} It also finds the permissions the token has.", connection::about_check("Slack"))),
                WsRow::Refresh => {
                    let mut d = connection::DetailText::new(format!("{} It reads the watched channels and For you. Slack limits how often it is asked, so 30 seconds is the least.", connection::about_refresh("Slack")));
                    d.accepts = Some(connection::refresh_accepts(30)); d.config = Some("[slack] refresh_secs".into()); d
                }
                _ => { let mut d = connection::DetailText::new("How far back a channel is read. What is new is what came after its caught-up mark; a channel with no mark yet shows all of this.");
                    d.accepts = Some("whole days, 1 to 30".into()); d.config = Some("[slack] days".into()); d },
            };
            if row != WsRow::Check { d.now = Some(ws_value(p, row)); }
            if matches!(row, WsRow::Workspace | WsRow::Token | WsRow::SignedIn | WsRow::Check) { d.checked = Some(check_text(p, "workspace", now)); }
            d.typing = p.edit.as_ref().map(|(target, input)| validate(p, target, &input.value()));
            detail.extend(d.text());
        }
    }
    // The sheet: the row's detail, then the help that row needs.
    let help = match ws_row(p) {
        WsRow::Gap | WsRow::ChannelsHeading | WsRow::ChannelAdd | WsRow::Channel(_) => crate::setup::channels_help(),
        WsRow::Api | WsRow::Refresh | WsRow::Days => vec![],
        _ => crate::setup::token_help(),
    };
    let mut detail_blocks: Vec<v::Block> = detail.iter().cloned().map(v::Block::line).collect();
    if !help.is_empty() {
        detail_blocks.push(v::Block::line(v::Text::new()));
        detail_blocks.extend(help);
    }
    v::SectionView {
        version: v::VERSION, scope: "connection".into(),
        body: v::Node::List { rows, selected: Some(p.section_rows[0]), editing },
        detail_title: detail_title(p, "connection"), detail,
        detail_blocks,
        actions: action_specs(p, "connection"),
    }
}

fn settings_view(p: &SlackPlugin, section: &str, rows: Vec<v::ListRow>, detail: Vec<v::Text>, editing: Option<v::Editing>) -> v::SectionView {
    v::SectionView {
        version: v::VERSION, scope: section.into(),
        body: v::Node::List { rows, selected: Some(p.section_rows[index(section)]), editing },
        detail_title: detail_title(p, section), detail,
        detail_blocks: vec![],
        actions: action_specs(p, section),
    }
}

fn editing_view(p: &SlackPlugin, row: usize, at: u16) -> Option<v::Editing> {
    p.edit.as_ref().map(|(_, input)| v::Editing { row, at: at + 1, text: input.value(), caret: input.cursor() })
}

fn typing_text(p: &SlackPlugin, detail: &mut Vec<v::Text>) {
    if let Some((target, input)) = &p.edit {
        let verdict = validate(p, target, &input.value());
        detail.extend([v::kit::heading("as you type"), v::kit::as_you_type(verdict.as_deref().map_err(String::as_str)), v::Text::new()]);
    }
}

fn detail_block(detail: &mut Vec<v::Text>, heading: &str, body: impl Into<v::Text>) {
    detail.extend([v::Text::new(), v::kit::heading(heading), body.into()]);
}

fn config_text(detail: &mut Vec<v::Text>, key: &str) {
    detail.extend([v::Text::new(), v::Text::of(format!("config: {key}"), v::Ink::Muted)]);
}

fn slack_add_line(action: &str, takes: &str, width: u16) -> v::Text {
    let mut line = v::kit::add_line(action, takes, width);
    if let Some(first) = line.0.first_mut() { first.text = "+ ".into(); }
    line
}

pub fn channels_view(p: &SlackPlugin, width: u16, now: i64) -> v::SectionView {
    let mut editing = None;
    let rows = ch_rows(p).into_iter().enumerate().map(|(i, row)| {
        let mut out = match row {
            ChRow::Add => {
                if p.edit.as_ref().is_some_and(|(target, _)| *target == Target::ChannelAdd) { editing = editing_view(p, i, ADD_AT); }
                let mut row = v::ListRow::new(format!("row:{i}"), slack_add_line("add a channel", "paste a link to it, or type #name", width));
                row.right = check_mark_text(p, "add");
                row
            }
            ChRow::Channel(k) => {
                let channel = &p.cfg.channels[k];
                let mut row = v::ListRow::new(format!("row:{i}"), v::Text::new()
                    .push(v::Seg::styled("▌ ", format!("slack.{}", channel.color)))
                    .push(v::Seg::styled(format!("#{}", channel.alias), p.cfg.style_for("channel"))));
                let name = if channel.name.is_empty() { p.data.channels.get(&channel.id).and_then(|d| d.info.as_ref()).map(|i| i.name.clone()).unwrap_or_default() } else { channel.name.clone() };
                row.detail = Some(v::Text::of(format!("   {}{} · {}", if name.is_empty() || name == channel.alias { String::new() } else { format!("#{name} · ") }, channel.kind.word(), if channel.watched { "watched" } else { "not watched" }), v::Ink::Muted));
                row.right = check_mark_text(p, &format!("channel:{}", channel.id));
                row
            }
            ChRow::Field(k, field) => {
                let target = match field { ChField::Alias => Some(Target::Alias(k)), ChField::Purpose => Some(Target::Purpose(k)), _ => None };
                if target.as_ref().is_some_and(|target| p.edit.as_ref().is_some_and(|(edit, _)| edit == target)) { editing = editing_view(p, i, CH_FIELD_AT); }
                let mut text = v::Text::plain("  ").push(v::Seg::styled(if field == ChField::Watched { "└─ " } else { "├─ " }, p.cfg.style_for("lines")));
                text.extend(v::kit::labelled(field.label(), FIELD_LABEL_W, v::Text::new()));
                if field == ChField::Colour { text.add(v::Seg::styled("██ ", format!("slack.{}", p.cfg.channels[k].color))); }
                text.add(v::Seg::new(field_value(p, k, field), v::Ink::Primary));
                v::ListRow::new(format!("row:{i}"), text)
            }
        };
        out.target = v::Target::new(format!("row:{i}"));
        out
    }).collect();
    let mut detail = Vec::new();
    match ch_row(p) {
        ChRow::Add => {
            typing_text(p, &mut detail);
            if p.checks.contains_key("add") { detail.extend([v::kit::heading("latest channel check"), check_text(p, "add", now), v::Text::new()]); }
            detail.push(v::Text::plain("Read another channel."));
            detail_block(&mut detail, "paste or type any of", v::Text::of("  a link to the channel or to a message in it", v::Ink::Secondary));
            detail.extend([v::Text::of("  its id: C0123ABCD", v::Ink::Secondary), v::Text::of("  #name", v::Ink::Secondary), v::Text::new(),
                v::Text::of("Slack is asked which channel it is; it is added as a team channel, watched, called by its name in Slack. Change any of that below it.", v::Ink::Muted)]);
        }
        ChRow::Channel(k) => {
            let channel = &p.cfg.channels[k];
            detail.extend([v::kit::heading("checked"), check_text(p, &format!("channel:{}", channel.id), now), v::Text::new(),
                v::Text::plain(format!("#{}: {}.", channel.alias, channel.kind.about())), v::Text::new(),
                v::kit::labelled("in Slack", 10, v::Text::plain(if channel.name.is_empty() { "—".into() } else { format!("#{}", channel.name) })),
                v::kit::labelled("id", 10, v::Text::plain(channel.id.clone()))]);
            if let Some(info) = p.data.channels.get(&channel.id).and_then(|d| d.info.as_ref()) && !info.topic.is_empty() {
                detail.push(v::kit::labelled("topic", 10, v::Text::of(&info.topic, v::Ink::Secondary)));
            }
            config_text(&mut detail, "[[slack.channel]]");
        }
        ChRow::Field(k, field) => {
            typing_text(p, &mut detail);
            detail.push(v::Text::plain(field.about()));
            detail_block(&mut detail, "accepts", v::Text::of(field.accepts(), v::Ink::Secondary));
            detail_block(&mut detail, "now", v::Text::of(field_value(p, k, field), v::Ink::Emphasis));
            config_text(&mut detail, &format!("[[slack.channel]] {}", field.key()));
        }
    }
    settings_view(p, "channels", rows, detail, editing)
}

pub fn alerts_view(p: &SlackPlugin, width: u16) -> v::SectionView {
    let mut editing = None;
    let rows = al_rows(p).into_iter().enumerate().map(|(i, row)| match row {
        AlRow::Hours => {
            if p.edit.as_ref().is_some_and(|(target, _)| *target == Target::Hours) { editing = editing_view(p, i, KV_W as u16); }
            v::ListRow::new(format!("row:{i}"), v::kit::labelled("keep resolved", KV_W, v::Text::of(crate::alert_view::hours(p.cfg.alerts.recent_hours), v::Ink::Primary)))
        }
        AlRow::Add => {
            if p.edit.as_ref().is_some_and(|(target, _)| *target == Target::RuleAdd) { editing = editing_view(p, i, ADD_AT); }
            v::ListRow::new(format!("row:{i}"), slack_add_line("add a mapping", "the start of an alert's name, then where it belongs", width))
        }
        AlRow::Rule(k) => v::ListRow::new(format!("row:{i}"), v::Text::new()
            .push(v::Seg::new("▌ ", v::Ink::AccentSecondary))
            .push(v::Seg::new(rule_words(&p.cfg.alerts.rules[k]), v::Ink::Emphasis).strong())),
        AlRow::Field(k, field) => {
            let target = match field { RuleField::Prefix => Target::RulePrefix(k), RuleField::Label => Target::RuleLabel(k), RuleField::Repo => Target::RuleRepo(k), RuleField::Service => Target::RuleService(k), RuleField::Environment => Target::RuleEnvironment(k) };
            if p.edit.as_ref().is_some_and(|(edit, _)| *edit == target) { editing = editing_view(p, i, CH_FIELD_AT); }
            let value = field.of(&p.cfg.alerts.rules[k]);
            let mut line = v::Text::plain("  ").push(v::Seg::styled(if field == RuleField::Environment { "└─ " } else { "├─ " }, p.cfg.style_for("lines")));
            line.extend(v::kit::labelled(field.label(), FIELD_LABEL_W, v::Text::of(if value.is_empty() { "—" } else { value }, v::Ink::Primary)));
            v::ListRow::new(format!("row:{i}"), line)
        }
        AlRow::Unmapped(name) => {
            if p.edit.as_ref().is_some_and(|(target, _)| *target == Target::MapUnmapped(name.clone())) { editing = editing_view(p, i, 16); }
            let first = i == 0 || !matches!(al_rows(p).get(i - 1), Some(AlRow::Unmapped(_)));
            v::ListRow::new(format!("row:{i}"), v::Text::new()
                .push(v::Seg::new(if first { "no rule places  " } else { "                " }, v::Ink::Muted))
                .push(v::Seg::styled(name, p.cfg.style_for("unmapped"))))
        }
    }).collect();
    let mut detail = Vec::new();
    match al_row(p) {
        AlRow::Hours => {
            typing_text(p, &mut detail);
            detail.push(v::Text::plain("How long a resolved alert, and a job that failed, stays in the Alerts view. Firing alerts stay while they fire."));
            detail_block(&mut detail, "accepts", v::Text::of("whole hours, 1 to 720", v::Ink::Secondary));
            detail_block(&mut detail, "now", v::Text::of(crate::alert_view::hours(p.cfg.alerts.recent_hours), v::Ink::Emphasis));
            config_text(&mut detail, "[slack.alerts] recent_hours");
        }
        AlRow::Add => {
            typing_text(p, &mut detail);
            detail.extend([v::Text::plain("Say where a kind of alert belongs."), v::Text::new(),
                v::Text::of("Type the start of its name; the repo it belongs to is asked next. The service, a label to match and an environment can be set under the rule. The first rule that matches an alert decides.", v::Ink::Muted)]);
        }
        AlRow::Rule(k) => {
            if let Some(rule) = p.cfg.alerts.rules.get(k) {
                detail.extend([v::Text::plain(rule_words(rule)), v::Text::new(),
                    v::Text::of("Alerts this rule takes belong to its repo and service; the repo then shows a firing marker and an Alerts tab. Rules are tried from the top.", v::Ink::Muted)]);
                config_text(&mut detail, "[[slack.alerts.map]]");
            }
        }
        AlRow::Field(k, field) => {
            typing_text(p, &mut detail);
            detail.push(v::Text::plain(field.about()));
            detail_block(&mut detail, "accepts", v::Text::of(field.accepts(), v::Ink::Secondary));
            let now = p.cfg.alerts.rules.get(k).map_or("", |rule| field.of(rule));
            detail_block(&mut detail, "now", v::Text::of(if now.is_empty() { "—" } else { now }, v::Ink::Emphasis));
            config_text(&mut detail, &format!("[[slack.alerts.map]] {}", field.key()));
        }
        AlRow::Unmapped(name) => {
            typing_text(p, &mut detail);
            detail.extend([v::Text::plain(format!("No rule places alerts named {name}.")), v::Text::new(),
                v::Text::of("They still show in the Alerts view, under unmapped. Type the repo they belong to (its path, or a link to it) or a service's name, and a rule for names starting with it is added at the top.", v::Ink::Muted)]);
        }
    }
    settings_view(p, "alerts", rows, detail, editing)
}

pub fn review_view(p: &SlackPlugin) -> v::SectionView {
    let mut editing = None;
    let rows = RV_ROWS.iter().copied().enumerate().map(|(i, row)| {
        let target = match row { RvRow::Template => Some(Target::Template), RvRow::Link => Some(Target::Link), RvRow::Reask => Some(Target::Reask), _ => None };
        if target.as_ref().is_some_and(|target| p.edit.as_ref().is_some_and(|(edit, _)| edit == target)) { editing = editing_view(p, i, KV_W as u16); }
        let mut list = v::ListRow::new(format!("row:{i}"), v::kit::labelled(row.label(), KV_W, v::Text::of(rv_value(p, row), v::Ink::Primary)));
        if row == RvRow::Group { list.right = check_mark_text(p, "groups"); }
        if row == RvRow::Template && ask::check_template(&p.cfg.review.template, &p.data.groups).is_err() { list.right = Some(v::Text::of("✗", v::Ink::Bad)); }
        list
    }).collect();
    let row = rv_row(p);
    let (about, accepts, key) = match row {
        RvRow::Channel => ("Where review asks go. The user group is mentioned only here.", "a channel in the registry", "[slack.review] channel"),
        RvRow::Group => ("The user group mentioned in the review channel, so its people see the ask. Picked from Slack's groups.", "one of Slack's user groups", "[slack.review] group, handle"),
        RvRow::Template => ("The message: {mrs} is the merge request linked with its words, or one bullet each for several; {group} mentions the user group picked above, and {@handle} any of Slack's user groups by its handle. Both are sent as Slack's mention of the group's id, so it pings, and only in the review channel. :name: is an emoji.", "text with {mrs}; {group} and {@handle} optional", "[slack.review] template"),
        RvRow::Link => ("The text one merge request is linked with; several are each named by their repo and number.", "a few characters", "[slack.review] link"),
        RvRow::Test => ("A channel to try a review ask in first: change the draft's channel to it; no one is mentioned there.", "a channel in the registry", "[slack.review] test_channel"),
        RvRow::Reask => ("When an ask with no reply is due again, its merge request's slack column turns to a warning and it offers to ask again in the thread (asked first): a reply that mentions the group and links to the ask. Working days are Monday to Friday, in your Slack time zone.", "2d (working days) · 4h (working hours) · 10:00 (that time on the next working day) · never", "[slack.review] reask"),
    };
    let mut detail = Vec::new();
    typing_text(p, &mut detail);
    if row == RvRow::Group { detail.extend([v::kit::heading("Slack's groups"), check_text(p, "groups", p.now), v::Text::new()]); }
    if row == RvRow::Template && p.edit.is_none() {
        let verdict = ask::check_template(&p.cfg.review.template, &p.data.groups);
        detail.extend([v::kit::heading("checked"), v::kit::as_you_type(verdict.as_deref().map_err(String::as_str)), v::Text::new()]);
    }
    detail.push(v::Text::plain(about));
    detail_block(&mut detail, "accepts", v::Text::of(accepts, v::Ink::Secondary));
    detail_block(&mut detail, "now", v::Text::of(rv_value(p, row), v::Ink::Emphasis));
    let mut blocks: Vec<v::Block> = detail.iter().cloned().map(v::Block::line).collect();
    if matches!(row, RvRow::Template | RvRow::Link | RvRow::Channel | RvRow::Group) {
        let cfg = match &p.edit { Some((Target::Template, input)) => with_template(p, &input.value()), _ => p.cfg.clone() };
        let channel = cfg.review.channel.clone();
        blocks.extend([v::Block::line(v::Text::new()), v::Block::line(v::kit::heading("with a sample merge request")),
            v::Block::doc(view::doc(&p.data, &cfg, &ask::text(&cfg, &p.data.groups, &channel, &ask::sample())))]);
        let mut two = ask::sample();
        two.push(ask::Item { label: "web !12".into(), url: "https://git.example/acme/team-a/web/-/merge_requests/12".into(),
            id: "acme/team-a/web!12".into(), words: "Show the stats".into(), tickets: vec![] });
        blocks.extend([v::Block::line(v::Text::new()), v::Block::line(v::kit::heading("with two")),
            v::Block::doc(view::doc(&p.data, &cfg, &ask::text(&cfg, &p.data.groups, &channel, &two)))]);
    }
    config_text(&mut detail, key);
    blocks.extend([v::Block::line(v::Text::new()), v::Block::line(v::Text::of(format!("config: {key}"), v::Ink::Muted))]);
    let mut out = settings_view(p, "review", rows, detail, editing);
    out.detail_blocks = blocks;
    out
}

pub fn list_intent(p: &mut SlackPlugin, section: &str, intent: &v::Intent) -> Outcome {
    match intent {
        v::Intent::Select { target } | v::Intent::Context { target } => click(p, section, target.as_str()),
        v::Intent::Activate { target } => {
            click(p, section, target.as_str());
            key(p, section, radar_core::view_tui::key_of(v::Chord::new(v::Key::Enter)))
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => intent.chord().map_or(Outcome::Ignored, |chord| {
            key(p, section, radar_core::view_tui::key_of(chord))
        }),
        _ => Outcome::Ignored,
    }
}

pub fn detail_title(p: &SlackPlugin, section: &str) -> String {
    match section {
        "connection" => match ws_row(p) {
            WsRow::Offer(k) => p
                .offers()
                .get(k)
                .map_or_else(|| "found".into(), |o| found(&o.var).title()),
            WsRow::Channel(i) => format!("#{}", p.cfg.channels[i].alias),
            row => ws_label(row).into(),
        },
        "channels" => match ch_row(p) {
            ChRow::Add => "add a channel".into(),
            ChRow::Channel(i) => format!("#{}", p.cfg.channels[i].alias),
            ChRow::Field(_, f) => f.label().into(),
        },
        "review" => rv_row(p).label().into(),
        "alerts" => match al_row(p) {
            AlRow::Hours => "keep resolved".into(),
            AlRow::Add => "add a mapping".into(),
            AlRow::Rule(i) => p
                .cfg
                .alerts
                .rules
                .get(i)
                .map_or_else(|| "mapping".into(), |r| format!("{}…", r.prefix)),
            AlRow::Field(_, f) => f.label().into(),
            AlRow::Unmapped(name) => name,
        },
        _ if on_density(p) => "density".into(),
        _ => selected_look(p).map_or_else(|| "look".into(), |(w, _)| w.to_string()),
    }
}

/// The density row is the one selected in message look.
fn on_density(p: &SlackPlugin) -> bool {
    matches!(look_rows().get(p.section_rows[4]), Some(LookRow::Density))
}

// --- menus -----------------------------------------------------------------------------

/// A channel's options, in the channels section and in the connection's
/// list of them.
fn channel_menu(p: &SlackPlugin, i: usize) -> Menu {
    let c = &p.cfg.channels[i];
    Menu::new(
        format!("channel:{i}"),
        format!("#{}", c.alias),
        vec![
            MenuItem::new('c', "check it", "ask Slack for its name and whether you are in it", "check"),
            // Not w: the host keeps it (it writes config.toml).
            MenuItem::new(
                'v',
                if c.watched { "stop watching" } else { "watch it" },
                if c.watched { "kept in the registry, not read" } else { "read on every refresh" },
                "watch",
            ),
            MenuItem::new('a', "add a channel", "paste a link to it, or type #name", "add"),
            MenuItem::new('d', "remove…", "take it out of the registry", "remove"),
        ],
    )
}

fn row_menu(p: &SlackPlugin, section: &str) -> Option<Menu> {
    match section {
        "connection" => match ws_row(p) {
            WsRow::Gap | WsRow::ChannelsHeading | WsRow::ChannelAdd => None,
            WsRow::Channel(i) => Some(channel_menu(p, i)),
            WsRow::Offer(k) => {
                let q = found(&p.offers().get(k)?.var);
                Some(Menu::new(
                    format!("offer:{k}"),
                    q.question(),
                    vec![MenuItem::new('y', "yes, use it", q.does, "use")],
                ))
            }
            WsRow::Api => Some(Menu::new(
                "api",
                "api",
                vec![MenuItem::new(
                    'e',
                    "change it",
                    "https://slack.com, https://slack-gov.com, or http://127.0.0.1:PORT",
                    "edit",
                )],
            )),
            WsRow::Token => Some(Menu::new(
                "token",
                connection::TOKEN,
                vec![
                    MenuItem::new('e', "change it", "env:NAME, or a user token", "edit"),
                    MenuItem::new(
                        'c',
                        "check the connection",
                        "ask Slack whether it works, who the token is and what it may do",
                        "check",
                    ),
                ],
            )),
            WsRow::Workspace | WsRow::SignedIn | WsRow::Check | WsRow::Need(_) => Some(Menu::new(
                "token",
                ws_label(ws_row(p)),
                vec![MenuItem::new(
                    'c',
                    "check the connection",
                    "ask Slack whether it works, who the token is and what it may do",
                    "check",
                )],
            )),
            WsRow::Refresh => Some(Menu::new(
                "refresh",
                connection::REFRESH,
                vec![MenuItem::new(
                    'e',
                    "change it",
                    connection::refresh_accepts(30),
                    "edit",
                )],
            )),
            WsRow::Days => Some(Menu::new(
                "days",
                "read back",
                vec![MenuItem::new(
                    'e',
                    "change it",
                    "whole days, 1 to 30",
                    "edit",
                )],
            )),
        },
        "channels" => match ch_row(p) {
            ChRow::Add => None,
            ChRow::Channel(i) => Some(channel_menu(p, i)),
            ChRow::Field(i, f) => {
                let items = match f {
                    ChField::Alias | ChField::Purpose => {
                        vec![MenuItem::new('e', "change it", f.accepts(), "edit")]
                    }
                    ChField::Kind => vec![MenuItem::new(
                        'e',
                        "change it…",
                        "team, service, test or alerts",
                        "kind",
                    )],
                    ChField::Reader => vec![MenuItem::new(
                        'e',
                        "change it…",
                        "how its messages become alerts",
                        "reader",
                    )],
                    ChField::Colour => vec![MenuItem::new(
                        'e',
                        "change it…",
                        "pick the channel's colour",
                        "colour",
                    )],
                    ChField::Watched => vec![MenuItem::new(
                        'e',
                        if p.cfg.channels[i].watched {
                            "stop watching"
                        } else {
                            "watch it"
                        },
                        "read on every refresh, or not",
                        "watch",
                    )],
                };
                Some(Menu::new(
                    format!("chfield:{i}:{}", f.label()),
                    f.label(),
                    items,
                ))
            }
        },
        "review" => {
            let r = rv_row(p);
            let item = match r {
                RvRow::Channel | RvRow::Test => {
                    MenuItem::new('e', "pick it…", "from the channels in the registry", "pick")
                }
                RvRow::Group => MenuItem::new('e', "pick it…", "from Slack's user groups", "pick"),
                _ => MenuItem::new('e', "change it", "type it", "edit"),
            };
            let mut items = vec![item];
            let rv = &p.cfg.review;
            let changed = match r {
                RvRow::Template => rv.template != config::default_template(),
                RvRow::Link => rv.link != "mr",
                _ => false,
            };
            if changed {
                items.push(MenuItem::new(
                    'r',
                    "back to the default",
                    "the team's format",
                    "reset",
                ));
            }
            let set = match r {
                RvRow::Group => !rv.group.is_empty(),
                RvRow::Test => !rv.test_channel.is_empty(),
                RvRow::Channel => !rv.channel.is_empty(),
                _ => false,
            };
            // c, not x: x is the host's (the actions menu), so x never
            // reached this row.
            if set {
                items.push(MenuItem::new('c', "clear it", "none", "clear"));
            }
            Some(Menu::new(format!("review:{}", r.label()), r.label(), items))
        }
        "alerts" => match al_row(p) {
            AlRow::Hours => Some(Menu::new(
                "alhours",
                "keep resolved",
                vec![MenuItem::new(
                    'e',
                    "change it",
                    "whole hours, 1 to 720",
                    "edit",
                )],
            )),
            AlRow::Add => None,
            AlRow::Rule(i) => Some(Menu::new(
                format!("rule:{i}"),
                format!("{}…", p.cfg.alerts.rules.get(i)?.prefix),
                [MenuItem::new(
                    'a',
                    "add a mapping",
                    "the start of an alert's name, then where it belongs",
                    "add",
                )]
                .into_iter()
                .chain((i > 0).then(|| MenuItem::new('<', "move up", "tried earlier", "up")))
                .chain(
                    (i + 1 < p.cfg.alerts.rules.len())
                        .then(|| MenuItem::new('>', "move down", "tried later", "down")),
                )
                .chain([MenuItem::new(
                    'd',
                    "remove…",
                    "alerts it placed become unmapped, unless another rule does",
                    "remove",
                )])
                .collect(),
            )),
            AlRow::Field(i, f) => {
                let set = !f.of(p.cfg.alerts.rules.get(i)?).is_empty();
                let mut items = vec![MenuItem::new('e', "change it", f.accepts(), "edit")];
                if set && f != RuleField::Prefix {
                    items.push(MenuItem::new('c', "clear it", "none", "clear"));
                }
                Some(Menu::new(
                    format!("rfield:{i}:{}", f.key()),
                    f.label(),
                    items,
                ))
            }
            AlRow::Unmapped(name) => {
                let k = unmapped_names(p).iter().position(|n| *n == name)?;
                Some(Menu::new(
                    format!("unmapped:{k}"),
                    name,
                    vec![MenuItem::new(
                        'e',
                        "map it…",
                        "a repo's path or a service's name",
                        "map",
                    )],
                ))
            }
        },
        _ if on_density(p) => Some(Menu::new(
            "density",
            "density",
            vec![MenuItem::new(
                'e',
                format!("switch to {}", p.cfg.density.other().word()),
                "compact is one line a message, cozy the roomier layout",
                "toggle",
            )],
        )),
        _ => {
            let (w, _) = selected_look(p)?;
            let spec = p.cfg.style_for(w);
            let has = |m: &str| spec.split_whitespace().any(|x| x == m);
            let onoff = |m: &str| {
                if has(m) {
                    "on, turn off"
                } else {
                    "off, turn on"
                }
            };
            let mut items = vec![MenuItem::new(
                'c',
                "colour…",
                "pick from the theme's colours",
                "colour",
            )];
            if !is_background(w) {
                items.extend([
                    MenuItem::new('o', "bold", onoff("bold"), "bold"),
                    MenuItem::new('i', "italic", onoff("italic"), "italic"),
                    MenuItem::new('u', "underline", onoff("underline"), "underline"),
                    MenuItem::new('d', "dim", onoff("dim"), "dim"),
                ]);
            }
            if default_styles().get(w).is_some_and(|d| *d != spec) {
                items.push(MenuItem::new(
                    'r',
                    "back to default",
                    look_summary(&default_styles()[w]),
                    "reset",
                ));
            }
            Some(Menu::new(format!("look:{w}"), w, items))
        }
    }
}

fn colour_menu(id: String, title: &str, current: &str) -> Menu {
    let items = radar_core::theme::SLOT_GROUPS
        .iter()
        .filter(|g| !matches!(**g, "plugins"))
        .flat_map(|g| {
            Slot::ALL
                .iter()
                .map(|s| s.token_name())
                .filter(move |n| slot_info(n).group == *g)
        })
        .map(|name| {
            let info = slot_info(name);
            let hint = if name == current {
                format!("in use · {}", info.meaning)
            } else {
                info.meaning
            };
            MenuItem::pick(info.label, hint, name).sample(name)
        })
        .collect();
    Menu::new(id, title, items)
}

fn channel_pick(p: &SlackPlugin, id: &str, title: &str, now: &str) -> Effect {
    let items = p
        .cfg
        .channels
        .iter()
        .map(|c| {
            let hint = if c.alias == now {
                format!("{} · now", c.kind.word())
            } else {
                c.kind.word().to_string()
            };
            MenuItem::pick(format!("#{}", c.alias), hint, c.alias.clone())
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return Effect::Notify("no channels yet: add one under channels".into());
    }
    Effect::Menu(Menu::new(id, title, items))
}

fn group_menu(p: &SlackPlugin) -> Effect {
    let mut groups: Vec<&UserGroup> = p.data.groups.iter().collect();
    // Yours first.
    let me = p
        .data
        .me
        .as_ref()
        .map(|m| m.user_id.clone())
        .unwrap_or_default();
    groups.sort_by_key(|g| (!g.users.contains(&me), g.handle.to_lowercase()));
    let items = groups
        .into_iter()
        .map(|g| {
            let mut hint = g.name.clone();
            if g.users.contains(&me) {
                hint = format!("{hint} · you are in it");
            }
            if g.id == p.cfg.review.group {
                hint = format!("{hint} · now");
            }
            MenuItem::pick(format!("@{}", g.handle), hint, g.id.clone())
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return Effect::Notify("Slack has no user groups for this token".into());
    }
    Effect::Menu(Menu::new(
        "review-group",
        "mention which user group?",
        items,
    ))
}

fn row_of(p: &SlackPlugin, row: ChRow) -> usize {
    ch_rows(p).iter().position(|r| *r == row).unwrap_or(0)
}

pub fn on_menu(p: &mut SlackPlugin, menu: &str, value: &str) -> Vec<Effect> {
    let (kind, arg) = menu.split_once(':').unwrap_or((menu, ""));
    let idx = |s: &str| s.split(':').next().and_then(|n| n.parse::<usize>().ok());
    let channels = p.cfg.channels.len();
    match (kind, value) {
        ("ask-check", "yes") if arg == "workspace" => check_token(p),
        ("ask-check", "yes") => match arg.strip_prefix("channel:") {
            Some(id) => check_channel(p, arg.to_string(), Ok(id.to_string())),
            None => vec![],
        },
        ("ask-check", _) => vec![],
        ("offer", "use") => {
            let Some(o) = idx(arg).and_then(|k| p.offers().get(k).cloned()) else {
                return vec![];
            };
            p.replace_token(format!("env:{}", o.var));
            p.section_rows[0] = ws_rows(p)
                .iter()
                .position(|r| *r == WsRow::Token)
                .unwrap_or(0);
            let mut fx = vec![Effect::Notify(format!("the token reads env:{}", o.var))];
            fx.extend(check_token(p));
            fx
        }
        ("token", "edit") => {
            // A typed token is not shown again: start from its env: name or empty.
            let now = &p.cfg.token;
            let start = if now.starts_with("env:") {
                now.clone()
            } else {
                String::new()
            };
            p.edit = Some((Target::Token, TextInput::new(&start)));
            vec![]
        }
        ("token", "check") => check_token(p),
        ("density", "toggle") => {
            p.cfg.density = p.cfg.density.other();
            vec![Effect::Notify(format!(
                "messages are {}",
                p.cfg.density.word()
            ))]
        }
        ("api", "edit") => {
            p.edit = Some((Target::Api, TextInput::new(p.cfg.api.trim())));
            vec![]
        }
        ("refresh", "edit") => {
            p.edit = Some((
                Target::Refresh,
                TextInput::new(&p.cfg.refresh_secs.to_string()),
            ));
            vec![]
        }
        ("days", "edit") => {
            p.edit = Some((Target::Days, TextInput::new(&p.cfg.days.to_string())));
            vec![]
        }
        ("channel", v) => {
            let Some(i) = idx(arg).filter(|i| *i < channels) else {
                return vec![];
            };
            match v {
                "check" => {
                    let id = p.cfg.channels[i].id.clone();
                    check_channel(p, format!("channel:{id}"), Ok(id))
                }
                "watch" => {
                    let c = &mut p.cfg.channels[i];
                    c.watched = !c.watched;
                    let say = if c.watched {
                        format!("#{} watched", c.alias)
                    } else {
                        format!("#{} not watched", c.alias)
                    };
                    vec![Effect::Notify(say)]
                }
                "add" => {
                    p.section_rows[1] = 0;
                    p.edit = Some((Target::ChannelAdd, TextInput::new("")));
                    vec![]
                }
                "remove" => vec![Effect::Menu(
                    Menu::confirm(
                        format!("remove:{i}"),
                        format!("take #{} out of the registry?", p.cfg.channels[i].alias),
                        "take it out",
                        "keep it",
                    )
                    .saying("it is no longer read; its mark is kept"),
                )],
                _ => vec![],
            }
        }
        ("remove", "yes") => {
            let Some(i) = idx(arg).filter(|i| *i < channels) else {
                return vec![];
            };
            let c = p.cfg.channels.remove(i);
            p.data.channels.remove(&c.id);
            p.section_rows[1] = p.section_rows[1].min(ch_rows(p).len() - 1);
            vec![Effect::Notify(format!("#{} taken out", c.alias))]
        }
        ("chfield", v) => {
            let Some(i) = idx(arg).filter(|i| *i < channels) else {
                return vec![];
            };
            match v {
                "edit" => {
                    let f = arg.split(':').nth(1).unwrap_or("");
                    let (target, now) = if f == "alias" {
                        (Target::Alias(i), p.cfg.channels[i].alias.clone())
                    } else {
                        (Target::Purpose(i), p.cfg.channels[i].purpose.clone())
                    };
                    p.edit = Some((target, TextInput::new(&now)));
                    vec![]
                }
                "kind" => vec![Effect::Menu(Menu::new(
                    format!("kind:{i}"),
                    format!("#{} is", p.cfg.channels[i].alias),
                    Kind::ALL
                        .iter()
                        .map(|k| MenuItem::pick(k.word(), k.about(), k.word()))
                        .collect(),
                ))],
                "colour" => vec![Effect::Menu(Menu::new(
                    format!("colour:{i}"),
                    "channel colour",
                    CHANNEL_COLORS
                        .iter()
                        .map(|c| {
                            let hint = if p.cfg.channels[i].color == *c {
                                "in use"
                            } else {
                                ""
                            };
                            MenuItem::pick(*c, hint, *c).sample(format!("slack.{c}"))
                        })
                        .collect(),
                ))],
                "reader" => vec![Effect::Menu(Menu::new(
                    format!("reader:{i}"),
                    format!("#{}'s messages are read as", p.cfg.channels[i].alias),
                    Reader::ALL
                        .iter()
                        .map(|r| {
                            let now = p.cfg.channels[i].reader == Some(*r);
                            MenuItem::pick(
                                r.word(),
                                if now {
                                    format!("{} · now", r.about())
                                } else {
                                    r.about().to_string()
                                },
                                r.word(),
                            )
                        })
                        .collect(),
                ))],
                "watch" => on_menu(p, &format!("channel:{i}"), "watch"),
                _ => vec![],
            }
        }
        ("reader", r) => {
            if let (Some(i), Some(reader)) = (
                idx(arg).filter(|i| *i < channels),
                Reader::ALL.iter().find(|x| x.word() == r),
            ) {
                p.cfg.channels[i].reader = Some(*reader);
            }
            vec![]
        }
        ("kind", k) => {
            if let (Some(i), Some(kind)) = (
                idx(arg).filter(|i| *i < channels),
                Kind::ALL.iter().find(|x| x.word() == k),
            ) {
                let c = &mut p.cfg.channels[i];
                c.kind = *kind;
                // An alerts channel is read by a reader (the usual one until
                // chosen); any other has none.
                if *kind == Kind::Alerts {
                    c.reader = c.reader.or(Some(Reader::Alertmanager));
                } else {
                    c.reader = None;
                }
            }
            vec![]
        }
        ("alhours", "edit") => {
            p.edit = Some((
                Target::Hours,
                TextInput::new(&p.cfg.alerts.recent_hours.to_string()),
            ));
            vec![]
        }
        ("rule", v) => {
            let Some(i) = idx(arg).filter(|i| *i < p.cfg.alerts.rules.len()) else {
                return vec![];
            };
            match v {
                "add" => {
                    p.section_rows[3] = 0;
                    p.edit = Some((Target::RuleAdd, TextInput::new("")));
                    vec![]
                }
                // The rules are tried from the top: their order matters.
                "up" | "down" => {
                    let j = if v == "up" { i.wrapping_sub(1) } else { i + 1 };
                    if j < p.cfg.alerts.rules.len() {
                        p.cfg.alerts.rules.swap(i, j);
                        p.section_rows[3] = al_rows(p)
                            .iter()
                            .position(|r| *r == AlRow::Rule(j))
                            .unwrap_or(0);
                    }
                    vec![]
                }
                "remove" => vec![Effect::Menu(
                    Menu::confirm(
                        format!("rule-remove:{i}"),
                        format!(
                            "remove the mapping for {}?",
                            rule_words(&p.cfg.alerts.rules[i])
                        ),
                        "remove it",
                        "keep it",
                    )
                    .saying("its alerts are placed by the next rule that matches, else unmapped"),
                )],
                _ => vec![],
            }
        }
        ("rule-remove", "yes") => {
            let Some(i) = idx(arg).filter(|i| *i < p.cfg.alerts.rules.len()) else {
                return vec![];
            };
            p.cfg.alerts.rules.remove(i);
            p.section_rows[3] = p.section_rows[3].min(al_rows(p).len() - 1);
            vec![Effect::Notify("mapping removed".into())]
        }
        ("rfield", v) => {
            let Some(i) = idx(arg).filter(|i| *i < p.cfg.alerts.rules.len()) else {
                return vec![];
            };
            let Some(f) = RuleField::ALL
                .iter()
                .copied()
                .find(|f| Some(f.key()) == arg.split(':').nth(1))
            else {
                return vec![];
            };
            match v {
                "edit" => {
                    let target = match f {
                        RuleField::Prefix => Target::RulePrefix(i),
                        RuleField::Label => Target::RuleLabel(i),
                        RuleField::Repo => Target::RuleRepo(i),
                        RuleField::Service => Target::RuleService(i),
                        RuleField::Environment => Target::RuleEnvironment(i),
                    };
                    p.edit = Some((target, TextInput::new(f.of(&p.cfg.alerts.rules[i]))));
                    vec![]
                }
                "clear" => {
                    let r = &mut p.cfg.alerts.rules[i];
                    match f {
                        RuleField::Prefix => {}
                        RuleField::Label => r.label.clear(),
                        RuleField::Repo => r.repo.clear(),
                        RuleField::Service => r.service.clear(),
                        RuleField::Environment => r.environment.clear(),
                    }
                    // A rule with neither a prefix nor a label takes nothing.
                    let r = &p.cfg.alerts.rules[i];
                    if r.prefix.is_empty() && r.label.is_empty() {
                        vec![Effect::Notify(
                            "that rule now takes no alerts: give it a name prefix or a label"
                                .into(),
                        )]
                    } else {
                        vec![]
                    }
                }
                _ => vec![],
            }
        }
        ("unmapped", "map") => {
            let Some(name) = idx(arg).and_then(|k| unmapped_names(p).get(k).cloned()) else {
                return vec![];
            };
            p.edit = Some((Target::MapUnmapped(name), TextInput::new("")));
            vec![]
        }
        ("colour", c) => {
            if let Some(i) = idx(arg).filter(|i| *i < channels)
                && CHANNEL_COLORS.contains(&c)
            {
                p.cfg.channels[i].color = c.to_string();
            }
            vec![]
        }
        ("review", v) => {
            let r = rv_row(p);
            match (r, v) {
                (RvRow::Channel, "pick") => vec![channel_pick(
                    p,
                    "review-channel",
                    "ask for review in",
                    &p.cfg.review.channel.clone(),
                )],
                (RvRow::Test, "pick") => vec![channel_pick(
                    p,
                    "review-test",
                    "try review asks in",
                    &p.cfg.review.test_channel.clone(),
                )],
                // Always Slack's list as it is now, so what is picked exists.
                (RvRow::Group, "pick") => fetch_groups(p, true),
                (RvRow::Template, "edit") => {
                    p.edit = Some((Target::Template, TextInput::new(&p.cfg.review.template)));
                    // {@handle} is checked against Slack's groups: read them
                    // if they are not held yet.
                    if p.data.groups.is_empty() {
                        fetch_groups(p, false)
                    } else {
                        vec![crate::settings_history::groups_cached(p.data.groups.len())]
                    }
                }
                (RvRow::Link, "edit") => {
                    p.edit = Some((Target::Link, TextInput::new(&p.cfg.review.link)));
                    vec![]
                }
                (RvRow::Reask, "edit") => {
                    p.edit = Some((Target::Reask, TextInput::new(&p.cfg.review.reask().spec())));
                    vec![]
                }
                (RvRow::Template, "reset") => {
                    p.cfg.review.template = config::default_template();
                    vec![]
                }
                (RvRow::Link, "reset") => {
                    p.cfg.review.link = "mr".into();
                    vec![]
                }
                (RvRow::Channel, "clear") => {
                    p.cfg.review.channel.clear();
                    vec![]
                }
                (RvRow::Test, "clear") => {
                    p.cfg.review.test_channel.clear();
                    vec![]
                }
                (RvRow::Group, "clear") => {
                    p.cfg.review.group.clear();
                    p.cfg.review.handle.clear();
                    vec![]
                }
                _ => vec![],
            }
        }
        ("review-channel", alias) => {
            p.cfg.review.channel = alias.to_string();
            vec![Effect::Notify(format!("review asks go to #{alias}"))]
        }
        ("review-test", alias) => {
            p.cfg.review.test_channel = alias.to_string();
            vec![Effect::Notify(format!(
                "review asks can be tried in #{alias}"
            ))]
        }
        ("review-group", id) => {
            if let Some(g) = p.data.groups.iter().find(|g| g.id == id) {
                p.cfg.review.group = g.id.clone();
                p.cfg.review.handle = g.handle.clone();
                return vec![Effect::Notify(format!(
                    "@{} is mentioned in review asks",
                    g.handle
                ))];
            }
            vec![]
        }
        ("look", v) => {
            let word = arg.to_string();
            let spec = p.cfg.style_for(&word);
            match v {
                "colour" => {
                    let current = spec.split_whitespace().next().unwrap_or("").to_string();
                    return vec![Effect::Menu(colour_menu(
                        format!("look-colour:{word}"),
                        "colour",
                        &current,
                    ))];
                }
                "reset" => {
                    if let Some(d) = default_styles().get(word.as_str()) {
                        p.cfg.style.insert(word.clone(), d.clone());
                    }
                }
                m => {
                    let mut words: Vec<String> =
                        spec.split_whitespace().map(String::from).collect();
                    match words.iter().skip(1).position(|x| x == m) {
                        Some(at) => {
                            words.remove(at + 1);
                        }
                        None => words.push(m.to_string()),
                    }
                    p.cfg.style.insert(word, words.join(" "));
                }
            }
            vec![]
        }
        ("look-colour", slot) => {
            let word = arg.to_string();
            let spec = p.cfg.style_for(&word);
            let mods: Vec<&str> = spec.split_whitespace().skip(1).collect();
            let new = std::iter::once(slot)
                .chain(mods)
                .collect::<Vec<_>>()
                .join(" ");
            p.cfg.style.insert(word, new);
            vec![]
        }
        _ => vec![],
    }
}

pub fn bulk(p: &SlackPlugin, section: &str) -> Vec<MenuItem> {
    match section {
        "connection" if !p.cfg.token.trim().is_empty() => vec![MenuItem::new(
            'c',
            "check everything",
            "the token, then each channel",
            "check-all",
        )],
        "channels" if !p.cfg.channels.is_empty() => vec![MenuItem::new(
            'c',
            "check every channel",
            "ask Slack for each one's name and whether you are in it",
            "ch-check-all",
        )],
        "look" => {
            let changed = default_styles()
                .iter()
                .filter(|(w, d)| p.cfg.style_for(w) != **d)
                .count();
            if changed == 0 {
                return vec![];
            }
            vec![MenuItem::new(
                'r',
                "every word back to its default",
                format!("{changed} changed"),
                "sl-reset-all",
            )]
        }
        _ => vec![],
    }
}

pub fn on_bulk(p: &mut SlackPlugin, value: &str) -> Vec<Effect> {
    match value {
        "check-all" => {
            let ids = p
                .cfg
                .channels
                .iter()
                .map(|channel| channel.id.clone())
                .collect();
            check_token_then(p, ids)
        }
        "ch-check-all" => {
            let ids: Vec<String> = p.cfg.channels.iter().map(|c| c.id.clone()).collect();
            ids.into_iter()
                .flat_map(|id| check_channel(p, format!("channel:{id}"), Ok(id)))
                .collect()
        }
        "sl-reset-all" => {
            p.cfg.style = default_styles();
            vec![Effect::Notify("every word back to its default".into())]
        }
        _ => vec![],
    }
}

// --- keys --------------------------------------------------------------------------------

pub fn key(p: &mut SlackPlugin, section: &str, key: KeyEvent) -> Outcome {
    if let Some((target, mut input)) = p.edit.take() {
        return match input.key(key) {
            Edit::Cancel => Outcome::done(),
            Edit::Typing => {
                p.edit = Some((target, input));
                Outcome::done()
            }
            Edit::Submit(text) => match validate(p, &target, &text) {
                Err(why) => {
                    p.edit = Some((target, input));
                    Outcome::Handled(vec![Effect::Notify(format!("not yet: {why}"))])
                }
                Ok(_) => Outcome::Handled(commit(p, target, text.trim().to_string())),
            },
        };
    }
    if section == "connection"
        && p.draft.is_some()
        && key.modifiers.is_empty()
        && key.code == KeyCode::Char('y')
    {
        return Outcome::Handled(p.copy_ask_draft());
    }
    let at = index(section);
    if let Some(mv) = Move::from_key(&key, p.section_scroll.height()) {
        // Headings are not selectable, in look and in connection.
        let ws = ws_rows(p);
        let can = |row: usize| match section {
            "look" => !matches!(look_rows().get(row), Some(LookRow::Header(_))),
            "connection" => ws.get(row).is_none_or(|r| r.selectable()),
            _ => true,
        };
        p.section_rows[at] = select(row_count(p, section), p.section_rows[at], mv, can);
        return Outcome::done();
    }
    if key.code == KeyCode::Enter {
        // A found token's question: ⏎ uses it. The check row: ⏎ asks.
        // (Every other row's menu has one thing, or starts with change it,
        // so ⏎ does that straight away, below.)
        if matches!((section, ch_row(p)), ("channels", ChRow::Add))
            || (section == "connection" && ws_row(p) == WsRow::ChannelAdd)
        {
            p.edit = Some((Target::ChannelAdd, TextInput::new("")));
            return Outcome::done();
        }
        if section == "alerts" && al_row(p) == AlRow::Add {
            p.edit = Some((Target::RuleAdd, TextInput::new("")));
            return Outcome::done();
        }
        let Some(menu) = row_menu(p, section) else {
            return Outcome::done();
        };
        // ⏎ goes straight to changing the row (typing, or choosing from
        // what it can be) when that is what the row is for; a row with
        // several different things to do offers them.
        let direct =
            menu.items.len() == 1 || menu.items.first().is_some_and(|i| i.key == Some('e'));
        if direct && let Some(first) = menu.items.first() {
            let value = first.value.clone();
            return Outcome::Handled(on_menu(p, &menu.id, &value));
        }
        return Outcome::Handled(vec![Effect::Menu(menu)]);
    }
    if let KeyCode::Char(c) = key.code
        && let Some(menu) = row_menu(p, section)
        && let Some(item) = menu.items.iter().find(|i| i.key == Some(c))
    {
        let value = item.value.clone();
        return Outcome::Handled(on_menu(p, &menu.id, &value));
    }
    match (section, key.code) {
        // radar's app manifest: `m` copies it, `o` opens Slack's new-app
        // page with it filled in.
        ("connection", KeyCode::Char('m')) => Outcome::Handled(vec![
            Effect::Copy(crate::manifest::text()),
            Effect::Notify(format!("copied radar's Slack app manifest: {} user scopes", crate::manifest::scopes().len())),
        ]),
        ("connection", KeyCode::Char('o')) => Outcome::Handled(vec![Effect::Open(crate::manifest::new_app_url())]),
        ("channels", KeyCode::Char('a')) => {
            p.section_rows[1] = 0;
            p.edit = Some((Target::ChannelAdd, TextInput::new("")));
            Outcome::done()
        }
        ("alerts", KeyCode::Char('a')) => {
            p.section_rows[3] = 0;
            p.edit = Some((Target::RuleAdd, TextInput::new("")));
            Outcome::done()
        }
        _ => Outcome::Ignored,
    }
}

fn commit(p: &mut SlackPlugin, target: Target, value: String) -> Vec<Effect> {
    match target {
        Target::Token => {
            p.replace_token(value);
            vec![ask_to_check("workspace", "the token")]
        }
        Target::Api => {
            let value = value.trim().trim_end_matches('/');
            let value = if value.is_empty() { config::DEFAULT_API } else { value };
            if let Some(why) = config::api_problem(value) {
                return vec![crate::view::failed(&why)];
            }
            p.replace_api(value.to_string());
            vec![ask_to_check("workspace", "the connection")]
        }
        Target::Refresh => {
            p.cfg.refresh_secs = value
                .trim_end_matches('s')
                .parse()
                .unwrap_or(p.cfg.refresh_secs);
            vec![Effect::Notify("changed".into())]
        }
        Target::Days => {
            p.cfg.days = value
                .trim_end_matches('d')
                .trim()
                .parse()
                .unwrap_or(p.cfg.days);
            vec![Effect::Notify(
                "changed; the next read goes back that far".into(),
            )]
        }
        Target::ChannelAdd => check_channel(p, "add".into(), config::channel_ref(&value)),
        Target::Alias(i) => {
            let old = std::mem::replace(&mut p.cfg.channels[i].alias, value.clone());
            for a in [&mut p.cfg.review.channel, &mut p.cfg.review.test_channel] {
                if *a == old {
                    *a = value.clone();
                }
            }
            vec![Effect::Notify(format!("#{old} is now #{value}"))]
        }
        Target::Purpose(i) => {
            p.cfg.channels[i].purpose = value;
            vec![]
        }
        Target::Template => {
            p.cfg.review.template = value;
            vec![Effect::Notify("template kept".into())]
        }
        Target::Link => {
            p.cfg.review.link = value;
            vec![]
        }
        Target::Reask => {
            if let Ok(r) = config::Reask::parse(&value) {
                p.cfg.review.set_reask(r);
            }
            vec![Effect::Notify(format!(
                "asks again {}",
                p.cfg.review.reask().words()
            ))]
        }
        Target::Hours => {
            p.cfg.alerts.recent_hours = value
                .trim_end_matches('h')
                .trim()
                .parse()
                .unwrap_or(p.cfg.alerts.recent_hours);
            vec![]
        }
        Target::RuleAdd => {
            p.cfg.alerts.rules.insert(
                0,
                Rule {
                    prefix: value,
                    ..Rule::default()
                },
            );
            // Where they belong, asked straight away.
            p.section_rows[3] = al_rows(p)
                .iter()
                .position(|r| *r == AlRow::Field(0, RuleField::Repo))
                .unwrap_or(0);
            p.edit = Some((Target::RuleRepo(0), TextInput::new("")));
            vec![]
        }
        Target::RulePrefix(i)
        | Target::RuleLabel(i)
        | Target::RuleRepo(i)
        | Target::RuleService(i)
        | Target::RuleEnvironment(i) => {
            let field = match target {
                Target::RulePrefix(_) => RuleField::Prefix,
                Target::RuleLabel(_) => RuleField::Label,
                Target::RuleRepo(_) => RuleField::Repo,
                Target::RuleService(_) => RuleField::Service,
                _ => RuleField::Environment,
            };
            if let Some(r) = p.cfg.alerts.rules.get_mut(i) {
                let value = if field == RuleField::Repo && !value.is_empty() {
                    crate::alert_view::repo_path(&value).unwrap_or(value)
                } else {
                    value
                };
                match field {
                    RuleField::Prefix => r.prefix = value,
                    RuleField::Label => r.label = value,
                    RuleField::Repo => r.repo = value,
                    RuleField::Service => r.service = value,
                    RuleField::Environment => r.environment = value,
                }
            }
            vec![]
        }
        Target::MapUnmapped(name) => {
            if let Ok(rule) = crate::alert_view::rule_for(&name, &value) {
                crate::alert_view::add_rule(p, rule);
            }
            vec![Effect::Notify(format!("alerts named {name}… are placed"))]
        }
    }
}

pub fn click(p: &mut SlackPlugin, section: &str, id: &str) -> Outcome {
    if p.edit.is_some() {
        return Outcome::done();
    }
    if let Some(i) = id
        .strip_prefix("row:")
        .and_then(|n| n.parse::<usize>().ok())
    {
        // A heading has no click area, so only a row that can be selected gets here.
        p.section_rows[index(section)] = i;
    }
    Outcome::done()
}

/// The section's actions as data, each under its own stable id
/// (`slack.<section>.<verb>`), with the kept-replies key every section has.
pub fn action_specs(p: &SlackPlugin, section: &str) -> Vec<v::ActionSpec> {
    let mut rows = key_rows(p, section);
    rows.push((
        format!("slack.{section}.kept_replies"),
        Binding::new("ctrl-r", "kept replies…"),
    ));
    rows.iter().map(|(id, b)| v::ActionSpec::named(id.as_str(), b)).collect()
}

/// A section's key rows, each with the id its action has.
fn key_rows(p: &SlackPlugin, section: &str) -> Vec<(String, Binding)> {
    let mut rows = row_keys(p, section);
    if section == "connection" && p.draft.is_some() {
        rows.push((
            "slack.connection.copy_review_ask".into(),
            Binding::new("y", "copy the retained review ask text"),
        ));
    }
    if section == "connection" {
        rows.extend([
            ("slack.connection.copy_manifest".into(), Binding::new("m", "copy radar's Slack app manifest")),
            ("slack.connection.new_app".into(), Binding::new("o", "open Slack's new-app page, the manifest filled in")),
        ]);
    }
    rows
}

fn row_keys(p: &SlackPlugin, section: &str) -> Vec<(String, Binding)> {
    let id = |verb: &str| format!("slack.{section}.{verb}");
    match (section, ws_row(p), ch_row(p)) {
        ("connection", WsRow::Offer(k), _) => {
            let Some(o) = p.offers().get(k).cloned() else {
                return vec![];
            };
            return vec![
                (String::new(), Binding::heading(found(&o.var).title())),
                (id("use"), Binding::new("⏎", "use it")),
            ];
        }
        ("connection", WsRow::Check, _) => {
            return vec![
                (String::new(), Binding::heading(connection::CHECK)),
                (id("check"), Binding::new("⏎", "ask Slack now")),
            ];
        }
        ("channels", _, ChRow::Add) => {
            return vec![
                (String::new(), Binding::heading("add a channel")),
                (id("add"), Binding::new("⏎", "paste a link to it, or type #name")),
            ];
        }
        ("connection", WsRow::Gap | WsRow::ChannelsHeading | WsRow::ChannelAdd, _) => {
            return vec![
                (String::new(), Binding::heading("add a channel")),
                (id("add_channel"), Binding::new("⏎", "paste a link to it, or type #name")),
            ];
        }
        ("alerts", _, _) if al_row(p) == AlRow::Add => {
            return vec![
                (String::new(), Binding::heading("add a mapping")),
                (id("add_mapping"), Binding::new("⏎", "type the start of an alert's name")),
            ];
        }
        _ => {}
    }
    let Some(menu) = row_menu(p, section) else {
        return vec![];
    };
    let mut b = vec![(String::new(), Binding::heading(menu.title))];
    b.extend(
        menu.items
            .iter()
            .filter_map(|i| Some((id(&i.value), Binding::new(key_text(i.key?), &i.label)))),
    );
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_core::ratatui::crossterm::event::KeyModifiers;

    fn identity(team: &str, user: &str, tz_offset: Option<i64>) -> crate::client::Me {
        crate::client::Me {
            team_id: team.into(),
            user_id: user.into(),
            tz_offset,
            ..Default::default()
        }
    }

    fn recorded(body: serde_json::Value) -> crate::Via {
        crate::Via(Some(crate::client::Recorded::new(vec![Ok(
            crate::client::Reply {
                status: 200,
                body: body.to_string(),
                ..Default::default()
            },
        )])))
    }

    fn task(fx: Vec<Effect>) -> radar_core::plugin::Task {
        fx.into_iter()
            .find_map(|e| match e {
                Effect::Spawn(task) | Effect::SpawnAction { task, .. } => Some(task),
                _ => None,
            })
            .expect("the check dispatches a task")
    }

    fn assert_skipped(fx: &[Effect]) {
        assert!(matches!(
            fx,
            [Effect::Observe { action, outcome: radar_core::journal::Outcome::Blocked }]
                if action.kind.as_str() == "slack.read.apply.skipped.stale"
        ));
    }

    #[tokio::test]
    async fn local_check_failure_supersedes_the_older_result_without_applying_it() {
        fn dispatch(p: &mut SlackPlugin, key: &str) -> Vec<Effect> {
            match key {
                "workspace" => check_token(p),
                "groups" => fetch_groups(p, true),
                _ => check_channel(p, key.into(), Ok("C1".into())),
            }
        }

        for key in ["workspace", "channel:C1", "groups"] {
            let mut p = SlackPlugin::new();
            // One synthetic response covers each method; no environment or
            // credentials are involved, even when the newer client fails.
            p.via = recorded(serde_json::json!({
                "ok": true, "user_id": "U1", "team_id": "T1",
                "channel": {"id": "C1", "name": "older-channel"},
                "usergroups": [{"id": "S1", "handle": "older-group", "users": []}]
            }));
            assert!(p.install_me(identity("T1", "U1", None)));
            p.checks.insert("unrelated".into(), (Check::Running, p.now));
            let older = task(dispatch(&mut p, key));
            p.via = crate::Via(None);
            assert!(p.cfg.token.is_empty());
            let local_failure = dispatch(&mut p, key);
            assert!(
                !local_failure
                    .iter()
                    .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
            );
            assert!(matches!(p.checks.get(key), Some((Check::Failed(_), _))));
            let before = (p.data.clone(), p.cfg.clone(), p.checks.clone());

            let fx = radar_core::Plugin::on_msg(&mut p, older.await);

            if key == "workspace" || key == "groups" {
                assert_eq!(fx.len(), 4);
                assert!(matches!(
                    fx[0],
                    Effect::Report(radar_core::journal::Outcome::Succeeded)
                ));
                assert_skipped(&fx[3..]);
            } else {
                assert!(matches!(
                    fx[0],
                    Effect::Report(radar_core::journal::Outcome::Partial)
                ));
                assert_skipped(&fx[fx.len() - 1..]);
            }
            assert_eq!((p.data, p.cfg, p.checks), before);
        }
    }

    #[tokio::test]
    async fn separate_channel_additions_both_apply_when_the_newer_finishes_first() {
        let mut p = SlackPlugin::new();
        assert!(p.install_me(identity("T1", "U1", None)));
        p.via = recorded(serde_json::json!({
            "ok": true, "channel": {"id": "C00000001", "name": "one"}
        }));
        let older = task(check_channel(&mut p, "add".into(), Ok("C00000001".into())));
        p.via = recorded(serde_json::json!({
            "ok": true, "channel": {"id": "C00000002", "name": "two"}
        }));
        let newer = task(check_channel(&mut p, "add".into(), Ok("C00000002".into())));

        radar_core::Plugin::on_msg(&mut p, newer.await);
        radar_core::Plugin::on_msg(&mut p, older.await);

        let ids: Vec<_> = p.cfg.channels.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["C00000002", "C00000001"]);
        assert!(p.data.channels.contains_key("C00000001"));
        assert!(p.data.channels.contains_key("C00000002"));
    }

    #[tokio::test]
    async fn failed_channel_add_does_not_cancel_another_pending_add() {
        let mut p = SlackPlugin::new();
        assert!(p.install_me(identity("T1", "U1", None)));
        p.via = recorded(serde_json::json!({
            "ok": true, "channel": {"id": "C00000001", "name": "one"}
        }));
        let older = task(check_channel(&mut p, "add".into(), Ok("C00000001".into())));
        p.via = crate::Via(None);
        assert!(p.cfg.token.is_empty());
        check_channel(&mut p, "add".into(), Ok("C00000002".into()));

        radar_core::Plugin::on_msg(&mut p, older.await);

        assert_eq!(p.cfg.channels.len(), 1);
        assert_eq!(p.cfg.channels[0].id, "C00000001");
    }

    #[tokio::test]
    async fn channel_add_from_a_replaced_token_is_not_applied() {
        let mut p = SlackPlugin::new();
        assert!(p.install_me(identity("T1", "U1", None)));
        p.via = recorded(serde_json::json!({
            "ok": true, "channel": {"id": "C1", "name": "one"}
        }));
        let older = task(check_channel(&mut p, "add".into(), Ok("C1".into())));
        p.replace_token("env:OTHER_SLACK_TOKEN".into());
        let before = (p.data.clone(), p.cfg.clone(), p.checks.clone());

        let fx = radar_core::Plugin::on_msg(&mut p, older.await);

        assert!(matches!(
            fx[0],
            Effect::Report(radar_core::journal::Outcome::Partial)
        ));
        assert_skipped(&fx[fx.len() - 1..]);
        assert_eq!((p.data, p.cfg, p.checks), before);
    }

    #[test]
    fn workspace_check_reuses_timezone_only_for_the_same_team_and_user() {
        for offset in [None, Some(1800)] {
            let mut p = SlackPlugin::new();
            p.via = crate::Via(Some(crate::client::Recorded::new(vec![])));
            assert!(p.install_me(identity("T1", "U1", Some(3600))));
            p.checks
                .insert("channel:C1".into(), (Check::Running, p.now));

            let fx = on_checked(
                &mut p,
                "workspace".into(),
                Ok(Checked::Me(identity("T1", "U1", offset))),
            );

            assert_eq!(p.data.me.as_ref().unwrap().tz_offset, offset.or(Some(3600)));
            assert!(p.checks.contains_key("channel:C1"));
            assert!(
                !fx.iter()
                    .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
            );
        }
    }

    #[test]
    fn workspace_check_refreshes_on_either_team_or_user_change_and_keeps_its_verdict() {
        for (team, user) in [("T2", "U1"), ("T1", "U2")] {
            let mut p = SlackPlugin::new();
            p.via = crate::Via(Some(crate::client::Recorded::new(vec![])));
            assert!(p.install_me(identity("T1", "U1", Some(3600))));
            p.checks
                .insert("channel:C1".into(), (Check::Running, p.now));

            let fx = on_checked(
                &mut p,
                "workspace".into(),
                Ok(Checked::Me(identity(team, user, None))),
            );

            assert_eq!(p.data.me.as_ref().unwrap().tz_offset, None);
            assert!(!p.checks.contains_key("channel:C1"));
            assert!(matches!(
                p.checks.get("workspace"),
                Some((Check::Passed(_), _))
            ));
            assert!(fx.iter().any(|e| matches!(e, Effect::SpawnAction { .. })));
        }
    }

    #[test]
    fn the_api_is_a_connection_row_edited_checked_and_kept() {
        let mut p = SlackPlugin::new();
        let rows = ws_rows(&p);
        let at = rows.iter().position(|r| *r == WsRow::Api).unwrap();
        assert!(at < rows.iter().position(|r| *r == WsRow::Token).unwrap());
        assert_eq!(ws_label(WsRow::Api), "api");
        assert_eq!(ws_value(&p, WsRow::Api), "https://slack.com");
        assert_eq!(validate(&p, &Target::Api, "https://slack-gov.com"), Ok("Slack's API at https://slack-gov.com".into()));
        assert_eq!(validate(&p, &Target::Api, "http://127.0.0.1:8123/"), Ok("Slack's API at http://127.0.0.1:8123".into()));
        assert!(validate(&p, &Target::Api, "http://slack.example").is_err());
        assert!(validate(&p, &Target::Api, "slack.com").is_err());

        p.data.me = Some(identity("T1", "U1", None));
        let fx = commit(&mut p, Target::Api, "http://127.0.0.1:8123/".into());
        assert_eq!(p.cfg.api, "http://127.0.0.1:8123");
        assert_eq!(p.data.me, None, "another API is asked who the token is again");
        assert!(matches!(&fx[..], [Effect::Menu(menu)] if menu.id == "ask-check:workspace"));

        // A refused address changes nothing; an empty one is Slack's own.
        commit(&mut p, Target::Api, "http://slack.example".into());
        assert_eq!(p.cfg.api, "http://127.0.0.1:8123");
        commit(&mut p, Target::Api, " ".into());
        assert_eq!(p.cfg.api, "https://slack.com");
    }

    #[test]
    fn a_client_is_built_for_the_configured_api_or_not_at_all() {
        let mut p = SlackPlugin::new();
        p.cfg.token = "xoxp-0123456789012345678901".into();
        p.cfg.api = "http://127.0.0.1:8123".into();
        assert!(p.client().is_ok());
        p.cfg.api = "http://slack.example".into();
        assert!(p.client().err().unwrap().contains("not usable"));
    }

    #[test]
    fn keeping_the_same_token_preserves_identity_and_still_offers_a_check() {
        let mut p = SlackPlugin::new();
        p.cfg.token = "env:SLACK_TOKEN".into();
        let me = identity("T1", "U1", Some(3600));
        p.data.me = Some(me.clone());
        p.checks.insert(
            "workspace".into(),
            (Check::Passed("signed in".into()), p.now),
        );

        let fx = commit(&mut p, Target::Token, "env:SLACK_TOKEN".into());

        assert_eq!(p.data.me, Some(me));
        assert!(matches!(
            p.checks.get("workspace"),
            Some((Check::Passed(_), _))
        ));
        assert!(matches!(&fx[..], [Effect::Menu(menu)] if menu.id == "ask-check:workspace"));
    }

    #[test]
    fn scoped_settings_reads_refuse_unauthenticated_dispatch_without_mutating_data() {
        let mut p = SlackPlugin::new();
        let recorded = crate::client::Recorded::new(vec![]);
        p.via = crate::Via(Some(recorded.clone()));
        // Even a cached-looking identity is not session authority.
        p.data.me = Some(identity("T1", "U1", None));
        let before = (p.data.clone(), p.cfg.clone());
        for fx in [
            check_channel(&mut p, "add".into(), Ok("C00000001".into())),
            fetch_groups(&mut p, true),
        ] {
            assert!(
                !fx.iter()
                    .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
            );
            assert!(fx.iter().any(|e| matches!(e, Effect::Notify(message) if message.contains("check the Slack connection"))));
        }
        assert_eq!((p.data, p.cfg), before);
        assert!(recorded.requests().is_empty());
    }

    #[test]
    fn failed_or_incomplete_workspace_checks_revoke_session_authority() {
        for result in [
            Err("invalid_auth".into()),
            Ok(Checked::Me(identity("T1", "", None))),
        ] {
            let mut p = SlackPlugin::new();
            assert!(p.install_me(identity("T1", "U1", None)));
            let effects = on_checked(&mut p, "workspace".into(), result);
            assert!(!p.authenticated());
            assert!(matches!(
                p.checks.get("workspace"),
                Some((Check::Failed(_), _))
            ));
            assert!(
                !effects
                    .iter()
                    .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
            );
        }
        let mut p = SlackPlugin::new();
        assert!(p.install_me(identity("T1", "U1", None)));
        check_token(&mut p);
        assert!(!p.authenticated());
    }

    fn test_channel(id: &str) -> Channel {
        let mut channel = crate::demo::sample().cfg.channels[0].clone();
        channel.id = id.into();
        channel
    }

    fn check_all_sample() -> SlackPlugin {
        let mut p = SlackPlugin::new();
        p.cfg.channels = vec![test_channel("C00000001")];
        p.via = recorded(serde_json::json!({"ok": true, "team_id": "T1", "user_id": "U1"}));
        p
    }

    fn channel_tasks(effects: Vec<Effect>) -> Vec<radar_core::plugin::Task> {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::SpawnAction { action, task }
                    if action.kind.as_str() == "slack.settings.channel.resolve" =>
                {
                    Some(task)
                }
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn check_all_waits_for_auth_then_checks_only_captured_remaining_channels() {
        for authenticated in [false, true] {
            let mut p = check_all_sample();
            if authenticated {
                assert!(p.install_me(identity("T1", "U1", None)));
            }
            p.cfg.channels.push(test_channel("C00000002"));
            let started = on_bulk(&mut p, "check-all");
            assert_eq!(
                started.len(),
                1,
                "only workspace auth starts before its result"
            );
            let auth = task(started);
            p.cfg.channels.retain(|channel| channel.id != "C00000002");
            p.cfg.channels.push(test_channel("C00000003"));
            let response = auth.await;
            let rec = crate::client::Recorded::new(vec![Ok(crate::client::Reply {
                status: 200,
                body: serde_json::json!({"ok": true, "channel": {"id": "C00000001"}}).to_string(),
                ..Default::default()
            })]);
            p.via = crate::Via(Some(rec.clone()));
            let children = channel_tasks(radar_core::Plugin::on_msg(&mut p, response));
            assert!(p.authenticated());
            assert_eq!(children.len(), 1);
            for child in children {
                radar_core::Plugin::on_msg(&mut p, child.await);
            }
            let requests = rec.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].method, "conversations.info");
            assert!(
                requests[0]
                    .params
                    .contains(&("channel".into(), "C00000001".into()))
            );
        }
    }

    #[tokio::test]
    async fn explicit_check_all_ignores_older_startup_bootstrap() {
        let mut p = check_all_sample();
        let bootstrap = task(p.read());
        let old_result = bootstrap.await;
        p.via = recorded(serde_json::json!({"ok": true, "team_id": "T1", "user_id": "U1"}));
        let check = task(on_bulk(&mut p, "check-all"));
        assert!(
            p.read().is_empty(),
            "background bootstrap cannot supersede workspace check"
        );
        let stale = radar_core::Plugin::on_msg(&mut p, old_result);
        assert!(channel_tasks(stale).is_empty());
        assert!(!p.authenticated());
        let children = channel_tasks(radar_core::Plugin::on_msg(&mut p, check.await));
        assert!(p.authenticated());
        assert_eq!(children.len(), 1);
    }

    #[tokio::test]
    async fn check_all_continuation_is_cancelled_by_failure_rotation_or_new_workspace_check() {
        for cancel in ["failure", "incomplete", "token", "new_check", "new_all"] {
            let mut p = check_all_sample();
            if cancel == "failure" {
                p.via = recorded(serde_json::json!({"ok": false, "error": "invalid_auth"}));
            } else if cancel == "incomplete" {
                p.via = recorded(serde_json::json!({"ok": true, "team_id": "T1"}));
            }
            let old = task(on_bulk(&mut p, "check-all"));
            let old_result = old.await;
            let replacement = match cancel {
                "token" => {
                    p.replace_token("env:OTHER_TOKEN".into());
                    None
                }
                "new_check" | "new_all" => {
                    p.via =
                        recorded(serde_json::json!({"ok": true, "team_id": "T1", "user_id": "U1"}));
                    Some(task(if cancel == "new_all" {
                        on_bulk(&mut p, "check-all")
                    } else {
                        check_token(&mut p)
                    }))
                }
                _ => None,
            };
            assert!(
                channel_tasks(radar_core::Plugin::on_msg(&mut p, old_result)).is_empty(),
                "{cancel}"
            );
            if let Some(replacement) = replacement {
                let children = channel_tasks(radar_core::Plugin::on_msg(&mut p, replacement.await));
                assert_eq!(children.len(), usize::from(cancel == "new_all"), "{cancel}");
            }
        }
    }

    #[test]
    fn home_end_and_pages_in_look_rest_on_a_word_never_a_heading() {
        let mut p = SlackPlugin::new();
        let at = index("look");
        for code in [
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Down,
            KeyCode::Up,
        ] {
            key(&mut p, "look", KeyEvent::new(code, KeyModifiers::NONE));
            assert!(
                matches!(
                    look_rows()[p.section_rows[at]],
                    LookRow::Word(..) | LookRow::Density
                ),
                "{code:?}"
            );
        }
        key(
            &mut p,
            "look",
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
        );
        assert_eq!(
            p.section_rows[at], 1,
            "the density row, under the first heading"
        );
    }
}
