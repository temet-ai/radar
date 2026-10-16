//! Configuration stores repository names and a typed token or environment reference.

use serde::{Deserialize, Serialize};

use crate::{auth, model::RepositoryId};

pub(crate) const MAX_REPOSITORIES: usize = 200;
/// `[github] api` when the file does not say.
pub const DEFAULT_API: &str = "https://api.github.com";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// The GitHub API address, `[github] api`; left out of the file while it
    /// is the default, https://api.github.com.
    #[serde(skip_serializing_if = "is_default_api")]
    pub api: String,
    pub token: String,
    /// What a token is made to allow, `[github] access`: `read` (the
    /// default, left out of the file) or `write`. The new-token page fills
    /// its permissions in from it.
    #[serde(skip_serializing_if = "TokenAccess::is_read")]
    pub access: TokenAccess,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
    #[serde(skip_serializing)]
    pub repositories: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Group {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub repositories: Vec<String>,
}

/// `[github] access`: what a token is made to allow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenAccess {
    /// Pull requests, checks and runs.
    #[default]
    Read,
    /// Also comment, approve, merge and re-run.
    Write,
}

impl TokenAccess {
    pub const ALL: [Self; 2] = [Self::Read, Self::Write];

    pub fn is_read(&self) -> bool {
        *self == Self::Read
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Read => "read only",
            Self::Write => "read and write",
        }
    }

    /// The fine-grained token's permissions, as the new-token page takes
    /// them: what radar reads, and for write what reviewing (pull requests),
    /// merging (contents) and re-running (actions) need. Statuses are only
    /// read.
    pub const fn permissions(self) -> [(&'static str, &'static str); 4] {
        match self {
            Self::Read => [("contents", "read"), ("pull_requests", "read"), ("actions", "read"), ("statuses", "read")],
            Self::Write => [("contents", "write"), ("pull_requests", "write"), ("actions", "write"), ("statuses", "read")],
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api: DEFAULT_API.into(),
            token: String::new(),
            access: TokenAccess::Read,
            groups: Vec::new(),
            repositories: Vec::new(),
        }
    }
}

fn is_default_api(api: &String) -> bool {
    api == DEFAULT_API
}

/// Where the API is, and the one place a request or a response URL is
/// compared with it: REST under `rest`, GraphQL at `graphql`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Endpoint {
    url: url::Url,
    rest: String,
    graphql: String,
}

impl Default for Endpoint {
    fn default() -> Self {
        Self::parse(DEFAULT_API).expect("the default API address is valid")
    }
}

impl Endpoint {
    /// `https://api.github.com`, `http://127.0.0.1:PORT` or a GitHub
    /// Enterprise Server `https://host/api/v3`. No credentials, query or
    /// fragment; a trailing slash is dropped.
    pub(crate) fn parse(value: &str) -> anyhow::Result<Self> {
        let value = value.trim().trim_end_matches('/');
        let url = url::Url::parse(value).map_err(|_| {
            anyhow::anyhow!("the GitHub API address must be a URL such as {DEFAULT_API}")
        })?;
        anyhow::ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some_and(|h| !h.is_empty()),
            "the GitHub API address must start with https:// (or http:// for a local server)"
        );
        anyhow::ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && !url.path().contains(['%', '\\']),
            "the GitHub API address is only a server address, with no login, query or fragment"
        );
        let rest = url.as_str().trim_end_matches('/').to_owned();
        // GitHub Enterprise Server serves REST under /api/v3 and GraphQL at
        // /api/graphql beside it; everywhere else GraphQL is /graphql.
        let graphql = match rest.strip_suffix("/api/v3") {
            Some(origin) => format!("{origin}/api/graphql"),
            None => format!("{rest}/graphql"),
        };
        Ok(Self { url, rest, graphql })
    }

    /// The REST base, with no trailing slash; a request path follows it.
    pub(crate) fn rest(&self) -> &str {
        &self.rest
    }

    /// The one URL a request with a body may go to.
    pub(crate) fn graphql(&self) -> &str {
        &self.graphql
    }

    pub(crate) fn is_https(&self) -> bool {
        self.url.scheme() == "https"
    }

    /// The host gh signs in to for this API: `github.com` for
    /// api.github.com, else the server's own host (and port).
    pub(crate) fn gh_host(&self) -> String {
        match (self.url.host_str(), self.url.port()) {
            (Some("api.github.com") | None, _) => "github.com".into(),
            (Some(host), Some(port)) => format!("{host}:{port}"),
            (Some(host), None) => host.into(),
        }
    }

    /// The path the REST base sits under (empty, or `/api/v3`).
    pub(crate) fn base_path(&self) -> &str {
        self.url.path().trim_end_matches('/')
    }

    /// Whether `other` is on this API's scheme, host and port, with no login
    /// or fragment: the test before a response's address is trusted at all.
    pub(crate) fn is_origin_of(&self, other: &url::Url) -> bool {
        other.scheme() == self.url.scheme()
            && other.host_str() == self.url.host_str()
            && other.port() == self.url.port()
            && other.username().is_empty()
            && other.password().is_none()
            && other.fragment().is_none()
    }
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("api", &self.api)
            .field(
                "token",
                &if self.token.is_empty() {
                    "public"
                } else if self.token.starts_with("env:") {
                    "environment reference"
                } else if self.token == auth::GH {
                    "gh sign-in"
                } else {
                    "[redacted]"
                },
            )
            .field("access", &self.access)
            .field("repositories", &self.repositories)
            .field("groups", &self.groups)
            .finish()
    }
}

