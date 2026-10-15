//! Observed Actions activity. None of these snapshots establishes merge readiness
//! or the complete set of workflows required on a branch.

use super::{PullRequest, RepositoryId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub repository: RepositoryId,
    pub id: u64,
    pub workflow_id: u64,
    pub run_number: u64,
    pub run_attempt: u64,
    pub name: Option<String>,
    pub display_title: String,
    pub path: String,
    pub event: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub head_sha: String,
    pub head_branch: Option<String>,
    pub head_repository: Option<RepositoryId>,
    pub created_at: String,
    pub updated_at: String,
    pub run_started_at: Option<String>,
    pub pull_requests: Vec<RunPullAssociation>,
}

/// GitHub may associate a run with PRs in other base repositories. Each
/// association keeps its own validated base identity; its head SHA may differ
/// from the run SHA (for example when testing a merge ref).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunPullAssociation {
    pub repository: RepositoryId,
    pub number: u64,
    pub head_sha: String,
    pub head_repository: Option<RepositoryId>,
}

/// What the Actions tab asks of the workflow-runs endpoint: only the newest
/// `per_page` runs (sized to the view), of one branch where the page implies
/// it (a pull request's source branch). The same ask is the same URL, so
/// the validators of its last read can be sent back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunsAsk {
    pub branch: Option<String>,
    pub per_page: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunPullMatch {
    Associated,
    HeadSha,
}

impl WorkflowRun {
    pub fn web_url(&self) -> String {
        format!(
            "{}/actions/runs/{}/attempts/{}",
            self.repository.web_url(),
            self.id,
            self.run_attempt
        )
    }

    pub fn status_label(&self) -> &'static str {
        status_label(&self.status, self.conclusion.as_deref())
    }

    /// A displayed relationship, not coverage of every PR-triggered workflow.
    /// Never correlate forks by branch name, PR number alone, or SHA alone.
    pub fn pull_match(&self, pull: &PullRequest) -> Option<RunPullMatch> {
        if self.repository != pull.repository {
            return None;
        }
        if self.pull_requests.iter().any(|association| {
            association.repository == pull.repository && association.number == pull.number
        }) {
            return Some(RunPullMatch::Associated);
        }
        if self.head_repository.is_some()
            && self.head_repository == pull.head_repository
            && self.head_sha.eq_ignore_ascii_case(&pull.head_sha)
        {
            return Some(RunPullMatch::HeadSha);
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowJob {
    pub repository: RepositoryId,
    pub id: u64,
    pub run_id: u64,
    /// Bound to the explicit attempt endpoint, including when GitHub omits the
    /// optional run_attempt field on individual jobs.
    pub run_attempt: u64,
    pub head_sha: String,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub steps: Vec<WorkflowStep>,
}

impl WorkflowJob {
    pub fn web_url(&self) -> String {
        format!(
            "{}/actions/runs/{}/job/{}",
            self.repository.web_url(),
            self.run_id,
            self.id
        )
    }

    pub fn status_label(&self) -> &'static str {
        status_label(&self.status, self.conclusion.as_deref())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub number: u64,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

impl WorkflowStep {
    pub fn status_label(&self) -> &'static str {
        status_label(&self.status, self.conclusion.as_deref())
    }
}

fn status_label(status: &str, conclusion: Option<&str>) -> &'static str {
    match status {
        "queued" => "queued",
        "in_progress" => "in progress",
        "waiting" => "waiting",
        "pending" => "pending",
        "requested" => "requested",
        "completed" => match conclusion {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_preserve_every_known_state_and_unknown_is_not_success() {
        for (value, label) in [
            ("queued", "queued"),
            ("in_progress", "in progress"),
            ("waiting", "waiting"),
            ("pending", "pending"),
            ("requested", "requested"),
        ] {
            assert_eq!(status_label(value, Some("success")), label);
        }
        for (value, label) in [
            ("success", "success"),
            ("failure", "failure"),
            ("cancelled", "cancelled"),
            ("neutral", "neutral"),
            ("skipped", "skipped"),
            ("timed_out", "timed out"),
            ("action_required", "action required"),
            ("stale", "stale"),
            ("startup_failure", "startup failure"),
        ] {
            assert_eq!(status_label("completed", Some(value)), label);
        }
        assert_eq!(status_label("future", Some("success")), "unknown status");
        assert_eq!(status_label("", None), "unknown status");
        assert_eq!(status_label("completed", None), "unknown conclusion");
        assert_eq!(
            status_label("completed", Some("future")),
            "unknown conclusion"
        );
    }
}
