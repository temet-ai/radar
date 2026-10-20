//! What stands between a merge request and merging, from GitLab's own data.
//!
//! GitLab's `detailedMergeStatus` is the source of truth: it names the
//! first thing in the way, and leads. Its mergeability checks (newer
//! servers) name the rest, already weighed against the repo's settings: a
//! pipeline counts only where the repo needs one to pass, threads only
//! where they must be resolved, approvals only where some are required.
//! Without the checks (an older server, or an older cache), the rest comes
//! from the merge request's own fields.

use radar_core::Tone;
use radar_core::ui::icon::Icon;

use crate::model::{Check, Mr, MrState, Pipeline};

/// One thing in the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Draft,
    Conflicts,
    Rebase,
    /// The repo needs a passing pipeline and this one has not (yet): its
    /// state, or `None` when there is none.
    Pipeline(Option<Pipeline>),
    /// Unresolved threads that must be resolved.
    Threads(u32),
    /// Approvals still required.
    Approvals(u32),
    ChangesRequested,
    /// GitLab is still working it out.
    Checking,
    /// Blocked by another merge request.
    Blocked,
    /// Anything else GitLab names, in words.
    Other(String),
}

impl Block {
    /// The words, as the STATE column and the lists say them.
    pub fn words(&self) -> String {
        let plural = |n: u32, one: &str, many: &str| if n == 1 { one } else { many }.to_string();
        match self {
            Self::Draft => "draft".into(),
            Self::Conflicts => "has conflicts".into(),
            Self::Rebase => "needs rebase".into(),
            Self::Pipeline(None) => "needs a pipeline".into(),
            Self::Pipeline(Some(p)) => format!("pipeline {}", p.word()),
            Self::Threads(n) => format!("{n} {} open", plural(*n, "thread", "threads")),
            Self::Approvals(n) => format!("needs {n} {}", plural(*n, "approval", "approvals")),
            Self::ChangesRequested => "changes requested".into(),
            Self::Checking => "checking".into(),
            Self::Blocked => "blocked by another MR".into(),
            Self::Other(w) => w.clone(),
        }
    }

    pub const fn tone(&self) -> Tone {
        match self {
            Self::Conflicts | Self::ChangesRequested => Tone::Bad,
            Self::Pipeline(Some(Pipeline::Running)) => Tone::Info,
            Self::Pipeline(Some(Pipeline::Pending)) => Tone::Muted,
            Self::Pipeline(_) => Tone::Bad,
            Self::Rebase | Self::Threads(_) | Self::Approvals(_) => Tone::Warn,
            Self::Draft | Self::Checking | Self::Blocked | Self::Other(_) => Tone::Muted,
        }
    }

    /// The author is the one to act: what makes their merge request `fix`.
    /// Approvals, a running pipeline and GitLab's checking wait on others.
    pub const fn on_author(&self) -> bool {
        match self {
            Self::Draft
            | Self::Conflicts
            | Self::Rebase
            | Self::Threads(_)
            | Self::ChangesRequested => true,
            Self::Pipeline(p) => !matches!(p, Some(Pipeline::Running | Pipeline::Pending)),
            Self::Approvals(_) | Self::Checking | Self::Blocked | Self::Other(_) => false,
        }
    }

    /// Its icon: the piece that is in the way.
    pub fn icon(&self) -> Icon {
        match self {
            Self::Draft => Icon::Draft,
            Self::Blocked => Icon::RunCanceled,
            Self::Conflicts => Icon::Conflict,
            Self::ChangesRequested => Icon::Error,
            Self::Rebase => Icon::Rebase,
            Self::Pipeline(Some(p)) => p.mark().icon(),
            Self::Pipeline(None) | Self::Checking => Icon::RunPending,
            Self::Threads(_) => Icon::Thread,
            Self::Approvals(_) => Icon::NeedsApproval,
            Self::Other(_) => Icon::Info,
        }
    }
}

