//! Explicit ticket routing scope, checked before page state or reads change.
//! Bare keys keep the existing project/held/first-site fallback. A supplied URL
//! or alias is a constraint, never a hint that may silently be discarded.

use radar_core::Subject;
use reqwest::Url;

use crate::{JiraPlugin, config::Site};

pub(crate) fn resolve<'a>(p: &'a JiraPlugin, subject: &Subject) -> Option<&'a Site> {
    if let Some(alias) = subject.field("site") {
        if alias.is_empty() {
            return None;
        }
        let site = p.cfg.site(alias)?;
        if let Some(given) = &subject.url
            && !url_matches(site, &subject.id, given)
        {
            return None;
        }
        return Some(site);
    }
    let Some(given) = &subject.url else {
        return p.cfg.site(&crate::join::site_for(p, &subject.id)?);
    };
    let (url, key) = browse_url(given)?;
    if key != subject.id {
        return None;
    }
    let mut sites = p.cfg.sites.iter().filter(|site| matches_origin(site, &url));
    let site = sites.next()?;
    // Two aliases may use different accounts on one origin. The URL cannot
    // choose between them, even if just one watches or holds this key.
    sites.next().is_none().then_some(site)
}

/// Shared with journal ownership, which additionally retains its explicit-site
/// requirement and stricter canonical SafeUrl/SubjectKey snapshot policy.
pub(crate) fn url_matches(site: &Site, key: &str, given: &str) -> bool {
    browse_url(given).is_some_and(|(url, found)| found == key && matches_origin(site, &url))
}

fn matches_origin(site: &Site, url: &Url) -> bool {
    // Keep Jira Cloud's established site_url normalization: configured paths,
    // queries and fragments do not form an installation prefix.
    let Some(base) = site
        .base_url()
        .ok()
        .and_then(|base| strict_url(&base, true))
    else {
        return false;
    };
    base.scheme() == url.scheme()
        && base.host_str() == url.host_str()
        && base.port_or_known_default() == url.port_or_known_default()
}

/// Parse an exact browse path without repairing it or inferring any site alias.
/// Recognition and Show share this gate; query/fragment never supply the key.
pub(crate) fn browse_url(given: &str) -> Option<(Url, &str)> {
    let url = strict_url(given, false)?;
    let (_, rest) = given.split_once("://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let path = rest[end..].split(['?', '#']).next()?;
    // Check the original path as well as the parsed one: no dot-segment
    // repair, encoded separators/key characters, suffixes or extra slashes.
    let key = path.strip_prefix("/browse/")?;
    (crate::config::is_ticket_key(key) && url.path() == path).then_some((url, key))
}

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
    if authority.is_empty()
        || (!configured && (authority.contains(['@', '%']) || authority.ends_with(':')))
    {
        return None;
    }
    let url = Url::parse(input).ok()?;
    if url.host_str().is_none()
        || (!configured && (!url.username().is_empty() || url.password().is_some()))
    {
        return None;
    }
    Some(url)
}

#[cfg(test)]
mod tests;
