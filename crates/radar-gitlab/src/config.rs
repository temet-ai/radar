//! The `[gitlab]` table: connection, tenancies, views and styles.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Tenancy colour names. Each is the domain slot `tenant.<name>`; its value
/// lives in the theme and can be overridden in `[theme.slots]`.
pub const TENANT_COLORS: [&str; 7] = [
    "cyan", "green", "magenta", "purple", "blue", "yellow", "orange",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_url")]
    pub url: String,
    /// `env:VAR`, `glab`, or a literal token.
    #[serde(default = "default_token")]
    pub token: String,
    /// Group path prefix that every source's `group` is relative to.
    #[serde(default)]
    pub base: String,
    #[serde(default = "default_refresh")]
    pub refresh_secs: u64,
    /// Refreshing pauses after this many minutes without a key or click;
    /// 0 never pauses.
    #[serde(default = "default_idle")]
    pub idle_minutes: u64,
    /// How far back merged and closed merge requests are fetched, for views
    /// that show them.
    #[serde(default = "default_history")]
    pub history_days: u64,
    /// Style spec per state and pipeline word, e.g. `review = "status.warning bold"`.
    #[serde(default = "default_styles")]
    pub style: BTreeMap<String, String>,
    #[serde(default, rename = "source", skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,
    #[serde(default = "default_views", rename = "view")]
    pub views: Vec<View>,
    /// A merge request's diff side by side (old left, new right) where it
    /// fits; `v` on the diff switches it and it is kept.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub side_by_side: bool,
    /// The Changes tab's files collapsed, the diff the whole width; `f`
    /// switches it and it is kept.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hide_files: bool,
}

/// A tenancy: every repo in one GitLab group, minus the hidden ones; or one
/// repo on its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// The group's path, or the repo's. Which it is, GitLab says on the
    /// first sync or check.
    pub group: String,
    pub code: String,
    #[serde(default = "default_color")]
    pub color: String,
    /// Repo paths relative to `group` that stay tracked but never show.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub name: String,
    #[serde(default)]
    pub filter: String,
    /// Show every repo, or only repos with a merge request in the view.
    #[serde(default, skip_serializing_if = "RepoScope::is_all")]
    pub repos: RepoScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoScope {
    #[default]
    All,
    WithMergeRequests,
}

impl RepoScope {
    pub fn is_all(&self) -> bool {
        *self == Self::All
    }

    pub fn flipped(self) -> Self {
        match self {
            Self::All => Self::WithMergeRequests,
            Self::WithMergeRequests => Self::All,
        }
    }
}

fn default_url() -> String {
    "https://gitlab.com".into()
}
fn default_token() -> String {
    "env:GITLAB_TOKEN".into()
}
fn default_refresh() -> u64 {
    60
}
fn default_color() -> String {
    "cyan".into()
}
fn default_idle() -> u64 {
    30
}
fn default_history() -> u64 {
    14
}