/// A GitLab status or check identifier as a block, read with the merge
/// request's fields for the numbers; `None` for one that is not in the way.
fn block_of(id: &str, checking: bool, mr: &Mr) -> Option<Block> {
    let open = mr.threads.saturating_sub(mr.threads_resolved).max(1);
    Some(match id {
        "" | "MERGEABLE" | "NOT_OPEN" => return None,
        "DRAFT_STATUS" => Block::Draft,
        "CONFLICT" => Block::Conflicts,
        "NEED_REBASE" => Block::Rebase,
        "CI_MUST_PASS" | "CI_STILL_RUNNING" => match mr.pipeline {
            // Passed, yet GitLab is not done with it.
            Some(Pipeline::Passed) if checking || id == "CI_STILL_RUNNING" => Block::Checking,
            p => Block::Pipeline(p),
        },
        "DISCUSSIONS_NOT_RESOLVED" => Block::Threads(open),
        "NOT_APPROVED" => Block::Approvals(mr.approvals_left.max(1)),
        "REQUESTED_CHANGES" => Block::ChangesRequested,
        "MERGE_REQUEST_BLOCKED" | "BLOCKED_STATUS" => Block::Blocked,
        "CHECKING" | "UNCHECKED" | "PREPARING" | "APPROVALS_SYNCING" => Block::Checking,
        "COMMITS_STATUS" => Block::Other("commits refused by push rules".into()),
        "LOCKED_PATHS" => Block::Other("touches locked paths".into()),
        "LOCKED_LFS_FILES" => Block::Other("touches locked files".into()),
        "EXTERNAL_STATUS_CHECKS" | "STATUS_CHECKS_MUST_PASS" => {
            Block::Other("external checks must pass".into())
        }
        "JIRA_ASSOCIATION" | "JIRA_ASSOCIATION_MISSING" => {
            Block::Other("needs an issue key in its title".into())
        }
        "SECURITY_POLICY_VIOLATIONS" | "SECURITY_POLICY_PIPELINE_CHECK" => {
            Block::Other("security policy".into())
        }
        "MERGE_TIME" => Block::Other("waits for its merge time".into()),
        "TITLE_REGEX" => Block::Other("title breaks the repo's rule".into()),
        "BROKEN_STATUS" => Block::Other("cannot be merged".into()),
        other => Block::Other(other.to_ascii_lowercase().replace('_', " ")),
    })
}

/// Without GitLab's checks: from the fields, the way GitLab would weigh
/// them where the repo's settings are not known (a pipeline must pass).
fn from_fields(mr: &Mr) -> Vec<Check> {
    let mut ids = Vec::new();
    let mut add = |id: &str| ids.push(Check { id: id.into(), checking: false });
    if mr.draft {
        add("DRAFT_STATUS");
    }
    if mr.conflicts {
        add("CONFLICT");
    }
    if mr.rebase {
        add("NEED_REBASE");
    }
    if matches!(
        mr.pipeline,
        Some(Pipeline::Failed | Pipeline::Canceled | Pipeline::Running | Pipeline::Pending | Pipeline::Manual)
    ) {
        add("CI_MUST_PASS");
    }
    if !mr.approved && mr.approvals_left > 0 {
        add("NOT_APPROVED");
    }
    ids
}

/// Everything in the way of an open merge request, GitLab's lead first;
/// empty when it is ready to merge. Merged and closed ones have nothing.
pub fn blocks(mr: &Mr) -> Vec<Block> {
    if mr.state != MrState::Opened {
        return vec![];
    }
    let mut out: Vec<Block> = Vec::new();
    let mut push = |b: Option<Block>| {
        if let Some(b) = b
            && !out
                .iter()
                .any(|o| std::mem::discriminant(o) == std::mem::discriminant(&b))
        {
            out.push(b);
        }
    };
    // Nothing said yet (an old cache): not ready until GitLab says so.
    if mr.merge_status.is_empty() {
        push(Some(Block::Checking));
    }
    push(block_of(&mr.merge_status, false, mr));
    let fallback;
    let checks = match &mr.checks {
        Some(c) => c,
        None => {
            fallback = from_fields(mr);
            &fallback
        }
    };
    for c in checks {
        push(block_of(&c.id, c.checking, mr));
    }
    out
}

/// The STATE column's words: `ready to merge`, the first block, or how a
/// merged or closed one ended.
#[cfg(test)]
pub fn cell(mr: &Mr) -> (String, Tone) {
    match mr.state {
        MrState::Merged => ("merged".into(), Tone::Muted),
        MrState::Closed => ("closed".into(), Tone::Muted),
        MrState::Opened => match blocks(mr).first() {
            None => ("ready to merge".into(), Tone::Good),
            Some(b) => (b.words(), b.tone()),
        },
    }
}


