//! The Slack app a user token for radar comes from, as the manifest Slack's
//! "Create New App, From a manifest" takes. It asks only for user scopes,
//! exactly those [`NEEDS`] lists (what radar-slack calls, and the pictures
//! it fetches with the token), so a token made from it has no more and no
//! fewer than radar uses. Setup's Slack step copies it with `m` and opens
//! Slack's new-app page with it filled in.

use crate::settings::NEEDS;

/// Slack's page for a new app.
pub(crate) const NEW_APP: &str = "https://api.slack.com/apps?new_app=1";
/// Where an app made before is found again.
pub(crate) const APPS: &str = "https://api.slack.com/apps";

/// Every user scope radar needs, once each, in [`NEEDS`]'s order.
pub(crate) fn scopes() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for (_, needs, _) in NEEDS {
        for s in needs {
            if !out.contains(s) {
                out.push(s);
            }
        }
    }
    out
}

/// The manifest, as JSON.
pub(crate) fn text() -> String {
    let manifest = serde_json::json!({
        "_metadata": { "major_version": 1, "minor_version": 1 },
        "display_information": {
            "name": "radar",
            "description": "radar reads Slack as you, with a user token.",
        },
        "oauth_config": { "scopes": { "user": scopes() } },
        "settings": {
            "org_deploy_enabled": false,
            "socket_mode_enabled": false,
            "token_rotation_enabled": false,
        },
    });
    serde_json::to_string_pretty(&manifest).expect("a manifest is plain JSON")
}

/// The new-app page with the manifest filled in.
pub(crate) fn new_app_url() -> String {
    let compact: serde_json::Value = serde_json::from_str(&text()).expect("the manifest is JSON");
    format!("{NEW_APP}&manifest_json={}", encode(&compact.to_string()))
}

/// Percent-encoding of everything but the unreserved characters.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The manifest asks for the user scopes radar calls Slack with, and
    /// nothing a bot would have.
    #[test]
    fn the_manifest_asks_for_exactly_the_user_scopes_radar_needs() {
        let m: serde_json::Value = serde_json::from_str(&text()).unwrap();
        let user: Vec<&str> = m["oauth_config"]["scopes"]["user"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        for want in [
            "channels:history",
            "channels:read",
            "groups:history",
            "groups:read",
            "search:read",
            "users:read",
            "usergroups:read",
            "chat:write",
            "reactions:write",
            "files:read",
        ] {
            assert!(user.contains(&want), "{want} missing from {user:?}");
        }
        assert_eq!(user.len(), scopes().len(), "each once");
        assert!(m["oauth_config"]["scopes"].get("bot").is_none(), "user scopes only");
        assert_eq!(m["display_information"]["name"], "radar");
    }

    #[test]
    fn the_new_app_link_carries_the_manifest_encoded() {
        let url = new_app_url();
        assert!(url.starts_with("https://api.slack.com/apps?new_app=1&manifest_json=%7B"));
        assert!(url.contains("channels%3Ahistory"));
        assert!(!url.contains(' ') && !url.contains('"') && !url.contains('{'));
    }
}
