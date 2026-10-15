//! Pure requested-target validation shared by navigation and journal ownership.
//! Neither caches nor returned provider identity establish a routing target.
use radar_core::Subject;
use reqwest::Url;

use super::{JOB, MERGE_REQUEST, PIPELINE, REPO};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub kind: &'static str,
    pub project: String,
    pub number: Option<u64>,
}

impl Target {
    fn new(kind: &'static str, project: &str, number: Option<u64>) -> Option<Self> {
        if crate::config::group_path_problem(project).is_some()
            || project.split('/').any(|s| matches!(s, "." | ".."))
        {
            return None;
        }
        Some(Self {
            kind,
            project: project.into(),
            number,
        })
    }

    fn from_id(subject: &Subject) -> Option<Self> {
        let (kind, marker) = match subject.kind.as_str() {
            REPO => return Self::new(REPO, &subject.id, None),
            MERGE_REQUEST => (MERGE_REQUEST, '!'),
            JOB => (JOB, '#'),
            PIPELINE => (PIPELINE, '#'),
            _ => return None,
        };
        let (project, number) = subject.id.rsplit_once(marker)?;
        Self::new(kind, project, Some(number.parse().ok()?))
    }

    /// Missing fields are not contradictions: old ID+URL-only journal subjects
    /// still work. Navigation separately requires its existing routing fields.
    pub fn consistent(base: &str, subject: &Subject) -> Option<Self> {
        let target = Self::from_id(subject)?;
        if subject
            .field("project")
            .is_some_and(|p| p != target.project)
        {
            return None;
        }
        for field in ["iid", "id"] {
            if let Some(value) = subject.field(field)
                && (target.number.is_none() || value.parse::<u64>().ok() != target.number)
            {
                return None;
            }
        }
        if let Some(url) = &subject.url
            && Self::from_url(base, url).as_ref() != Some(&target)
        {
            return None;
        }
        Some(target)
    }

    pub fn number_field(&self) -> Option<&'static str> {
        match self.kind {
            MERGE_REQUEST => Some("iid"),
            JOB | PIPELINE => Some("id"),
            _ => None,
        }
    }

    pub fn from_url(base: &str, input: &str) -> Option<Self> {
        // Existing configured credentials do not change an origin. They are
        // never accepted from a supplied permalink or copied into a target.
        let base = strict_url(base, true)?;
        let url = strict_url(input, false)?;
        if base.scheme() != url.scheme()
            || base.host_str() != url.host_str()
            || base.port_or_known_default() != url.port_or_known_default()
        {
            return None;
        }
        let prefix = base.path().trim_end_matches('/');
        let path = url.path().strip_prefix(prefix)?.strip_prefix('/')?;
        if let Some((project, route)) = path.split_once("/-/") {
            let (kind, rest) = if let Some(rest) = route.strip_prefix("merge_requests/") {
                (MERGE_REQUEST, rest)
            } else if let Some(rest) = route.strip_prefix("jobs/") {
                (JOB, rest)
            } else {
                (PIPELINE, route.strip_prefix("pipelines/")?)
            };
            // Existing MR /diffs and other subroutes refer to the same item.
            let number = rest.split('/').next()?.parse().ok()?;
            return Self::new(kind, project, Some(number));
        }
        Self::new(REPO, path.trim_end_matches('/'), None)
    }
}

/// Check the spelling before URL parsing can remove whitespace, turn backslashes
/// into slashes, resolve dot segments, or discard an empty userinfo component.
fn strict_url(input: &str, configured: bool) -> Option<Url> {
    if input
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return None;
    }
    let (scheme, rest) = input.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.is_empty() || (!configured && authority.contains('@')) {
        return None;
    }
    let path = rest[end..].split(['?', '#']).next()?;
    for segment in path.split('/') {
        let lower = segment.to_ascii_lowercase();
        if lower.contains("%2f") || lower.contains("%5c") {
            return None;
        }
        let dots = lower.replace("%2e", ".");
        if matches!(dots.as_str(), "." | "..") {
            return None;
        }
    }
    let url = Url::parse(input).ok()?;
    if url.host_str().is_none()
        || (!configured && (!url.username().is_empty() || url.password().is_some()))
    {
        return None;
    }
    Some(url)
}
