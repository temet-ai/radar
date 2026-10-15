//! Minimal allowlisted evidence: local navigation is separate from remote reads.
use crate::join::{self, Target};
use radar_core::Subject;
use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, Reason, StableCode, SubjectKey, SubjectRole, SubjectSnapshot,
};
use radar_core::plugin::Effect;

pub(crate) fn subject(subject: &Subject) -> Option<SubjectSnapshot> {
    let target = join::target(subject)?;
    let id = match &target {
        Target::Repository(repo) => repo.full_name(),
        Target::Pull(repo, number) => format!("{}/pull/{number}", repo.full_name()),
        Target::Run(repo, id, attempt) => {
            format!("{}/actions/runs/{id}/attempts/{attempt}", repo.full_name())
        }
    };
    if radar_core::log::contains_secret(&id) {
        return None;
    }
    SubjectKey::new("github", join::INSTANCE, target.kind(), &id)
        .ok()
        .map(|key| SubjectSnapshot::new(key, SubjectRole::Primary))
}

/// Rebuild only an exact, supported owner target. Stored titles and aliases
/// cannot supply routing identity, and canonicalization cannot repair a key.
pub(crate) fn from_journal(p: &crate::GithubPlugin, key: &SubjectKey) -> Option<Subject> {
    if key.provider != "github"
        || SubjectKey::new(&key.provider, &key.instance, &key.kind, &key.id).ok()? != *key
        || key.instance != format!("{}/", join::INSTANCE)
    {
        return None;
    }
    let target = join::from_url(&format!("{}{}", key.instance, key.id))?;
    if target.kind() != key.kind || p.repos.is_empty() {
        return None;
    }
    // A supported but untracked repository keeps its canonical web fallback.
    // Show remains responsible for deciding whether a native page is available.
    let restored = target.subject();
    (subject(&restored)?.key == *key).then_some(restored)
}

/// Explicit choices from the owner's held identity, never automatic bindings.
pub(crate) fn related(p: &crate::GithubPlugin, source: &Subject) -> Vec<Subject> {
    let Some(target) = join::target(source) else {
        return vec![];
    };
    if !p.repos.iter().any(|r| &r.id == target.repository()) {
        return vec![];
    }
    let mut repository = join::repository_subject(target.repository());
    repository.label = format!(
        "{}repository · {}",
        if matches!(target, Target::Pull(..)) {
            "base "
        } else {
            ""
        },
        target.repository().full_name(),
    );
    let mut out = vec![repository];
    if matches!(target, Target::Pull(..)) {
        let held = p.subject_for(&target);
        if let Some(Target::Repository(repo)) =
            held.field("source_repository_url").and_then(join::from_url)
            && &repo != target.repository()
        {
            let mut source = join::repository_subject(&repo);
            source.label = format!("source repository · {}", repo.full_name());
            out.push(source);
        }
    }
    out
}

pub(crate) fn action(kind: &'static str, target: &Target) -> ActionDescriptor {
    let mut action = ActionDescriptor::new(kind, Origin::Plugin("github".into()));
    if let Some(subject) = subject(&target.subject()) {
        action.subjects.push(subject);
    } else {
        action.metadata.redacted = true;
        action.metadata.omitted_subjects = 1;
    }
    action
}

pub(crate) fn connection_action() -> ActionDescriptor {
    ActionDescriptor::new("github.connection.check", Origin::Plugin("github".into()))
}

/// One question for every configured repository: what moved.
pub(crate) fn probe_action() -> ActionDescriptor {
    ActionDescriptor::new(
        "github.repositories.probe",
        Origin::Plugin("github".into()),
    )
}

pub(crate) fn discovery_action() -> ActionDescriptor {
    ActionDescriptor::new(
        "github.repositories.discover",
        Origin::Plugin("github".into()),
    )
}

pub(crate) fn opened(target: &Target) -> Effect {
    Effect::Observe {
        action: action("github.page.open", target),
        outcome: Outcome::Succeeded,
    }
}

pub(crate) fn finished(
    scope: &ActionDescriptor,
    outcome: Outcome,
    count: Option<usize>,
    truncated: bool,
) -> Vec<Effect> {
    let mut effects = vec![Effect::Report(outcome)];
    if let Some(count) = count {
        let mut observation = scope.clone();
        observation.kind = StableCode::new("github.read.items");
        observation.metadata.count = Some(u32::try_from(count).unwrap_or(u32::MAX));
        observation.metadata.truncated = truncated || count > u32::MAX as usize;
        if outcome == Outcome::Failed {
            observation.metadata.reason = Some(Reason::Unspecified);
        }
        effects.push(Effect::Observe {
            action: observation,
            outcome,
        });
    }
    effects
}

