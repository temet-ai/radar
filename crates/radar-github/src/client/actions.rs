//! Read-only Actions endpoints, pinned to repository/run/attempt identities.
//!
//! https://docs.github.com/en/rest/actions/workflow-runs
//! https://docs.github.com/en/rest/actions/workflow-jobs
//! The general jobs endpoint defaults to latest and can race a rerun. We use
//! only the explicit attempt endpoint after validating its run detail.

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RunProof {
    repository: RepositoryId,
    id: u64,
    attempt: u64,
    head_sha: String,
}

impl From<&WorkflowRun> for RunProof {
    fn from(run: &WorkflowRun) -> Self {
        Self {
            repository: run.repository.clone(),
            id: run.id,
            attempt: run.run_attempt,
            head_sha: run.head_sha.clone(),
        }
    }
}

impl Client {
    /// The newest runs, as many as the page asks for (one page, never more),
    /// of one branch when the ask names it. Asked with the validator of the
    /// same ask's last read (`known`): a 304 is `Same`, with nothing parsed
    /// and nothing counted against the rate limit. Completion is about this
    /// ask only; it is not complete PR or branch-health proof.
    pub async fn workflow_runs_if(
        &self,
        repository: &RepositoryId,
        ask: &RunsAsk,
        known: &[Validator],
    ) -> Result<Fresh<Collection<WorkflowRun>>, ApiError> {
        let repository = self.allowed(repository)?;
        if ask.per_page == 0 || ask.per_page > PER_PAGE {
            return Err(error(ApiErrorKind::InvalidTarget));
        }
        let query = match &ask.branch {
            Some(branch) if branch.is_empty() || branch.len() > 255 => {
                return Err(error(ApiErrorKind::InvalidTarget));
            }
            Some(branch) if branch.chars().any(char::is_control) => {
                return Err(error(ApiErrorKind::InvalidTarget));
            }
            Some(branch) => url::form_urlencoded::Serializer::new(String::new())
                .append_pair("branch", branch)
                .finish(),
            None => String::new(),
        };
        let path = format!("/repos/{}/actions/runs", repository.full_name());
        let paging = Paging {
            per_page: ask.per_page,
            max_pages: 1,
            bounded: true,
        };
        self.collect_paged(
            &path,
            &query,
            Some("workflow_runs"),
            known,
            paging,
            |raw: RawWorkflowRun| {
                let run = raw.checked(&self.api, &repository, None, None)?;
                Ok((run.id, run))
            },
        )
        .await
    }

    /// Resolve a run's latest observed attempt, without ever using an API URL
    /// supplied in a response as the destination.
    pub async fn workflow_run(
        &self,
        repository: &RepositoryId,
        run_id: u64,
    ) -> Result<WorkflowRun, ApiError> {
        self.read_workflow_run(repository, run_id, None).await
    }

    pub async fn workflow_run_attempt(
        &self,
        repository: &RepositoryId,
        run_id: u64,
        attempt: u64,
    ) -> Result<WorkflowRun, ApiError> {
        self.read_workflow_run(repository, run_id, Some(attempt))
            .await
    }

    async fn read_workflow_run(
        &self,
        repository: &RepositoryId,
        run_id: u64,
        attempt: Option<u64>,
    ) -> Result<WorkflowRun, ApiError> {
        // A failed or refused refresh must not leave the older proof usable.
        *self
            .verified_run
            .lock()
            .map_err(|_| error(ApiErrorKind::Unavailable))? = None;
        let repository = self.allowed(repository)?;
        if run_id == 0 || attempt == Some(0) {
            return Err(error(ApiErrorKind::InvalidTarget));
        }
        let mut path = format!("/repos/{}/actions/runs/{run_id}", repository.full_name());
        if let Some(attempt) = attempt {
            path.push_str(&format!("/attempts/{attempt}"));
        }
        let (raw, _) = self.read::<RawWorkflowRun>(&path, "").await?;
        let run = raw.checked(&self.api, &repository, Some(run_id), attempt)?;
        *self
            .verified_run
            .lock()
            .map_err(|_| error(ApiErrorKind::Unavailable))? = Some(RunProof::from(&run));
        Ok(run)
    }

