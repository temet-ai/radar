//! Validated, host-qualified subjects for navigation and private history.
//! Labels and provider responses never establish a requested routing identity.

use std::ops::Range;

use crate::model::RepositoryId;
use radar_core::{Offer, Owned, Point, Subject};

pub(crate) const REPOSITORY: &str = "repository";
pub(crate) const PULL_REQUEST: &str = "pull-request";
pub(crate) const WORKFLOW_RUN: &str = "workflow-run";
pub(crate) const INSTANCE: &str = "https://github.com";
pub(crate) const OWNS: [Owned; 3] = [
    Owned::page(REPOSITORY, "repositories"),
    Owned::page(PULL_REQUEST, "pull requests"),
    Owned::page(WORKFLOW_RUN, "workflow runs"),
];

pub(crate) const OFFERS: [Offer; 6] = [
    Offer::new(Point::Actions, REPOSITORY, "repositories"),
    Offer::new(Point::Actions, PULL_REQUEST, "pull requests"),
    Offer::new(
        Point::Line,
        REPOSITORY,
        "the repository detail pane and page, under its title",
    ),
    Offer::new(
        Point::Line,
        PULL_REQUEST,
        "the pull request detail pane and page, under its title",
    ),
    Offer::new(
        Point::Section,
        REPOSITORY,
        "the repository detail pane and overview",
    ),
    Offer::new(
        Point::Section,
        PULL_REQUEST,
        "the pull request detail pane and overview",
    ),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Repository(RepositoryId),
    Pull(RepositoryId, u64),
    Run(RepositoryId, u64, u64),
}

impl Target {
    pub(crate) fn repository(&self) -> &RepositoryId {
        match self {
            Self::Repository(repo) | Self::Pull(repo, _) | Self::Run(repo, _, _) => repo,
        }
    }

    pub(crate) fn url(&self) -> String {
        match self {
            Self::Repository(repo) => repo.web_url(),
            Self::Pull(repo, number) => format!("{}/pull/{number}", repo.web_url()),
            Self::Run(repo, id, attempt) => {
                format!("{}/actions/runs/{id}/attempts/{attempt}", repo.web_url())
            }
        }
    }

    pub(crate) fn subject(&self) -> Subject {
        match self {
            Self::Repository(repo) => repository_subject(repo),
            Self::Pull(repo, number) => pull_subject(repo, *number),
            Self::Run(repo, id, attempt) => run_subject(repo, *id, *attempt),
        }
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Repository(_) => REPOSITORY,
            Self::Pull(_, _) => PULL_REQUEST,
            Self::Run(_, _, _) => WORKFLOW_RUN,
        }
    }
}

pub(crate) fn repository_subject(repo: &RepositoryId) -> Subject {
    Subject::new(REPOSITORY, repo.web_url(), repo.full_name())
        .with_url(repo.web_url())
        .with("provider", "github")
        .with("instance", INSTANCE)
        .with("repository", repo.full_name())
        .with("repository_url", repo.web_url())
}

pub(crate) fn pull_subject(repo: &RepositoryId, number: u64) -> Subject {
    let url = format!("{}/pull/{number}", repo.web_url());
    Subject::new(
        PULL_REQUEST,
        &url,
        format!("{} #{number}", repo.full_name()),
    )
    .with_url(url)
    .with("provider", "github")
    .with("instance", INSTANCE)
    .with("repository", repo.full_name())
    .with("repository_url", repo.web_url())
    .with("number", number.to_string())
}

pub(crate) fn run_subject(repo: &RepositoryId, id: u64, attempt: u64) -> Subject {
    let url = Target::Run(repo.clone(), id, attempt).url();
    Subject::new(
        WORKFLOW_RUN,
        &url,
        format!("{} run {id} · attempt {attempt}", repo.full_name()),
    )
    .with_url(url)
    .with("provider", "github")
    .with("instance", INSTANCE)
    .with("repository", repo.full_name())
    .with("repository_url", repo.web_url())
    .with("run_id", id.to_string())
    .with("attempt", attempt.to_string())
}

/// No URL parser normalization is allowed to turn an unsafe path into a target.
/// The supported surface is exactly an HTTPS repository or numbered PR URL.
pub(crate) fn from_url(input: &str) -> Option<Target> {
    if input.len() > 1024 || input.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let path = input.strip_prefix("https://github.com/")?;
    let parts: Vec<_> = path.split('/').collect();
    if !matches!(parts.len(), 2 | 4 | 7) {
        return None;
    }
    let repo = RepositoryId::parse(&format!("{}/{}", parts[0], parts[1])).ok()?;
    let target = if parts.len() == 2 {
        Target::Repository(repo)
    } else if parts.len() == 7 {
        if parts[2] != "actions" || parts[3] != "runs" || parts[5] != "attempts" {
            return None;
        }
        let number = |raw: &str| {
            raw.parse::<u64>()
                .ok()
                .filter(|n| *n > 0 && n.to_string() == raw)
        };
        Target::Run(repo, number(parts[4])?, number(parts[6])?)
    } else {
        if parts[2] != "pull"
            || parts[3].is_empty()
            || !parts[3].bytes().all(|c| c.is_ascii_digit())
        {
            return None;
        }
        let number = parts[3].parse::<u64>().ok().filter(|n| *n > 0)?;
        if number.to_string() != parts[3] {
            return None;
        }
        Target::Pull(repo, number)
    };
    Some(target)
}

