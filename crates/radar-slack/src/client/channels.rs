//! Bounded, transient observations of channel resolution. Evidence contains no
//! provider rows, identifiers, names, cursor bytes or error text.

use super::{ChannelInfo, Client, Lookup, MAX_LIST_PAGES, channel_of};
use anyhow::{Result, bail};
use serde_json::Value;

#[derive(Debug)]
pub(crate) struct ChannelRead<T> {
    pub(crate) result: Result<T>,
    pub(crate) evidence: ChannelEvidence,
}

/// Info uses the first slot; name resolution uses yours and then public.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ChannelEvidence {
    pub(crate) phases: [Option<ChannelPhase>; 2],
}

impl ChannelEvidence {
    /// Parser or structural gaps, independently of how traversal ended.
    pub(crate) fn has_gaps(&self) -> bool {
        self.phases.iter().flatten().any(ChannelPhase::has_gaps)
    }

    pub(crate) fn end(&self) -> Option<ChannelEnd> {
        self.phases.iter().flatten().next_back().map(|p| p.end)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelKind {
    Info,
    Yours,
    Public,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelEnd {
    Found,
    NoMatch,
    Request,
    LocalCap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelMembership {
    Explicit,
    Defaulted,
    ListDerived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChannelPhase {
    pub(crate) kind: ChannelKind,
    /// Successful logical responses, excluding 429 retries and failed requests.
    pub(crate) requests: usize,
    /// List array occurrences, including duplicates and every row on a matched
    /// page. For info, one present `channel` slot, even if malformed; zero if absent.
    pub(crate) returned: usize,
    /// Defaults used by the eight-field parser. A matched yours row's member
    /// flag is list-derived instead, so its overwritten parser default is excluded.
    pub(crate) defaulted_fields: usize,
    /// Parsed rows with a blank id/name, or an info id different from the request.
    /// Each affected row counts once, however many identity fields have gaps.
    pub(crate) identity_gaps: usize,
    /// Successful responses with missing/malformed collections or cursor evidence.
    pub(crate) incomplete_pages: usize,
    pub(crate) end: ChannelEnd,
    /// The matched channel's membership source; absent when no match was returned.
    pub(crate) membership: Option<ChannelMembership>,
}

impl ChannelPhase {
    fn new(kind: ChannelKind) -> Self {
        Self {
            kind,
            requests: 0,
            returned: 0,
            defaulted_fields: 0,
            identity_gaps: 0,
            incomplete_pages: 0,
            end: ChannelEnd::Request,
            membership: None,
        }
    }

    pub(crate) fn has_gaps(&self) -> bool {
        self.defaulted_fields > 0 || self.identity_gaps > 0 || self.incomplete_pages > 0
    }

    fn observe(&mut self, row: &Value, parsed: &ChannelInfo, list_member: bool) {
        self.defaulted_fields += ["id", "name"]
            .into_iter()
            .filter(|key| row.get(*key).and_then(Value::as_str).is_none())
            .count();
        self.defaulted_fields += ["is_private", "is_member", "is_im", "is_archived"]
            .into_iter()
            .filter(|key| {
                !(list_member && *key == "is_member")
                    && row.get(*key).and_then(Value::as_bool).is_none()
            })
            .count();
        self.defaulted_fields += ["topic", "purpose"]
            .into_iter()
            .filter(|key| {
                row.get(*key)
                    .and_then(|nested| nested.get("value"))
                    .and_then(Value::as_str)
                    .is_none()
            })
            .count();
        self.identity_gaps +=
            usize::from(parsed.id.trim().is_empty() || parsed.name.trim().is_empty());
    }
}

fn membership(row: &Value) -> ChannelMembership {
    if row.get("is_member").and_then(Value::as_bool).is_some() {
        ChannelMembership::Explicit
    } else {
        ChannelMembership::Defaulted
    }
}

impl Client {
    /// Compatibility wrapper: provider values and raw errors are unchanged.
    pub async fn channel_info(&self, id: &str) -> Result<ChannelInfo> {
        self.channel_info_observed(id).await.result
    }

    pub(crate) async fn channel_info_observed(&self, id: &str) -> ChannelRead<ChannelInfo> {
        let mut phase = ChannelPhase::new(ChannelKind::Info);
        let req = Self::read("conversations.info", vec![("channel".into(), id.into())]);
        let result = match self.call(req).await {
            Ok((body, _)) => {
                phase.requests = 1;
                phase.returned = usize::from(body.get("channel").is_some());
                let row = body.get("channel").unwrap_or(&Value::Null);
                phase.incomplete_pages = usize::from(!row.is_object());
                let channel = channel_of(row);
                phase.observe(row, &channel, false);
                // A row counts once even if it has both blank fields and an id mismatch.
                phase.identity_gaps = usize::from(phase.identity_gaps > 0 || channel.id != id);
                phase.membership = Some(membership(row));
                phase.end = ChannelEnd::Found;
                Ok(channel)
            }
            Err(error) => Err(error),
        };
        ChannelRead {
            result,
            evidence: ChannelEvidence {
                phases: [Some(phase), None],
            },
        }
    }

    /// The channel called `name` (no leading `#`, any case): first among the
    /// channels you are in, public and private (`users.conversations`), then
    /// among every public one (`conversations.list`), each paged to its end.
    /// A private channel you are not in is in neither: Slack shows it to no
    /// one outside it.
    // Settings use the observed form; keep this compatibility wrapper.
    #[allow(dead_code)]
    pub async fn find_channel(&self, name: &str) -> Result<Lookup> {
        self.find_channel_observed(name).await.result
    }

    pub(crate) async fn find_channel_observed(&self, name: &str) -> ChannelRead<Lookup> {
        let want = name.trim().trim_start_matches('#').to_lowercase();
        let mine = vec![
            (
                "types".to_string(),
                "public_channel,private_channel".to_string(),
            ),
            ("exclude_archived".into(), "true".into()),
            ("limit".into(), "999".into()),
        ];
        let mut yours = ChannelPhase::new(ChannelKind::Yours);
        let result = self
            .list_until_observed("users.conversations", mine, &want, &mut yours)
            .await;
        let mut evidence = ChannelEvidence {
            phases: [Some(yours), None],
        };
        let mut looked = Lookup {
            yours: yours.returned,
            ..Lookup::default()
        };
        match result {
            Ok(Some(channel)) => {
                looked.found = Some(channel);
                return ChannelRead {
                    result: Ok(looked),
                    evidence,
                };
            }
            Err(error) => {
                return ChannelRead {
                    result: Err(error),
                    evidence,
                };
            }
            Ok(None) => {}
        }
        let public_params = vec![
            ("types".to_string(), "public_channel".to_string()),
            ("exclude_archived".into(), "true".into()),
            ("limit".into(), "1000".into()),
        ];
        let mut public = ChannelPhase::new(ChannelKind::Public);
        let result = self
            .list_until_observed("conversations.list", public_params, &want, &mut public)
            .await;
        evidence.phases[1] = Some(public);
        looked.public = public.returned;
        ChannelRead {
            result: result.map(|found| {
                looked.found = found;
                looked
            }),
            evidence,
        }
    }

    async fn list_until_observed(
        &self,
        method: &'static str,
        params: Vec<(String, String)>,
        want: &str,
        phase: &mut ChannelPhase,
    ) -> Result<Option<ChannelInfo>> {
        let mut cursor = String::new();
        for _ in 0..MAX_LIST_PAGES {
            let (body, next) = self.cursor_page(method, params.clone(), &cursor).await?;
            phase.requests += 1;
            let rows = body.get("channels").and_then(Value::as_array);
            let cursor_field = body
                .pointer("/response_metadata/next_cursor")
                .and_then(Value::as_str);
            phase.incomplete_pages += usize::from(
                rows.is_none()
                    || cursor_field.is_none()
                    || cursor_field.is_some_and(|s| !s.is_empty() && s.trim().is_empty()),
            );
            let mut found = None;
            for row in rows.into_iter().flatten() {
                let mut channel = channel_of(row);
                let matches = found.is_none() && channel.name.to_lowercase() == want;
                let list_member = matches && phase.kind == ChannelKind::Yours;
                phase.returned += 1;
                phase.observe(row, &channel, list_member);
                if matches {
                    phase.membership = Some(if list_member {
                        // Slack may omit is_member in users.conversations.
                        channel.is_member = true;
                        ChannelMembership::ListDerived
                    } else {
                        membership(row)
                    });
                    found = Some(channel);
                }
            }
            // The whole matched page is consumed; a match precedes cursor/end/cap checks.
            if found.is_some() {
                phase.end = ChannelEnd::Found;
                return Ok(found);
            }
            cursor = next;
            if cursor.is_empty() {
                phase.end = ChannelEnd::NoMatch;
                return Ok(None);
            }
        }
        phase.end = ChannelEnd::LocalCap;
        bail!(
            "Slack listed more than {MAX_LIST_PAGES} pages of channels; paste a link to it instead"
        )
    }
}

#[cfg(test)]
mod tests;