impl Settings {
    pub fn validated(mut self) -> anyhow::Result<Self> {
        let api = self.api.trim().trim_end_matches('/');
        self.api = if api.is_empty() {
            DEFAULT_API.into()
        } else {
            Endpoint::parse(api)?.rest().to_owned()
        };
        self.token = self.token.trim().to_owned();
        auth::validate_reference(&self.token)?;
        if auth::is_typed(&self.token) {
            radar_core::log::secret(&self.token);
        }
        // Older configurations contain a flat repository list. Give each
        // owner a group so those repositories remain visible after upgrading.
        let legacy = std::mem::take(&mut self.repositories);
        if self.groups.is_empty() {
            for value in legacy {
                let repo = repository(&value)?;
                let name = format!("github.com/{}", repo.owner);
                let index = self
                    .groups
                    .iter()
                    .position(|g| g.name == name)
                    .unwrap_or_else(|| {
                        self.groups.push(Group {
                            name,
                            owner: Some(repo.owner.clone()),
                            repositories: vec![],
                        });
                        self.groups.len() - 1
                    });
                let name = repo.full_name();
                if !self.groups[index].repositories.contains(&name) {
                    self.groups[index].repositories.push(name);
                }
            }
        }
        let mut names = Vec::new();
        let mut repos = Vec::new();
        for group in &mut self.groups {
            group.name = group.name.trim().into();
            anyhow::ensure!(!group.name.is_empty(), "group name cannot be empty");
            anyhow::ensure!(
                !names.contains(&group.name.to_lowercase()),
                "group names must be unique"
            );
            names.push(group.name.to_lowercase());
            if let Some(owner_name) = &group.owner {
                group.owner = Some(owner(owner_name)?);
            }
            let mut members = Vec::new();
            for value in &group.repositories {
                let repo = repository(value)?.full_name();
                anyhow::ensure!(
                    group
                        .owner
                        .as_ref()
                        .is_none_or(|owner| repo.starts_with(&format!("{owner}/"))),
                    "repository must belong to the group's GitHub owner"
                );
                anyhow::ensure!(
                    !repos.contains(&repo),
                    "repository can belong to only one group"
                );
                repos.push(repo.clone());
                members.push(repo);
            }
            group.repositories = members;
        }
        anyhow::ensure!(
            repos.len() <= MAX_REPOSITORIES,
            "track at most {MAX_REPOSITORIES} repositories"
        );
        self.repositories = repos;
        Ok(self)
    }

    pub fn ids(&self) -> Vec<RepositoryId> {
        self.repositories
            .iter()
            .filter_map(|r| RepositoryId::parse(r).ok())
            .collect()
    }
}

pub(crate) fn repository(value: &str) -> anyhow::Result<RepositoryId> {
    let value = value.trim();
    let path = value
        .strip_prefix("https://github.com/")
        .or_else(|| value.strip_prefix("github.com/"))
        .unwrap_or(value);
    Ok(RepositoryId::parse(path)?)
}

