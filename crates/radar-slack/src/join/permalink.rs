//! Conservative local routing against the currently held workspace URL.
//! This does not authenticate the account or establish remote thread ownership.

use reqwest::Url;

pub(super) struct Permalink {
    pub channel: String,
    pub root: String,
}

pub(super) fn timestamp(value: &str) -> bool {
    value.split_once('.').is_some_and(|(seconds, fraction)| {
        !seconds.is_empty()
            && !fraction.is_empty()
            && seconds
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit())
    })
}

/// Only ordinary HTTP(S) workspace URLs. Reject parser repairs of the raw path
/// so dot segments, encoded separators and backslashes cannot invent a route.
fn workspace_url(raw: &str) -> Option<Url> {
    if raw
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return None;
    }
    let (_, after_scheme) = raw.split_once("://")?;
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    if authority.contains(['@', '%']) {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let workspace = url.host_str()?.strip_suffix(".slack.com")?;
    if workspace.is_empty()
        || workspace.len() > 63
        || workspace.starts_with('-')
        || workspace.ends_with('-')
        || !workspace
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return None;
    }
    let suffix = &after_scheme[authority_end..];
    let raw_path = suffix.split(['?', '#']).next()?;
    if !raw_path.is_empty() && raw_path != url.path() {
        return None;
    }
    Some(url)
}

pub(super) fn parse(workspace: &str, candidate: &str) -> Option<Permalink> {
    let current = workspace_url(workspace)?;
    if current.path() != "/" || current.query().is_some() || current.fragment().is_some() {
        return None;
    }
    let url = workspace_url(candidate)?;
    if url.host_str()? != current.host_str()? {
        return None;
    }
    let mut path = url.path().split('/');
    if !path.next()?.is_empty() || path.next()? != "archives" {
        return None;
    }
    let channel = path.next()?;
    let message = crate::model::ts_of_p(path.next()?)?;
    if path.next().is_some() || !crate::config::is_channel_id(channel) {
        return None;
    }
    let mut root = None;
    for (key, value) in url.query_pairs() {
        if key == "thread_ts" {
            if !timestamp(&value) || root.as_ref().is_some_and(|previous| previous != &value) {
                return None;
            }
            root = Some(value);
        }
    }
    Some(Permalink {
        channel: channel.into(),
        root: root.map_or(message, |root| root.into_owned()),
    })
}