    /// The exact attempt shown on the page, never a mutable latest-jobs alias.
    /// A caller-constructed or cached run cannot authorize a jobs read.
    pub async fn workflow_jobs(
        &self,
        run: &WorkflowRun,
    ) -> Result<Collection<WorkflowJob>, ApiError> {
        self.allowed(&run.repository)?;
        let proof = RunProof::from(run);
        if self
            .verified_run
            .lock()
            .map_err(|_| error(ApiErrorKind::Unavailable))?
            .as_ref()
            != Some(&proof)
        {
            return Err(error(ApiErrorKind::InvalidTarget));
        }
        let path = format!(
            "/repos/{}/actions/runs/{}/attempts/{}/jobs",
            proof.repository.full_name(),
            proof.id,
            proof.attempt
        );
        self.collect(&path, "", Some("jobs"), |raw: RawWorkflowJob| {
            let job = raw.checked(&self.api, &proof)?;
            Ok((job.id, job))
        })
        .await
    }
}

#[derive(Deserialize)]
struct RawWorkflowRun {
    id: u64,
    workflow_id: u64,
    run_number: u64,
    run_attempt: u64,
    name: Option<String>,
    display_title: Option<String>,
    path: String,
    event: String,
    status: Option<String>,
    conclusion: Option<String>,
    head_sha: String,
    head_branch: Option<String>,
    repository: RawRepository,
    head_repository: Option<RawRepository>,
    url: String,
    html_url: String,
    created_at: String,
    updated_at: String,
    run_started_at: Option<String>,
    pull_requests: Option<Vec<RawRunPull>>,
}

impl RawWorkflowRun {
    fn checked(
        self,
        api: &Endpoint,
        repository: &RepositoryId,
        run_id: Option<u64>,
        attempt: Option<u64>,
    ) -> Result<WorkflowRun, ApiError> {
        let bad = || error(ApiErrorKind::InvalidResponse);
        if self.id == 0
            || self.workflow_id == 0
            || self.run_number == 0
            || self.run_attempt == 0
            || run_id.is_some_and(|id| id != self.id)
            || attempt.is_some_and(|attempt| attempt != self.run_attempt)
            || !valid_sha(&self.head_sha)
            || self.repository.identity()? != *repository
            || !valid_run_url(Some(api), &self.url, repository, self.id, self.run_attempt)
            || !valid_run_url(None, &self.html_url, repository, self.id, self.run_attempt)
        {
            return Err(bad());
        }
        let head_repository = self
            .head_repository
            .as_ref()
            .map(RawRepository::identity)
            .transpose()?;
        let pull_requests = self
            .pull_requests
            .unwrap_or_default()
            .into_iter()
            .map(|pull| pull.checked(api))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(WorkflowRun {
            repository: repository.clone(),
            id: self.id,
            workflow_id: self.workflow_id,
            run_number: self.run_number,
            run_attempt: self.run_attempt,
            name: self.name,
            display_title: self.display_title.unwrap_or_default(),
            path: self.path,
            event: self.event,
            status: self.status.unwrap_or_default(),
            conclusion: self.conclusion,
            head_sha: self.head_sha.to_ascii_lowercase(),
            head_branch: self.head_branch,
            head_repository,
            created_at: self.created_at,
            updated_at: self.updated_at,
            run_started_at: self.run_started_at,
            pull_requests,
        })
    }
}

/// A run's own address: under the configured API with `api`, or on the
/// GitHub web site without it.
fn valid_run_url(
    api: Option<&Endpoint>,
    value: &str,
    repository: &RepositoryId,
    run: u64,
    attempt: u64,
) -> bool {
    let prefix = api.map_or_else(
        || "https://github.com/".to_owned(),
        |api| format!("{}/repos/", api.rest()),
    );
    let Some((repo, tail)) = value
        .strip_prefix(&prefix)
        .and_then(|path| path.split_once("/actions/runs/"))
    else {
        return false;
    };
    RepositoryId::parse(repo).ok().as_ref() == Some(repository)
        && (tail == run.to_string() || tail == format!("{run}/attempts/{attempt}"))
}

#[derive(Deserialize)]
struct RawRunPull {
    number: u64,
    url: String,
    head: RawRunPullBranch,
    base: RawRunPullBranch,
}

#[derive(Deserialize)]
struct RawRunPullBranch {
    sha: String,
    repo: Option<RawRunPullRepository>,
}

