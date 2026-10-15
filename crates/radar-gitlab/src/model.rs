//! Merge requests and projects, and how an MR is classified from your side.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub username: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pipeline {
    Passed,
    Failed,
    Running,
    Pending,
    Manual,
    Canceled,
    Skipped,
}

impl Pipeline {
    /// The kit's mark for this state: `✓ ✗ ● ◷ ▷ ⊘ »`.
    pub const fn mark(self) -> radar_core::ui::Mark {
        use radar_core::ui::Mark;
        match self {
            Self::Passed => Mark::Done,
            Self::Failed => Mark::Failed,
            Self::Running => Mark::Running,
            Self::Pending => Mark::Waiting,
            Self::Manual => Mark::Manual,
            Self::Canceled => Mark::Stopped,
            Self::Skipped => Mark::Skipped,
        }
    }

    pub fn from_graphql(status: &str) -> Option<Self> {
        Some(match status {
            "SUCCESS" => Self::Passed,
            "FAILED" => Self::Failed,
            "RUNNING" => Self::Running,
            "PENDING"
            | "CREATED"
            | "WAITING_FOR_RESOURCE"
            | "PREPARING"
            | "SCHEDULED"
            | "WAITING_FOR_CALLBACK" => Self::Pending,
            "MANUAL" => Self::Manual,
            "CANCELED" | "CANCELING" => Self::Canceled,
            "SKIPPED" => Self::Skipped,
            _ => return None,
        })
    }

    pub const fn word(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Running => "running",
            Self::Pending => "pending",
            Self::Manual => "manual",
            Self::Canceled => "canceled",
            Self::Skipped => "skipped",
        }
    }
}

/// Where a merge request is in its life. Only open ones are fetched unless a
/// view asks for the others (`state:merged`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MrState {
    #[default]
    Opened,
    Merged,
    Closed,
}

impl MrState {
    pub const ALL: [Self; 3] = [Self::Opened, Self::Merged, Self::Closed];

    /// GitLab's `state`; a locked merge request is closed to us.
    pub fn from_graphql(state: &str) -> Self {
        match state {
            "merged" => Self::Merged,
            "closed" | "locked" => Self::Closed,
            _ => Self::Opened,
        }
    }

    /// The word a filter and the look use: `open`, `merged`, `closed`.
    pub const fn word(self) -> &'static str {
        match self {
            Self::Opened => "open",
            Self::Merged => "merged",
            Self::Closed => "closed",
        }
    }

    /// `open`, `opened`, `merged`, `closed`.
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "open" | "opened" => Some(Self::Opened),
            "merged" => Some(Self::Merged),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mr {
    pub iid: u64,
    pub title: String,
    pub draft: bool,
    pub web_url: String,
    /// RFC 3339, UTC.
    pub updated_at: String,
    pub source_branch: String,
    pub target_branch: String,
    /// The source branch's head commit (`diffHeadSha`); empty when GitLab has
    /// none yet. A review is of this sha; old caches have none.
    #[serde(default)]
    pub head_sha: String,
    pub notes: u32,
    pub conflicts: bool,
    /// GitLab's `detailedMergeStatus`, e.g. `MERGEABLE`, `NOT_APPROVED`, `NEED_REBASE`.
    pub merge_status: String,
    /// GitLab's `shouldBeRebased`: the target moved on and the project
    /// merges only on top of it.
    #[serde(default)]
    pub rebase: bool,
    /// GitLab's mergeability checks that failed or are still checking, in
    /// its order; `None` when the server does not list them.
    #[serde(default)]
    pub checks: Option<Vec<Check>>,
    pub approved: bool,
    pub approvals_required: u32,
    pub approvals_left: u32,
    pub author: Person,
    pub reviewers: Vec<String>,
    pub approved_by: Vec<String>,
    pub pipeline: Option<Pipeline>,
    pub pipeline_url: Option<String>,
    pub additions: u32,
    pub deletions: u32,
    pub files: u32,
    /// The head pipeline's stages in order, with GitLab's status word.
    #[serde(default)]
    pub stages: Vec<Stage>,
    #[serde(default)]
    pub duration_secs: Option<u32>,
    /// Resolvable threads, and how many of them are resolved.
    #[serde(default)]
    pub threads: u32,
    #[serde(default)]
    pub threads_resolved: u32,
    #[serde(default)]
    pub last_comment: Option<Comment>,
    pub project_path: String,
    pub project_name: String,
    #[serde(default)]
    pub state: MrState,
    /// When it was merged or closed, RFC 3339; empty while open.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ended_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

/// One of GitLab's mergeability checks that is not passing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// GitLab's identifier: `NOT_APPROVED`, `CI_MUST_PASS`, `NEED_REBASE`, …
    pub id: String,
    /// Still being worked out (a running pipeline), rather than failed.
    #[serde(default)]
    pub checking: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub name: String,
    /// GitLab's word: `success`, `failed`, `running`, `manual`, `skipped`, …
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub author: String,
    pub body: String,
    pub at: String,
}

