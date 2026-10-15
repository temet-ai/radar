//! The `[jira]` table: sites (each with its address, account, token, colour
//! and watched projects), views, refreshing and the look words.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Site colour names. Each is the domain slot `jira.<name>`; its value lives
/// in the theme and can be overridden in `[theme.slots]`.
pub const SITE_COLORS: [&str; 7] = [
    "blue", "cyan", "green", "magenta", "purple", "yellow", "orange",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_refresh")]
    pub refresh_secs: u64,
    /// Refreshing pauses after this many minutes without a key or click;
    /// 0 never pauses.
    #[serde(default = "default_idle")]
    pub idle_minutes: u64,
    /// What Jira does when work on a ticket starts elsewhere
    /// (`workspace.started`): ask to assign it to you and move it, or not.
    #[serde(default, skip_serializing_if = "OnStart::is_ask")]
    pub on_start: OnStart,
    /// The status "start" moves a ticket to, by name; empty takes the first
    /// move into an in-progress status Jira offers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub start_status: String,
    /// Style spec per look word, e.g. `doing = "status.info bold"`.
    #[serde(default = "default_styles")]
    pub style: BTreeMap<String, String>,
    #[serde(default, rename = "site", skip_serializing_if = "Vec::is_empty")]
    pub sites: Vec<Site>,
    #[serde(default = "default_views", rename = "view")]
    pub views: Vec<View>,
}

/// `[jira] on_start`: whether starting work on a ticket asks Jira to assign
/// and move it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnStart {
    #[default]
    Ask,
    Off,
}

impl OnStart {
    pub fn is_ask(&self) -> bool {
        *self == Self::Ask
    }
}

/// One Jira Cloud site, like a GitLab tenancy: its own colour, account and
/// projects; its tickets group under it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Site {
    /// Short name shown in the tree and the breadcrumb: `acme`.
    pub name: String,
    /// `https://acme.atlassian.net`, or `env:NAME`.
    pub url: String,
    /// The account's email, or `env:NAME`.
    #[serde(default)]
    pub email: String,
    /// An API token: `env:NAME`, or the token itself.
    #[serde(default)]
    pub token: String,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default, rename = "project", skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<Project>,
    /// Where the site was imported from, e.g. `~/work/x/.mcp.json`; empty
    /// when it was typed in or taken from the environment.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from: String,
}

/// A watched project, and which of its fields holds the acceptance criteria.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub key: String,
    /// A field id such as `customfield_10050`; empty when none is set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ac_field: String,
}

/// A view: conditions in words (see `view.rs`), or JQL typed by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<String>,
    /// JQL typed by hand; when set it is used instead of `when`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub jql: String,
    /// Ask only this site (its name); empty asks every site.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub site: String,
}

fn default_refresh() -> u64 {
    120
}
fn default_idle() -> u64 {
    30
}
fn default_color() -> String {
    "blue".into()
}