pub(crate) fn skipped(scope: &ActionDescriptor) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::new("github.read.application.skipped");
    Effect::Observe {
        action,
        outcome: Outcome::Blocked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_core::Plugin;
    use crate::model::RepositoryId;

    #[test]
    fn source_navigation_roundtrips_repositories_pulls_and_pinned_attempts() {
        let mut p = crate::demo::sample();
        let repo = RepositoryId::parse("acme/api").unwrap();
        for target in [
            Target::Repository(repo.clone()),
            Target::Pull(repo.clone(), 42),
            Target::Run(repo, 1001, 2),
        ] {
            let key = p.journal_subject(&target.subject()).unwrap().key;
            let restored = p.subject_from_journal(&key).unwrap();
            assert_eq!(join::target(&restored), Some(target.clone()));
            assert_eq!(p.journal_subject(&restored).unwrap().key, key);
            assert!(restored.field("title").is_none());
            assert!(matches!(
                p.show(&restored),
                radar_core::Showing::Page {
                    screen: "github",
                    ..
                }
            ));
            assert_eq!(p.pages.last().unwrap().target, target);
        }
    }

    #[test]
    fn source_navigation_keeps_canonical_web_fallback_for_untracked_repositories() {
        let mut p = crate::demo::sample();
        let target = Target::Repository(RepositoryId::parse("alice/api").unwrap());
        let key = p.journal_subject(&target.subject()).unwrap().key;
        let restored = p.subject_from_journal(&key).unwrap();
        assert_eq!(
            restored.url.as_deref(),
            Some("https://github.com/alice/api")
        );
        assert_eq!(p.journal_subject(&restored).unwrap().key, key);
        assert!(matches!(p.show(&restored), radar_core::Showing::Cannot));
    }

    #[test]
    fn source_navigation_rejects_wrong_scope_unsafe_aliases_and_missing_config() {
        let mut p = crate::demo::sample();
        let key = SubjectKey::new(
            "github",
            join::INSTANCE,
            join::PULL_REQUEST,
            "acme/api/pull/42",
        )
        .unwrap();
        for (field, value) in [
            ("provider", "other-provider"),
            ("kind", "repository"),
            ("instance", "https://foreign.example/"),
            ("instance", "https://user:password@github.com/"),
            ("instance", "https://github.com/../"),
            ("instance", "https://github.com/\n"),
            ("id", "acme/api/pull/042"),
            ("id", "Acme/api/pull/42"),
            ("id", "acme/../api/pull/42"),
            ("id", "acme/api/pull/42\n"),
            ("id", "acme/api/pull/42?token=secret"),
            ("id", "acme/api/issues/42"),
            ("id", "acme/api/actions/runs/1001"),
            ("id", "acme/api/actions/runs/1001/attempts/latest"),
        ] {
            let mut bad = key.clone();
            match field {
                "provider" => bad.provider = value.into(),
                "kind" => bad.kind = value.into(),
                "instance" => bad.instance = value.into(),
                _ => bad.id = value.into(),
            }
            assert!(p.subject_from_journal(&bad).is_none(), "{field}: {value:?}");
        }
        p.repos.clear();
        assert!(p.subject_from_journal(&key).is_none());
    }

    #[test]
    fn related_source_candidates_use_held_exact_repository_identity() {
        let mut p = crate::demo::sample();
        let repo = RepositoryId::parse("acme/api").unwrap();
        let source = join::pull_subject(&repo, 42).with(
            "source_repository_url",
            "https://github.com/untrusted/injected",
        );
        let candidates = p.related(&source);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].id, "https://github.com/acme/api");
        assert_eq!(candidates[0].label, "base repository · acme/api");
        assert_eq!(candidates[1].id, "https://github.com/alice/api");
        assert_eq!(candidates[1].label, "source repository · alice/api");
        assert!(candidates.iter().all(|s| s.field("title").is_none()));
        p.repos[0].pulls.as_mut().unwrap().items[0].head_repository = None;
        assert_eq!(p.related(&source).len(), 1);
        assert!(p.related(&source.with("provider", "foreign")).is_empty());
    }

    #[test]
    fn history_never_copies_content_or_unvalidated_routing() {
        let repo = RepositoryId::parse("acme/api").unwrap();
        let mut value = join::pull_subject(&repo, 7);
        value.label = "PRIVATE TITLE".into();
        value.fields.insert("body".into(), "PRIVATE BODY".into());
        let saved = subject(&value).unwrap();
        let json = serde_json::to_string(&saved).unwrap();
        assert!(!json.contains("PRIVATE"));
        assert_eq!(saved.key.instance, "https://github.com/");
        assert_eq!(saved.key.id, "acme/api/pull/7");
        assert!(subject(&value.with("number", "8")).is_none());
    }

    #[test]
    fn real_read_outcome_precedes_independent_application_rejection() {
        let target = Target::Repository(RepositoryId::parse("acme/api").unwrap());
        let scope = action("github.repository.read", &target);
        let mut effects = finished(&scope, Outcome::Partial, Some(100), true);
        effects.push(skipped(&scope));
        assert!(matches!(effects[0], Effect::Report(Outcome::Partial)));
        let Effect::Observe { action, outcome } = &effects[1] else {
            panic!()
        };
        assert_eq!(*outcome, Outcome::Partial);
        assert_eq!(action.metadata.count, Some(100));
        assert!(action.metadata.truncated);
        assert!(matches!(
            effects[2],
            Effect::Observe {
                outcome: Outcome::Blocked,
                ..
            }
        ));
        let Effect::Observe { action, outcome } = opened(&target) else {
            panic!()
        };
        assert_eq!(action.kind.as_str(), "github.page.open");
        assert_eq!(outcome, Outcome::Succeeded);
    }
}
