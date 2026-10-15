//! The `[slack]` table: the workspace token, the channels registry (alias,
//! channel, purpose, kind, colour, watched), how review is asked for, how
//! often to read, and the look words.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Channel colour names. Each is the domain slot `slack.<name>`; its value
/// lives in the theme and can be overridden in `[theme.slots]`.
pub const CHANNEL_COLORS: [&str; 7] = [
    "blue", "cyan", "green", "magenta", "purple", "yellow", "orange",
];

/// How a conversation's messages are laid out: one line each, or the
/// roomier layout with a line for the author and a blank line between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    /// Time, author and text on one line; no blank lines between messages.
    #[default]
    Compact,
    /// The author and time on a line, the text under it, a blank line after.
    Cozy,
}

impl Density {
    pub fn word(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Cozy => "cozy",
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::Compact => Self::Cozy,
            Self::Cozy => Self::Compact,
        }
    }

    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// What a channel is for, which decides how its messages are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// The team talking: its threads are discussions.
    #[default]
    Team,
    /// A service's channel: other people about something the team runs.
    Service,
    /// Where things are tried out: review asks can be posted here first.
    Test,
    /// Where a bot raises alerts: its messages are read into alerts by the
    /// channel's reader, and shown under Alerts.
    Alerts,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Team, Kind::Service, Kind::Test, Kind::Alerts];

    pub fn word(self) -> &'static str {
        match self {
            Kind::Team => "team",
            Kind::Service => "service",
            Kind::Test => "test",
            Kind::Alerts => "alerts",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Kind::Team => "the team talking: its threads are discussions",
            Kind::Service => "a service's channel: what others say about it",
            Kind::Test => "for trying things: review asks can go here first",
            Kind::Alerts => {
                "a bot raising alerts: read into alerts by its reader, shown under Alerts"
            }
        }
    }
}

/// How an alerts channel's messages are read into alerts. Each reader is
/// for one shape of message and refuses the rest (see `alerts.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reader {
    /// One attachment per alert, titled `[FIRING]` or `[RESOLVED] <AlertName>`,
    /// with its environment, priority, summary and console link in its text.
    Alertmanager,
    /// A job's failure: a header saying it failed, details in a context.
    JobFailed,
    /// Any bot message with a status word (firing, resolved, failed, ok, …).
    Status,
}

impl Reader {
    pub const ALL: [Reader; 3] = [Reader::Alertmanager, Reader::JobFailed, Reader::Status];

    pub fn word(self) -> &'static str {
        match self {
            Reader::Alertmanager => "alertmanager",
            Reader::JobFailed => "job-failed",
            Reader::Status => "status",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Reader::Alertmanager => {
                "[FIRING] and [RESOLVED] alerts, one attachment each, with fields in the text"
            }
            Reader::JobFailed => "a job that failed: a header saying so, details in a context",
            Reader::Status => "any bot message with a status word: firing, resolved, failed, ok…",
        }
    }
}

/// One line of the mapping from alerts to repos, services and
/// environments: an alert matches by its name's start, a label it carries,
/// or both; the first rule that matches decides.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    /// The start of the alert's name (any case).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prefix: String,
    /// A label the alert carries: `service=billing`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// The repo it belongs to: its path in GitLab, or the last part of it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub repo: String,
    /// The service, by the name you give it; a repo's own name when empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub service: String,
    /// The environment, when the alert does not say.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub environment: String,
}

/// The `[slack.alerts]` table: how long resolved alerts are shown, and the
/// mapping from alerts to repos, services and environments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alerts {
    /// Resolved alerts, and failed jobs, are shown this many hours.
    #[serde(default = "default_recent")]
    pub recent_hours: u64,
    #[serde(default, rename = "map", skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
}

impl Default for Alerts {
    fn default() -> Self {
        Self {
            recent_hours: default_recent(),
            rules: Vec::new(),
        }
    }
}

