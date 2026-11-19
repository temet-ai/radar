//! The channels the signed-in user is in, for the picker in setup and
//! Settings: `users.conversations`, public and private, paged to its end.

use super::{ChannelInfo, Client, MAX_LIST_PAGES, channel_of};
use anyhow::Result;
use serde_json::Value;

/// What the list of your channels came to.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct JoinedRead {
    /// Not archived, by name; every one is a channel you are in.
    pub(crate) channels: Vec<ChannelInfo>,
    /// Private channels were left out: the token has no `groups:read`.
    pub(crate) private_skipped: bool,
    /// radar stopped at its page limit with more still listed.
    pub(crate) truncated: bool,
}

/// Slack refused the call for want of a scope (see `explain`).
fn lacks_scope(error: &anyhow::Error) -> bool {
    format!("{error:#}").starts_with("the token lacks a permission")
}

impl Client {
    /// The channels you are in. With `private` the private ones are asked
    /// for too; a token without `groups:read` is refused for that, and the
    /// public ones are asked for alone (`private_skipped`).
    pub(crate) async fn joined_channels(&self, private: bool) -> Result<JoinedRead> {
        match self.joined_pages(private).await {
            Err(error) if private && lacks_scope(&error) => {
                let mut got = self.joined_pages(false).await?;
                got.private_skipped = true;
                Ok(got)
            }
            other => other.map(|mut got| {
                got.private_skipped = !private;
                got
            }),
        }
    }

    async fn joined_pages(&self, private: bool) -> Result<JoinedRead> {
        let types = if private { "public_channel,private_channel" } else { "public_channel" };
        let params = vec![
            ("types".to_string(), types.to_string()),
            ("exclude_archived".into(), "true".into()),
            ("limit".into(), "999".into()),
        ];
        let mut got = JoinedRead::default();
        let mut cursor = String::new();
        for _ in 0..MAX_LIST_PAGES {
            let (body, next) = self.cursor_page("users.conversations", params.clone(), &cursor).await?;
            for row in body.get("channels").and_then(Value::as_array).into_iter().flatten() {
                let mut channel = channel_of(row);
                let seen = got.channels.iter().any(|c| c.id == channel.id);
                if channel.id.trim().is_empty() || channel.name.trim().is_empty() || channel.is_archived || seen {
                    continue;
                }
                // Slack may leave is_member out of this list; being in it is the proof.
                channel.is_member = true;
                got.channels.push(channel);
            }
            cursor = next;
            if cursor.is_empty() {
                break;
            }
        }
        got.truncated = !cursor.is_empty();
        got.channels.sort_by_cached_key(|c| (c.name.to_lowercase(), c.id.clone()));
        Ok(got)
    }
}

#[cfg(test)]
mod tests;