/// The newest commit on a repo's default branch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MainCommit {
    /// The full id: what the reference compares ([`Project::pin`]). Empty in
    /// a cache saved before it was kept.
    #[serde(default)]
    pub sha: String,
    pub short_id: String,
    pub author: String,
    /// RFC 3339.
    pub at: String,
    pub title: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub full_path: String,
    #[serde(default)]
    pub default_branch: String,
    /// The latest pipeline on the default branch, if one is among the recent.
    #[serde(default)]
    pub main_pipeline: Option<Pipeline>,
    #[serde(default)]
    pub main_pipeline_url: Option<String>,
    /// That pipeline's number, and when it last changed (its `updatedAt`,
    /// RFC 3339): with the head commit's full id they are the reference a
    /// sync compares to see whether main moved, without fetching the project
    /// in full ([`Project::pin`]). Empty in a cache saved before they were
    /// kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_pipeline_id: Option<u64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub main_pipeline_at: String,
    /// RFC 3339.
    #[serde(default)]
    pub last_activity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_commit: Option<MainCommit>,
}

/// What tells that a project's default branch has not moved: the branch, its
/// head commit, and its newest pipeline (number, state, last change). Held
/// as the reference ([`Project::pin`]) and asked again of GitLab by a cheap
/// query, so a project is fetched in full only when they differ.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MainRef {
    pub branch: String,
    pub head_sha: String,
    pub pipeline_id: Option<u64>,
    pub pipeline: Option<Pipeline>,
    pub pipeline_at: String,
}

impl Project {
    /// The reference to compare with, when it is complete: a branch, a head
    /// commit, and a pipeline on it with its number and state (one still
    /// running is compared too: its end changes the state). A cache saved
    /// before these were kept, or no pipeline among the recent ones (it may
    /// be older than the ten GitLab was asked for, and a pin asks for the
    /// branch's own) has none: the project is fetched in full.
    pub fn pin(&self) -> Option<MainRef> {
        let head_sha = self.main_commit.as_ref().map(|c| c.sha.clone())?;
        let pipeline = self.main_pipeline?;
        let id = self.main_pipeline_id?;
        if self.default_branch.is_empty() || head_sha.is_empty() {
            return None;
        }
        Some(MainRef {
            branch: self.default_branch.clone(),
            head_sha,
            pipeline_id: Some(id),
            pipeline: Some(pipeline),
            pipeline_at: self.main_pipeline_at.clone(),
        })
    }
}

/// What a merge request's page shows beyond the list query: fetched when the
/// page opens.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MrDetail {
    /// Markdown, as written.
    pub description: String,
    pub files: Vec<FileStat>,
    /// Discussions with at least one comment, system notes left out.
    pub discussions: Vec<Discussion>,
    pub pipeline: Option<PipelineDetail>,
    /// Newest first, as GitLab lists them.
    pub commits: Vec<CommitInfo>,
    /// The commits the diff is between: what a comment on a line names.
    pub refs: Option<DiffRefs>,
    /// What a merge would do, as the merge request and its repo are set.
    pub merge: MergeSettings,
}

/// The commits a merge request's diff is between.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffRefs {
    pub base: String,
    pub head: String,
    pub start: String,
}

/// What GitLab will do when it merges: squash the commits, delete the
/// source branch (the merge request's own choice, or the repo forcing it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergeSettings {
    pub squash: bool,
    pub remove_source: bool,
    /// The repo deletes it whatever the merge request says.
    pub remove_forced: bool,
}