impl Alerts {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn default_recent() -> u64 {
    24
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// The user token: `env:NAME`, or the token itself.
    #[serde(default)]
    pub token: String,
    /// Where Slack's Web API is: `https://slack.com` (the default),
    /// `https://slack-gov.com` for GovSlack, or a plain `http://127.0.0.1:PORT`
    /// for a local stand-in. Calls go to `{api}/api/{method}`; the token is
    /// sent there and to this host's subdomains, nowhere else.
    #[serde(default = "default_api", skip_serializing_if = "is_default_api")]
    pub api: String,
    #[serde(default = "default_refresh")]
    pub refresh_secs: u64,
    /// Reading pauses after this many minutes without a key or click; 0
    /// never pauses.
    #[serde(default = "default_idle")]
    pub idle_minutes: u64,
    /// How far back a channel is read when it has no caught-up mark.
    #[serde(default = "default_days")]
    pub days: u64,
    #[serde(default, rename = "channel", skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<Channel>,
    #[serde(default)]
    pub review: Review,
    /// Alerts: how long resolved ones stay, and what they belong to.
    #[serde(default, skip_serializing_if = "Alerts::is_default")]
    pub alerts: Alerts,
    /// How messages are laid out in a thread, the detail pane and direct
    /// messages: `compact` (one line each) or `cozy`.
    #[serde(default, skip_serializing_if = "Density::is_default")]
    pub density: Density,
    /// Style spec per look word, e.g. `mention = "status.warning bold"`.
    #[serde(default = "default_styles")]
    pub style: BTreeMap<String, String>,
}

/// A channel radar reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    /// What radar calls it: `team`.
    pub alias: String,
    /// Slack's id: `C0123ABCD`.
    pub id: String,
    /// Its name in Slack, without `#`, as last checked.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// What it is for, in your words.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub purpose: String,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default = "default_color")]
    pub color: String,
    /// Read on every refresh and listed in the nav pane.
    #[serde(default = "yes")]
    pub watched: bool,
    /// How an alerts channel's messages are read; none for other kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reader: Option<Reader>,
}

/// How review is asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Review {
    /// The alias of the channel asks go to.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub channel: String,
    /// The user group to mention there: its id (`S0123`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    /// Its handle, without `@`, as Slack gave it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub handle: String,
    /// The message: `{group}` the mention (only in the review channel),
    /// `{mrs}` the merge request linked with its words (bullets for several).
    #[serde(default = "default_template")]
    pub template: String,
    /// The text of the link to one merge request.
    #[serde(default = "default_link")]
    pub link: String,
    /// The alias of a channel to try asks in first.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub test_channel: String,
    /// When a merge request offers to ask again in the thread, with no
    /// reply yet: `2d` (working days), `4h` (working hours), `10:00` (that
    /// time on the next working day), or `never`. See [`Reask`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reask: Option<String>,
    /// The older setting: whole working days, 0 never. Read when `reask`
    /// is not set; dropped once `reask` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reask_days: Option<u64>,
}

/// When to offer to ask for review again, with no reply yet. Working days
/// are Monday to Friday, in your Slack time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reask {
    Never,
    /// After this many working hours (weekend hours do not count).
    Hours(u32),
    /// After this many working days.
    Days(u32),
    /// At this time of day (minutes after midnight) on the first working
    /// day it comes round after the ask.
    At(u32),
}

impl Reask {
    /// What can be typed: `2d`, `2 days`, `4h`, `4 hours`, `10:00`,
    /// `at 10:00`, `never`, `0`.
    pub fn parse(s: &str) -> Result<Self, String> {
        const WHAT: &str =
            "2d (working days), 4h (working hours), 10:00 (a time on working days) or never";
        let s = s.trim().to_lowercase();
        let s = s.strip_prefix("after").unwrap_or(&s).trim();
        let s = s.strip_prefix("at").unwrap_or(s).trim();
        if s == "never" || s == "0" {
            return Ok(Self::Never);
        }
        if let Some((h, m)) = s.split_once(':') {
            let (Ok(h), Ok(m)) = (h.trim().parse::<u32>(), m.trim().parse::<u32>()) else {
                return Err(WHAT.into());
            };
            if h > 23 || m > 59 {
                return Err("a time of day, 00:00 to 23:59".into());
            }
            return Ok(Self::At(h * 60 + m));
        }
        let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
        let unit = s[digits.len()..].trim();
        let Ok(n) = digits.parse::<u32>() else {
            return Err(WHAT.into());
        };
        if n == 0 {
            return Ok(Self::Never);
        }
        match unit {
            "d" | "day" | "days" | "working day" | "working days" if n <= 20 => Ok(Self::Days(n)),
            "h" | "hour" | "hours" | "working hour" | "working hours" if n <= 160 => {
                Ok(Self::Hours(n))
            }
            "d" | "day" | "days" | "working day" | "working days" => {
                Err("at most 20 working days".into())
            }
            "h" | "hour" | "hours" | "working hour" | "working hours" => {
                Err("at most 160 working hours".into())
            }
            _ => Err(WHAT.into()),
        }
    }