/// Actions PR associations use this abbreviated shape, not RawRepository.
#[derive(Deserialize)]
struct RawRunPullRepository {
    id: u64,
    name: String,
    url: String,
}

impl RawRunPullRepository {
    fn identity(&self, api: &Endpoint) -> Result<RepositoryId, ApiError> {
        let bad = || error(ApiErrorKind::InvalidResponse);
        let repository = self
            .url
            .strip_prefix(api.rest())
            .and_then(|rest| rest.strip_prefix("/repos/"))
            .and_then(|name| RepositoryId::parse(name).ok())
            .ok_or_else(bad)?;
        if self.id == 0 || !self.name.eq_ignore_ascii_case(&repository.name) {
            return Err(bad());
        }
        Ok(repository)
    }
}

impl RawRunPull {
    fn checked(self, api: &Endpoint) -> Result<RunPullAssociation, ApiError> {
        let bad = || error(ApiErrorKind::InvalidResponse);
        let repository = self.base.repo.as_ref().ok_or_else(bad)?.identity(api)?;
        let head_repository = self
            .head
            .repo
            .as_ref()
            .map(|repo| repo.identity(api))
            .transpose()?;
        let Some((url_repository, number)) = self
            .url
            .strip_prefix(api.rest())
            .and_then(|rest| rest.strip_prefix("/repos/"))
            .and_then(|path| path.split_once("/pulls/"))
        else {
            return Err(bad());
        };
        if self.number == 0
            || !valid_sha(&self.head.sha)
            || !valid_sha(&self.base.sha)
            || RepositoryId::parse(url_repository).ok().as_ref() != Some(&repository)
            || number != self.number.to_string()
        {
            return Err(bad());
        }
        Ok(RunPullAssociation {
            repository,
            number: self.number,
            head_sha: self.head.sha.to_ascii_lowercase(),
            head_repository,
        })
    }
}

#[derive(Deserialize)]
struct RawWorkflowJob {
    id: u64,
    run_id: u64,
    run_attempt: Option<u64>,
    head_sha: String,
    url: String,
    run_url: String,
    name: String,
    status: Option<String>,
    conclusion: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    steps: Option<Vec<RawWorkflowStep>>,
}

impl RawWorkflowJob {
    fn checked(self, api: &Endpoint, proof: &RunProof) -> Result<WorkflowJob, ApiError> {
        let bad = || error(ApiErrorKind::InvalidResponse);
        let job_url_matches = self
            .url
            .strip_prefix(api.rest())
            .and_then(|rest| rest.strip_prefix("/repos/"))
            .and_then(|path| path.split_once("/actions/jobs/"))
            .is_some_and(|(repository, job)| {
                RepositoryId::parse(repository).ok().as_ref() == Some(&proof.repository)
                    && job == self.id.to_string()
            });
        if self.id == 0
            || self.run_id != proof.id
            || self
                .run_attempt
                .is_some_and(|attempt| attempt != proof.attempt)
            || !valid_sha(&self.head_sha)
            || !self.head_sha.eq_ignore_ascii_case(&proof.head_sha)
            || !job_url_matches
            || !valid_run_url(
                Some(api),
                &self.run_url,
                &proof.repository,
                proof.id,
                proof.attempt,
            )
        {
            return Err(bad());
        }
        let mut numbers = HashSet::new();
        let steps = self
            .steps
            .unwrap_or_default()
            .into_iter()
            .map(|step| {
                if step.number == 0 || !numbers.insert(step.number) {
                    return Err(bad());
                }
                Ok(WorkflowStep {
                    number: step.number,
                    name: step.name,
                    status: step.status.unwrap_or_default(),
                    conclusion: step.conclusion,
                    started_at: step.started_at,
                    completed_at: step.completed_at,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(WorkflowJob {
            repository: proof.repository.clone(),
            id: self.id,
            run_id: proof.id,
            run_attempt: proof.attempt,
            head_sha: proof.head_sha.clone(),
            name: self.name,
            status: self.status.unwrap_or_default(),
            conclusion: self.conclusion,
            started_at: self.started_at,
            completed_at: self.completed_at,
            steps,
        })
    }
}

#[derive(Deserialize)]
struct RawWorkflowStep {
    number: u64,
    name: String,
    status: Option<String>,
    conclusion: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
}

#[cfg(test)]
mod tests;
