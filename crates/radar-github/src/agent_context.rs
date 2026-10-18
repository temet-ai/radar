//! Provider-owned context and fresh PR resolution without navigating the UI.
use crate::{GithubPlugin, PullRequest, Target, history, model};
use radar_core::Plugin;
use radar_core::agent_context::{
    AgentContext, AgentContextRequest, ChangeResolution, ChangeResolveRequest, ContextField,
    DiffStats, Resolve,
};
use radar_core::context::Document;
use radar_core::plugin::{Effect, Msg};

fn target(p: &GithubPlugin, key: &radar_core::journal::SubjectKey) -> Option<Target> {
    let subject = p.subject_from_journal(key)?;
    let target = crate::join::target(&subject)?;
    p.repos
        .iter()
        .any(|r| &r.id == target.repository())
        .then_some(target)
}
fn held<'a>(p: &'a GithubPlugin, target: &Target) -> Option<&'a PullRequest> {
    let Target::Pull(repo, n) = target else {
        return None;
    };
    p.seen
        .get(&(repo.clone(), *n))
        .map(|s| &s.pull)
        .or_else(|| {
            p.repos
                .iter()
                .find(|r| &r.id == repo)?
                .pulls
                .as_ref()?
                .items
                .iter()
                .find(|r| r.number == *n)
        })
}
pub(crate) fn document(p: &GithubPlugin, request: &AgentContextRequest) -> Option<Document> {
    let target = target(p, &request.subject)?;
    let pull = held(p, &target)?;
    let context = AgentContext::new("GitHub pull request", request.subject.clone(), &pull.title)
        .link(Some(pull.web_url()))
        .field(ContextField::new("project", pull.repository.full_name()))
        .field(ContextField::new("author", &pull.author))
        .field(ContextField::new("state", &pull.state))
        .field(ContextField::new("source_branch", &pull.head_branch))
        .field(ContextField::new("target_branch", &pull.base_branch))
        .field(ContextField::new("head_sha", &pull.head_sha))
        .field(ContextField::new(
            "description",
            pull.body.as_deref().unwrap_or_default(),
        ));
    let mut doc = Document::from_context(context)
        .section("description", pull.body.clone().unwrap_or_default());
    if let Some(seen) = p.seen.get(&(pull.repository.clone(), pull.number)) {
        doc.observed_at = Some(seen.at);
        if let Some(checks) = &seen.checks {
            doc = doc.section("checks", serde_json::to_string_pretty(checks).ok()?);
            doc.missing.extend(checks.errors.clone());
        }
    }
    doc.missing.push(
        "PR review comments are not exposed by this GitHub cache; inspect the local diff".into(),
    );
    Some(doc)
}
pub(crate) fn resolve(
    p: &mut GithubPlugin,
    request: &ChangeResolveRequest,
) -> Option<(Resolve, Vec<Effect>)> {
    let target = target(p, &request.subject)?;
    let Target::Pull(repo, n) = &target else {
        return None;
    };
    let id = (repo.clone(), *n);
    if let Some(state) = p.review_reads.get(&id).cloned() {
        if state.is_some() {
            p.review_reads.remove(&id);
        }
        return Some((
            match state {
                None => Resolve::Pending,
                Some(Err(error)) => Resolve::Failed(error),
                Some(Ok(())) => resolution(p, &request.subject, &target),
            },
            vec![],
        ));
    }
    if p.seen.get(&id).is_some_and(|s| p.now - s.at < 30) {
        return Some((resolution(p, &request.subject, &target), vec![]));
    }
    let (ids, token, api, via, generation) = (
        p.cfg.ids(),
        p.cfg.token.clone(),
        p.cfg.api.clone(),
        p.via.clone(),
        p.generation,
    );
    let action = history::action("github.pull-request.read", &target);
    let scope = action.clone();
    p.review_reads.insert(id.clone(), None);
    Some((
        Resolve::Pending,
        vec![Effect::SpawnAction {
            action,
            task: Box::pin(async move {
                let result = async {
                    let client = via.client(&ids, &token, &api)?;
                    let pull = client.pull(&id.0, id.1).await?;
                    let checks = client.checks(&pull).await.ok();
                    Ok::<_, model::ApiError>((pull, checks))
                }
                .await
                .map_err(|e| e.to_string());
                Box::new(crate::Answer::ReviewRead {
                    generation,
                    id,
                    action: scope,
                    result: Box::new(result),
                }) as Msg
            }),
        }],
    ))
}
fn resolution(
    p: &GithubPlugin,
    subject: &radar_core::journal::SubjectKey,
    target: &Target,
) -> Resolve {
    let Some(pull) = held(p, target) else {
        return Resolve::Failed("PR detail is unavailable".into());
    };
    if pull.state != "open" || pull.merged {
        return Resolve::Failed("this pull request is no longer open".into());
    }
    if pull.head_sha.is_empty() {
        return Resolve::Failed("GitHub gave no head commit".into());
    }
    Resolve::Ready(ChangeResolution {
        subject: subject.clone(),
        title: pull.title.clone(),
        link: Some(pull.web_url()),
        repository: pull
            .head_repository
            .as_ref()
            .unwrap_or(&pull.repository)
            .full_name(),
        source_branch: pull.head_branch.clone(),
        target_branch: pull.base_branch.clone(),
        head_sha: pull.head_sha.clone(),
        diff_stats: DiffStats::default(),
        description: pull.body.clone().unwrap_or_default(),
        threads: vec![],
    })
}