/// Which slot and modifiers each word is drawn in: status categories,
/// ticket types, ages, the site name (modifiers on the site's colour) and
/// the tree's `band`, `heading` and `lines`.
pub fn default_styles() -> BTreeMap<String, String> {
    [
        ("to do", "text.secondary"),
        ("doing", "status.info bold"),
        ("done", "status.success"),
        ("bug", "status.error"),
        ("story", "status.success"),
        ("task", "accent.secondary"),
        ("epic", "accent.tertiary bold"),
        ("subtask", "text.secondary"),
        ("other", "text.secondary"),
        ("fresh", "text.primary"),
        ("aging", "status.warning"),
        ("stale", "text.muted"),
        ("key", "accent.secondary"),
        ("label", "accent.secondary"),
        ("you", "accent.primary"),
        ("failed", "status.error"),
        ("site", "bold"),
        ("band", "bg.overlay"),
        ("heading", "bg.overlay"),
        ("lines", "text.muted"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// The views a new config starts with. The first is what the screen opens on.
pub fn default_views() -> Vec<View> {
    [
        ("My tickets", &["assignee:me", "status:open"][..]),
        ("This sprint", &["sprint:open", "project:watched"][..]),
        ("Recently updated", &["project:watched", "updated:14d"][..]),
    ]
    .into_iter()
    .map(|(name, when)| View {
        name: name.into(),
        when: when.iter().map(|s| s.to_string()).collect(),
        jql: String::new(),
        site: String::new(),
    })
    .collect()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            refresh_secs: default_refresh(),
            idle_minutes: default_idle(),
            on_start: OnStart::default(),
            start_status: String::new(),
            style: default_styles(),
            sites: Vec::new(),
            views: default_views(),
        }
    }
}

impl Settings {
    /// The style spec for a word; a missing entry falls back to the default.
    pub fn style_for(&self, word: &str) -> String {
        self.style
            .get(word)
            .cloned()
            .or_else(|| default_styles().get(word).cloned())
            .unwrap_or_else(|| "text.primary".into())
    }

    pub fn site(&self, name: &str) -> Option<&Site> {
        self.sites.iter().find(|s| s.name == name)
    }
}

impl Site {
    /// The acceptance-criteria field set for a project, if any.
    pub fn ac_field(&self, project: &str) -> Option<&str> {
        self.projects
            .iter()
            .find(|p| p.key == project)
            .map(|p| p.ac_field.as_str())
            .filter(|f| !f.is_empty())
    }

    pub fn watches(&self, project: &str) -> bool {
        self.projects.iter().any(|p| p.key == project)
    }

    /// The site's address, resolved: `https://acme.atlassian.net`.
    pub fn base_url(&self) -> Result<String> {
        Ok(site_url(&resolve(&self.url, "address")?))
    }

    /// A ticket's page on the site, if the address resolves.
    pub fn browse(&self, key: &str) -> Option<String> {
        self.base_url().ok().map(|b| format!("{b}/browse/{key}"))
    }

    /// The address without `https://`, for showing: `acme.atlassian.net`;
    /// the variable it is read from while that is not set.
    pub fn host(&self) -> String {
        match self.base_url() {
            Ok(url) => host_of(&url),
            Err(_) => self.url.trim().to_string(),
        }
    }
}

/// Read a setting that may name an environment variable (`env:NAME`).
pub fn resolve(spec: &str, what: &str) -> Result<String> {
    let spec = spec.trim();
    if let Some(var) = spec.strip_prefix("env:") {
        return std::env::var(var).with_context(|| format!("{var} is not set"));
    }
    if spec.is_empty() {
        bail!("no {what} set");
    }
    Ok(spec.to_string())
}

/// What someone pasted, as a site's address: scheme and host only.
/// `acme.atlassian.net/jira/software/projects/ABC` → `https://acme.atlassian.net`.
pub fn site_url(input: &str) -> String {
    let s = input.trim();
    let (scheme, rest) = match s.split_once("://") {
        Some((scheme, rest)) => (scheme, rest),
        None => ("https", s),
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    format!("{scheme}://{host}")
}

pub fn host_of(url: &str) -> String {
    site_url(url)
        .split_once("://")
        .map_or_else(String::new, |(_, h)| h.to_string())
}

/// Why an address cannot be a Jira site, if it cannot.
pub fn url_problem(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return Some("paste the site's link, e.g. https://acme.atlassian.net".into());
    }
    if let Some(var) = s.strip_prefix("env:") {
        return match std::env::var(var) {
            Ok(v) if !v.trim().is_empty() => None,
            _ => Some(format!("{var} is not set in this shell")),
        };
    }
    let host = host_of(s);
    if host.is_empty() || !host.contains('.') || host.contains(char::is_whitespace) {
        return Some("not a web address: a host like acme.atlassian.net".into());
    }
    None
}

/// A project key: a capital letter, then capitals, digits or `_`.
pub fn is_project_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && key.len() <= 20
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// A ticket key: a project key, `-`, a number.
pub fn is_ticket_key(key: &str) -> bool {
    key.rsplit_once('-').is_some_and(|(project, n)| {
        is_project_key(project)
            && !n.is_empty()
            && !n.starts_with('0')
            && n.chars().all(|c| c.is_ascii_digit())
    })
}

/// A project key from what someone pasted: the key itself (any case), a
/// ticket key or link, a board or project link.
pub fn project_key_of(input: &str) -> String {
    let s = input.trim();
    let upper = |t: &str| t.trim().to_ascii_uppercase();
    if let Some((_, rest)) = s.split_once("/projects/") {
        return upper(rest.split(['/', '?', '#']).next().unwrap_or(rest));
    }
    if let Some((_, rest)) = s.split_once("/browse/") {
        let key = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        return upper(key.rsplit_once('-').map_or(key, |(p, _)| p));
    }
    let up = upper(s);
    match up.rsplit_once('-') {
        Some((p, n)) if n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty() => p.to_string(),
        _ => up,
    }
}

/// The variable names the team's tools read a Jira site from, and so the
/// names a new site starts with.
pub const DEFAULT_URL: &str = "JIRA_BASE_URL";
pub const DEFAULT_EMAIL: &str = "JIRA_EMAIL";
pub const DEFAULT_TOKEN: &str = "JIRA_TOKEN";
pub const DEFAULT_PROJECT: &str = "JIRA_PROJECT_KEY";
pub const DEFAULT_AC_FIELD: &str = "JIRA_ACCEPTANCE_CRITERIA_FIELD";

/// A project to watch from `<prefix>_PROJECT_KEY` and
/// `<prefix>_ACCEPTANCE_CRITERIA_FIELD`, when the key is set and sound.
/// They are not secrets, so their values are used.
fn project_from(vars: &[(String, String)], prefix: &str) -> Option<Project> {
    let value = |name: String| {
        vars.iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let key = value(format!("{prefix}_PROJECT_KEY")).filter(|k| is_project_key(k))?;
    Some(Project {
        key,
        ac_field: value(format!("{prefix}_ACCEPTANCE_CRITERIA_FIELD")).unwrap_or_default(),
    })
}

/// The project a new site starts with, from `JIRA_PROJECT_KEY` and
/// `JIRA_ACCEPTANCE_CRITERIA_FIELD`.
pub fn default_projects(vars: &[(String, String)]) -> Vec<Project> {
    project_from(vars, "JIRA").into_iter().collect()
}

/// A site's details found in the environment: the variable names only, so
/// radar can offer them without showing what they hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// What the variables start with: `JIRA`, `JIRA_ACME`.
    pub prefix: String,
    pub url: String,
    pub email: String,
    pub token: String,
    /// The site's host, from the address variable: to tell sets apart.
    pub host: String,
    /// The project and field the set names, if it names one.
    pub project: Option<Project>,
}

impl Found {
    /// A site name from the prefix: `JIRA` → `jira`, `JIRA_ACME` → `acme`.
    pub fn name(&self) -> String {
        let p = self.prefix.to_ascii_lowercase();
        let short = p
            .strip_prefix("jira_")
            .or_else(|| p.strip_suffix("_jira"))
            .unwrap_or(&p);
        short.to_string()
    }

    pub fn site(&self, color: &str) -> Site {
        Site {
            name: self.name(),
            url: format!("env:{}", self.url),
            email: format!("env:{}", self.email),
            token: format!("env:{}", self.token),
            color: color.to_string(),
            projects: self.project.iter().cloned().collect(),
            from: String::new(),
        }
    }

    /// `JIRA_BASE_URL · JIRA_EMAIL · JIRA_TOKEN`
    pub fn names(&self) -> String {
        format!("{} · {} · {}", self.url, self.email, self.token)
    }
}

/// A site radar can add for you: found in the environment, or in a
/// workspace's `.mcp.json`. Holds variable names and plain settings only.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    /// The site as it would be added (its colour is picked when it is).
    pub site: Site,
    /// What it is: `JIRA_BASE_URL · JIRA_EMAIL · JIRA_TOKEN`, or the
    /// `.mcp.json` server entry's name.
    pub label: String,
    /// Where it was found: `in the environment`, or `in ~/work/x/.mcp.json`.
    pub source: String,
    /// The host, when the address is written out; empty when it is a variable.
    pub host: String,
    /// The `.mcp.json` held a token typed in, which radar leaves there.
    pub token_typed: bool,
}