/// Why it cannot be merged now, in words; `None` when it can.
pub fn first_words(mr: &Mr) -> Option<String> {
    blocks(mr).first().map(Block::words)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::mr;

    fn checks(ids: &[&str]) -> Option<Vec<Check>> {
        Some(
            ids.iter()
                .map(|i| Check {
                    id: (*i).into(),
                    checking: false,
                })
                .collect(),
        )
    }

    fn words(m: &Mr) -> Vec<String> {
        blocks(m).iter().map(Block::words).collect()
    }

    #[test]
    fn gitlabs_status_leads_and_its_checks_follow() {
        let mut m = mr(1, "alice", "acme/team-a/api");
        m.merge_status = "DRAFT_STATUS".into();
        m.draft = true;
        m.checks = checks(&["DRAFT_STATUS", "NOT_APPROVED", "NEED_REBASE"]);
        assert_eq!(words(&m), ["draft", "needs 1 approval", "needs rebase"]);
        assert_eq!(cell(&m).0, "draft");
    }

    #[test]
    fn passed_but_behind_its_target_needs_a_rebase_not_passed() {
        let mut m = mr(1, "alice", "acme/team-a/api");
        m.pipeline = Some(Pipeline::Passed);
        m.approved = true;
        m.approvals_left = 0;
        m.merge_status = "NEED_REBASE".into();
        m.rebase = true;
        m.checks = checks(&["NEED_REBASE"]);
        assert_eq!(cell(&m), ("needs rebase".into(), Tone::Warn));
        // Without GitLab's checks too: the fields say the same.
        m.checks = None;
        assert_eq!(words(&m), ["needs rebase"]);
    }

    #[test]
    fn ready_only_when_gitlab_says_mergeable_and_nothing_is_in_the_way() {
        let mut m = mr(1, "alice", "acme/team-a/api");
        m.merge_status = "MERGEABLE".into();
        m.approved = true;
        m.approvals_left = 0;
        m.checks = checks(&[]);
        assert_eq!(cell(&m), ("ready to merge".into(), Tone::Good));
        m.merge_status = "CHECKING".into();
        assert_eq!(cell(&m).0, "checking");
    }

    #[test]
    fn a_repo_that_needs_no_approvals_is_not_waiting_for_any() {
        let mut m = mr(1, "alice", "acme/team-a/api");
        m.merge_status = "MERGEABLE".into();
        m.approved = true;
        m.approvals_required = 0;
        m.approvals_left = 0;
        m.checks = None;
        assert!(blocks(&m).is_empty());
    }

    #[test]
    fn the_pipeline_counts_as_the_repo_says_and_in_its_own_words() {
        let mut m = mr(1, "alice", "acme/team-a/api");
        m.pipeline = Some(Pipeline::Failed);
        m.merge_status = "CI_MUST_PASS".into();
        m.checks = checks(&["CI_MUST_PASS", "NOT_APPROVED"]);
        assert_eq!(cell(&m), ("pipeline failed".into(), Tone::Bad));
        m.pipeline = Some(Pipeline::Running);
        m.merge_status = "CI_STILL_RUNNING".into();
        m.checks = Some(vec![Check {
            id: "CI_MUST_PASS".into(),
            checking: true,
        }]);
        assert_eq!(words(&m), ["pipeline running"]);
        // A repo that merges whatever the pipeline says: not in the way.
        m.pipeline = Some(Pipeline::Failed);
        m.merge_status = "NOT_APPROVED".into();
        m.checks = checks(&["NOT_APPROVED"]);
        assert_eq!(words(&m), ["needs 1 approval"]);
    }

    #[test]
    fn threads_that_must_be_resolved_are_counted() {
        let mut m = mr(1, "alice", "acme/team-a/api");
        m.threads = 3;
        m.threads_resolved = 1;
        m.merge_status = "DISCUSSIONS_NOT_RESOLVED".into();
        m.checks = checks(&["DISCUSSIONS_NOT_RESOLVED", "NOT_APPROVED"]);
        assert_eq!(words(&m), ["2 threads open","needs 1 approval"]);
    }

    #[test]
    fn an_unknown_check_is_said_in_gitlabs_words() {
        let mut m = mr(1, "alice", "acme/team-a/api");
        m.merge_status = "SOMETHING_NEW".into();
        m.checks = checks(&[]);
        assert_eq!(words(&m), ["something new"]);
    }

    #[test]
    fn who_has_to_act() {
        assert!(Block::Rebase.on_author());
        assert!(Block::Threads(1).on_author());
        assert!(Block::Pipeline(Some(Pipeline::Failed)).on_author());
        assert!(!Block::Pipeline(Some(Pipeline::Running)).on_author());
        assert!(!Block::Approvals(1).on_author());
    }
}