pub(crate) fn target(subject: &Subject) -> Option<Target> {
    let parsed = from_url(&subject.id)?;
    if subject.kind != parsed.kind()
        || subject
            .url
            .as_ref()
            .is_some_and(|u| from_url(u).as_ref() != Some(&parsed))
        || subject.field("repository_url").is_some_and(|url| {
            from_url(url) != Some(Target::Repository(parsed.repository().clone()))
        })
        || subject.field("provider").is_some_and(|v| v != "github")
        || subject.field("instance").is_some_and(|v| v != INSTANCE)
        || subject
            .field("repository")
            .is_some_and(|v| RepositoryId::parse(v).ok().as_ref() != Some(parsed.repository()))
    {
        return None;
    }
    if let Some(number) = subject.field("number") {
        match &parsed {
            Target::Pull(_, expected) if number == expected.to_string() => {}
            _ => return None,
        }
    }
    for (field, expected) in match &parsed {
        Target::Run(_, id, attempt) => {
            vec![("run_id", id.to_string()), ("attempt", attempt.to_string())]
        }
        _ => vec![],
    } {
        if subject.field(field).is_some_and(|value| value != expected) {
            return None;
        }
    }
    if !matches!(parsed, Target::Run(_, _, _))
        && (subject.field("run_id").is_some() || subject.field("attempt").is_some())
    {
        return None;
    }
    Some(parsed)
}

/// Recognize whole URL-shaped tokens, never a trusted prefix of a longer URL.
pub(crate) fn recognise(text: &str) -> Vec<(Range<usize>, Subject)> {
    let mut out = Vec::new();
    for (start, _) in text.match_indices("https://github.com/") {
        let prefix = &text[..start];
        if prefix
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace() && !matches!(c, '(' | '[' | '<' | '"' | '\''))
        {
            continue;
        }
        let token_start = prefix
            .rfind(char::is_whitespace)
            .map_or(0, |i| i + prefix[i..].chars().next().unwrap().len_utf8());
        let context = &prefix[token_start..];
        // A candidate inside another URL/query is part of that outer token.
        if context.contains([':', '/', '=', '@', '\\']) || context.contains("www.") {
            continue;
        }
        let raw = &text[start..];
        let end = raw.find(char::is_whitespace).unwrap_or(raw.len());
        // Only punctuation at the end may close a prose/Markdown wrapper.
        // A ')' inside a longer token cannot make its prefix a trusted URL.
        let token = raw[..end].trim_end_matches([',', ';', ')', ']', '>', '"', '\'']);
        if let Some(target) = from_url(token) {
            out.push((start..start + token.len(), target.subject()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects_are_canonical_and_provider_scoped() {
        let repo = RepositoryId::parse("Acme/API").unwrap();
        for target in [Target::Repository(repo.clone()), Target::Pull(repo, 17)] {
            let s = target.subject();
            assert_eq!(super::target(&s), Some(target.clone()));
            assert_eq!(s.id, target.url());
            assert_eq!(s.field("provider"), Some("github"));
            assert_eq!(s.field("instance"), Some(INSTANCE));
        }
    }

    #[test]
    fn contradictory_routing_fields_and_urls_are_rejected() {
        let repo = RepositoryId::parse("acme/api").unwrap();
        let subject = pull_subject(&repo, 17);
        for (field, value) in [
            ("repository", "acme/other"),
            ("number", "18"),
            ("instance", "https://other.example"),
            ("provider", "other"),
        ] {
            assert!(target(&subject.clone().with(field, value)).is_none());
        }
        assert!(
            target(
                &subject
                    .clone()
                    .with_url("https://github.com/acme/api/pull/18")
            )
            .is_none()
        );
        let mut wrong_kind = subject;
        wrong_kind.kind = REPOSITORY.into();
        assert!(target(&wrong_kind).is_none());
    }

    #[test]
    fn unsupported_or_unsafe_addresses_never_become_targets() {
        for address in [
            "http://github.com/acme/api",
            "https://api.github.com/acme/api",
            "https://github.com.evil.test/acme/api",
            "https://user@github.com/acme/api",
            "https://github.com:443/acme/api",
            "https://github.com/acme/api?token=secret",
            "https://github.com/acme/api#part",
            "https://github.com/acme/../api",
            "https://github.com/acme/%61pi",
            "https://github.com/acme/api/pull/0",
            "https://github.com/acme/api/pull/01",
            "https://github.com/acme/api/pull/1/files",
            "https://github.com/acme/api/pull/+1",
            "https://github.com/acme/api/pull/1\n",
            "https://github.com/acme/api/",
        ] {
            assert!(from_url(address).is_none(), "{address}");
        }
    }

    #[test]
    fn recognition_preserves_utf8_ranges_and_does_not_slice_urls() {
        let text = "é [review](https://github.com/acme/api/pull/7) and https://github.com/acme/api?private=yes";
        let found = recognise(text);
        assert_eq!(found.len(), 1);
        assert_eq!(
            &text[found[0].0.clone()],
            "https://github.com/acme/api/pull/7"
        );
        for external in [
            "https://evil.test/https://github.com/acme/api",
            "https://evil.test/?next=https://github.com/acme/api",
            "www.evil.test?next=https://github.com/acme/api",
            "https://github.com/acme/api)evil",
        ] {
            assert!(recognise(external).is_empty(), "{external}");
        }
    }
}