impl Offer {
    fn from_env(f: &Found) -> Self {
        Self {
            site: f.site(""),
            label: f.names(),
            source: "in the environment".into(),
            host: f.host.clone(),
            token_typed: false,
        }
    }
}

/// `/home/alice/work` → `~/work`, for showing a path.
pub fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

/// A `.mcp.json` env value as a radar setting: `${VAR}` or `$VAR` names a
/// variable (`env:VAR`); anything else is the value itself.
fn env_ref(value: &str) -> Result<String, String> {
    let v = value.trim();
    let name = v
        .strip_prefix("${")
        .and_then(|r| r.strip_suffix('}'))
        .or_else(|| v.strip_prefix('$'));
    match name {
        Some(n)
            if !n.is_empty()
                && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
        {
            Ok(format!("env:{n}"))
        }
        _ => Err(v.to_string()),
    }
}

/// The Jira sites a `.mcp.json` describes: every server entry whose `env`
/// carries `JIRA_BASE_URL`, read the way the team's tools read it. The
/// address, project and field are taken as written; `${VAR}` becomes
/// `env:VAR`; a token written out is never copied (`token_typed`). `path`
/// is where the file is, for saying so. Malformed JSON is an error.
pub fn mcp_offers(text: &str, path: &str) -> Result<Vec<Offer>, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let Some(servers) = v.get("mcpServers").and_then(|s| s.as_object()) else {
        return Ok(vec![]);
    };
    let shown = tilde(path);
    let mut out = Vec::new();
    for (server, entry) in servers {
        let Some(env) = entry.get("env").and_then(|e| e.as_object()) else {
            continue;
        };
        let get = |k: &str| env.get(k).and_then(|v| v.as_str()).map(str::trim);
        let Some(url) = get(DEFAULT_URL).filter(|u| !u.is_empty()) else {
            continue;
        };
        let (url, host) = match env_ref(url) {
            Ok(var) => (var, String::new()),
            Err(literal) => (site_url(&literal), host_of(&literal)),
        };
        // The email is not a secret: a written one is kept as written.
        let email = match get(DEFAULT_EMAIL) {
            Some(e) => env_ref(e).unwrap_or_else(|literal| literal),
            None => format!("env:{DEFAULT_EMAIL}"),
        };
        let (token, token_typed) = match get(DEFAULT_TOKEN) {
            Some(t) => match env_ref(t) {
                Ok(var) => (var, false),
                Err(_) => (String::new(), true),
            },
            None => (format!("env:{DEFAULT_TOKEN}"), false),
        };
        let plain = |k: &str| get(k).filter(|v| !v.is_empty() && env_ref(v).is_err());
        let projects = plain(DEFAULT_PROJECT)
            .filter(|k| is_project_key(k))
            .map(|key| Project {
                key: key.to_string(),
                ac_field: plain(DEFAULT_AC_FIELD).unwrap_or_default().to_string(),
            })
            .into_iter()
            .collect();
        let name = host
            .split('.')
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or(server)
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(20)
            .collect();
        out.push(Offer {
            site: Site {
                name,
                url,
                email,
                token,
                color: String::new(),
                projects,
                from: shown.clone(),
            },
            label: server.clone(),
            source: format!("in {shown}"),
            host,
            token_typed,
        });
    }
    Ok(out)
}

