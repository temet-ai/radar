//! Only loaded repository and pull-request identities, never a provider search.

use std::collections::BTreeSet;

use radar_core::{palette::PaletteTarget, ui::icon::Icon};

use crate::{GithubPlugin, join::Target};

pub(crate) fn targets(p: &GithubPlugin) -> Vec<PaletteTarget> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut add = |target: Target, words: Vec<String>, icon| {
        let mut subject = p.subject_for(&target);
        if matches!(target, Target::Pull(..))
            && let Some(title) = words.first()
        {
            subject.label = format!("{} · {title}", subject.label);
        }
        if crate::join::target(&subject).as_ref() == Some(&target)
            && seen.insert((subject.kind.clone(), subject.id.clone()))
        {
            let mut target = PaletteTarget::scoped(subject, crate::join::INSTANCE);
            target.words = words;
            target.icon = Some(icon);
            out.push(target);
        }
    };
    for repo in &p.repos {
        if repo.repository.is_some() || repo.pulls.is_some() {
            add(
                Target::Repository(repo.id.clone()),
                vec![repo.id.full_name(), "repository".into()],
                Icon::Repo,
            );
        }
    }
    let listed = p.repos.iter().flat_map(|repo| {
        repo.pulls
            .iter()
            .flat_map(|pulls| &pulls.items)
            .filter(move |pull| pull.repository == repo.id)
    });
    let held = p.pages.iter().filter_map(|page| {
        let pull = &page.held.as_ref()?.pull;
        (page.target == Target::Pull(pull.repository.clone(), pull.number)).then_some(pull)
    });
    let remembered = p.seen.iter().filter_map(|((repo, number), seen)| {
        (&seen.pull.repository == repo && seen.pull.number == *number).then_some(&seen.pull)
    });
    for pull in p
        .pull_detail
        .iter()
        .chain(held)
        .chain(remembered)
        .chain(listed)
    {
        if p.repos.iter().any(|repo| repo.id == pull.repository) {
            add(
                Target::Pull(pull.repository.clone(), pull.number),
                vec![
                    pull.title.clone(),
                    pull.head_branch.clone(),
                    pull.repository.full_name(),
                    "pull request".into(),
                ],
                Icon::Change,
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        RepoState,
        model::{Collection, Completion, PullRequest, RepositoryId},
        test_support::RepositoryBuilder,
    };
    use radar_core::Plugin;

    fn pull(repository: RepositoryId) -> PullRequest {
        PullRequest {
            repository,
            number: 12,
            title: "Repair the Watch bell".into(),
            body: None,
            author: "vimes".into(),
            state: "open".into(),
            draft: false,
            head_sha: "a".repeat(40),
            head_repository: None,
            head_branch: "watch-bell".into(),
            base_branch: "main".into(),
            updated_at: "2026-01-01T12:00:00Z".into(),
            merged: false,
        }
    }

    #[test]
    fn cached_repository_and_pull_targets_use_existing_scoped_routes_once() {
        let mut p = GithubPlugin::default();
        let repository = RepositoryBuilder::default().build();
        let pull = pull(repository.id.clone());
        let mut state = RepoState::new(repository.id.clone());
        state.repository = Some(repository);
        state.pulls = Some(Collection {
            items: vec![pull.clone(), pull.clone()],
            completion: Completion::complete(1),
        });
        p.repos.push(state);
        p.pull_detail = Some(pull);
        let targets = p.palette_targets();
        assert_eq!(targets.len(), 2);
        assert_eq!(
            targets[0].subject.id,
            "https://github.com/ankh-morpork/watch"
        );
        assert_eq!(
            targets[1].subject.id,
            "https://github.com/ankh-morpork/watch/pull/12"
        );
        assert!(targets[1].words.contains(&"Repair the Watch bell".into()));
        for target in targets {
            let route = crate::join::target(&target.subject).unwrap();
            assert!(p.repos.iter().any(|repo| &repo.id == route.repository()));
            let radar_core::Showing::Page { screen, effects } = p.show(&target.subject) else {
                panic!("cached target opens")
            };
            assert_eq!(screen, "github");
            assert_eq!(p.pages.last().unwrap().target, route);
            // Provider reads remain unevaluated effect envelopes.
            drop(effects);
        }
    }

    #[test]
    fn configured_only_and_untracked_pull_targets_are_absent() {
        let mut p = GithubPlugin::default();
        p.repos.push(RepoState::new(
            RepositoryId::parse("ankh-morpork/watch").unwrap(),
        ));
        p.pull_detail = Some(pull(RepositoryId::parse("lancre/witches").unwrap()));
        assert!(p.palette_targets().is_empty());
    }
    #[test]
    fn palette_identity_survives_pull_title_and_head_updates() {
        let mut p = GithubPlugin::default();
        let repository = RepositoryBuilder::default().build();
        let mut state = RepoState::new(repository.id.clone());
        state.repository = Some(repository.clone());
        state.pulls = Some(Collection {
            items: vec![pull(repository.id)],
            completion: Completion::complete(1),
        });
        p.repos.push(state);
        let before: Vec<_> = p
            .palette_targets()
            .into_iter()
            .map(|target| target.identity)
            .collect();
        let pull = &mut p.repos[0].pulls.as_mut().unwrap().items[0];
        pull.title = "Inspect the Watch bell".into();
        pull.head_sha = "b".repeat(40);
        pull.state = "closed".into();
        let after: Vec<_> = p
            .palette_targets()
            .into_iter()
            .map(|target| target.identity)
            .collect();
        assert_eq!(after, before);
    }
}
