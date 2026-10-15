//! Neutral stand-ins shared by shots and tests. No remote requests run here.

#[cfg(test)]
use crate::model::{CheckRun, Checks, CommitStatus};
use crate::{
    GithubPlugin, RepoState,
    config::Settings,
    model::{Collection, Completion, PullRequest, Repository, RepositoryId},
};

pub(crate) fn sample() -> GithubPlugin {
    let id = RepositoryId::parse("acme/api").unwrap();
    let second = RepositoryId::parse("acme/docs").unwrap();
    let mut p = GithubPlugin::default();
    p.apply_config(Settings {
        token: "env:GITHUB_TOKEN".into(),
        groups: vec![],
        repositories: vec![id.full_name(), second.full_name()],
        ..Settings::default()
    });
    p.started = true;
    p.now = radar_core::plugin::DEMO_NOW;
    p.last_sync = Some(p.now - 25);
    p.repos = vec![
        RepoState { id: id.clone(), repository: Some(Repository { id: id.clone(), description: Some("An example API service. Read the [contribution guide](https://github.com/acme/api/blob/main/CONTRIBUTING.md).".into()), default_branch: "main".into(), private: false, archived: false }), pulls: Some(Collection { items: vec![pull(&id, 42, "Keep background refreshes tied to their request"), pull(&id, 43, "Document the public endpoints")], completion: Completion::complete(1) }), error: None, loading: false, request: 0, sync: Default::default() },
        RepoState { id: second.clone(), repository: Some(Repository { id: second, description: Some("Documentation and examples.".into()), default_branch: "main".into(), private: false, archived: false }), pulls: Some(Collection { items: vec![], completion: Completion::complete(1) }), error: None, loading: false, request: 0, sync: Default::default() },
    ];
    crate::actions::seed(&mut p);
    p
}

pub(crate) fn pull(id: &RepositoryId, number: u64, title: &str) -> PullRequest {
    PullRequest { repository: id.clone(), number, title: title.into(), body: Some("## What changed\n\nResponses stay with the repository and commit they were requested for.\n\n- Keep the selected pull request stable\n- Show unavailable checks honestly\n\nSee the [design notes](https://github.com/acme/api/issues/12).".into()), author: "alice".into(), state: "open".into(), draft: number == 43, head_sha: "a".repeat(40), head_repository: Some(RepositoryId::parse("alice/api").unwrap()), head_branch: "refresh-ownership".into(), base_branch: "main".into(), updated_at: "2026-10-06T10:00:00Z".into(), merged: false }
}

#[cfg(test)]
pub(crate) fn checks(pull: &PullRequest) -> Checks {
    Checks {
        head_sha: pull.head_sha.clone(),
        head_repository: pull.head_repository.clone(),
        check_runs: Collection {
            items: vec![
                CheckRun {
                    repository: pull.repository.clone(),
                    id: 101,
                    name: "unit tests".into(),
                    status: "completed".into(),
                    conclusion: Some("success".into()),
                    head_sha: pull.head_sha.clone(),
                },
                CheckRun {
                    repository: pull
                        .head_repository
                        .clone()
                        .unwrap_or_else(|| pull.repository.clone()),
                    id: 102,
                    name: "platform checks".into(),
                    status: "in_progress".into(),
                    conclusion: None,
                    head_sha: pull.head_sha.clone(),
                },
            ],
            completion: Completion::complete(2),
        },
        statuses: Collection {
            items: vec![CommitStatus {
                repository: pull.repository.clone(),
                id: 201,
                context: "preview deployment".into(),
                state: "success".into(),
                description: Some("Preview is available.".into()),
            }],
            completion: Completion::complete(2),
        },
        errors: vec![],
    }
}

#[cfg(test)]
pub(crate) fn page() -> GithubPlugin {
    let mut p = sample();
    let pull = p.selected_pull().unwrap().clone();
    p.open_target(crate::join::Target::Pull(
        pull.repository.clone(),
        pull.number,
    ));
    p.page_loading = false;
    p.checks = Some(checks(&pull));
    p.pull_detail = Some(pull);
    p
}