/// Which slot and modifiers each word is drawn in. The keys are the state
/// and pipeline words, `shown`/`hidden` in sources, `tenant` (modifiers
/// added to a tenancy's own colour slot), and the tree's `band`, `heading`
/// and `lines`.
pub fn default_styles() -> BTreeMap<String, String> {
    [
        ("review", "status.warning bold"),
        ("fix", "status.error bold"),
        ("ready", "status.success bold"),
        ("waiting", "status.info"),
        ("draft", "text.muted"),
        ("you", "accent.primary"),
        ("team", "text.secondary"),
        ("bot", "text.muted"),
        ("merged", "accent.tertiary"),
        ("closed", "text.muted italic"),
        ("label", "accent.secondary"),
        ("passed", "status.success"),
        ("failed", "status.error bold"),
        ("running", "status.info"),
        ("manual", "status.info"),
        ("pending", "text.muted"),
        ("canceled", "text.muted"),
        ("skipped", "text.muted"),
        ("shown", "status.success"),
        ("hidden", "text.muted"),
        ("tenant", "bold"),
        // The repositories tree. `band` and `heading` are backgrounds: the
        // slot's colour goes behind the row. Panels are `bg.surface`, so a
        // band is the next shade up.
        ("band", "bg.overlay"),
        ("heading", "bg.overlay"),
        ("lines", "text.muted"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// The views a new config starts with, named for what they show. The first
/// is what radar opens on.
pub fn default_views() -> Vec<View> {
    use RepoScope::{All, WithMergeRequests as Busy};
    [
        ("Everything", "", All),
        ("Needs my action", "is:review,fix,ready", Busy),
        ("Opened by me", "author:@me", Busy),
        ("Opened by teammates", "-author:@me -is:bot", Busy),
        ("Opened by bots", "is:bot", Busy),
    ]
    .into_iter()
    .map(|(name, filter, repos)| View {
        name: name.into(),
        filter: filter.into(),
        repos,
    })
    .collect()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            url: default_url(),
            token: default_token(),
            base: String::new(),
            refresh_secs: default_refresh(),
            idle_minutes: default_idle(),
            history_days: default_history(),
            style: default_styles(),
            sources: Vec::new(),
            views: default_views(),
            side_by_side: false,
            hide_files: false,
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

    /// Full GitLab path of a source's group.
    pub fn group_path(&self, source: &Source) -> String {
        join_path(&self.base, &source.group)
    }

    /// The source a project belongs to (longest matching group) and the
    /// project's path relative to that group: empty when the source is the
    /// repo itself.
    pub fn source_for(&self, project_full_path: &str) -> Option<(&Source, String)> {
        self.sources
            .iter()
            .filter_map(|s| {
                let group = self.group_path(s);
                if project_full_path == group {
                    return Some((group.len(), s, String::new()));
                }
                project_full_path
                    .strip_prefix(&group)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .map(|rel| (group.len(), s, rel.to_string()))
            })
            .max_by_key(|(len, _, _)| *len)
            .map(|(_, s, rel)| (s, rel))
    }

    pub fn is_hidden(&self, project_full_path: &str) -> bool {
        self.source_for(project_full_path)
            .is_some_and(|(s, rel)| s.hide.iter().any(|h| h == &rel))
    }
}

/// Turn what someone pastes into a group path. Accepts a GitLab web URL
/// (`https://gitlab.com/acme/platform/team-a/`, including `/-/…` pages), a path
/// with or without slashes, and a path that starts part-way through `base`.
pub fn group_path_of(input: &str) -> String {
    let mut s = input.trim();
    if let Some(rest) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
    {
        s = rest.split_once('/').map_or("", |(_, path)| path);
    }
    let s = s.split("/-/").next().unwrap_or(s);
    let s = s.split(['?', '#']).next().unwrap_or(s);
    s.trim_matches('/').to_string()
}

/// Why a group path cannot be right, if it cannot. GitLab paths are segments
/// of letters, digits, `.`, `_` and `-`, joined by `/`.
pub fn group_path_problem(path: &str) -> Option<String> {
    if path.is_empty() {
        return Some("paste the group's GitLab link or type its path".into());
    }
    if path.contains("://") || path.contains("http:") || path.contains("https:") {
        return Some(
            "that has a link inside a path; clear the field (ctrl-u) and paste just the link"
                .into(),
        );
    }
    path.split('/')
        .find(|seg| {
            seg.is_empty()
                || !seg
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        })
        .map(|seg| {
            if seg.is_empty() {
                "a group path has no empty parts (//)".into()
            } else {
                format!("\"{seg}\" is not a GitLab path part (letters, digits, . _ -)")
            }
        })
}

/// A pasted group, relative to `base`. The input may repeat all of `base`,
/// only its tail (`platform/tenants/x` under `acme/platform`), or none of it.
pub fn relative_group(input: &str, base: &str) -> String {
    let path = group_path_of(input);
    let base = base.trim_matches('/');
    if base.is_empty() {
        return path;
    }
    if let Some(rest) = path.strip_prefix(base).and_then(|r| r.strip_prefix('/')) {
        return rest.to_string();
    }
    // The longest tail of `base` that the path starts with.
    let segs: Vec<&str> = base.split('/').collect();
    for start in 1..segs.len() {
        let tail = segs[start..].join("/");
        if let Some(rest) = path.strip_prefix(&tail).and_then(|r| r.strip_prefix('/')) {
            return rest.to_string();
        }
    }
    path
}

pub fn join_path(base: &str, rest: &str) -> String {
    let base = base.trim_matches('/');
    let rest = rest.trim_matches('/');
    match (base.is_empty(), rest.is_empty()) {
        (true, _) => rest.to_string(),
        (_, true) => base.to_string(),
        _ => format!("{base}/{rest}"),
    }
}

/// The server's host name, as glab knows it: `gitlab.com`.
pub fn host_of(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string()
}

/// Resolve the configured token: `env:VAR`, `glab` (asks the glab CLI), or a literal.
pub fn resolve_token(spec: &str, url: &str) -> Result<String> {
    let spec = spec.trim();
    if let Some(var) = spec.strip_prefix("env:") {
        return std::env::var(var).with_context(|| format!("{var} is not set"));
    }
    if spec == "glab" {
        let host = host_of(url);
        let out = std::process::Command::new("glab")
            .args(["config", "get", "token", "--host", &host])
            .output()
            .context("running glab")?;
        let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if token.is_empty() {
            bail!("glab has no token for {host}; run `glab auth login`");
        }
        return Ok(token);
    }
    if spec.is_empty() {
        bail!("no GitLab token configured");
    }
    Ok(spec.to_string())
}

/// Variables in `vars` (name, value) that hold a GitLab token, by name:
/// names with GITLAB and TOKEN in them whose value looks like a token, a
/// personal access token (`glpat-`) first; not the one in use (`have`).
/// Values are read only to tell; never shown or saved.
pub fn token_offers(vars: &[(String, String)], have: &str) -> Vec<String> {
    let mut found: Vec<(bool, &String)> = vars
        .iter()
        .filter(|(name, value)| {
            let up = name.to_uppercase();
            let v = value.trim();
            up.contains("GITLAB")
                && up.contains("TOKEN")
                && v.len() >= 20
                && !v.contains(char::is_whitespace)
                && have.trim() != format!("env:{name}")
        })
        .map(|(name, value)| (!value.trim().starts_with("glpat-"), name))
        .collect();
    found.sort();
    found.dedup();
    found.into_iter().map(|(_, n)| n.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_in_the_environment_is_offered_by_name_and_not_once_in_use() {
        let vars: Vec<(String, String)> = [
            ("GITLAB_HOST", "https://gitlab.example"),
            ("MY_GITLAB_TOKEN", "abcdefghijklmnopqrstuvwxyz"),
            ("GITLAB_TOKEN", "glpat-abcdefghijklmnopqrst"),
            ("GITLAB_TOKEN_NOTE", "not a token at all"),
            ("OTHER_TOKEN", "glpat-abcdefghijklmnopqrst"),
        ]
        .iter()
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect();
        assert_eq!(token_offers(&vars, ""), vec!["GITLAB_TOKEN", "MY_GITLAB_TOKEN"]);
        assert_eq!(token_offers(&vars, "env:GITLAB_TOKEN"), vec!["MY_GITLAB_TOKEN"]);
    }

    fn sample() -> Settings {
        Settings {
            base: "acme/platform".into(),
            sources: vec![
                Source {
                    group: "team-a".into(),
                    code: "a".into(),
                    color: "cyan".into(),
                    hide: vec![],
                },
                Source {
                    group: "team-a/tools".into(),
                    code: "at".into(),
                    color: "green".into(),
                    hide: vec![],
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn pasted_groups_become_paths_relative_to_the_base() {
        let base = "acme/platform";
        for input in [
            "https://gitlab.com/acme/platform/tenants/team-a/",
            "https://gitlab.example.com/acme/platform/tenants/team-a/-/merge_requests?state=opened",
            "/platform/tenants/team-a/",
            "platform/tenants/team-a",
            "acme/platform/tenants/team-a",
            "tenants/team-a",
            "  /tenants/team-a/  ",
        ] {
            assert_eq!(relative_group(input, base), "tenants/team-a", "{input}");
        }
        assert_eq!(
            relative_group("https://gitlab.com/other/group", base),
            "other/group"
        );
        assert_eq!(relative_group("/acme/platform/x/", ""), "acme/platform/x");
    }

    #[test]
    fn broken_group_paths_are_explained() {
        assert_eq!(group_path_problem("tenants/team-a"), None);
        assert!(
            group_path_problem("tenants/team-ahttps://gitlab.com/acme")
                .unwrap()
                .contains("link inside")
        );
        assert!(
            group_path_problem("tenants//team-a")
                .unwrap()
                .contains("empty")
        );
        assert!(
            group_path_problem("tenants/team a")
                .unwrap()
                .contains("not a GitLab path part")
        );
        assert!(group_path_problem("").is_some());
    }

    #[test]
    fn the_table_round_trips_through_toml() {
        let mut cfg = sample();
        cfg.sources[0].hide.push("legacy".into());
        cfg.style
            .insert("review".into(), "accent.primary underline".into());
        let value = toml::Value::try_from(&cfg).unwrap();
        let back: Settings = value.try_into().unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn source_for_picks_the_longest_group_and_relative_path() {
        let mut cfg = sample();
        cfg.sources[0].hide.push("legacy".into());
        let (s, rel) = cfg.source_for("acme/platform/team-a/tools/linter").unwrap();
        assert_eq!((s.code.as_str(), rel.as_str()), ("at", "linter"));
        let (s, rel) = cfg.source_for("acme/platform/team-a/api").unwrap();
        assert_eq!((s.code.as_str(), rel.as_str()), ("a", "api"));
        assert!(cfg.is_hidden("acme/platform/team-a/legacy"));
        assert!(cfg.source_for("other/group/repo").is_none());
    }

    #[test]
    fn a_tenancy_can_be_one_repo_and_owns_it_over_its_group() {
        let mut cfg = sample();
        cfg.sources.push(Source {
            group: "team-a/api".into(),
            code: "api".into(),
            color: "blue".into(),
            hide: vec![],
        });
        let (s, rel) = cfg.source_for("acme/platform/team-a/api").unwrap();
        assert_eq!((s.code.as_str(), rel.as_str()), ("api", ""));
        let (s, _) = cfg.source_for("acme/platform/team-a/web").unwrap();
        assert_eq!(s.code, "a");
        assert_eq!(
            relative_group("https://gitlab.com/acme/platform/team-a/api/-/merge_requests/12", "acme/platform"),
            "team-a/api",
            "a pasted repo link"
        );
    }

    #[test]
    fn an_empty_table_gets_defaults_and_missing_styles_fall_back() {
        let cfg: Settings =
            toml::from_str("base = \"acme\"\n[style]\nreview = \"accent.primary\"\n").unwrap();
        assert_eq!(cfg.url, "https://gitlab.com");
        assert_eq!(cfg.views, default_views());
        assert_eq!(cfg.style_for("review"), "accent.primary");
        assert_eq!(cfg.style_for("failed"), "status.error bold");
    }
}
