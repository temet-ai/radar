//! Validated GitHub identities and read-only response snapshots.

use serde::{Deserialize, Serialize};
use std::fmt;

mod actions;
pub use actions::{
    RunPullAssociation, RunPullMatch, RunsAsk, WorkflowJob, WorkflowRun, WorkflowStep,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiErrorKind {
    InvalidTarget,
    InvalidTokenReference,
    MissingToken,
    /// The token is `gh`, and gh gave none (not installed, not signed in).
    NoGhSignIn,
    InvalidToken,
    Authentication,
    Forbidden,
    RateLimit,
    NotFound,
    Redirect,
    Network,
    Body,
    BodyTooLarge,
    InvalidResponse,
    Unavailable,
}

/// Errors deliberately contain no response bodies, URLs or resolved credentials.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub kind: ApiErrorKind,
    pub status: Option<u16>,
    pub retry_after_secs: Option<u64>,
    pub rate_limit_reset: Option<u64>,
}

impl ApiError {
    pub fn new(kind: ApiErrorKind) -> Self {
        Self {
            kind,
            status: None,
            retry_after_secs: None,
            rate_limit_reset: None,
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.kind {
            ApiErrorKind::InvalidTarget => "GitHub target is invalid or not configured",
            ApiErrorKind::InvalidTokenReference => {
                "GitHub token must be empty, gh, a typed token or an env:NAME reference"
            }
            ApiErrorKind::MissingToken => "GitHub token environment variable is missing or empty",
            ApiErrorKind::NoGhSignIn => "gh gave no token; run gh auth login",
            ApiErrorKind::InvalidToken => "GitHub token is invalid",
            ApiErrorKind::Authentication => "GitHub authentication failed; check the token",
            ApiErrorKind::Forbidden => "GitHub access denied; check repository permissions",
            ApiErrorKind::RateLimit => "GitHub rate limit reached; wait before refreshing",
            ApiErrorKind::NotFound => "GitHub resource is missing or inaccessible",
            ApiErrorKind::Redirect => {
                "GitHub redirected this resource; update its configured identity"
            }
            ApiErrorKind::Network => "GitHub request failed or timed out",
            ApiErrorKind::Body => "GitHub response body could not be read",
            ApiErrorKind::BodyTooLarge => "GitHub response exceeded the safe size limit",
            ApiErrorKind::InvalidResponse => "GitHub response did not match the requested resource",
            ApiErrorKind::Unavailable => "GitHub service is temporarily unavailable",
        })?;
        if self.kind == ApiErrorKind::RateLimit {
            if let Some(seconds) = self.retry_after_secs {
                write!(f, "; retry after {seconds}s")?;
            }
            if let Some(epoch) = self.rate_limit_reset {
                write!(f, "; reset at Unix time {epoch}")?;
            }
        }
        Ok(())
    }
}
impl std::error::Error for ApiError {}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RepositoryId {
    pub owner: String,
    pub name: String,
}

impl RepositoryId {
    pub fn parse(input: &str) -> Result<Self, ApiError> {
        let (owner, name) = input.split_once('/').ok_or_else(invalid_target)?;
        if owner.is_empty()
            || owner.len() > 39
            || name.is_empty()
            || name.len() > 100
            || !owner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || owner.starts_with('-')
            || owner.ends_with('-')
            || owner.contains("--")
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            || matches!(name, "." | "..")
            || radar_core::log::contains_secret(input)
        {
            return Err(invalid_target());
        }
        Ok(Self {
            owner: owner.to_ascii_lowercase(),
            name: name.to_ascii_lowercase(),
        })
    }

    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
    pub fn web_url(&self) -> String {
        format!("https://github.com/{}", self.full_name())
    }
    pub fn validated(&self) -> Result<Self, ApiError> {
        Self::parse(&self.full_name())
    }
}

impl fmt::Display for RepositoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

fn invalid_target() -> ApiError {
    ApiError::new(ApiErrorKind::InvalidTarget)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    pub id: RepositoryId,
    pub description: Option<String>,
    pub default_branch: String,
    pub private: bool,
    pub archived: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    pub repository: RepositoryId,
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub author: String,
    pub state: String,
    pub draft: bool,
    pub head_sha: String,
    pub head_repository: Option<RepositoryId>,
    pub head_branch: String,
    pub base_branch: String,
    pub updated_at: String,
    pub merged: bool,
}

impl PullRequest {
    pub fn web_url(&self) -> String {
        format!("{}/pull/{}", self.repository.web_url(), self.number)
    }
}

/// Completion describes the requested endpoint and scope, never merge readiness.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completion {
    pub complete: bool,
    pub reason: Option<String>,
    pub pages: u32,
    #[serde(default)]
    pub error: Option<ApiErrorKind>,
}