    /// As kept in the config: `2d`, `4h`, `10:00`, `never`.
    pub fn spec(self) -> String {
        match self {
            Self::Never => "never".into(),
            Self::Hours(n) => format!("{n}h"),
            Self::Days(n) => format!("{n}d"),
            Self::At(m) => format!("{:02}:{:02}", m / 60, m % 60),
        }
    }

    /// In words, for the settings row.
    pub fn words(self) -> String {
        let plural = |n: u32, one: &str| {
            if n == 1 {
                format!("1 {one}")
            } else {
                format!("{n} {one}s")
            }
        };
        match self {
            Self::Never => "never".into(),
            Self::Hours(n) => format!("after {} without a reply", plural(n, "working hour")),
            Self::Days(n) => format!("after {} without a reply", plural(n, "working day")),
            Self::At(_) => format!("at {} on the next working day without a reply", self.spec()),
        }
    }
}

impl Review {
    /// When to ask again: `reask`, else the older `reask_days`, else after
    /// 2 working days.
    pub fn reask(&self) -> Reask {
        if let Some(r) = self.reask.as_deref().and_then(|s| Reask::parse(s).ok()) {
            return r;
        }
        match self.reask_days {
            Some(0) => Reask::Never,
            Some(n) => Reask::Days(u32::try_from(n.min(20)).unwrap_or(20)),
            None => Reask::Days(DEFAULT_REASK_DAYS),
        }
    }

    pub fn set_reask(&mut self, r: Reask) {
        self.reask = Some(r.spec());
        self.reask_days = None;
    }
}

impl Default for Review {
    fn default() -> Self {
        Self {
            channel: String::new(),
            group: String::new(),
            handle: String::new(),
            template: default_template(),
            link: default_link(),
            test_channel: String::new(),
            reask: None,
            reask_days: None,
        }
    }
}

fn default_refresh() -> u64 {
    120
}
fn default_idle() -> u64 {
    30
}
fn default_days() -> u64 {
    7
}
fn default_color() -> String {
    "blue".into()
}
fn yes() -> bool {
    true
}
pub fn default_template() -> String {
    "{group} :gitlab: {mrs}".into()
}
fn default_link() -> String {
    "mr".into()
}
const DEFAULT_REASK_DAYS: u32 = 2;

/// Which slot and modifiers each word is drawn in.
pub fn default_styles() -> BTreeMap<String, String> {
    [
        ("author", "accent.secondary"),
        ("you", "accent.primary"),
        ("pinged", "status.warning bold"),
        ("mention", "accent.tertiary"),
        ("link", "status.info underline"),
        ("emoji", "text.muted"),
        ("code", "data.string"),
        ("quote", "text.secondary italic"),
        ("new", "status.success bold"),
        ("time", "text.muted"),
        ("channel", "text.primary bold"),
        ("band", "bg.overlay"),
        ("heading", "bg.overlay"),
        ("lines", "text.muted"),
        ("firing", "status.error bold"),
        ("failed", "status.error bold"),
        ("resolved", "status.success"),
        ("priority", "status.warning"),
        ("unmapped", "text.muted italic"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            token: String::new(),
            api: default_api(),
            refresh_secs: default_refresh(),
            idle_minutes: default_idle(),
            days: default_days(),
            channels: Vec::new(),
            review: Review::default(),
            alerts: Alerts::default(),
            density: Density::default(),
            style: default_styles(),
        }
    }
}

/// Slack's own Web API.
pub const DEFAULT_API: &str = "https://slack.com";

fn default_api() -> String {
    DEFAULT_API.into()
}

fn is_default_api(api: &String) -> bool {
    api == DEFAULT_API
}