/// A GitHub user or organization URL, or its login.
pub(crate) fn owner(value: &str) -> anyhow::Result<String> {
    let value = value.trim();
    let path = value
        .strip_prefix("https://github.com/")
        .or_else(|| value.strip_prefix("github.com/"))
        .unwrap_or(value);
    let login = path.strip_suffix('/').unwrap_or(path);
    Ok(RepositoryId::parse(&format!("{login}/placeholder"))?.owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_round_trip_contains_only_references() {
        let cfg = Settings {
            token: "env:GH_TOKEN".into(),
            groups: vec![],
            repositories: vec!["Acme/Api".into(), "https://github.com/acme/api".into()],
            ..Settings::default()
        }
        .validated()
        .unwrap();
        assert_eq!(cfg.repositories, ["acme/api"]);
        let text = toml::to_string(&cfg).unwrap();
        assert_eq!(
            toml::from_str::<Settings>(&text)
                .unwrap()
                .validated()
                .unwrap(),
            cfg
        );
    }

    /// `access` is read from the file and saved only once it is write.
    #[test]
    fn the_access_is_loaded_and_saved_only_when_it_is_write() {
        let cfg = toml::from_str::<Settings>("token = 'gh'\naccess = 'write'").unwrap().validated().unwrap();
        assert_eq!(cfg.access, TokenAccess::Write);
        let saved = toml::to_string(&cfg).unwrap();
        assert!(saved.contains("access = \"write\""), "{saved}");
        assert_eq!(toml::from_str::<Settings>(&saved).unwrap().access, TokenAccess::Write);
        assert_eq!(toml::from_str::<Settings>("token = ''").unwrap().access, TokenAccess::Read);
        assert!(!toml::to_string(&Settings::default()).unwrap().contains("access"));
        assert!(toml::from_str::<Settings>("access = 'admin'").is_err());
    }

    #[test]
    fn api_defaults_to_github_com_and_a_default_is_not_written_out() {
        assert_eq!(Settings::default().api, DEFAULT_API);
        let cfg = toml::from_str::<Settings>("token = ''").unwrap();
        assert_eq!(cfg.api, "https://api.github.com");
        let blank = toml::from_str::<Settings>("api = ' '")
            .unwrap()
            .validated()
            .unwrap();
        assert_eq!(blank.api, DEFAULT_API);
        let text = toml::to_string(&cfg.validated().unwrap()).unwrap();
        assert!(!text.contains("api"), "{text}");
    }

    #[test]
    fn a_custom_api_survives_load_and_save() {
        let cfg = toml::from_str::<Settings>(
            "api = 'https://ghe.example/api/v3/'
token = 'env:GH_TOKEN'",
        )
        .unwrap()
        .validated()
        .unwrap();
        assert_eq!(cfg.api, "https://ghe.example/api/v3");
        assert_eq!(
            Endpoint::parse(&cfg.api).unwrap().graphql(),
            "https://ghe.example/api/graphql"
        );
        let text = toml::to_string(&cfg).unwrap();
        assert!(
            text.contains("api = \"https://ghe.example/api/v3\""),
            "{text}"
        );
        let again = toml::from_str::<Settings>(&text)
            .unwrap()
            .validated()
            .unwrap();
        assert_eq!(again, cfg);
        assert!(format!("{cfg:?}").contains("ghe.example"));
    }

    #[test]
    fn a_bad_api_is_refused_when_the_configuration_is_checked() {
        for bad in [
            "gitlab.com",
            "ftp://x.test",
            "https://u:p@x.test",
            "https://x.test?a=1",
        ] {
            let cfg = Settings {
                api: bad.into(),
                ..Settings::default()
            };
            assert!(cfg.validated().is_err(), "{bad}");
        }
    }

    #[test]
    fn malformed_credentials_and_ambiguous_repository_urls_are_refused() {
        assert!(
            Settings {
                token: "short".into(),
                ..Settings::default()
            }
            .validated()
            .is_err()
        );
        for bad in [
            "https://github.com/acme/api?token=x",
            "https://github.com/acme/api/",
            "https://user@github.com/acme/api",
            "https://elsewhere.test/acme/api",
            "acme/api/extra",
            "acme/../api",
        ] {
            assert!(repository(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn owner_urls_and_logins_are_accepted_without_a_repository() {
        assert_eq!(
            owner("https://github.com/AliceExample").unwrap(),
            "aliceexample"
        );
        assert_eq!(owner("github.com/AliceExample/").unwrap(), "aliceexample");
        assert_eq!(owner("acme-org").unwrap(), "acme-org");
        for bad in [
            "https://elsewhere.test/acme",
            "github.com/acme/api",
            "acme/api",
        ] {
            assert!(owner(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn repository_accepts_the_address_shown_in_the_ui() {
        assert_eq!(
            repository("github.com/AliceExample/Notes").unwrap().full_name(),
            "aliceexample/notes"
        );
        assert_eq!(
            repository("https://github.com/AliceExample/Notes")
                .unwrap()
                .full_name(),
            "aliceexample/notes"
        );
    }

    #[test]
    fn a_typed_token_is_never_printed_by_debug() {
        let token = "github_pat_synthetic_debug_123";
        let cfg = Settings {
            token: token.into(),
            groups: vec![],
            repositories: vec![],
            ..Settings::default()
        }
        .validated()
        .unwrap();
        assert!(!format!("{cfg:?}").contains(token));
        assert!(radar_core::log::contains_secret(token));
    }

    #[test]
    fn named_groups_round_trip_with_empty_groups_and_legacy_repositories_migrate() {
        let cfg = Settings {
            groups: vec![
                Group {
                    name: "Client work".into(),
                    owner: None,
                    repositories: vec!["acme/api".into()],
                },
                Group {
                    name: "github.com/acme-org".into(),
                    owner: Some("acme-org".into()),
                    repositories: vec![],
                },
            ],
            ..Settings::default()
        }
        .validated()
        .unwrap();
        let text = toml::to_string(&cfg).unwrap();
        assert_eq!(text.matches("repositories =").count(), 1);
        assert_eq!(
            toml::from_str::<Settings>(&text)
                .unwrap()
                .validated()
                .unwrap(),
            cfg
        );
        let legacy = toml::from_str::<Settings>("repositories = ['acme/api', 'acme-org/web']")
            .unwrap()
            .validated()
            .unwrap();
        assert_eq!(legacy.groups.len(), 2);
        assert_eq!(legacy.groups[0].name, "github.com/acme");
        assert_eq!(legacy.groups[1].name, "github.com/acme-org");
    }
}