/// The `.mcp.json` files in `start` and each folder above it, nearest first.
pub fn mcp_files(start: &std::path::Path) -> Vec<std::path::PathBuf> {
    start
        .ancestors()
        .map(|d| d.join(".mcp.json"))
        .filter(|f| f.is_file())
        .collect()
}

/// Every site the `.mcp.json` files from `start` upwards describe. A file
/// that cannot be read is skipped, and logged by its path only.
pub fn mcp_found(start: &std::path::Path) -> Vec<Offer> {
    let mut out = Vec::new();
    for file in mcp_files(start) {
        let path = file.display().to_string();
        match std::fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|text| mcp_offers(&text, &path))
        {
            Ok(found) => {
                if !found.is_empty() {
                    radar_core::log::info(format!("jira {} site(s) offered from {path}", found.len()));
                }
                out.extend(found);
            }
            Err(e) => radar_core::log::warn(format!("jira skipped {path}: {e}")),
        }
    }
    out
}

/// Every set of Jira variables in `vars` (name, value): a prefix with an
/// address, an account and a token. Two sets for the same address and
/// account are one site; the first (shortest prefix) is kept. Values are
/// compared, never returned.
pub fn found_in(vars: &[(String, String)]) -> Vec<Found> {
    let get = |name: &str| {
        vars.iter()
            .find(|(n, v)| n == name && !v.trim().is_empty())
            .map(|(n, _)| n.clone())
    };
    let value = |name: &str| {
        vars.iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default()
    };
    let mut prefixes: Vec<String> = vars
        .iter()
        .filter_map(|(n, _)| {
            ["_BASE_URL", "_URL"]
                .iter()
                .find_map(|s| n.strip_suffix(s))
                .map(str::to_string)
        })
        .filter(|p| p.contains("JIRA") || p.contains("ATLASSIAN"))
        .collect();
    prefixes.sort_by_key(|p| (p.len(), p.clone()));
    prefixes.dedup();
    let mut out: Vec<(Found, (String, String))> = Vec::new();
    for prefix in prefixes {
        let pick = |suffixes: &[&str]| {
            suffixes
                .iter()
                .find_map(|s| get(&format!("{prefix}_{s}")))
        };
        // The team's own names first: JIRA_BASE_URL, JIRA_TOKEN.
        let (Some(url), Some(email), Some(token)) = (
            pick(&["BASE_URL", "URL"]),
            pick(&["EMAIL", "USERNAME", "USER"]),
            pick(&["TOKEN", "API_TOKEN"]),
        ) else {
            continue;
        };
        let host = host_of(&value(&url));
        let same = (host.clone(), value(&email).to_ascii_lowercase());
        if out.iter().any(|(_, s)| *s == same) {
            continue;
        }
        let project = project_from(vars, &prefix);
        out.push((
            Found {
                prefix,
                url,
                email,
                token,
                host,
                project,
            },
            same,
        ));
    }
    out.into_iter().map(|(f, _)| f).collect()
}