/// Why `text` cannot be Slack's API address, or `None` when it can: an
/// `https://` origin (host, maybe a port), or `http://` for this machine only,
/// since the token travels in every call.
pub fn api_problem(text: &str) -> Option<String> {
    let text = text.trim();
    let Ok(u) = reqwest::Url::parse(text) else {
        return Some("an address like https://slack.com".into());
    };
    let Some(host) = u.host_str() else {
        return Some("an address like https://slack.com".into());
    };
    if !u.username().is_empty() || u.password().is_some() {
        return Some("no user or password in the address: the token is the credential".into());
    }
    if u.query().is_some() || u.fragment().is_some() || !matches!(u.path(), "" | "/") {
        return Some("only the server: no path, query or fragment".into());
    }
    match u.scheme() {
        "https" => None,
        "http" if is_loopback(host) => None,
        "http" => Some("https://, or http:// only for this machine (127.0.0.1, localhost)".into()),
        _ => Some("starts with https://".into()),
    }
}

/// This machine, as a host name or address (a bracketed IPv6 host included).
pub(crate) fn is_loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

impl Settings {
    /// The Web API address to call, without a trailing `/`; a refusal when a
    /// hand-edited value could send the token somewhere unsafe.
    pub fn resolved_api(&self) -> Result<String> {
        let api = self.api.trim();
        if api.is_empty() {
            return Ok(DEFAULT_API.into());
        }
        if let Some(why) = api_problem(api) {
            bail!("[slack] api {api:?} is not usable: {why}");
        }
        Ok(api.trim_end_matches('/').to_string())
    }

    /// The style spec for a word; a missing entry falls back to the default.
    pub fn style_for(&self, word: &str) -> String {
        self.style
            .get(word)
            .cloned()
            .or_else(|| default_styles().get(word).cloned())
            .unwrap_or_else(|| "text.primary".into())
    }

    pub fn channel(&self, alias: &str) -> Option<&Channel> {
        self.channels.iter().find(|c| c.alias == alias)
    }

    pub fn by_id(&self, id: &str) -> Option<&Channel> {
        self.channels.iter().find(|c| c.id == id)
    }

    pub fn watched(&self) -> impl Iterator<Item = &Channel> {
        self.channels.iter().filter(|c| c.watched)
    }

    /// The token, resolved (never shown).
    pub fn resolved_token(&self) -> Result<String> {
        if self.token.trim().is_empty() {
            bail!("no token set: add one under connection");
        }
        let t = resolve(&self.token, "token")?;
        radar_core::log::secret(&t);
        Ok(t)
    }

    /// A colour no channel uses yet, else the next in turn.
    pub fn next_color(&self) -> String {
        CHANNEL_COLORS
            .iter()
            .find(|c| !self.channels.iter().any(|ch| ch.color == **c))
            .unwrap_or(&CHANNEL_COLORS[self.channels.len() % CHANNEL_COLORS.len()])
            .to_string()
    }
}

/// Read a setting that may name an environment variable (`env:NAME`).
pub fn resolve(spec: &str, what: &str) -> Result<String> {
    let spec = spec.trim();
    if let Some(var) = spec.strip_prefix("env:") {
        let v = std::env::var(var).with_context(|| format!("{var} is not set"))?;
        if v.trim().is_empty() {
            bail!("{var} is empty");
        }
        return Ok(v.trim().to_string());
    }
    if spec.is_empty() {
        bail!("no {what} set");
    }
    Ok(spec.to_string())
}

/// What kind of token a value is, from its prefix (the value is never
/// shown): `xoxp-` a user token, `xoxb-` a bot's, `xapp-` an app-level one.
pub fn token_kind(value: &str) -> &'static str {
    match value.get(..5) {
        Some("xoxp-") => "a user token",
        Some("xoxb-") => "a bot token",
        Some("xapp-") => "an app-level token",
        Some(p) if p.starts_with("xox") => "a Slack token",
        _ => "not a Slack token",
    }
}

/// A variable in the environment that holds a user token: offered on the
/// first run and under connection, by name only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub var: String,
}