impl Completion {
    pub fn failed(pages: u32, error: &ApiError) -> Self {
        Self {
            complete: false,
            reason: Some(error.to_string()),
            pages,
            error: Some(error.kind),
        }
    }
    pub fn complete(pages: u32) -> Self {
        Self {
            complete: true,
            reason: None,
            pages,
            error: None,
        }
    }
    /// A list asked for its newest rows only, by design: complete for what
    /// was asked, with the bound said in the reason.
    pub fn bounded(pages: u32, shown: usize) -> Self {
        Self {
            complete: true,
            reason: Some(format!("newest {shown} shown; older ones are not read")),
            pages,
            error: None,
        }
    }
    pub fn partial(pages: u32, reason: &str) -> Self {
        Self {
            complete: false,
            reason: Some(reason.into()),
            pages,
            error: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection<T> {
    pub items: Vec<T>,
    pub completion: Completion,
}

impl<T> Collection<T> {
    pub fn unavailable(reason: &str) -> Self {
        Self {
            items: vec![],
            completion: Completion::partial(0, reason),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRun {
    pub repository: RepositoryId,
    pub id: u64,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub head_sha: String,
}

impl CheckRun {
    pub fn status_label(&self) -> &'static str {
        match self.status.as_str() {
            "queued" => "queued",
            "in_progress" => "in progress",
            "waiting" => "waiting",
            "pending" => "pending",
            "requested" => "requested",
            "completed" => match self.conclusion.as_deref() {
                Some("success") => "success",
                Some("failure") => "failure",
                Some("neutral") => "neutral",
                Some("cancelled") => "cancelled",
                Some("skipped") => "skipped",
                Some("timed_out") => "timed out",
                Some("action_required") => "action required",
                Some("stale") => "stale",
                Some("startup_failure") => "startup failure",
                _ => "unknown conclusion",
            },
            _ => "unknown status",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitStatus {
    pub repository: RepositoryId,
    pub id: u64,
    pub context: String,
    pub state: String,
    pub description: Option<String>,
}

impl CommitStatus {
    pub fn status_label(&self) -> &'static str {
        match self.state.as_str() {
            "pending" => "pending",
            "success" => "success",
            "failure" => "failure",
            "error" => "error",
            _ => "unknown status",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checks {
    pub head_sha: String,
    pub head_repository: Option<RepositoryId>,
    pub check_runs: Collection<CheckRun>,
    pub statuses: Collection<CommitStatus>,
    pub errors: Vec<String>,
}

impl Checks {
    /// A check run or commit status that has not finished yet.
    pub fn running(&self) -> bool {
        self.check_runs
            .items
            .iter()
            .any(|run| run.status != "completed")
            || self
                .statuses
                .items
                .iter()
                .any(|status| status.state == "pending")
    }
}

/// What GitHub said describes a response (`ETag`, `Last-Modified`). Sent
/// back on the next read of the same URL, an unchanged answer is a 304 with
/// no body, which does not count against the primary rate limit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Validator {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
}

/// A conditional read: new data with the validators that describe it, or
/// `Same` when GitHub answered 304 and the data held is still current.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fresh<T, V = Vec<Validator>> {
    New(T, V),
    Same,
}

/// Validators of each check list read for one head, by `owner/repo/check-runs`
/// and `owner/repo/statuses`, one per page.
pub type CheckTags = std::collections::BTreeMap<String, Vec<Validator>>;

/// One rate-limit bucket as GitHub last reported it (`x-ratelimit-*`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimit {
    pub resource: String,
    pub limit: u64,
    pub remaining: u64,
    /// Unix seconds when the bucket refills.
    pub reset: i64,
}

/// What moved in a repository's open pull requests, read for many
/// repositories in one GraphQL request: how many are open and when the most
/// recently updated one changed. Equal markers mean nothing to read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub open: u64,
    pub latest: Option<String>,
}

/// What one repository's probe learned: its marker, and when it was asked
/// for in full, its description and open pull requests as a REST read would
/// have given them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probed {
    pub id: RepositoryId,
    pub marker: Option<Marker>,
    pub full: Option<Full>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Full {
    pub repository: Repository,
    pub pulls: Collection<PullRequest>,
}

/// What one client sent: requests, how many were 304s, and the last
/// rate-limit report of each bucket it touched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Traffic {
    pub requests: u64,
    pub not_modified: u64,
    pub rates: Vec<RateLimit>,
}

impl Traffic {
    /// Fold another client's traffic in: counts add, and per bucket the
    /// report with the later reset wins (the lower remaining on a tie).
    pub fn add(&mut self, other: Traffic) {
        self.requests += other.requests;
        self.not_modified += other.not_modified;
        for rate in other.rates {
            self.note(rate);
        }
    }

    pub fn note(&mut self, rate: RateLimit) {
        match self.rates.iter_mut().find(|r| r.resource == rate.resource) {
            Some(held) if rate.reset > held.reset => *held = rate,
            Some(held) if rate.reset == held.reset && rate.remaining < held.remaining => {
                *held = rate
            }
            Some(_) => {}
            None => self.rates.push(rate),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_identity_is_canonical_and_rejects_unsafe_segments() {
        assert_eq!(
            RepositoryId::parse("Octo-Cat/Hello.World")
                .unwrap()
                .full_name(),
            "octo-cat/hello.world"
        );
        for invalid in [
            "",
            "owner",
            "/repo",
            "owner/",
            "owner/a/b",
            "owner/..",
            "owner/%2f",
            "owner/repo?token=x",
            "owner/repo#x",
            "owner\\repo",
            "https://github.com/a/b",
            "owner/repo\n",
            "a--b/repo",
            "-a/repo",
            "a-/repo",
        ] {
            assert!(RepositoryId::parse(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn unknown_status_is_not_success_and_missing_conclusion_is_unknown() {
        let mut run = CheckRun {
            repository: RepositoryId::parse("org/repo").unwrap(),
            id: 1,
            name: "CI".into(),
            status: "future".into(),
            conclusion: Some("success".into()),
            head_sha: "a".repeat(40),
        };
        assert_eq!(run.status_label(), "unknown status");
        run.status = "completed".into();
        run.conclusion = None;
        assert_eq!(run.status_label(), "unknown conclusion");
        let status = CommitStatus {
            repository: RepositoryId::parse("org/repo").unwrap(),
            id: 1,
            context: "CI".into(),
            state: "future".into(),
            description: None,
        };
        assert_eq!(status.status_label(), "unknown status");
    }
}