/// One file's changes, as GitLab's diffs API sends them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub old_path: String,
    pub new_path: String,
    pub new_file: bool,
    pub renamed: bool,
    pub deleted: bool,
    /// Unified hunks; empty when GitLab left them out.
    pub diff: String,
    /// GitLab left the hunks out: too large, or collapsed.
    pub too_large: bool,
}

impl FileDiff {
    /// `new file`, `deleted`, `renamed from …`, or nothing.
    pub fn how(&self) -> Option<String> {
        if self.new_file {
            Some("new file".into())
        } else if self.deleted {
            Some("deleted".into())
        } else if self.renamed {
            Some(format!("renamed from {}", self.old_path))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileStat {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Discussion {
    /// GitLab's id for it (`gid://gitlab/Discussion/<sha>`): what a reply
    /// or a resolve names.
    pub id: String,
    /// A thread that can be resolved (a review comment), not a plain comment.
    pub resolvable: bool,
    pub resolved: bool,
    /// The first note opens it; the rest are replies.
    pub notes: Vec<Note>,
}

impl Discussion {
    /// `path:line` when it is on a line of the diff.
    pub fn place(&self) -> Option<String> {
        let n = self.notes.first()?;
        let path = n.path.as_deref()?;
        let file = path.rsplit('/').next().unwrap_or(path);
        Some(match n.line {
            Some(l) => format!("{file}:{l}"),
            None => file.to_string(),
        })
    }

    pub fn is_open(&self) -> bool {
        self.resolvable && !self.resolved
    }

    /// Where on a diff it is, from its first note: (path, old line, new line).
    pub fn on_line(&self) -> Option<(&str, Option<u32>, Option<u32>)> {
        let n = self.notes.first()?;
        Some((n.path.as_deref()?, n.old_line, n.new_line))
    }

    /// The id the REST API names it by: the last part of [`Self::id`].
    pub fn rest_id(&self) -> &str {
        self.id.rsplit('/').next().unwrap_or(&self.id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    /// Display name, or the username when there is none.
    pub author: String,
    pub body: String,
    /// RFC 3339.
    pub at: String,
    pub path: Option<String>,
    pub line: Option<u32>,
    /// On a diff: its line in the old file and in the new one.
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

/// A pipeline with its jobs, stage by stage.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineDetail {
    pub iid: String,
    pub stages: Vec<StageJobs>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StageJobs {
    pub name: String,
    /// GitLab's word, lower case: `success`, `failed`, `manual`, …
    pub status: String,
    pub jobs: Vec<Job>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    /// From its link: `…/-/jobs/<id>`.
    pub id: Option<u64>,
    pub name: String,
    /// Lower case, as stages have it.
    pub status: String,
    pub duration_secs: Option<u32>,
    pub url: Option<String>,
    pub allow_failure: bool,
}

/// A job as GitLab's jobs API has it: what a failed one's diagnosis and a
/// job's page show.
#[derive(Debug, Clone, PartialEq)]
pub struct JobInfo {
    pub id: u64,
    pub project: String,
    pub name: String,
    pub stage: String,
    /// Lower case: `failed`, `success`, `running`, …
    pub status: String,
    pub duration_secs: Option<u32>,
    /// GitLab's word (`script_failure`, `job_execution_timeout`, …); empty
    /// when it did not fail.
    pub failure_reason: String,
    pub url: String,
    pub allow_failure: bool,
    /// The pipeline's id (not its iid).
    pub pipeline: u64,
}

/// A part of a job's log: its text and where it sits in the whole, in bytes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TracePart {
    pub text: String,
    pub start: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommitInfo {
    pub short_id: String,
    pub title: String,
    pub author: String,
    /// RFC 3339.
    pub at: String,
    pub url: String,
}

/// One pipeline run on a branch, for a repo's page.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineRun {
    pub iid: String,
    pub status: Option<Pipeline>,
    /// RFC 3339.
    pub created_at: String,
    pub duration_secs: Option<u32>,
    pub url: Option<String>,
    /// What started it: `push`, `schedule`, `web`, …
    pub source: String,
    pub user: String,
    pub commit_sha: String,
    pub commit_title: String,
}

/// What an MR needs from you. One word, one colour, everywhere it appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    Review,
    Fix,
    Ready,
    Waiting,
    Draft,
    Team,
    Bot,
}

impl Role {
    pub const ALL: [Self; 7] = [
        Self::Review,
        Self::Fix,
        Self::Ready,
        Self::Waiting,
        Self::Draft,
        Self::Team,
        Self::Bot,
    ];

    pub const fn word(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Fix => "fix",
            Self::Ready => "ready",
            Self::Waiting => "waiting",
            Self::Draft => "draft",
            Self::Team => "team",
            Self::Bot => "bot",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.word() == word)
    }

    /// The sentence the detail panel shows for this state.
    pub const fn sentence(self) -> &'static str {
        match self {
            Self::Review => "waiting for your review",
            Self::Fix => "yours, and it needs fixing",
            Self::Ready => "yours, and ready to merge",
            Self::Waiting => "yours, waiting on reviewers",
            Self::Draft => "yours, still a draft",
            Self::Team => "a teammate's",
            Self::Bot => "opened by a bot",
        }
    }
}

pub fn is_bot(username: &str) -> bool {
    let u = username.to_ascii_lowercase();
    (u.starts_with("group_") || u.starts_with("project_")) && u.contains("_bot_")
        || u.contains("renovate")
        || u.ends_with("-bot")
        || u.ends_with("_bot")
}

impl Mr {
    pub fn role(&self, me: &str) -> Role {
        if is_bot(&self.author.username) {
            return Role::Bot;
        }
        if self.author.username != me {
            // The team asks for review on Slack and rarely assigns reviewers, so
            // an open, unassigned MR that still needs approvals is everyone's to
            // review. An assigned one is only its reviewers'.
            let approved = self.approved_by.iter().any(|r| r == me);
            let asked = if self.reviewers.is_empty() {
                !self.draft && self.approvals_left > 0
            } else {
                self.reviewers.iter().any(|r| r == me)
            };
            return if asked && !approved {
                Role::Review
            } else {
                Role::Team
            };
        }
        if self.draft {
            return Role::Draft;
        }
        // As its STATE says: ready when nothing is in the way, fix when
        // something in the way is yours to do (a rebase, conflicts, a red
        // pipeline, open threads), else waiting on others.
        let blocks = crate::ready::blocks(self);
        if blocks.is_empty() {
            Role::Ready
        } else if blocks.iter().any(crate::ready::Block::on_author) {
            Role::Fix
        } else {
            Role::Waiting
        }
    }

    /// GitLab's merge status in words, or `None` when it is simply mergeable.
    pub fn merge_words(&self) -> String {
        match self.merge_status.as_str() {
            "MERGEABLE" => "can merge".into(),
            "NOT_APPROVED" => "needs approval".into(),
            "NEED_REBASE" => "needs rebase".into(),
            "CONFLICT" => "has conflicts".into(),
            "CI_MUST_PASS" => "pipeline must pass".into(),
            "CI_STILL_RUNNING" => "pipeline still running".into(),
            "DISCUSSIONS_NOT_RESOLVED" => "open threads".into(),
            "DRAFT_STATUS" => "draft".into(),
            "CHECKING" | "UNCHECKED" | "PREPARING" => "checking".into(),
            "BLOCKED_STATUS" => "blocked by another MR".into(),
            "" => "—".into(),
            other => other.to_ascii_lowercase().replace('_', " "),
        }
    }

    pub fn approvals_words(&self) -> String {
        let given = self.approved_by.len() as u32;
        let required = self.approvals_required.max(given + self.approvals_left);
        if required == 0 {
            return if given == 0 {
                "none needed".into()
            } else {
                format!("{given} given")
            };
        }
        format!("{given} of {required}")
    }
}

#[cfg(any(test, feature = "demo"))]
pub mod fixtures {
    use super::*;

    pub fn mr(iid: u64, author: &str, project: &str) -> Mr {
        Mr {
            iid,
            title: format!("change {iid}"),
            draft: false,
            web_url: format!("https://gitlab.com/{project}/-/merge_requests/{iid}"),
            updated_at: "2026-10-06T07:00:00Z".into(),
            source_branch: format!("feature/ABC-{iid}"),
            target_branch: "main".into(),
            head_sha: String::new(),
            notes: 0,
            conflicts: false,
            merge_status: "NOT_APPROVED".into(),
            rebase: false,
            // From the fields below, so a test can change one of them.
            checks: None,
            approved: false,
            approvals_required: 1,
            approvals_left: 1,
            author: Person {
                username: author.into(),
                name: author.into(),
            },
            reviewers: vec![],
            approved_by: vec![],
            pipeline: Some(Pipeline::Passed),
            pipeline_url: None,
            additions: 10,
            deletions: 2,
            files: 1,
            stages: vec![
                Stage {
                    name: "build".into(),
                    status: "success".into(),
                },
                Stage {
                    name: "test".into(),
                    status: "success".into(),
                },
                Stage {
                    name: "deploy".into(),
                    status: "manual".into(),
                },
            ],
            duration_secs: Some(372),
            threads: 2,
            threads_resolved: 1,
            last_comment: Some(Comment {
                author: "Bob Example".into(),
                body: "Should this read from the cached view? The dashboard polls every 30s."
                    .into(),
                at: "2026-10-06T06:00:00Z".into(),
            }),
            project_name: project.rsplit('/').next().unwrap().into(),
            project_path: project.into(),
            state: MrState::Opened,
            ended_at: String::new(),
            labels: vec![],
        }
    }

    /// What a merge request's page fetches, for the fixture above.
    #[cfg(any(test, feature = "demo"))]
    pub fn detail() -> MrDetail {
        let note = |author: &str, body: &str, at: &str, line: Option<u32>| Note {
            author: author.into(),
            body: body.into(),
            at: at.into(),
            path: line.map(|_| "src/main/java/acme/stats/StatsController.java".into()),
            line,
            old_line: None,
            new_line: line,
        };
        MrDetail {
            description: "Closes ABC-12\n\n## What\n\n- adds `GET /stats`\n- [x] tests\n\n<!-- template -->\nSee [the ticket](https://example.com/ABC-12)."
                .into(),
            files: vec![
                FileStat {
                    path: "src/main/java/acme/stats/StatsController.java".into(),
                    additions: 42,
                    deletions: 0,
                },
                FileStat {
                    path: "src/test/java/acme/stats/StatsControllerTest.java".into(),
                    additions: 33,
                    deletions: 3,
                },
                FileStat {
                    path: "src/main/java/acme/stats/StatsService.java".into(),
                    additions: 18,
                    deletions: 2,
                },
                FileStat {
                    path: "docs/README.md".into(),
                    additions: 6,
                    deletions: 1,
                },
                FileStat {
                    path: "build.gradle".into(),
                    additions: 3,
                    deletions: 1,
                },
            ],
            discussions: vec![
                Discussion {
                    id: "gid://gitlab/Discussion/6a9c1750b37d513a43987b574953fceb50b03ce7".into(),
                    resolvable: true,
                    resolved: false,
                    notes: vec![
                        note(
                            "Bob Example",
                            "Should this read from the cached view?",
                            "2026-10-06T06:00:00Z",
                            Some(21),
                        ),
                        note(
                            "Alice Example",
                            "Good shout, switching to the cached one.",
                            "2026-10-06T06:30:00Z",
                            Some(21),
                        ),
                    ],
                },
                Discussion {
                    id: "gid://gitlab/Discussion/0b1e2c3d4e5f60718293a4b5c6d7e8f901234567".into(),
                    resolvable: true,
                    resolved: true,
                    notes: vec![note(
                        "Bob Example",
                        "Typo in the name.",
                        "2026-10-06T05:00:00Z",
                        Some(9),
                    )],
                },
            ],
            pipeline: Some(PipelineDetail {
                iid: "1843".into(),
                stages: vec![
                    StageJobs {
                        name: "build".into(),
                        status: "success".into(),
                        jobs: vec![Job {
                            id: Some(1001),
                            name: "build-artifacts".into(),
                            status: "success".into(),
                            duration_secs: Some(123),
                            url: Some("https://gitlab.com/acme/team-a/api/-/jobs/1001".into()),
                            allow_failure: false,
                        }],
                    },
                    StageJobs {
                        name: "test".into(),
                        status: "failed".into(),
                        jobs: vec![
                            Job {
                                id: Some(1002),
                                name: "unit-test".into(),
                                status: "success".into(),
                                duration_secs: Some(222),
                                url: Some("https://gitlab.com/acme/team-a/api/-/jobs/1002".into()),
                                allow_failure: false,
                            },
                            Job {
                                id: Some(1003),
                                name: "lint".into(),
                                status: "failed".into(),
                                duration_secs: Some(31),
                                url: Some("https://gitlab.com/acme/team-a/api/-/jobs/1003".into()),
                                allow_failure: true,
                            },
                        ],
                    },
                ],
            }),
            commits: vec![
                CommitInfo {
                    short_id: "9612f87c".into(),
                    title: "Resolve merge conflicts".into(),
                    author: "Alice Example".into(),
                    at: "2026-10-06T06:40:00Z".into(),
                    url: "https://gitlab.com/acme/team-a/api/-/commit/9612f87c".into(),
                },
                CommitInfo {
                    short_id: "6be09cb6".into(),
                    title: "ABC-12: Add a stats endpoint".into(),
                    author: "Alice Example".into(),
                    at: "2026-10-05T14:39:00Z".into(),
                    url: "https://gitlab.com/acme/team-a/api/-/commit/6be09cb6".into(),
                },
            ],
            refs: Some(DiffRefs {
                base: "1111111111111111111111111111111111111111".into(),
                head: "9612f87c00000000000000000000000000000000".into(),
                start: "2222222222222222222222222222222222222222".into(),
            }),
            merge: MergeSettings {
                squash: true,
                remove_source: true,
                remove_forced: false,
            },
        }
    }

    /// The fixture's files as GitLab's diffs API sends them: the controller
    /// is new; the test has a hunk that changes three lines.
    #[cfg(any(test, feature = "demo"))]
    pub fn diffs() -> Vec<FileDiff> {
        let mut controller = String::from("@@ -0,0 +1,24 @@\n");
        for l in [
            "package acme.stats;",
            "",
            "import java.time.LocalDate;",
            "import java.util.UUID;",
            "",
            "@RestController",
            "@RequestMapping(\"/api/stats\")",
            "@RequiredArgsConstructor",
            "public class StatsControler {",
            "",
            "    private final StatsService statsService;",
            "",
            "    @GetMapping(\"/user/{userId}\")",
            "    public UserStats userStats(",
            "            @PathVariable UUID userId,",
            "            @RequestParam LocalDate from,",
            "            @RequestParam LocalDate to) {",
            "        return statsService.forUser(userId, from, to);",
            "    }",
            "",
            "    public Totals totals() { return statsService.totals(); }",
            "",
            "    // more to come",
            "}",
        ] {
            controller.push('+');
            controller.push_str(l);
            controller.push('\n');
        }
        vec![
            FileDiff {
                old_path: "src/main/java/acme/stats/StatsController.java".into(),
                new_path: "src/main/java/acme/stats/StatsController.java".into(),
                new_file: true,
                diff: controller,
                ..FileDiff::default()
            },
            FileDiff {
                old_path: "src/test/java/acme/stats/StatsControllerTest.java".into(),
                new_path: "src/test/java/acme/stats/StatsControllerTest.java".into(),
                diff: "@@ -10,8 +10,9 @@ class StatsControllerTest {\n     @Test\n     void totals() {\n-        assertEquals(0, totals());\n-        // TODO\n-        fail();\n+        var t = controller.totals();\n+        assertEquals(0, t.count());\n+        assertEquals(0, t.sum());\n+        assertNotNull(t);\n     }\n \n }\n"
                    .into(),
                ..FileDiff::default()
            },
            FileDiff {
                old_path: "src/main/java/acme/stats/StatsService.java".into(),
                new_path: "src/main/java/acme/stats/StatsService.java".into(),
                diff: "@@ -4,6 +4,8 @@ public class StatsService {\n     private final Repo repo;\n \n-    public Totals totals() { return null; }\n+    public Totals totals() {\n+        return repo.totals();\n+    }\n \n }\n".into(),
                ..FileDiff::default()
            },
            FileDiff {
                old_path: "docs/README.md".into(),
                new_path: "docs/README.md".into(),
                diff: "@@ -1,3 +1,8 @@\n # Stats\n-Numbers.\n+Numbers per user and per day.\n+\n+## Endpoints\n+\n+- `GET /api/stats`\n+- `GET /api/stats/totals`\n".into(),
                ..FileDiff::default()
            },
            FileDiff {
                old_path: "build.gradle".into(),
                new_path: "build.gradle".into(),
                diff: "@@ -8,3 +8,5 @@ dependencies {\n-    implementation 'acme:core:1.2'\n+    implementation 'acme:core:1.3'\n+    implementation 'acme:cache:2.0'\n+    testImplementation 'acme:test:1.0'\n }\n".into(),
                ..FileDiff::default()
            },
        ]
    }

    #[cfg(test)]
    pub fn runs() -> Vec<PipelineRun> {
        vec![
            PipelineRun {
                iid: "1842".into(),
                status: Some(Pipeline::Failed),
                created_at: "2026-10-06T05:09:58Z".into(),
                duration_secs: Some(748),
                url: Some("https://gitlab.com/acme/team-a/api/-/pipelines/2".into()),
                source: "push".into(),
                user: "Alice Example".into(),
                commit_sha: "2c2ea8c3".into(),
                commit_title: "feat: bulk import".into(),
            },
            PipelineRun {
                iid: "1838".into(),
                status: Some(Pipeline::Passed),
                created_at: "2026-10-06T03:36:23Z".into(),
                duration_secs: Some(82),
                url: Some("https://gitlab.com/acme/team-a/api/-/pipelines/1".into()),
                source: "schedule".into(),
                user: "Bob Example".into(),
                commit_sha: "0110f094".into(),
                commit_title: "chore: update dependencies".into(),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::mr;
    use super::*;

    const P: &str = "acme/team-a/api";

    #[test]
    fn someone_elses_mr_that_asks_me_is_review_until_i_approve() {
        let mut m = mr(1, "alice", P);
        m.reviewers = vec!["me".into()];
        assert_eq!(m.role("me"), Role::Review);
        m.approved_by = vec!["me".into()];
        assert_eq!(m.role("me"), Role::Team);
    }

    #[test]
    fn an_unassigned_mr_needing_approval_is_open_for_review() {
        let mut m = mr(1, "alice", P);
        assert_eq!(m.role("me"), Role::Review);
        m.reviewers = vec!["bob".into()];
        assert_eq!(m.role("me"), Role::Team, "assigned to someone else");
        m.reviewers.clear();
        m.approvals_left = 0;
        assert_eq!(m.role("me"), Role::Team, "already has its approvals");
        m.approvals_left = 1;
        m.draft = true;
        assert_eq!(m.role("me"), Role::Team, "drafts are not up for review");
    }

    #[test]
    fn my_mr_moves_through_draft_waiting_fix_ready() {
        let mut m = mr(2, "me", P);
        m.draft = true;
        assert_eq!(m.role("me"), Role::Draft);
        m.draft = false;
        assert_eq!(m.role("me"), Role::Waiting);
        m.pipeline = Some(Pipeline::Failed);
        assert_eq!(m.role("me"), Role::Fix);
        m.pipeline = Some(Pipeline::Passed);
        m.approved = true;
        m.merge_status = "MERGEABLE".into();
        assert_eq!(m.role("me"), Role::Ready);
        m.conflicts = true;
        assert_eq!(m.role("me"), Role::Fix);
        // Passed and approved, but behind main: rebasing it is mine to do.
        m.conflicts = false;
        m.merge_status = "NEED_REBASE".into();
        m.rebase = true;
        assert_eq!(m.role("me"), Role::Fix);
        // Open threads that must be resolved are mine too; approvals are not.
        m.rebase = false;
        m.merge_status = "DISCUSSIONS_NOT_RESOLVED".into();
        assert_eq!(m.role("me"), Role::Fix);
        m.merge_status = "NOT_APPROVED".into();
        m.approved = false;
        assert_eq!(m.role("me"), Role::Waiting);
    }

    #[test]
    fn bots_are_recognised_whoever_they_ask() {
        let mut m = mr(3, "group_127362209_bot_72dadc5b48d427c769007f2b372c72f1", P);
        m.reviewers = vec!["me".into()];
        assert_eq!(m.role("me"), Role::Bot);
        assert!(is_bot("renovate-bot"));
        assert!(!is_bot("alice.example"));
    }

    #[test]
    fn approvals_read_as_given_of_required() {
        let mut m = mr(4, "me", P);
        assert_eq!(m.approvals_words(), "0 of 1");
        m.approved_by = vec!["alice".into()];
        m.approvals_left = 0;
        assert_eq!(m.approvals_words(), "1 of 1");
        m.approvals_required = 0;
        m.approved_by.clear();
        assert_eq!(m.approvals_words(), "none needed");
    }
}