/// User tokens in the environment, by name: names with SLACK and TOKEN in
/// them whose value is a user token, those naming USER first. Values are
/// read only to tell a user token from others; never shown or saved.
pub fn offers(vars: &[(String, String)], have: &str) -> Vec<Offer> {
    let mut found: Vec<&String> = vars
        .iter()
        .filter(|(name, value)| {
            let up = name.to_uppercase();
            up.contains("SLACK")
                && up.contains("TOKEN")
                && value.trim().starts_with("xoxp-")
                && have.trim() != format!("env:{name}")
        })
        .map(|(name, _)| name)
        .collect();
    found.sort_by_key(|n| (!n.to_uppercase().contains("USER"), n.to_string()));
    found.dedup();
    found
        .into_iter()
        .map(|var| Offer { var: var.clone() })
        .collect()
}

/// A channel as typed or pasted: a link to it or to a message in it, an id,
/// or a name. `Ok(id)` for an id or link, `Err(name)` for a name.
pub fn channel_ref(input: &str) -> std::result::Result<String, String> {
    let s = input.trim();
    if let Some(at) = s.find("/archives/") {
        let rest = &s[at + "/archives/".len()..];
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if is_channel_id(&id) {
            return Ok(id);
        }
    }
    if let Some(id) = s
        .strip_prefix("<#")
        .and_then(|r| r.split(['|', '>']).next())
        && is_channel_id(id)
    {
        return Ok(id.to_string());
    }
    if is_channel_id(s) {
        return Ok(s.to_string());
    }
    Err(s.trim_start_matches('#').to_lowercase())
}

/// `C0123ABCD`, `G0123ABCD` (an old private channel), `D…` a DM.
pub fn is_channel_id(s: &str) -> bool {
    s.len() >= 9
        && s.len() <= 12
        && matches!(s.chars().next(), Some('C' | 'G' | 'D'))
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// Whether `s` reads as a channel name Slack allows.
pub fn is_channel_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars().count() <= 80
        && s.chars()
            .all(|c| c.is_lowercase() || c.is_ascii_digit() || "-_.".contains(c))
}

/// The longest alias: as long as a channel's name may be in Slack.
pub const ALIAS_MAX: usize = 80;

/// What an alias may be, in words.
pub const ALIAS_ACCEPTS: &str = "1 to 80 letters, digits, -, _ or .";

fn alias_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '.')
}

/// Whether `s` is a usable alias: letters, digits, `-`, `_` and `.`, as
/// long as a channel's name may be, so a channel's own name always is one.
pub fn alias_problem(s: &str) -> Option<&'static str> {
    if s.is_empty() || s.chars().count() > ALIAS_MAX {
        Some(ALIAS_ACCEPTS)
    } else if !s.chars().all(alias_char) {
        Some("letters, digits, -, _ or . only")
    } else {
        None
    }
}