/// What radar can offer to add: the `.mcp.json` sites, then the sets in
/// `vars` (the environment) for a host no `.mcp.json` site has; none for a
/// host or variable a site already uses.
pub fn offers(sites: &[Site], vars: &[(String, String)], mcp: &[Offer]) -> Vec<Offer> {
    let value = |spec: &str| -> String {
        match spec.trim().strip_prefix("env:") {
            Some(var) => vars
                .iter()
                .find(|(n, _)| n == var)
                .map(|(_, v)| v.clone())
                .unwrap_or_default(),
            None => spec.to_string(),
        }
    };
    let host = |o: &Offer| {
        if o.host.is_empty() {
            host_of(&value(&o.site.url))
        } else {
            o.host.clone()
        }
    };
    let used: Vec<String> = sites.iter().map(|s| host_of(&value(&s.url))).collect();
    let mut out: Vec<Offer> = Vec::new();
    let env = found_in(vars);
    let candidates = mcp.iter().cloned().chain(env.iter().map(Offer::from_env));
    for o in candidates {
        let h = host(&o);
        let taken = sites
            .iter()
            .any(|s| s.url.trim() == o.site.url.trim() || s.name == o.site.name)
            || (!h.is_empty() && used.contains(&h))
            || out.iter().any(|x| !h.is_empty() && host(x) == h);
        if !taken {
            out.push(o);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn the_table_round_trips_through_toml() {
        let mut cfg = Settings::default();
        cfg.sites.push(Site {
            name: "acme".into(),
            url: "https://acme.atlassian.net".into(),
            email: "env:JIRA_EMAIL".into(),
            token: "env:JIRA_TOKEN".into(),
            color: "cyan".into(),
            projects: vec![Project {
                key: "ABC".into(),
                ac_field: "customfield_10050".into(),
            }],
            from: "~/work/acme/.mcp.json".into(),
        });
        cfg.views[0].jql = "project = ABC".into();
        cfg.style.insert("doing".into(), "accent.primary".into());
        let value = toml::Value::try_from(&cfg).unwrap();
        let back: Settings = value.try_into().unwrap();
        assert_eq!(back, cfg);
        let text = toml::to_string(&cfg).unwrap();
        assert!(text.contains("[[site.project]]"), "{text}");
    }

    #[test]
    fn on_start_asks_by_default_is_kept_when_off_and_not_written_when_ask() {
        let cfg: Settings = toml::from_str("").unwrap();
        assert_eq!((cfg.on_start, cfg.start_status.as_str()), (OnStart::Ask, ""));
        let text = toml::to_string(&cfg).unwrap();
        assert!(!text.contains("on_start") && !text.contains("start_status"), "{text}");
        let cfg: Settings = toml::from_str("on_start = \"off\"\nstart_status = \"In Progress\"\n").unwrap();
        assert_eq!(cfg.on_start, OnStart::Off);
        let back: Settings = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn an_empty_table_gets_defaults_and_missing_styles_fall_back() {
        let cfg: Settings = toml::from_str("[style]\ndoing = \"accent.primary\"\n").unwrap();
        assert_eq!(cfg.refresh_secs, 120);
        assert_eq!(cfg.views, default_views());
        assert_eq!(cfg.style_for("doing"), "accent.primary");
        assert_eq!(cfg.style_for("bug"), "status.error");
    }

    #[test]
    fn pasted_addresses_become_a_site() {
        for input in [
            "https://acme.atlassian.net",
            "https://acme.atlassian.net/",
            "acme.atlassian.net",
            "https://acme.atlassian.net/jira/software/projects/ABC/boards/3",
            "https://acme.atlassian.net/browse/ABC-12?focused=1",
        ] {
            assert_eq!(site_url(input), "https://acme.atlassian.net", "{input}");
        }
        assert!(url_problem("acme").is_some());
        assert!(url_problem("").is_some());
        assert!(url_problem("https://acme.atlassian.net").is_none());
    }

    #[test]
    fn keys_are_checked_before_they_go_into_a_path() {
        for good in ["ABC-12", "A1_B-3"] {
            assert!(is_ticket_key(good), "{good}");
        }
        for bad in ["abc-12", "ABC-", "ABC-012", "ABC 12", "../x-1", "ABC-1/comment", "-1"] {
            assert!(!is_ticket_key(bad), "{bad}");
        }
        assert!(is_project_key("ABC"));
        assert!(!is_project_key("AB C") && !is_project_key("1AB") && !is_project_key(""));
    }

    #[test]
    fn a_project_key_from_whatever_is_pasted() {
        for input in [
            "ABC",
            "abc",
            "ABC-12",
            "https://acme.atlassian.net/browse/ABC-12",
            "https://acme.atlassian.net/jira/software/projects/ABC/boards/3",
            " https://acme.atlassian.net/jira/software/c/projects/ABC/issues ",
        ] {
            assert_eq!(project_key_of(input), "ABC", "{input}");
        }
    }

    #[test]
    fn sets_of_variables_are_found_by_name_and_deduplicated_by_value() {
        let found = found_in(&vars(&[
            ("JIRA_URL", "https://acme.atlassian.net"),
            ("JIRA_EMAIL", "alice@example.com"),
            ("JIRA_API_TOKEN", "t1"),
            // The team's own names win over the others with the same prefix.
            ("JIRA_BASE_URL", "https://acme.atlassian.net"),
            ("JIRA_TOKEN", "t1"),
            ("JIRA_PROJECT_KEY", "ABC"),
            ("JIRA_ACCEPTANCE_CRITERIA_FIELD", "customfield_10050"),
            // The same site under other names: offered once.
            ("ATLASSIAN_URL", "acme.atlassian.net/"),
            ("ATLASSIAN_USERNAME", "Alice@example.com"),
            ("ATLASSIAN_API_TOKEN", "t1"),
            ("JIRA_OTHER_BASE_URL", "https://other.atlassian.net"),
            ("JIRA_OTHER_USERNAME", "alice@example.com"),
            ("JIRA_OTHER_API_TOKEN", "t2"),
            // No token: not a site.
            ("JIRA_HALF_URL", "https://half.atlassian.net"),
            ("JIRA_HALF_EMAIL", "a@b.c"),
        ]));
        let names: Vec<(String, String)> = found.iter().map(|f| (f.name(), f.names())).collect();
        assert_eq!(
            names,
            [
                ("jira".to_string(), "JIRA_BASE_URL · JIRA_EMAIL · JIRA_TOKEN".to_string()),
                (
                    "other".to_string(),
                    "JIRA_OTHER_BASE_URL · JIRA_OTHER_USERNAME · JIRA_OTHER_API_TOKEN".to_string()
                ),
            ]
        );
        assert_eq!(
            found[0].site("blue").projects,
            [Project { key: "ABC".into(), ac_field: "customfield_10050".into() }],
            "the project and field the set names"
        );
        let site = found[1].site("cyan");
        assert_eq!(site.url, "env:JIRA_OTHER_BASE_URL");
        assert_eq!(site.token, "env:JIRA_OTHER_API_TOKEN");
        assert!(site.projects.is_empty());
        for f in &found {
            let shown = format!("{f:?}");
            assert!(!shown.contains("t1") && !shown.contains("t2"), "{shown}");
        }
    }

    const MCP_REFS: &str = r#"{ "mcpServers": {
        "tools": { "command": "uv", "env": {
            "JIRA_BASE_URL": "https://acme.atlassian.net",
            "JIRA_EMAIL": "${JIRA_EMAIL}",
            "JIRA_TOKEN": "${JIRA_TOKEN}",
            "JIRA_PROJECT_KEY": "ABC",
            "JIRA_ACCEPTANCE_CRITERIA_FIELD": "customfield_10050",
            "GITLAB_TOKEN": "${GITLAB_TOKEN}" } },
        "other": { "command": "x", "env": { "CMM_ENV": "test" } },
        "plain": { "command": "y" } } }"#;

    #[test]
    fn an_mcp_json_with_references_becomes_a_site_reading_the_same_variables() {
        let found = mcp_offers(MCP_REFS, "/work/acme/.mcp.json").unwrap();
        assert_eq!(found.len(), 1, "only the entry with JIRA_BASE_URL");
        let o = &found[0];
        assert_eq!(
            o.site,
            Site {
                name: "acme".into(),
                url: "https://acme.atlassian.net".into(),
                email: "env:JIRA_EMAIL".into(),
                token: "env:JIRA_TOKEN".into(),
                color: String::new(),
                projects: vec![Project { key: "ABC".into(), ac_field: "customfield_10050".into() }],
                from: "/work/acme/.mcp.json".into(),
            }
        );
        assert_eq!((o.label.as_str(), o.source.as_str()), ("tools", "in /work/acme/.mcp.json"));
        assert!(!o.token_typed);
    }

    #[test]
    fn a_token_typed_into_an_mcp_json_is_never_copied() {
        let text = r#"{ "mcpServers": { "tools": { "env": {
            "JIRA_BASE_URL": "${JIRA_SITE}",
            "JIRA_EMAIL": "alice@example.com",
            "JIRA_TOKEN": "not-a-real-token-but-typed-in" } } } }"#;
        let o = &mcp_offers(text, "/w/.mcp.json").unwrap()[0];
        assert!(o.token_typed);
        assert_eq!(o.site.token, "");
        assert_eq!(o.site.url, "env:JIRA_SITE");
        assert_eq!(o.site.email, "alice@example.com", "an email is not a secret");
        assert_eq!(o.site.name, "tools", "no host written: named after the entry");
        assert!(!format!("{o:?}").contains("typed-in"), "the token is nowhere in the offer");
    }

    #[test]
    fn two_servers_two_sites_and_missing_names_default_to_the_teams() {
        let text = r#"{ "mcpServers": {
            "a": { "env": { "JIRA_BASE_URL": "https://acme.atlassian.net", "JIRA_PROJECT_KEY": "${P}" } },
            "b": { "env": { "JIRA_BASE_URL": "beta.atlassian.net/jira", "JIRA_PROJECT_KEY": "not a key" } } } }"#;
        let found = mcp_offers(text, "/w/.mcp.json").unwrap();
        let sites: Vec<(&str, &str, &str, &str, usize)> = found
            .iter()
            .map(|o| (o.site.name.as_str(), o.site.url.as_str(), o.site.email.as_str(), o.site.token.as_str(), o.site.projects.len()))
            .collect();
        assert_eq!(
            sites,
            [
                ("acme", "https://acme.atlassian.net", "env:JIRA_EMAIL", "env:JIRA_TOKEN", 0),
                ("beta", "https://beta.atlassian.net", "env:JIRA_EMAIL", "env:JIRA_TOKEN", 0),
            ]
        );
    }

    #[test]
    fn a_malformed_mcp_json_is_an_error_not_a_panic() {
        assert!(mcp_offers("{ \"mcpServers\": ", "/w/.mcp.json").unwrap_err().starts_with("not JSON"));
        assert!(mcp_offers("[]", "/w/.mcp.json").unwrap().is_empty());
        assert!(mcp_offers(r#"{"mcpServers": {"a": {"env": {"JIRA_BASE_URL": 3}}}}"#, "/w").unwrap().is_empty());
    }

    #[test]
    fn mcp_json_files_are_found_here_and_above() {
        let root = std::env::temp_dir().join(format!("radar-jira-mcp-{}", std::process::id()));
        let deep = root.join("a/b");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(root.join(".mcp.json"), MCP_REFS).unwrap();
        std::fs::write(root.join("a/.mcp.json"), "{ broken").unwrap();
        let files = mcp_files(&deep);
        assert_eq!(files[0], root.join("a/.mcp.json"), "nearest first");
        assert!(files.contains(&root.join(".mcp.json")));
        let found = mcp_found(&deep);
        assert_eq!(found.len(), 1, "the broken one is skipped");
        assert_eq!(found[0].site.from, tilde(&root.join(".mcp.json").display().to_string()));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn offers_skip_what_a_site_has_and_prefer_the_mcp_json_for_a_host() {
        let mcp = mcp_offers(MCP_REFS, "/w/.mcp.json").unwrap();
        let env = vars(&[
            ("JIRA_BASE_URL", "https://acme.atlassian.net"),
            ("JIRA_EMAIL", "a@b.c"),
            ("JIRA_TOKEN", "t"),
            ("JIRA_OTHER_URL", "https://other.atlassian.net"),
            ("JIRA_OTHER_EMAIL", "a@b.c"),
            ("JIRA_OTHER_TOKEN", "t"),
        ]);
        let labels = |sites: &[Site]| -> Vec<String> {
            offers(sites, &env, &mcp).into_iter().map(|o| o.label).collect()
        };
        assert_eq!(
            labels(&[]),
            ["tools", "JIRA_OTHER_URL · JIRA_OTHER_EMAIL · JIRA_OTHER_TOKEN"],
            "acme once, from the .mcp.json"
        );
        // A site reading acme through a variable takes it out.
        let mut s = mcp[0].site.clone();
        s.name = "mine".into();
        s.url = "env:JIRA_BASE_URL".into();
        assert_eq!(labels(&[s]), ["JIRA_OTHER_URL · JIRA_OTHER_EMAIL · JIRA_OTHER_TOKEN"]);
        assert_eq!(
            default_projects(&vars(&[("JIRA_PROJECT_KEY", "ABC"), ("JIRA_ACCEPTANCE_CRITERIA_FIELD", "customfield_1")])),
            [Project { key: "ABC".into(), ac_field: "customfield_1".into() }]
        );
        assert!(default_projects(&vars(&[("JIRA_PROJECT_KEY", "abc def")])).is_empty());
    }

    #[test]
    fn env_settings_resolve_and_say_which_variable_is_missing() {
        assert_eq!(resolve("https://acme.atlassian.net", "address").unwrap(), "https://acme.atlassian.net");
        let e = resolve("env:RADAR_JIRA_TEST_SURELY_UNSET", "token").unwrap_err();
        assert_eq!(e.to_string(), "RADAR_JIRA_TEST_SURELY_UNSET is not set");
        assert!(resolve(" ", "token").is_err());
    }
}