/// An alias for a channel: its name in Slack, as it is; `-2`, `-3`… after
/// it only when another channel has that alias already.
pub fn alias_for(name: &str, taken: &[&str]) -> String {
    let base: String = name
        .chars()
        .filter(|c| alias_char(*c))
        .take(ALIAS_MAX)
        .collect();
    let base = if base.is_empty() {
        "channel".to_string()
    } else {
        base
    };
    if !taken.contains(&base.as_str()) {
        return base;
    }
    (2..)
        .map(|n| {
            let tail = format!("-{n}");
            let keep: String = base.chars().take(ALIAS_MAX - tail.len()).collect();
            format!("{keep}{tail}")
        })
        .find(|a| !taken.contains(&a.as_str()))
        .unwrap_or(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn a_config_without_a_slack_table_is_the_default_and_round_trips() {
        let s = Settings::default();
        let text = toml::to_string(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back, s);
        let empty: Settings = toml::from_str("").unwrap();
        assert_eq!(empty, s);
    }

    #[test]
    fn the_api_defaults_to_slack_and_round_trips_when_set() {
        let s = Settings::default();
        assert_eq!(s.api, "https://slack.com");
        assert_eq!(s.resolved_api().unwrap(), "https://slack.com");
        // An existing config, and a default one written back, carry no `api`.
        let old: Settings = toml::from_str("token = \"env:T\"\nrefresh_secs = 90\n").unwrap();
        assert_eq!(old.api, "https://slack.com");
        assert!(!toml::to_string(&old).unwrap().contains("api"));
        for api in ["https://slack-gov.com", "http://127.0.0.1:8123"] {
            let s: Settings = toml::from_str(&format!("api = \"{api}/\"\n")).unwrap();
            assert_eq!(s.resolved_api().unwrap(), api);
            let text = toml::to_string(&s).unwrap();
            assert!(text.contains("api = "), "{text}");
            assert_eq!(toml::from_str::<Settings>(&text).unwrap(), s);
        }
        let blank: Settings = toml::from_str("api = \"  \"\n").unwrap();
        assert_eq!(blank.resolved_api().unwrap(), "https://slack.com");
    }

    #[test]
    fn only_https_or_this_machine_is_an_api() {
        for ok in ["https://slack.com", "https://slack-gov.com/", "http://127.0.0.1:8123", "http://localhost:1", "http://[::1]:9"] {
            assert_eq!(api_problem(ok), None, "{ok}");
        }
        for bad in ["slack.com", "ftp://slack.com", "http://slack.com", "http://10.0.0.5", "https://u:p@slack.com",
            "https://slack.com/api", "https://slack.com/?x=1", ""] {
            assert!(api_problem(bad).is_some(), "{bad}");
        }
        let s: Settings = toml::from_str("api = \"http://slack.example\"\n").unwrap();
        assert!(s.resolved_api().is_err());
    }

    #[test]
    fn channels_and_review_round_trip() {
        let mut s = Settings {
            token: "env:SLACK_USER_TOKEN".into(),
            ..Settings::default()
        };
        s.channels.push(Channel {
            alias: "team".into(),
            id: "C0123ABCD".into(),
            name: "team".into(),
            purpose: "where we talk".into(),
            kind: Kind::Team,
            color: "cyan".into(),
            watched: true,
            reader: None,
        });
        s.channels.push(Channel {
            alias: "team-alerts".into(),
            id: "C0000ALRT1".into(),
            name: "team-alerts".into(),
            purpose: String::new(),
            kind: Kind::Alerts,
            color: "orange".into(),
            watched: true,
            reader: Some(Reader::Alertmanager),
        });
        s.alerts.recent_hours = 48;
        s.alerts.rules.push(Rule {
            prefix: "api-".into(),
            repo: "acme/team-a/api".into(),
            service: "api".into(),
            ..Rule::default()
        });
        s.review.channel = "team".into();
        s.review.group = "S0123ABCD".into();
        s.review.handle = "review-squad".into();
        let text = toml::to_string(&s).unwrap();
        assert!(text.contains("[[channel]]"), "{text}");
        assert!(text.contains("kind = \"team\""), "{text}");
        assert!(
            text.contains("reader = \"alertmanager\"") && text.contains("[[alerts.map]]"),
            "{text}"
        );
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn when_to_ask_again_reads_days_hours_a_time_or_never() {
        assert_eq!(Reask::parse("2d"), Ok(Reask::Days(2)));
        assert_eq!(Reask::parse("after 3 working days"), Ok(Reask::Days(3)));
        assert_eq!(Reask::parse("4h"), Ok(Reask::Hours(4)));
        assert_eq!(Reask::parse("6 hours"), Ok(Reask::Hours(6)));
        assert_eq!(Reask::parse("10:00"), Ok(Reask::At(600)));
        assert_eq!(Reask::parse("at 9:30"), Ok(Reask::At(570)));
        assert_eq!(Reask::parse("never"), Ok(Reask::Never));
        assert_eq!(Reask::parse("0"), Ok(Reask::Never));
        assert!(Reask::parse("25:00").is_err());
        assert!(Reask::parse("3 weeks").is_err());
        assert!(Reask::parse("21d").is_err());
        for r in [
            Reask::Days(2),
            Reask::Hours(4),
            Reask::At(570),
            Reask::Never,
        ] {
            assert_eq!(Reask::parse(&r.spec()), Ok(r));
        }
        assert_eq!(
            Reask::At(570).words(),
            "at 09:30 on the next working day without a reply"
        );
        assert_eq!(
            Reask::Hours(1).words(),
            "after 1 working hour without a reply"
        );
    }

    #[test]
    fn the_older_reask_days_is_still_read_until_reask_is_set() {
        let s: Settings = toml::from_str("[review]\nreask_days = 1\n").unwrap();
        assert_eq!(s.review.reask(), Reask::Days(1));
        let mut r = s.review.clone();
        r.set_reask(Reask::At(600));
        let text = toml::to_string(&r).unwrap();
        assert!(
            text.contains("reask = \"10:00\"") && !text.contains("reask_days"),
            "{text}"
        );
        assert_eq!(Review::default().reask(), Reask::Days(2));
    }

    #[test]
    fn user_tokens_are_offered_by_name_those_naming_user_first() {
        let v = vars(&[
            ("SLACK_BOT_TOKEN", "xoxb-1"),
            ("SLACK_TOKEN", "xoxp-2"),
            ("SLACK_APP_USER_OAUTH_TOKEN", "xoxp-3"),
            ("SLACK_USER_TOKEN", ""),
            ("OTHER_TOKEN", "xoxp-4"),
            ("SLACK_SIGNING_SECRET", "xoxp-5"),
        ]);
        let names: Vec<String> = offers(&v, "").into_iter().map(|o| o.var).collect();
        assert_eq!(names, ["SLACK_APP_USER_OAUTH_TOKEN", "SLACK_TOKEN"]);
        let names: Vec<String> = offers(&v, "env:SLACK_TOKEN")
            .into_iter()
            .map(|o| o.var)
            .collect();
        assert_eq!(
            names,
            ["SLACK_APP_USER_OAUTH_TOKEN"],
            "the one in use is not offered"
        );
    }

    #[test]
    fn a_token_kind_is_told_by_its_prefix_only() {
        assert_eq!(token_kind("xoxp-123"), "a user token");
        assert_eq!(token_kind("xoxb-123"), "a bot token");
        assert_eq!(token_kind("xapp-1"), "an app-level token");
        assert_eq!(token_kind("hello"), "not a Slack token");
        assert_eq!(token_kind(""), "not a Slack token");
    }

    #[test]
    fn channels_are_read_from_links_ids_and_names() {
        assert_eq!(
            channel_ref("https://acme.slack.com/archives/C0123ABCD"),
            Ok("C0123ABCD".into())
        );
        assert_eq!(
            channel_ref("https://acme.slack.com/archives/C0123ABCD/p1700000000123456?thread_ts=1"),
            Ok("C0123ABCD".into())
        );
        assert_eq!(channel_ref("<#C0123ABCD|team>"), Ok("C0123ABCD".into()));
        assert_eq!(channel_ref(" C0123ABCD "), Ok("C0123ABCD".into()));
        assert_eq!(channel_ref("#Team-Chat"), Err("team-chat".into()));
        assert!(is_channel_name("team-chat"));
        assert!(!is_channel_name("team chat"));
    }

    /// A channel added by its link or name is called what Slack calls it,
    /// unchanged: never its last word (`#bot` for `acme-team-bot`).
    #[test]
    fn an_alias_is_the_channels_own_name_unchanged_and_never_clashes() {
        assert_eq!(alias_for("acme-team-alerts", &[]), "acme-team-alerts");
        assert_eq!(
            alias_for("topic-acme-billing-reports-bot", &[]),
            "topic-acme-billing-reports-bot"
        );
        assert_eq!(alias_for("team", &["team"]), "team-2");
        assert_eq!(alias_for("team", &["team", "team-2"]), "team-3");
        let long = "a".repeat(80);
        let again = alias_for(&long, &[long.as_str()]);
        assert_eq!((again.len(), again.ends_with("-2")), (80, true));
    }

    #[test]
    fn any_channel_name_is_a_usable_alias() {
        assert_eq!(alias_problem("team"), None);
        assert_eq!(alias_problem("int-team-acme-billing-reports"), None);
        assert_eq!(alias_problem("team.v2_chat"), None);
        assert_eq!(alias_problem(&"a".repeat(80)), None);
        assert!(alias_problem(&"a".repeat(81)).is_some());
        assert!(alias_problem("a b").is_some());
        assert!(alias_problem("").is_some());
    }

    #[test]
    fn an_env_token_is_read_and_an_empty_one_refused() {
        // SAFETY: a variable only this test uses.
        unsafe { std::env::set_var("RADAR_SLACK_TEST_EMPTY", "") };
        let s = Settings {
            token: "env:RADAR_SLACK_TEST_EMPTY".into(),
            ..Settings::default()
        };
        assert!(
            s.resolved_token()
                .unwrap_err()
                .to_string()
                .contains("empty")
        );
        let s = Settings::default();
        assert!(
            s.resolved_token()
                .unwrap_err()
                .to_string()
                .contains("no token")
        );
    }
}
