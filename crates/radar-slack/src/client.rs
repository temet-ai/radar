//! A small Slack Web API client.
//!
//! Reads are paged and wait out a rate limit; writes are built by
//! [`post_request`] / [`react_request`], sent once and never retried: a write
//! that may or may not have arrived must not be sent a second time.
//!
//! Everything goes through a [`Transport`], so tests run against recorded
//! replies and never reach Slack.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod pace;
pub(crate) use pace::{Pace, Stop, all};
#[cfg(test)]
pub(crate) use pace::IN_FLIGHT;

mod channels;
pub(crate) use channels::{ChannelEnd, ChannelEvidence, ChannelKind, ChannelMembership};
#[cfg(test)]
pub(crate) use channels::ChannelPhase;

/// One call to Slack's Web API.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub method: &'static str,
    /// Query params for reads.
    pub params: Vec<(String, String)>,
    /// A write (posts, reactions): sent as a JSON body, never retried.
    pub write: bool,
    /// JSON body for writes.
    pub body: Option<Value>,
}

/// What came back.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reply {
    pub status: u16,
    /// The Retry-After header, seconds.
    pub retry_after: Option<u64>,
    /// The x-oauth-scopes header.
    pub scopes: Option<String>,
    pub body: String,
}

pub type Fut<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

pub trait Transport: Send + Sync {
    fn send<'a>(&'a self, req: &'a Request) -> Fut<'a, Result<Reply>>;
}

/// The real transport: HTTPS to `base` (`https://slack.com`), the token as a
/// Bearer header.
pub struct Http {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl Http {
    pub fn new(base: &str, token: &str) -> Result<Self> {
        radar_core::log::secret(token);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("radar")
            .build()
            .context("could not set up the HTTPS client")?;
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            token: token.trim().to_string(),
            http,
        })
    }
}

impl Transport for Http {
    fn send<'a>(&'a self, req: &'a Request) -> Fut<'a, Result<Reply>> {
        Box::pin(async move {
            let url = format!("{}/api/{}", self.base, req.method);
            let builder = if req.write {
                let body = serde_json::to_vec(req.body.as_ref().unwrap_or(&Value::Null))?;
                self.http
                    .post(&url)
                    .header("Content-Type", "application/json; charset=utf-8")
                    .body(body)
            } else {
                self.http.get(&url).query(&req.params)
            };
            radar_core::log::debug(|| {
                format!("slack: {} {}", if req.write { "POST" } else { "GET" }, req.method)
            });
            let resp = builder.bearer_auth(&self.token).send().await?;
            let status = resp.status().as_u16();
            let header = |name: &str| {
                resp.headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            };
            let retry_after = header("retry-after").and_then(|v| v.trim().parse().ok());
            let scopes = header("x-oauth-scopes");
            let body = resp.text().await?;
            Ok(Reply {
                status,
                retry_after,
                scopes,
                body,
            })
        })
    }
}

pub type Wait = Arc<dyn Fn(u64) -> Fut<'static, ()> + Send + Sync>;

#[derive(Clone)]
pub struct Client {
    transport: Arc<dyn Transport>,
    /// How to wait out a rate limit (tests pass one that records and returns
    /// at once).
    wait: Wait,
    /// Shared by every client of a plugin, so tasks that run together keep
    /// to one cap and one wait.
    pace: Arc<Pace>,
}

/// Retries of a rate-limited read.
const MAX_RETRIES: u32 = 3;
/// Longest single wait, seconds.
const MAX_WAIT: u64 = 60;
/// Most pages of a cursor-paged read.
const MAX_CURSOR_PAGES: usize = 20;
/// Most pages of a search.
const MAX_SEARCH_PAGES: usize = 5;
/// Matches to a page when a search is for what is newer than what is held.
const SEARCH_SINCE_PAGE: usize = 20;
/// Most pages of a list of channels: a large workspace's public channels
/// run to tens of pages, and a page of private ones is often nearly empty.
const MAX_LIST_PAGES: usize = 100;

/// Where a channel looked up by name was found, and how many were looked at.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lookup {
    pub found: Option<ChannelInfo>,
    /// Channels you are in, public and private.
    pub yours: usize,
    /// Public channels.
    pub public: usize,
}

/// Transient evidence about a bounded read, never part of a cached message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ListEvidence {
    /// Rows were discarded, or usable paging evidence remained unread at a local bound.
    pub(crate) truncated: bool,
    /// Successful pages whose collection or paging fields were incomplete.
    /// A page counts once, regardless of how many fields were malformed.
    pub(crate) incomplete_pages: usize,
}

impl ListEvidence {
    pub(crate) fn merge(&mut self, other: Self) {
        self.truncated |= other.truncated;
        self.incomplete_pages = self.incomplete_pages.saturating_add(other.incomplete_pages);
    }
}

#[derive(Debug)]
pub(crate) struct BootstrapRead {
    pub me: Me,
    pub timezone: TimezoneEvidence,
}

/// How the bootstrap read obtained the timezone used by the existing parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimezoneEvidence {
    /// An i64 field was returned; no range or returned-user identity validation.
    Explicit,
    /// Authentication returned no usable owner, so no profile read was sent.
    NotRequested,
    /// The profile call returned, but the legacy parser defaulted the field.
    Defaulted,
    /// The profile call failed, so whoami retained its UTC fallback.
    Failed,
}

#[derive(Debug)]
pub(crate) struct MessagesRead {
    pub(crate) messages: Vec<Message>,
    pub(crate) evidence: ListEvidence,
}

#[derive(Debug)]
pub(crate) struct SearchRead {
    pub matches: Vec<Found>,
    pub evidence: ListEvidence,
}

#[derive(Debug)]
pub(crate) struct GroupsRead {
    pub groups: Vec<UserGroup>,
    pub incomplete_collection: bool,
    /// Membership rows with one or more malformed fields, counted once each.
    pub incomplete_entries: usize,
}

impl Client {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            wait: Arc::new(|secs| Box::pin(tokio::time::sleep(Duration::from_secs(secs)))),
            pace: Arc::default(),
        }
    }

    pub(crate) fn with_pace(mut self, pace: Arc<Pace>) -> Self {
        self.pace = pace;
        self
    }

    /// Wait `secs` for `method`, once for everyone: a call that finds
    /// another already waiting on it waits for that wait, not its own.
    async fn back_off(&self, gate: &tokio::sync::Mutex<()>, secs: u64) {
        match gate.try_lock() {
            Ok(held) => {
                (self.wait)(secs).await;
                drop(held);
            }
            Err(_) => drop(gate.lock().await),
        }
    }

    pub fn with_wait(mut self, wait: Wait) -> Self {
        self.wait = wait;
        self
    }

    pub fn http(base: &str, token: &str) -> Result<Self> {
        Ok(Self::new(Arc::new(Http::new(base, token)?)))
    }

    /// Send `req`; a rate-limited read waits and tries again, a write never
    /// does.
    pub async fn call(&self, req: Request) -> Result<(Value, Reply)> {
        let mut retries = 0;
        let gate = self.pace.pause(req.method);
        loop {
            // Another call to this method may be waiting out a 429: so wait.
            drop(gate.lock().await);
            let reply = match self.transport.send(&req).await {
                Ok(r) => r,
                Err(e) if req.write => {
                    bail!("could not reach Slack; nothing was sent again: {e:#}")
                }
                Err(e) => bail!("could not reach Slack: {e:#}"),
            };
            if reply.status == 429 {
                if req.write {
                    let n = reply.retry_after.unwrap_or(1);
                    radar_core::log::warn(format!("slack: {} rate limited, not sent", req.method));
                    bail!(
                        "Slack is busy (rate limited): it was not sent, and radar does not send it again; try again in {n}s"
                    );
                }
                if retries >= MAX_RETRIES {
                    bail!("Slack is still rate limiting after {MAX_RETRIES} tries");
                }
                retries += 1;
                let secs = reply.retry_after.unwrap_or(1).min(MAX_WAIT);
                radar_core::log::info(format!(
                    "slack: {} rate limited, waiting {secs}s (retry {retries})",
                    req.method
                ));
                self.back_off(&gate, secs).await;
                continue;
            }
            // Slack down for a moment: a read waits (as long as it says,
            // else 1s, 2s, 4s) and asks again; a write is never sent twice.
            if reply.status >= 500 && !req.write && retries < MAX_RETRIES {
                let secs = reply.retry_after.unwrap_or(1 << retries).min(MAX_WAIT);
                retries += 1;
                radar_core::log::info(format!(
                    "slack: {} answered HTTP {}, waiting {secs}s (retry {retries})",
                    req.method, reply.status
                ));
                self.back_off(&gate, secs).await;
                continue;
            }
            if !(200..300).contains(&reply.status) {
                bail!("Slack answered HTTP {}", reply.status);
            }
            let body: Value = serde_json::from_str(&reply.body)
                .map_err(|e| anyhow!("Slack's reply was not JSON: {e}"))?;
            if body.get("ok").and_then(Value::as_bool) != Some(true) {
                let error = body.get("error").and_then(Value::as_str).unwrap_or("unknown_error");
                let needed = body.get("needed").and_then(Value::as_str);
                bail!("{}", explain(error, needed));
            }
            return Ok((body, reply));
        }
    }

    fn read(method: &'static str, params: Vec<(String, String)>) -> Request {
        Request {
            method,
            params,
            write: false,
            body: None,
        }
    }

    /// One page of a cursor-paged read; the next cursor, if there is one.
    async fn cursor_page(
        &self,
        method: &'static str,
        mut params: Vec<(String, String)>,
        cursor: &str,
    ) -> Result<(Value, String)> {
        if !cursor.is_empty() {
            params.push(("cursor".into(), cursor.into()));
        }
        let (body, _) = self.call(Self::read(method, params)).await?;
        let next = body
            .pointer("/response_metadata/next_cursor")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok((body, next))
    }

    pub async fn auth_test(&self) -> Result<Me> {
        let (body, reply) = self.call(Self::read("auth.test", vec![])).await?;
        let me = Me {
            user_id: text_of(&body, "user_id"),
            user: text_of(&body, "user"),
            team: text_of(&body, "team"),
            team_id: text_of(&body, "team_id"),
            url: text_of(&body, "url"),
            scopes: reply
                .scopes
                .as_deref()
                .unwrap_or("")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
            tz_offset: None,
        };
        Ok(me)
    }

    /// Who the token is, with their time zone from their profile (a token
    /// without `users:read` is counted in UTC).
    /// Retained compatibility wrapper; bootstrap callers use the observed read below.
    #[allow(dead_code)]
    pub async fn whoami(&self) -> Result<Me> {
        Ok(self.whoami_observed().await?.me)
    }

    pub(crate) async fn whoami_observed(&self) -> Result<BootstrapRead> {
        let mut me = self.auth_test().await?;
        if crate::cache::AccountOwner::of(&me).is_none() {
            return Ok(BootstrapRead { me, timezone: TimezoneEvidence::NotRequested });
        }
        let (offset, timezone) = match self.user_observed(&me.user_id).await {
            Ok((user, explicit)) => (
                user.tz_offset,
                if explicit {
                    TimezoneEvidence::Explicit
                } else {
                    TimezoneEvidence::Defaulted
                },
            ),
            Err(e) => {
                radar_core::log::warn(format!("slack: your time zone: {e:#}; counting in UTC"));
                (0, TimezoneEvidence::Failed)
            }
        };
        me.tz_offset = Some(offset);
        Ok(BootstrapRead { me, timezone })
    }

    /// Newest first, as Slack returns them; at most `max`.
    // Retain the legacy Vec wrapper; CLI reads now keep the observed evidence.
    #[allow(dead_code)]
    pub async fn history(
        &self,
        channel: &str,
        oldest: Option<&str>,
        max: usize,
    ) -> Result<Vec<Message>> {
        Ok(self.history_observed(channel, oldest, max).await?.messages)
    }

    pub(crate) async fn history_observed(
        &self,
        channel: &str,
        oldest: Option<&str>,
        max: usize,
    ) -> Result<MessagesRead> {
        let mut params = vec![("channel".to_string(), channel.to_string())];
        if let Some(o) = oldest {
            params.push(("oldest".into(), o.into()));
        }
        params.push(("inclusive".into(), "false".into()));
        params.push(("limit".into(), max.clamp(1, 200).to_string()));
        self.messages_paged("conversations.history", params, max)
            .await
    }

    /// The parent first, then its replies.
    // Retain the legacy Vec wrapper; page reads now use its observed counterpart.
    #[allow(dead_code)]
    pub async fn replies(&self, channel: &str, ts: &str, max: usize) -> Result<Vec<Message>> {
        Ok(self.replies_observed(channel, ts, max).await?.messages)
    }

    pub(crate) async fn replies_observed(
        &self,
        channel: &str,
        ts: &str,
        max: usize,
    ) -> Result<MessagesRead> {
        self.replies_after(channel, ts, None, max).await
    }

    /// A thread's replies newer than `oldest` (all of them without it), and
    /// the parent when Slack sends it along.
    pub(crate) async fn replies_after(
        &self,
        channel: &str,
        ts: &str,
        oldest: Option<&str>,
        max: usize,
    ) -> Result<MessagesRead> {
        let mut params = vec![
            ("channel".to_string(), channel.to_string()),
            ("ts".into(), ts.into()),
        ];
        if let Some(o) = oldest {
            params.push(("oldest".into(), o.into()));
            params.push(("inclusive".into(), "false".into()));
        }
        params.push(("limit".into(), "200".into()));
        self.messages_paged("conversations.replies", params, max)
            .await
    }

    async fn messages_paged(
        &self,
        method: &'static str,
        params: Vec<(String, String)>,
        max: usize,
    ) -> Result<MessagesRead> {
        let mut out: Vec<Message> = Vec::new();
        let mut evidence = ListEvidence::default();
        let mut cursor = String::new();
        for _ in 0..MAX_CURSOR_PAGES {
            if out.len() >= max {
                break;
            }
            let (body, next) = self.cursor_page(method, params.clone(), &cursor).await?;
            let items = body.get("messages").and_then(Value::as_array);
            // Assess evidence without rewriting the legacy cursor bytes or
            // changing whether the next request is made.
            let usable_next = !next.trim().is_empty();
            let incomplete = items.is_none()
                || (!next.is_empty() && !usable_next)
                || body.get("response_metadata").is_some_and(|metadata| {
                    !metadata.is_object()
                        || metadata.get("next_cursor").is_some_and(|v| !v.is_string())
                })
                || body.get("has_more").is_some_and(|more| {
                    more.as_bool().is_none() || (more.as_bool() == Some(true) && !usable_next)
                });
            evidence.incomplete_pages += usize::from(incomplete);
            if let Some(items) = items {
                // Preserve the permissive parser and every legacy row. This
                // evidence describes the collection, not message validity.
                out.extend(items.iter().map(message));
            }
            cursor = next;
            if cursor.is_empty() {
                break;
            }
        }
        evidence.truncated = out.len() > max || !cursor.trim().is_empty();
        out.truncate(max);
        Ok(MessagesRead {
            messages: out,
            evidence,
        })
    }

    /// Retained compatibility wrapper; sync callers use the observed read below.
    #[allow(dead_code)]
    pub async fn search(&self, query: &str, max: usize) -> Result<Vec<Found>> {
        Ok(self.search_observed(query, max).await?.matches)
    }

    pub(crate) async fn search_observed(&self, query: &str, max: usize) -> Result<SearchRead> {
        self.search_paged(query, max, max.clamp(1, 100), &|_| false)
            .await
    }

    /// What is newer than what is held: the newest match first, as one small
    /// read; when that is held already, nothing is newer and that is all it
    /// cost. When it is not, the matches again from the newest, a page of
    /// [`SEARCH_SINCE_PAGE`] at a time, until one that is held.
    pub(crate) async fn search_since(
        &self,
        query: &str,
        max: usize,
        held: &(dyn Fn(&Found) -> bool + Sync),
    ) -> Result<SearchRead> {
        let probe = self.search_paged(query, 1, 1, held).await?;
        if probe.matches.first().is_none_or(held) {
            return Ok(probe);
        }
        let mut wide = self.search_paged(query, max, SEARCH_SINCE_PAGE, held).await?;
        wide.evidence.incomplete_pages += probe.evidence.incomplete_pages;
        Ok(wide)
    }

    /// Pages of `per_page` matches, newest first, up to `max` of them; a
    /// page with a match `held` says is held is the last (what follows it
    /// is older, and held too), and is not a cut-off.
    async fn search_paged(
        &self,
        query: &str,
        max: usize,
        per_page: usize,
        held: &(dyn Fn(&Found) -> bool + Sync),
    ) -> Result<SearchRead> {
        let mut out: Vec<Found> = Vec::new();
        let mut evidence = ListEvidence::default();
        for page in 1..=MAX_SEARCH_PAGES {
            if out.len() >= max {
                break;
            }
            let params = vec![
                ("query".to_string(), query.to_string()),
                ("count".into(), per_page.to_string()),
                ("sort".into(), "timestamp".into()),
                ("sort_dir".into(), "desc".into()),
                ("page".into(), page.to_string()),
            ];
            let (body, _) = self.call(Self::read("search.messages", params)).await?;
            let items = body.pointer("/messages/matches").and_then(Value::as_array);
            let matches = items.map(Vec::as_slice).unwrap_or_default();
            let page_found: Vec<Found> = matches.iter().map(found).collect();
            let reached_held = page_found.iter().any(held);
            out.extend(page_found);
            let reported_pages = body
                .pointer("/messages/paging/pages")
                .and_then(Value::as_u64);
            let pages = reported_pages.unwrap_or(1);
            let unread_pages = pages > page as u64;
            let incomplete = items.is_none()
                || body.pointer("/messages/paging").is_some_and(|paging| {
                    !paging.is_object() || paging.get("pages").is_some_and(|v| v.as_u64().is_none())
                })
                || (matches.is_empty() && unread_pages)
                || reported_pages
                    .is_some_and(|pages| pages < page as u64 && (!matches.is_empty() || page > 1));
            evidence.incomplete_pages += usize::from(incomplete);
            // An empty nonfinal page keeps the legacy stop, but does not prove
            // that a local row or page bound discarded further results.
            evidence.truncated |= out.len() > max
                || (!reached_held
                    && !matches.is_empty()
                    && unread_pages
                    && (out.len() >= max || page == MAX_SEARCH_PAGES));
            if reached_held || matches.is_empty() || page as u64 >= pages {
                break;
            }
        }
        out.truncate(max);
        Ok(SearchRead {
            matches: out,
            evidence,
        })
    }

    pub async fn user(&self, id: &str) -> Result<User> {
        Ok(self.user_observed(id).await?.0)
    }

    /// The name shown for `id`, asked for once per round of reading however
    /// many tasks want it, a few at a time. `None`: `stop` was set (by an
    /// earlier failure) before this one was sent.
    pub(crate) async fn user_name(&self, id: &str, stop: &Stop) -> Option<Result<String, String>> {
        let cell = self.pace.lookup(id);
        if let Some(done) = cell.get() {
            return Some(done.clone());
        }
        let turn = self.pace.name_turn().await;
        if cell.get().is_none() && stop.is_set() {
            return None;
        }
        let got = cell
            .get_or_init(|| async { self.user(id).await.map(|u| u.shown()).map_err(|e| format!("{e:#}")) })
            .await
            .clone();
        drop(turn);
        Some(got)
    }

    /// A place among the `conversations.replies` reads in flight.
    pub(crate) async fn replies_turn(&self) -> tokio::sync::SemaphorePermit<'_> {
        self.pace.replies_turn().await
    }

    async fn user_observed(&self, id: &str) -> Result<(User, bool)> {
        let req = Self::read("users.info", vec![("user".into(), id.into())]);
        let (body, _) = self.call(req).await?;
        let u = body.get("user").unwrap_or(&Value::Null);
        let profile = u.get("profile").unwrap_or(&Value::Null);
        let pick = |k: &str| {
            let p = text_of(profile, k);
            if p.is_empty() { text_of(u, k) } else { p }
        };
        let tz_offset = u.get("tz_offset").and_then(Value::as_i64);
        Ok((
            User {
                id: text_of(u, "id"),
                name: text_of(u, "name"),
                real_name: pick("real_name"),
                display_name: pick("display_name"),
                tz_offset: tz_offset.unwrap_or(0),
            },
            tz_offset.is_some(),
        ))
    }

    /// Retained compatibility wrapper; settings and sync use observed results.
    #[allow(dead_code)]
    pub async fn usergroups(&self) -> Result<Vec<UserGroup>> {
        Ok(self.usergroups_observed().await?.groups)
    }

    pub(crate) async fn usergroups_observed(&self) -> Result<GroupsRead> {
        let req = Self::read(
            "usergroups.list",
            vec![("include_users".into(), "true".into())],
        );
        let (body, _) = self.call(req).await?;
        let items = body.get("usergroups").and_then(Value::as_array);
        let mut incomplete_entries = 0;
        let groups = items
            .into_iter()
            .flatten()
            .map(|g| {
                let incomplete = !g.is_object()
                    || g.get("id")
                        .and_then(Value::as_str)
                        .is_none_or(str::is_empty)
                    || g.get("users")
                        .and_then(Value::as_array)
                        .is_none_or(|users| {
                            users.iter().any(|u| u.as_str().is_none_or(str::is_empty))
                        });
                incomplete_entries += usize::from(incomplete);
                UserGroup {
                    id: text_of(g, "id"),
                    handle: text_of(g, "handle"),
                    name: text_of(g, "name"),
                    users: strings(g.get("users")),
                }
            })
            .collect();
        Ok(GroupsRead {
            groups,
            incomplete_collection: items.is_none(),
            incomplete_entries,
        })
    }

    /// Posts once; never retried.
    pub async fn post(&self, channel: &str, text: &str, thread_ts: Option<&str>) -> Result<Posted> {
        let (body, _) = self.call(post_request(channel, text, thread_ts)).await?;
        Ok(Posted {
            channel: text_of(&body, "channel"),
            ts: text_of(&body, "ts"),
        })
    }

    /// Reacts once; never retried.
    pub async fn react(&self, channel: &str, ts: &str, name: &str) -> Result<()> {
        self.call(react_request(channel, ts, name)).await?;
        Ok(())
    }
}

/// What `chat.postMessage` is sent, so a caller can show or test it.
pub fn post_request(channel: &str, text: &str, thread_ts: Option<&str>) -> Request {
    let mut body = json!({ "channel": channel, "text": text });
    if let Some(t) = thread_ts {
        body["thread_ts"] = json!(t);
    }
    body["unfurl_links"] = json!(false);
    body["unfurl_media"] = json!(false);
    Request {
        method: "chat.postMessage",
        params: vec![],
        write: true,
        body: Some(body),
    }
}

/// What `reactions.add` is sent.
pub fn react_request(channel: &str, ts: &str, name: &str) -> Request {
    Request {
        method: "reactions.add",
        params: vec![],
        write: true,
        body: Some(json!({ "channel": channel, "timestamp": ts, "name": name })),
    }
}

/// Slack's error code in words a person can act on.
pub fn explain(error: &str, needed: Option<&str>) -> String {
    match error {
        "not_authed" | "invalid_auth" => "the token was refused (invalid_auth)".to_string(),
        "token_revoked" | "token_expired" | "account_inactive" => {
            format!("the token no longer works ({error})")
        }
        "missing_scope" => format!(
            "the token lacks a permission: {}",
            needed.unwrap_or("(Slack did not say which)")
        ),
        "channel_not_found" => "no such channel, or the token cannot see it".to_string(),
        "not_in_channel" => "you are not in that channel".to_string(),
        "ratelimited" => "rate limited".to_string(),
        other => format!("Slack says {other}"),
    }
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}
fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Reaction {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub count: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub users: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Message {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    /// User id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// `username` or `bot_profile.name` for bot posts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub reply_count: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reply_users: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_reply: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,
    /// Files and images attached, each with its link.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<File>,
    /// Messages shared into it (a link to another message, unfurled).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shared: Vec<Shared>,
    /// An "edited" object was present.
    #[serde(skip_serializing_if = "is_false")]
    pub edited: bool,
    /// What a bot's attachments say, kept as they came (a bot's message
    /// only): the alert readers work from these.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cards: Vec<Card>,
    /// What a bot's blocks say, in order (a bot's message only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<Block>,
}

/// One of a bot's attachments: colour, title and its link, text, and fields.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Card {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub color: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title_link: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pretext: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fallback: String,
    /// Each field's title and value.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<(String, String)>,
}

/// One of a bot's blocks: its type (`header`, `section`, `context`,
/// `actions`) and every text in it (a button as `<url|text>`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Block {
    pub kind: String,
    pub texts: Vec<String>,
}

/// A file or image attached to a message: its name and where it is, and for
/// an image what to fetch to draw it (a terminal that cannot draws a line
/// naming it and its link).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(from = "FileKept")]
pub struct File {
    pub name: String,
    /// Its page in Slack (`permalink`), else its own address.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(skip_serializing_if = "is_false")]
    pub image: bool,
    /// An image's picture: Slack's largest thumbnail up to 1024 wide, else
    /// the file itself (`url_private`, fetched with the token).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub src: String,
    /// An image's size in pixels, as Slack gives it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<(u32, u32)>,
}

/// A file as a cache holds it: a name alone (older caches), or whole.
#[derive(Deserialize)]
#[serde(untagged)]
enum FileKept {
    Name(String),
    Whole {
        #[serde(default)]
        name: String,
        #[serde(default)]
        url: String,
        #[serde(default)]
        image: bool,
        #[serde(default)]
        src: String,
        #[serde(default)]
        size: Option<(u32, u32)>,
    },
}

impl From<FileKept> for File {
    fn from(f: FileKept) -> Self {
        match f {
            FileKept::Name(name) => Self { name, ..Self::default() },
            FileKept::Whole { name, url, image, src, size } => Self { name, url, image, src, size },
        }
    }
}

/// Whether `url` is one of Slack's own hosts, the only ones the token is
/// sent to: `slack.com` and its subdomains over https, and the host of the
/// configured API `api` (`[slack] api`) and its subdomains. Another host
/// gets nothing. An API address that is a bare IP or `localhost` matches
/// only itself, on its scheme and port.
pub fn slack_host(url: &str, api: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else { return false };
    let Some(host) = u.host_str() else { return false };
    if u.scheme() == "https" && (host == "slack.com" || host.ends_with(".slack.com")) {
        return true;
    }
    let Ok(a) = reqwest::Url::parse(api) else { return false };
    let Some(api_host) = a.host_str() else { return false };
    // Scheme and port are the API's own; credentials are never part of it.
    if u.scheme() != a.scheme()
        || u.port_or_known_default() != a.port_or_known_default()
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return false;
    }
    host == api_host
        || (api_host != "localhost"
            && api_host.parse::<std::net::IpAddr>().is_err()
            && host.ends_with(&format!(".{api_host}")))
}

/// A picture's bytes: `src` fetched with the token when it is on Slack's
/// own hosts (`files.slack.com` wants it) or the configured API's, without
/// it anywhere else.
pub async fn picture(src: &str, token: Option<&str>, api: &str) -> Result<Vec<u8>> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("radar")
        .build()?;
    let mut req = http.get(src);
    if slack_host(src, api)
        && let Some(t) = token
    {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await?;
    let status = resp.status();
    if !status.is_success() {
        bail!("Slack answered {status}");
    }
    // A sign-in page instead of a picture: the token cannot read files.
    let kind = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if kind.starts_with("text/html") {
        bail!("not a picture ({kind}): the token may lack files:read");
    }
    Ok(resp.bytes().await?.to_vec())
}

/// What to fetch to draw a file that is an image, and its size: the largest
/// thumbnail Slack made that is at most 1024 wide (scaled to it), else the
/// file itself.
fn picture_of(f: &Value) -> (String, Option<(u32, u32)>) {
    let num = |k: &str| f.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
    let size = num("original_w").zip(num("original_h"));
    for w in [1024, 960, 800, 720, 480, 360] {
        if let Some(url) = f.get(format!("thumb_{w}")).and_then(Value::as_str).filter(|u| !u.is_empty()) {
            let tw = num(&format!("thumb_{w}_w"));
            let th = num(&format!("thumb_{w}_h"));
            return (url.to_string(), tw.zip(th).or(size));
        }
    }
    (f.get("url_private").and_then(Value::as_str).unwrap_or_default().to_string(), size)
}

/// A message shared into another (Slack unfurls a link to a message, or
/// "share message"): who wrote it, its text (mrkdwn) and its link.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Shared {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Me {
    pub user_id: String,
    pub user: String,
    pub team: String,
    pub team_id: String,
    pub url: String,
    /// From the x-oauth-scopes header.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// Your time zone in Slack, seconds east of UTC (from `users.info`):
    /// working days and times of day are counted in it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tz_offset: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelInfo {
    pub id: String,
    pub name: String,
    pub is_private: bool,
    pub is_member: bool,
    pub is_im: bool,
    pub is_archived: bool,
    pub topic: String,
    pub purpose: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct User {
    pub id: String,
    pub name: String,
    pub real_name: String,
    pub display_name: String,
    /// Seconds east of UTC.
    pub tz_offset: i64,
}

impl User {
    /// Display name, else real name, else handle, else id.
    pub fn shown(&self) -> String {
        [&self.display_name, &self.real_name, &self.name, &self.id]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UserGroup {
    pub id: String,
    pub handle: String,
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub users: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Found {
    pub channel_id: String,
    pub channel_name: String,
    pub is_im: bool,
    pub is_mpim: bool,
    pub permalink: String,
    pub message: Message,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Posted {
    pub channel: String,
    pub ts: String,
}

fn text_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn opt_of(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(String::from))
        .collect()
}

fn channel_of(c: &Value) -> ChannelInfo {
    let nested = |k: &str| {
        c.get(k)
            .and_then(|t| t.get("value"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    ChannelInfo {
        id: text_of(c, "id"),
        name: text_of(c, "name"),
        is_private: flag(c, "is_private"),
        is_member: flag(c, "is_member"),
        is_im: flag(c, "is_im"),
        is_archived: flag(c, "is_archived"),
        topic: nested("topic"),
        purpose: nested("purpose"),
    }
}

/// A bot's text when it sent only attachments: the title (`*title*`) first,
/// then the text or fallback, one per line.
fn attachment_text(v: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    for a in v.get("attachments").and_then(Value::as_array).into_iter().flatten() {
        if let Some(t) = opt_of(a, "title") {
            lines.push(format!("*{t}*"));
        }
        if let Some(t) = opt_of(a, "text").or_else(|| opt_of(a, "fallback")) {
            lines.push(t);
        }
    }
    lines.join("\n")
}

/// A bot's attachments as they came, for the alert readers.
fn cards_of(v: &Value) -> Vec<Card> {
    v.get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|a| Card {
            color: text_of(a, "color"),
            title: text_of(a, "title"),
            title_link: text_of(a, "title_link"),
            pretext: text_of(a, "pretext"),
            text: text_of(a, "text"),
            fallback: text_of(a, "fallback"),
            fields: a
                .get("fields")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|f| (text_of(f, "title"), text_of(f, "value")))
                .collect(),
        })
        .collect()
}

/// A bot's blocks, each with its type and the texts in it.
fn blocks_of(v: &Value) -> Vec<Block> {
    let text_at = |b: &Value, key: &str| b.get(key).and_then(|t| t.get("text")).and_then(Value::as_str).map(String::from);
    let button = |b: &Value| {
        let url = b.get("url").and_then(Value::as_str)?;
        let label = b
            .get("text")
            .and_then(|t| t.get("text"))
            .and_then(Value::as_str)
            .unwrap_or(url);
        Some(format!("<{url}|{label}>"))
    };
    v.get("blocks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|b| {
            let mut texts: Vec<String> = text_at(b, "text").into_iter().collect();
            for f in b.get("fields").and_then(Value::as_array).into_iter().flatten() {
                texts.extend(f.get("text").and_then(Value::as_str).map(String::from));
            }
            if let Some(a) = b.get("accessory") {
                texts.extend(button(a));
            }
            for e in b.get("elements").and_then(Value::as_array).into_iter().flatten() {
                match e.get("type").and_then(Value::as_str) {
                    Some("button") => texts.extend(button(e)),
                    _ => texts.extend(e.get("text").and_then(Value::as_str).map(String::from)),
                }
            }
            Block { kind: text_of(b, "type"), texts }
        })
        .filter(|b| !b.texts.is_empty())
        .collect()
}

/// One message from Slack's JSON, by hand: the cache format is ours, not
/// Slack's.
pub fn message(v: &Value) -> Message {
    let bot_name = opt_of(v, "username").or_else(|| {
        v.get("bot_profile")
            .and_then(|b| opt_of(b, "name"))
    });
    let is_bot = bot_name.is_some()
        || v.get("bot_id").is_some()
        || v.get("subtype").and_then(Value::as_str) == Some("bot_message");
    let mut text = text_of(v, "text");
    let from_attachments = text.is_empty() && is_bot;
    if from_attachments {
        text = attachment_text(v);
    }
    let reactions = v
        .get("reactions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|r| Reaction {
            name: text_of(r, "name"),
            count: r
                .get("count")
                .and_then(Value::as_u64)
                .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX)),
            users: strings(r.get("users")),
        })
        .collect();
    let mut files: Vec<File> = v
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let image = text_of(f, "mimetype").starts_with("image/");
            let (src, size) = if image { picture_of(f) } else { (String::new(), None) };
            Some(File {
                name: opt_of(f, "name").or_else(|| opt_of(f, "title"))?,
                url: opt_of(f, "permalink").or_else(|| opt_of(f, "url_private")).unwrap_or_default(),
                image,
                src,
                size,
            })
        })
        .collect();
    // Images in blocks.
    for b in v.get("blocks").and_then(Value::as_array).into_iter().flatten() {
        if b.get("type").and_then(Value::as_str) == Some("image")
            && let Some(url) = opt_of(b, "image_url")
        {
            let name = b
                .pointer("/title/text")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .or_else(|| opt_of(b, "alt_text"))
                .unwrap_or_else(|| "image".into());
            let size = b
                .get("image_width")
                .and_then(Value::as_u64)
                .zip(b.get("image_height").and_then(Value::as_u64))
                .map(|(w, h)| (w as u32, h as u32));
            files.push(File { name, src: url.clone(), url, image: true, size });
        }
    }
    // Attachments, when the text is not already made of them: messages
    // shared into this one, and images. A link's own preview is left out:
    // the link is in the text.
    let mut shared = Vec::new();
    for a in v.get("attachments").and_then(Value::as_array).into_iter().flatten() {
        if from_attachments {
            break;
        }
        let share = flag(a, "is_msg_unfurl") || flag(a, "is_share");
        if share {
            shared.push(Shared {
                author: opt_of(a, "author_name").or_else(|| opt_of(a, "author_subname")).unwrap_or_default(),
                text: opt_of(a, "text").unwrap_or_default(),
                url: opt_of(a, "from_url").or_else(|| opt_of(a, "original_url")).unwrap_or_default(),
            });
        } else if let Some(url) = opt_of(a, "image_url") {
            let name = opt_of(a, "title").or_else(|| opt_of(a, "fallback")).unwrap_or_else(|| "image".into());
            let size = a
                .get("image_width")
                .and_then(Value::as_u64)
                .zip(a.get("image_height").and_then(Value::as_u64))
                .map(|(w, h)| (w as u32, h as u32));
            files.push(File { name, src: url.clone(), url, image: true, size });
        }
    }
    let (cards, blocks) = if is_bot { (cards_of(v), blocks_of(v)) } else { (vec![], vec![]) };
    Message {
        cards,
        blocks,
        ts: text_of(v, "ts"),
        thread_ts: opt_of(v, "thread_ts"),
        user: opt_of(v, "user"),
        bot_name,
        subtype: opt_of(v, "subtype"),
        text,
        reply_count: v
            .get("reply_count")
            .and_then(Value::as_u64)
            .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX)),
        reply_users: strings(v.get("reply_users")),
        latest_reply: opt_of(v, "latest_reply"),
        reactions,
        files,
        shared,
        edited: v.get("edited").is_some_and(Value::is_object),
    }
}

fn found(m: &Value) -> Found {
    let ch = m.get("channel").unwrap_or(&Value::Null);
    Found {
        channel_id: text_of(ch, "id"),
        channel_name: text_of(ch, "name"),
        is_im: flag(ch, "is_im"),
        is_mpim: flag(ch, "is_mpim"),
        permalink: text_of(m, "permalink"),
        message: message(m),
    }
}

/// The test transport: recorded replies, handed out in order; every request
/// is kept.
#[cfg(test)]
pub(crate) struct Recorded {
    pub replies: std::sync::Mutex<std::collections::VecDeque<Result<Reply>>>,
    pub seen: std::sync::Mutex<Vec<Request>>,
}

#[cfg(test)]
impl Recorded {
    pub fn new(replies: Vec<Result<Reply>>) -> Arc<Self> {
        Arc::new(Self {
            replies: std::sync::Mutex::new(replies.into()),
            seen: std::sync::Mutex::new(Vec::new()),
        })
    }

    pub fn requests(&self) -> Vec<Request> {
        self.seen.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl Transport for Recorded {
    fn send<'a>(&'a self, req: &'a Request) -> Fut<'a, Result<Reply>> {
        self.seen.lock().unwrap().push(req.clone());
        let next = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(anyhow!("no recorded reply left")));
        Box::pin(async move { next })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn ok(body: Value) -> Result<Reply> {
        Ok(Reply {
            status: 200,
            body: body.to_string(),
            ..Reply::default()
        })
    }

    fn limited(secs: Option<u64>) -> Result<Reply> {
        Ok(Reply {
            status: 429,
            retry_after: secs,
            body: String::new(),
            ..Reply::default()
        })
    }

    fn setup(replies: Vec<Result<Reply>>) -> (Client, Arc<Recorded>, Arc<Mutex<Vec<u64>>>) {
        let rec = Recorded::new(replies);
        let waits = Arc::new(Mutex::new(Vec::new()));
        let w = waits.clone();
        let client = Client::new(rec.clone()).with_wait(Arc::new(move |s| {
            w.lock().unwrap().push(s);
            Box::pin(async {})
        }));
        (client, rec, waits)
    }

    fn has(req: &Request, k: &str, v: &str) -> bool {
        req.params.iter().any(|(a, b)| a == k && b == v)
    }

    fn msg(ts: &str, text: &str) -> Value {
        json!({ "type": "message", "user": "U1", "ts": ts, "text": text })
    }

    #[tokio::test]
    async fn auth_test_reads_scopes_from_the_header() {
        let reply = Reply {
            status: 200,
            scopes: Some("channels:history, search:read".into()),
            body: json!({"ok": true, "user_id": "U1", "user": "alice", "team": "Acme",
                "team_id": "T1", "url": "https://acme.slack.com/"})
            .to_string(),
            ..Reply::default()
        };
        let (c, _, _) = setup(vec![Ok(reply)]);
        let me = c.auth_test().await.unwrap();
        assert_eq!(me.user_id, "U1");
        assert_eq!(me.url, "https://acme.slack.com/");
        assert_eq!(me.scopes, vec!["channels:history", "search:read"]);
    }

    fn bootstrap_auth_reply() -> Result<Reply> {
        Ok(Reply {
            status: 200,
            scopes: Some("channels:history, users:read,, ".into()),
            body: json!({"ok": true, "user_id": "U1", "user": "alice", "team": "Acme",
                "team_id": "T1", "url": "https://acme.slack.com/"})
            .to_string(),
            ..Reply::default()
        })
    }

    fn bootstrap_me(offset: i64) -> Me {
        Me {
            user_id: "U1".into(),
            user: "alice".into(),
            team: "Acme".into(),
            team_id: "T1".into(),
            url: "https://acme.slack.com/".into(),
            scopes: vec!["channels:history".into(), "users:read".into()],
            tz_offset: Some(offset),
        }
    }

    fn bootstrap_failures(kind: &str) -> Vec<Result<Reply>> {
        match kind {
            "transport" => vec![Err(anyhow!("network stopped"))],
            "provider" => vec![ok(
                json!({"ok": false, "error": "missing_scope", "needed": "users:read"}),
            )],
            "http" => (0..=MAX_RETRIES)
                .map(|_| {
                    Ok(Reply {
                        status: 503,
                        ..Reply::default()
                    })
                })
                .collect(),
            "json" => vec![Ok(Reply {
                status: 200,
                body: "not JSON".into(),
                ..Reply::default()
            })],
            "rate_limit" => (0..=MAX_RETRIES).map(|_| limited(None)).collect(),
            _ => unreachable!(),
        }
    }

    fn profile_request() -> Request {
        Client::read("users.info", vec![("user".into(), "U1".into())])
    }

    #[tokio::test]
    async fn observed_bootstrap_auth_failure_never_requests_profile() {
        for kind in ["transport", "provider", "http", "json", "rate_limit"] {
            let (plain, plain_rec, plain_waits) = setup(bootstrap_failures(kind));
            let (c, rec, waits) = setup(bootstrap_failures(kind));
            let error = c.whoami_observed().await.unwrap_err().to_string();
            assert_eq!(plain.whoami().await.unwrap_err().to_string(), error);
            let count = if matches!(kind, "rate_limit" | "http") {
                MAX_RETRIES as usize + 1
            } else {
                1
            };
            assert_eq!(
                rec.requests(),
                vec![Client::read("auth.test", vec![]); count]
            );
            assert_eq!(plain_rec.requests(), rec.requests());
            assert_eq!(*plain_waits.lock().unwrap(), *waits.lock().unwrap());
            assert_eq!(
                waits.lock().unwrap().len(),
                if matches!(kind, "rate_limit" | "http") {
                    MAX_RETRIES as usize
                } else {
                    0
                }
            );
        }
    }

    #[tokio::test]
    async fn observed_bootstrap_profile_failure_keeps_identity_and_utc_fallback() {
        for kind in ["transport", "provider", "http", "json", "rate_limit"] {
            let replies = || {
                let mut replies = vec![bootstrap_auth_reply()];
                replies.extend(bootstrap_failures(kind));
                replies
            };
            let (plain, plain_rec, plain_waits) = setup(replies());
            let (c, rec, waits) = setup(replies());
            let got = c.whoami_observed().await.unwrap();
            assert_eq!(got.me, bootstrap_me(0));
            assert_eq!(got.timezone, TimezoneEvidence::Failed);
            assert_eq!(plain.whoami().await.unwrap(), got.me);
            let count = if matches!(kind, "rate_limit" | "http") {
                MAX_RETRIES as usize + 1
            } else {
                1
            };
            let mut requests = vec![Client::read("auth.test", vec![])];
            requests.extend(vec![profile_request(); count]);
            assert_eq!(rec.requests(), requests);
            assert_eq!(plain_rec.requests(), requests);
            assert_eq!(*plain_waits.lock().unwrap(), *waits.lock().unwrap());
            assert_eq!(
                waits.lock().unwrap().len(),
                if matches!(kind, "rate_limit" | "http") {
                    MAX_RETRIES as usize
                } else {
                    0
                }
            );
        }
    }

    #[tokio::test]
    async fn observed_bootstrap_explicit_integer_offsets_include_zero_and_i64_limits() {
        for offset in [i64::MIN, -28800, 0, 19800, i64::MAX] {
            let body = json!({"ok": true, "user": {"tz_offset": offset}});
            let (plain, plain_rec, _) = setup(vec![bootstrap_auth_reply(), ok(body.clone())]);
            let (c, rec, _) = setup(vec![bootstrap_auth_reply(), ok(body)]);
            let got = c.whoami_observed().await.unwrap();
            assert_eq!(got.me, bootstrap_me(offset));
            assert_eq!(got.timezone, TimezoneEvidence::Explicit);
            assert_eq!(plain.whoami().await.unwrap(), got.me);
            assert_eq!(
                rec.requests(),
                [Client::read("auth.test", vec![]), profile_request()]
            );
            assert_eq!(plain_rec.requests(), rec.requests());
        }
    }

    #[tokio::test]
    async fn observed_bootstrap_absent_or_malformed_offset_or_user_defaults_to_utc() {
        let mut bodies = vec![
            json!({"ok": true}),
            json!({"ok": true, "user": {}}),
            json!({"ok": true, "user": {"profile": {"tz_offset": 3600}}}),
        ];
        for malformed in [
            Value::Null,
            json!(false),
            json!(1),
            json!("user"),
            json!([]),
        ] {
            bodies.push(json!({"ok": true, "user": malformed}));
        }
        for malformed in [
            Value::Null,
            json!(false),
            json!("0"),
            json!("3600"),
            json!(0.0),
            json!(1.5),
            json!([]),
            json!({}),
            json!(u64::MAX),
            serde_json::from_str::<Value>("-9223372036854775809").unwrap(),
        ] {
            bodies.push(json!({"ok": true, "user": {"tz_offset": malformed}}));
        }
        for body in bodies {
            let (plain, plain_rec, _) = setup(vec![bootstrap_auth_reply(), ok(body.clone())]);
            let (c, rec, _) = setup(vec![bootstrap_auth_reply(), ok(body.clone())]);
            let got = c.whoami_observed().await.unwrap();
            assert_eq!(got.me, bootstrap_me(0), "{body}");
            assert_eq!(got.timezone, TimezoneEvidence::Defaulted, "{body}");
            assert_eq!(plain.whoami().await.unwrap(), got.me);
            assert_eq!(
                rec.requests(),
                [Client::read("auth.test", vec![]), profile_request()]
            );
            assert_eq!(plain_rec.requests(), rec.requests());
        }
    }

    #[tokio::test]
    async fn observed_bootstrap_wrappers_preserve_auth_then_profile_retry_order() {
        let replies = || {
            vec![
                limited(Some(7)),
                bootstrap_auth_reply(),
                limited(Some(500)),
                ok(json!({"ok": true, "user": {"tz_offset": -3600}})),
            ]
        };
        let (plain, plain_rec, plain_waits) = setup(replies());
        let (c, rec, waits) = setup(replies());
        let got = c.whoami_observed().await.unwrap();
        assert_eq!(got.me, bootstrap_me(-3600));
        assert_eq!(got.timezone, TimezoneEvidence::Explicit);
        assert_eq!(plain.whoami().await.unwrap(), got.me);
        let requests = [
            Client::read("auth.test", vec![]),
            Client::read("auth.test", vec![]),
            profile_request(),
            profile_request(),
        ];
        assert_eq!(rec.requests(), requests);
        assert_eq!(plain_rec.requests(), requests);
        assert_eq!(*waits.lock().unwrap(), [7, 60]);
        assert_eq!(*plain_waits.lock().unwrap(), [7, 60]);
    }

    #[tokio::test]
    async fn observed_user_wrapper_keeps_profile_precedence_and_permissive_parser() {
        for (profile, explicit) in [
            (
                json!({"real_name": "Profile real", "display_name": "Profile display"}),
                true,
            ),
            (json!({"real_name": "", "display_name": ""}), false),
            (Value::Null, true),
            (json!(false), false),
            (json!([]), true),
        ] {
            let body = json!({"ok": true, "user": {"id": "U1", "name": "alice",
                "real_name": "User real", "display_name": "User display", "profile": profile,
                "tz_offset": if explicit { json!(-3600) } else { json!("-3600") }}});
            let (plain, plain_rec, _) = setup(vec![ok(body.clone())]);
            let (c, rec, _) = setup(vec![ok(body)]);
            let (user, got_explicit) = c.user_observed("U1").await.unwrap();
            let has_profile_name =
                profile.get("real_name").and_then(Value::as_str) == Some("Profile real");
            assert_eq!(
                user,
                User {
                    id: "U1".into(),
                    name: "alice".into(),
                    real_name: if has_profile_name {
                        "Profile real".into()
                    } else {
                        "User real".into()
                    },
                    display_name: if has_profile_name {
                        "Profile display".into()
                    } else {
                        "User display".into()
                    },
                    tz_offset: if explicit { -3600 } else { 0 }
                }
            );
            assert_eq!(got_explicit, explicit);
            assert_eq!(plain.user("U1").await.unwrap(), user);
            assert_eq!(rec.requests(), [profile_request()]);
            assert_eq!(plain_rec.requests(), rec.requests());
        }
    }

    #[tokio::test]
    async fn history_follows_the_cursor() {
        let p1 = json!({"ok": true, "messages": [msg("3.0", "c"), msg("2.0", "b")],
            "response_metadata": {"next_cursor": "c2"}});
        let p2 = json!({"ok": true, "messages": [msg("1.0", "a")],
            "response_metadata": {"next_cursor": ""}});
        let (c, rec, _) = setup(vec![ok(p1), ok(p2)]);
        let got = c.history("C1", Some("0.5"), 50).await.unwrap();
        let texts: Vec<_> = got.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["c", "b", "a"]);
        let seen = rec.requests();
        assert_eq!(seen.len(), 2);
        assert!(has(&seen[0], "channel", "C1") && has(&seen[0], "oldest", "0.5"));
        assert!(has(&seen[0], "inclusive", "false") && has(&seen[0], "limit", "50"));
        assert!(!seen[0].params.iter().any(|(k, _)| k == "cursor"));
        assert!(has(&seen[1], "cursor", "c2"));
    }

    #[tokio::test]
    async fn history_stops_at_max() {
        let p1 = json!({"ok": true, "messages": [msg("3.0", "c"), msg("2.0", "b"), msg("1.0", "a")],
            "response_metadata": {"next_cursor": "c2"}});
        let (c, rec, _) = setup(vec![ok(p1)]);
        let got = c.history("C1", None, 2).await.unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(rec.requests().len(), 1);
        assert!(has(&rec.requests()[0], "limit", "2"));
    }

    #[tokio::test]
    async fn observed_row_bounds_distinguish_exact_end_overflow_and_unread_cursor() {
        for (count, next, truncated) in [
            (2, "", false),
            (2, "c2", true),
            (3, "", true),
            (3, "c2", true),
        ] {
            let messages: Vec<_> = (0..count).map(|n| msg(&n.to_string(), "row")).collect();
            let body = json!({"ok": true, "messages": messages,
                "response_metadata": {"next_cursor": next}});
            let (c, rec, _) = setup(vec![ok(body)]);
            let got = c.history_observed("C1", None, 2).await.unwrap();
            assert_eq!(got.messages.len(), 2);
            assert_eq!(
                got.messages
                    .iter()
                    .map(|m| m.ts.as_str())
                    .collect::<Vec<_>>(),
                ["0", "1"]
            );
            assert_eq!(
                got.evidence,
                ListEvidence {
                    truncated,
                    incomplete_pages: 0
                },
                "{count} rows, {next:?}"
            );
            assert_eq!(rec.requests().len(), 1);
        }
    }

    #[tokio::test]
    async fn observed_zero_bound_makes_no_requests_or_claims_about_unseen_rows() {
        let (c, rec, _) = setup(vec![]);
        let history = c.history_observed("C1", None, 0).await.unwrap();
        let replies = c.replies_observed("C1", "1.0", 0).await.unwrap();
        assert!(history.messages.is_empty() && replies.messages.is_empty());
        assert_eq!(history.evidence, ListEvidence::default());
        assert_eq!(replies.evidence, ListEvidence::default());
        assert!(rec.requests().is_empty());
    }

    #[tokio::test]
    async fn observed_page_bound_is_complete_only_without_an_unread_cursor_or_overflow() {
        for (last_cursor, extra_row, max, truncated) in [
            ("", false, 100, false),
            ("", false, MAX_CURSOR_PAGES, false),
            ("c21", false, 100, true),
            ("", true, MAX_CURSOR_PAGES, true),
        ] {
            let pages = (0..MAX_CURSOR_PAGES)
                .map(|page| {
                    let mut messages = vec![msg(&page.to_string(), "row")];
                    let next = if page + 1 == MAX_CURSOR_PAGES {
                        if extra_row {
                            messages.push(msg("overflow", "discarded"));
                        }
                        last_cursor.to_string()
                    } else {
                        format!("c{}", page + 2)
                    };
                    ok(json!({"ok": true, "messages": messages,
                    "response_metadata": {"next_cursor": next}}))
                })
                .collect();
            let (c, rec, _) = setup(pages);
            let got = c.replies_observed("C1", "0", max).await.unwrap();
            assert_eq!(got.messages.len(), MAX_CURSOR_PAGES);
            assert_eq!(
                got.messages.last().unwrap().ts,
                (MAX_CURSOR_PAGES - 1).to_string()
            );
            assert_eq!(
                got.evidence,
                ListEvidence {
                    truncated,
                    incomplete_pages: 0
                },
                "last cursor {last_cursor:?}, extra row {extra_row}, max {max}"
            );
            let seen = rec.requests();
            assert_eq!(seen.len(), MAX_CURSOR_PAGES);
            for (page, req) in seen.iter().enumerate() {
                let mut expected = vec![
                    ("channel".into(), "C1".into()),
                    ("ts".into(), "0".into()),
                    ("limit".into(), "200".into()),
                ];
                if page > 0 {
                    expected.push(("cursor".into(), format!("c{}", page + 1)));
                }
                assert_eq!(*req, Client::read("conversations.replies", expected));
            }
        }
    }

    #[tokio::test]
    async fn observed_missing_or_non_array_collections_are_incomplete_not_truncated() {
        let mut pages = vec![json!({"ok": true})];
        pages.extend(
            [
                Value::Null,
                json!({}),
                json!("rows"),
                json!(1),
                json!(false),
            ]
            .into_iter()
            .map(|messages| json!({"ok": true, "messages": messages})),
        );
        for body in pages {
            let (c, rec, _) = setup(vec![ok(body.clone())]);
            let got = c.history_observed("C1", None, 50).await.unwrap();
            assert!(got.messages.is_empty(), "{body}");
            assert_eq!(
                got.evidence,
                ListEvidence {
                    truncated: false,
                    incomplete_pages: 1
                },
                "{body}"
            );
            assert_eq!(rec.requests().len(), 1);
        }
    }

    #[tokio::test]
    async fn observed_paging_absence_is_ordinary_but_malformed_types_are_incomplete() {
        let mut cases = vec![
            (json!({"ok": true, "messages": []}), false),
            (
                json!({"ok": true, "messages": [], "response_metadata": {}}),
                false,
            ),
            (
                json!({"ok": true, "messages": [], "response_metadata": {"next_cursor": ""}}),
                false,
            ),
            (
                json!({"ok": true, "messages": [], "has_more": false}),
                false,
            ),
            (json!({"ok": true, "messages": [], "has_more": true}), true),
            (
                json!({"ok": true, "messages": [], "has_more": true,
                "response_metadata": {"next_cursor": ""}}),
                true,
            ),
        ];
        for malformed in [Value::Null, json!(false), json!(1), json!([]), json!("bad")] {
            cases.push((
                json!({"ok": true, "messages": [], "response_metadata": malformed}),
                true,
            ));
        }
        for malformed in [Value::Null, json!(false), json!(1), json!([]), json!({})] {
            cases.push((
                json!({"ok": true, "messages": [],
                "response_metadata": {"next_cursor": malformed}}),
                true,
            ));
        }
        for malformed in [Value::Null, json!("true"), json!(1), json!([]), json!({})] {
            cases.push((
                json!({"ok": true, "messages": [], "has_more": malformed}),
                true,
            ));
        }
        for (body, incomplete) in cases {
            let (c, rec, _) = setup(vec![ok(body.clone())]);
            let got = c.replies_observed("C1", "1.0", 50).await.unwrap();
            assert_eq!(
                got.evidence,
                ListEvidence {
                    truncated: false,
                    incomplete_pages: usize::from(incomplete)
                },
                "{body}"
            );
            assert_eq!(rec.requests().len(), 1);
        }
    }

    #[tokio::test]
    async fn observed_empty_pages_still_follow_cursors_regardless_of_has_more() {
        for has_more in [true, false] {
            let (c, rec, _) = setup(vec![
                ok(json!({"ok": true, "messages": [], "has_more": has_more,
                    "response_metadata": {"next_cursor": "c2"}})),
                ok(json!({"ok": true, "messages": [msg("1.0", "last")]})),
            ]);
            let got = c.history_observed("C1", None, 50).await.unwrap();
            assert_eq!(got.messages, vec![message(&msg("1.0", "last"))]);
            assert_eq!(got.evidence, ListEvidence::default());
            let seen = rec.requests();
            assert_eq!(seen.len(), 2);
            assert!(has(&seen[1], "cursor", "c2"));
        }
    }

    #[tokio::test]
    async fn observed_whitespace_cursor_is_incomplete_but_still_requested_unchanged() {
        let cursor = " \t\n ";
        for has_more in [None, Some(false), Some(true)] {
            let mut first = json!({"ok": true, "messages": [],
                "response_metadata": {"next_cursor": cursor}});
            if let Some(has_more) = has_more {
                first["has_more"] = json!(has_more);
            }
            let (c, rec, _) = setup(vec![
                ok(first),
                ok(json!({"ok": true, "messages": [msg("1.0", "last")]})),
            ]);
            let got = c.history_observed("C1", None, 50).await.unwrap();
            assert_eq!(got.messages, vec![message(&msg("1.0", "last"))]);
            assert_eq!(
                got.evidence,
                ListEvidence {
                    truncated: false,
                    incomplete_pages: 1
                }
            );
            let params = vec![
                ("channel".into(), "C1".into()),
                ("inclusive".into(), "false".into()),
                ("limit".into(), "50".into()),
            ];
            let mut next_params = params.clone();
            next_params.push(("cursor".into(), cursor.into()));
            assert_eq!(
                rec.requests(),
                vec![
                    Client::read("conversations.history", params),
                    Client::read("conversations.history", next_params),
                ]
            );
        }
    }

    #[tokio::test]
    async fn observed_whitespace_cursor_at_page_bound_does_not_certify_extra_rows() {
        let cursor = " \t ";
        let pages = (0..MAX_CURSOR_PAGES)
            .map(|_| {
                ok(json!({"ok": true, "messages": [],
            "has_more": true, "response_metadata": {"next_cursor": cursor}}))
            })
            .collect();
        let (c, rec, _) = setup(pages);
        let got = c.replies_observed("C1", "1.0", 50).await.unwrap();
        assert!(got.messages.is_empty());
        assert_eq!(
            got.evidence,
            ListEvidence {
                truncated: false,
                incomplete_pages: MAX_CURSOR_PAGES
            }
        );
        let seen = rec.requests();
        assert_eq!(seen.len(), MAX_CURSOR_PAGES);
        assert!(!seen[0].params.iter().any(|(key, _)| key == "cursor"));
        assert!(seen[1..].iter().all(|req| has(req, "cursor", cursor)));
    }

    #[tokio::test]
    async fn observed_whitespace_cursor_at_row_bound_only_truncates_actual_overflow() {
        for count in [1, 2] {
            let messages: Vec<_> = (0..count).map(|n| msg(&n.to_string(), "row")).collect();
            let (c, rec, _) = setup(vec![ok(json!({"ok": true, "messages": messages,
                "has_more": true, "response_metadata": {"next_cursor": " "}}))]);
            let got = c.history_observed("C1", None, 1).await.unwrap();
            assert_eq!(got.messages.len(), 1);
            assert_eq!(
                got.evidence,
                ListEvidence {
                    truncated: count > 1,
                    incomplete_pages: 1
                }
            );
            assert_eq!(rec.requests().len(), 1);
        }
    }

    #[tokio::test]
    async fn observed_incomplete_pages_accumulate_once_each_and_survive_later_success() {
        for last_cursor in ["", "c4"] {
            let (c, rec, _) = setup(vec![
                ok(
                    json!({"ok": true, "has_more": "bad", "response_metadata": {"next_cursor": "c2"}}),
                ),
                ok(
                    json!({"ok": true, "messages": false, "response_metadata": {"next_cursor": "c3"}}),
                ),
                ok(json!({"ok": true, "messages": [msg("1.0", "last")],
                    "response_metadata": {"next_cursor": last_cursor}})),
            ]);
            let got = c.history_observed("C1", None, 1).await.unwrap();
            assert_eq!(got.messages, vec![message(&msg("1.0", "last"))]);
            assert_eq!(
                got.evidence,
                ListEvidence {
                    truncated: !last_cursor.is_empty(),
                    incomplete_pages: 2
                }
            );
            assert_eq!(rec.requests().len(), 3);
        }
    }

    #[tokio::test]
    async fn observed_reads_preserve_every_permissively_parsed_message_row() {
        let rows = vec![
            Value::Null,
            json!(false),
            json!(1),
            json!([]),
            json!("row"),
            json!({"ts": 1, "text": false}),
            msg("valid", "kept"),
        ];
        let expected: Vec<_> = rows.iter().map(message).collect();
        let (c, _, _) = setup(vec![ok(json!({"ok": true, "messages": rows}))]);
        let got = c
            .history_observed("C1", None, expected.len())
            .await
            .unwrap();
        assert_eq!(got.messages, expected);
        assert_eq!(got.evidence, ListEvidence::default());
    }

    #[tokio::test]
    async fn observed_wrappers_preserve_rows_requests_and_rate_limit_retries() {
        let pages = || {
            vec![
                limited(Some(7)),
                ok(json!({"ok": true, "messages": [msg("3.0", "first"), null],
                "response_metadata": {"next_cursor": "c2"}})),
                limited(Some(500)),
                ok(json!({"ok": true, "messages": [], "response_metadata": {"next_cursor": "c3"}})),
                ok(json!({"ok": true, "messages": [msg("1.0", "last")]})),
            ]
        };
        for history in [true, false] {
            let (plain, plain_rec, plain_waits) = setup(pages());
            let (observed, observed_rec, observed_waits) = setup(pages());
            let (plain, observed) = if history {
                (
                    plain.history("C1", Some("0.5"), 50).await.unwrap(),
                    observed
                        .history_observed("C1", Some("0.5"), 50)
                        .await
                        .unwrap(),
                )
            } else {
                (
                    plain.replies("C1", "0.5", 50).await.unwrap(),
                    observed.replies_observed("C1", "0.5", 50).await.unwrap(),
                )
            };
            assert_eq!(plain, observed.messages);
            assert_eq!(observed.evidence, ListEvidence::default());
            assert_eq!(*plain_waits.lock().unwrap(), [7, 60]);
            assert_eq!(*observed_waits.lock().unwrap(), [7, 60]);
            let (method, params) = if history {
                (
                    "conversations.history",
                    vec![
                        ("channel".into(), "C1".into()),
                        ("oldest".into(), "0.5".into()),
                        ("inclusive".into(), "false".into()),
                        ("limit".into(), "50".into()),
                    ],
                )
            } else {
                (
                    "conversations.replies",
                    vec![
                        ("channel".into(), "C1".into()),
                        ("ts".into(), "0.5".into()),
                        ("limit".into(), "200".into()),
                    ],
                )
            };
            let expected: Vec<_> = ["", "", "c2", "c2", "c3"]
                .into_iter()
                .map(|cursor| {
                    let mut params = params.clone();
                    if !cursor.is_empty() {
                        params.push(("cursor".into(), cursor.into()));
                    }
                    Client::read(method, params)
                })
                .collect();
            assert_eq!(plain_rec.requests(), expected);
            assert_eq!(observed_rec.requests(), expected);
        }
    }

    #[test]
    fn list_evidence_merges_without_losing_prior_omissions() {
        let mut evidence = ListEvidence {
            truncated: true,
            incomplete_pages: 1,
        };
        evidence.merge(ListEvidence {
            truncated: false,
            incomplete_pages: 2,
        });
        evidence.merge(ListEvidence::default());
        assert_eq!(
            evidence,
            ListEvidence {
                truncated: true,
                incomplete_pages: 3
            }
        );
    }

    #[tokio::test]
    async fn replies_parse_thread_fields() {
        let parent = json!({"user": "U1", "ts": "10.0", "thread_ts": "10.0", "text": "question",
            "reply_count": 2, "reply_users": ["U2", "U1"], "latest_reply": "12.0",
            "reactions": [{"name": "eyes", "count": 2, "users": ["U1", "U2"]}]});
        let body = json!({"ok": true, "messages": [
            parent,
            {"user": "U2", "ts": "11.0", "thread_ts": "10.0", "text": "one"},
            {"user": "U1", "ts": "12.0", "thread_ts": "10.0", "text": "two", "edited": {"user": "U1", "ts": "12.5"},
             "files": [{"name": "log.txt"}, {"title": "Notes"}]}]});
        let (c, rec, _) = setup(vec![ok(body)]);
        let got = c.replies("C1", "10.0", 100).await.unwrap();
        assert_eq!(got.len(), 3);
        assert!(has(&rec.requests()[0], "ts", "10.0") && has(&rec.requests()[0], "limit", "200"));
        let p = &got[0];
        assert_eq!(p.reply_count, 2);
        assert_eq!(p.reply_users, ["U2", "U1"]);
        assert_eq!(p.latest_reply.as_deref(), Some("12.0"));
        assert_eq!(
            p.reactions,
            vec![Reaction {
                name: "eyes".into(),
                count: 2,
                users: vec!["U1".into(), "U2".into()]
            }]
        );
        assert!(got[2].edited && !got[1].edited);
        let names: Vec<&str> = got[2].files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["log.txt", "Notes"]);
    }

    /// As Slack sends them: a screenshot, a shared message, a link's own
    /// preview, and a preview with an image.
    #[test]
    fn files_shares_and_images_are_kept_with_their_links() {
        let v = json!({"ts": "1.0", "user": "U1", "text": "This is still open. see <https://x.example/a|a doc>",
            "files": [{"name": "Screenshot.png", "title": "Screenshot.png", "mimetype": "image/png",
                       "permalink": "https://acme.slack.com/files/U1/F1/screenshot.png", "url_private": "https://files.slack.com/F1"},
                      {"title": "notes.csv", "mimetype": "text/csv", "url_private": "https://files.slack.com/F2"}],
            "attachments": [
                {"is_msg_unfurl": true, "is_share": true, "author_name": "Bob",
                 "text": "<!subteam^S1> :gitlab: <https://git.example/acme/api/-/merge_requests/9|mr> please",
                 "fallback": "[October 5th] bob: <!subteam^S1> …", "from_url": "https://acme.slack.com/archives/C1/p100"},
                {"title": "A doc", "title_link": "https://x.example/a", "from_url": "https://x.example/a", "text": "preview"},
                {"title": "The tracker", "from_url": "https://y.example", "image_url": "https://y.example/shot.png"},
                {"fallback": "[no preview available]", "from_url": "https://z.example"}]});
        let m = message(&v);
        assert_eq!(
            m.files,
            [
                // An image is drawn from the file itself when Slack made no
                // thumbnail.
                File {
                    name: "Screenshot.png".into(),
                    url: "https://acme.slack.com/files/U1/F1/screenshot.png".into(),
                    image: true,
                    src: "https://files.slack.com/F1".into(),
                    size: None,
                },
                File { name: "notes.csv".into(), url: "https://files.slack.com/F2".into(), ..File::default() },
                File {
                    name: "The tracker".into(),
                    url: "https://y.example/shot.png".into(),
                    image: true,
                    src: "https://y.example/shot.png".into(),
                    size: None,
                },
            ]
        );
        assert_eq!(
            m.shared,
            [Shared {
                author: "Bob".into(),
                text: "<!subteam^S1> :gitlab: <https://git.example/acme/api/-/merge_requests/9|mr> please".into(),
                url: "https://acme.slack.com/archives/C1/p100".into(),
            }]
        );
        // An older cache kept files as names.
        let old: Message = serde_json::from_str(r#"{"ts":"1.0","files":["a.png"]}"#).unwrap();
        assert_eq!(old.files, [File { name: "a.png".into(), ..File::default() }]);
    }

    #[test]
    fn bot_message_text_comes_from_attachments() {
        let v = json!({"ts": "1.0", "subtype": "bot_message", "username": "deploybot", "text": "",
            "attachments": [{"title": "Build passed", "text": "all green", "fallback": "x"}]});
        let m = message(&v);
        assert_eq!(m.text, "*Build passed*\nall green");
        assert_eq!(m.bot_name.as_deref(), Some("deploybot"));
        let v = json!({"ts": "1.0", "bot_profile": {"name": "ci"}, "text": "",
            "attachments": [{"fallback": "only a fallback"}]});
        let m = message(&v);
        assert_eq!(m.text, "only a fallback");
        assert_eq!(m.bot_name.as_deref(), Some("ci"));
    }

    #[test]
    fn a_bots_attachments_and_blocks_are_kept_for_the_alert_readers_and_a_persons_are_not() {
        let v = json!({"ts": "1.0", "subtype": "bot_message", "username": "alertbot", "text": "",
            "attachments": [{"color": "danger", "title": "[FIRING:1] HighErrors", "title_link": "https://alerts.example/1",
                "text": "*Environment:* prod", "fields": [{"title": "Priority", "value": "P1", "short": true}]}],
            "blocks": [
                {"type": "header", "text": {"type": "plain_text", "text": "Nightly export failed"}},
                {"type": "section", "text": {"type": "mrkdwn", "text": "see the run"},
                    "fields": [{"type": "mrkdwn", "text": "*Env*\nprod"}],
                    "accessory": {"type": "button", "text": {"type": "plain_text", "text": "Logs"}, "url": "https://ci.example/9"}},
                {"type": "context", "elements": [{"type": "mrkdwn", "text": "job: nightly-export"}]},
                {"type": "divider"}
            ]});
        let m = message(&v);
        assert_eq!(m.cards.len(), 1);
        assert_eq!(m.cards[0].title, "[FIRING:1] HighErrors");
        assert_eq!(m.cards[0].fields, [("Priority".to_string(), "P1".to_string())]);
        let kinds: Vec<&str> = m.blocks.iter().map(|b| b.kind.as_str()).collect();
        assert_eq!(kinds, ["header", "section", "context"], "a divider says nothing");
        assert_eq!(m.blocks[1].texts, ["see the run", "*Env*\nprod", "<https://ci.example/9|Logs>"]);
        let person = message(&json!({"ts": "1.0", "user": "U1", "text": "hi",
            "attachments": [{"title": "[FIRING] X"}], "blocks": [{"type": "header", "text": {"text": "X failed"}}]}));
        assert!(person.cards.is_empty() && person.blocks.is_empty());
    }

    #[tokio::test]
    async fn a_rate_limited_read_waits_and_tries_again() {
        let body = json!({"ok": true, "user_id": "U1"});
        let (c, rec, waits) = setup(vec![limited(Some(7)), ok(body)]);
        assert!(c.auth_test().await.is_ok());
        assert_eq!(*waits.lock().unwrap(), vec![7]);
        assert_eq!(rec.requests().len(), 2);
    }

    #[tokio::test]
    async fn a_read_gives_up_after_three_retries() {
        let (c, rec, waits) = setup(vec![limited(None), limited(Some(2)), limited(Some(500)), limited(None)]);
        let err = c.auth_test().await.unwrap_err().to_string();
        assert!(err.contains("still rate limiting"), "{err}");
        assert_eq!(rec.requests().len(), 4);
        assert_eq!(*waits.lock().unwrap(), vec![1, 2, 60]);
    }

    #[tokio::test]
    async fn a_rate_limited_write_is_not_sent_again() {
        let (c, rec, waits) = setup(vec![limited(Some(5)), ok(json!({"ok": true}))]);
        let err = c.post("C1", "hi", None).await.unwrap_err().to_string();
        assert!(err.contains("not sent"), "{err}");
        assert!(err.contains("5s"), "{err}");
        assert_eq!(rec.requests().len(), 1);
        assert!(waits.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_network_error_on_a_write_is_not_retried() {
        let (c, rec, _) = setup(vec![Err(anyhow!("connection reset")), ok(json!({"ok": true}))]);
        let err = c.react("C1", "1.0", "eyes").await.unwrap_err().to_string();
        assert!(err.contains("nothing was sent again"), "{err}");
        assert!(err.contains("connection reset"), "{err}");
        assert_eq!(rec.requests().len(), 1);
    }

    #[tokio::test]
    async fn a_network_error_on_a_read_is_not_retried() {
        let (c, rec, _) = setup(vec![Err(anyhow!("timed out"))]);
        let err = c.auth_test().await.unwrap_err().to_string();
        assert_eq!(err, "could not reach Slack: timed out");
        assert_eq!(rec.requests().len(), 1);
    }

    #[tokio::test]
    async fn missing_scope_names_the_scope() {
        let body = json!({"ok": false, "error": "missing_scope", "needed": "search:read", "provided": "x"});
        let (c, _, _) = setup(vec![ok(body)]);
        let err = c.search("hello", 10).await.unwrap_err().to_string();
        assert!(err.contains("search:read"), "{err}");
    }

    #[tokio::test]
    async fn channel_not_found_is_friendly() {
        let (c, _, _) = setup(vec![ok(json!({"ok": false, "error": "channel_not_found"}))]);
        let err = c.channel_info("C9").await.unwrap_err().to_string();
        assert_eq!(err, "no such channel, or the token cannot see it");
    }

    #[test]
    fn explain_covers_the_known_errors() {
        assert_eq!(explain("invalid_auth", None), "the token was refused (invalid_auth)");
        assert_eq!(explain("token_revoked", None), "the token no longer works (token_revoked)");
        assert_eq!(explain("not_in_channel", None), "you are not in that channel");
        assert_eq!(explain("ratelimited", None), "rate limited");
        assert_eq!(explain("fatal_error", None), "Slack says fatal_error");
    }

    #[tokio::test]
    async fn http_500_is_reported_after_three_retries() {
        let down = || {
            Ok(Reply {
                status: 500,
                ..Reply::default()
            })
        };
        let (c, rec, waits) = setup(vec![down(), down(), down(), down()]);
        let err = c.auth_test().await.unwrap_err().to_string();
        assert_eq!(err, "Slack answered HTTP 500");
        assert_eq!(rec.requests().len(), 4);
        assert_eq!(*waits.lock().unwrap(), vec![1, 2, 4]);
    }

    fn ch(id: &str, name: &str) -> Value {
        json!({"id": id, "name": name, "is_member": true,
            "topic": {"value": "planning"}, "purpose": {"value": "the team"}})
    }

    fn page(channels: Vec<Value>, next: &str) -> Result<Reply> {
        ok(json!({"ok": true, "channels": channels, "response_metadata": {"next_cursor": next}}))
    }

    #[tokio::test]
    async fn find_channel_looks_in_yours_first_and_stops_once_found() {
        let (c, rec, _) = setup(vec![
            page(vec![ch("C0", "random")], "n2"),
            page(vec![ch("C1", "team")], "n3"),
            page(vec![], ""),
        ]);
        let got = c.find_channel("#Team").await.unwrap();
        let info = got.found.unwrap();
        assert_eq!((info.id.as_str(), info.topic.as_str(), info.purpose.as_str()), ("C1", "planning", "the team"));
        assert!(info.is_member);
        let seen = rec.requests();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].method, "users.conversations");
        assert!(has(&seen[0], "types", "public_channel,private_channel"));
        assert!(has(&seen[0], "exclude_archived", "true") && has(&seen[0], "limit", "999"));
        assert!(has(&seen[1], "cursor", "n2"));
    }

    /// Slack pages a list before it filters it: a workspace's list can run
    /// to dozens of nearly empty pages before the channel turns up.
    #[tokio::test]
    async fn find_channel_pages_to_the_end_however_many_nearly_empty_pages_come_first() {
        let mut replies: Vec<Result<Reply>> = (0..30).map(|i| page(vec![ch(&format!("C9{i:08}"), "other")], &format!("n{i}"))).collect();
        replies.push(page(vec![ch("C0PRIVATE1", "team-private")], ""));
        let (c, rec, _) = setup(replies);
        let got = c.find_channel("team-private").await.unwrap();
        assert_eq!(got.found.unwrap().id, "C0PRIVATE1");
        assert_eq!(rec.requests().len(), 31);
    }

    #[tokio::test]
    async fn find_channel_then_looks_through_every_public_channel() {
        let (c, rec, _) = setup(vec![
            page(vec![ch("C0", "random")], ""),
            page(vec![ch("C2", "general"), ch("C3", "other")], "p2"),
            page(vec![ch("C4", "deploys")], ""),
        ]);
        let got = c.find_channel("deploys").await.unwrap();
        assert_eq!(got.found.unwrap().id, "C4");
        assert_eq!((got.yours, got.public), (1, 3));
        let seen = rec.requests();
        assert_eq!(seen[1].method, "conversations.list");
        assert!(has(&seen[1], "types", "public_channel") && has(&seen[1], "limit", "1000"));
        assert!(has(&seen[2], "cursor", "p2"));
    }

    #[tokio::test]
    async fn find_channel_says_how_many_it_looked_at_when_none_is_called_that() {
        let (c, _, _) = setup(vec![page(vec![ch("C0", "random")], ""), page(vec![ch("C2", "general")], "")]);
        let got = c.find_channel("team").await.unwrap();
        assert_eq!(got, Lookup { found: None, yours: 1, public: 1 });
    }

    #[tokio::test]
    async fn search_pages_by_number() {
        let m = |ts: &str, text: &str, im: bool| json!({"ts": ts, "user": "U2", "username": "bob",
            "text": text, "permalink": format!("https://acme.slack.com/archives/C1/p{ts}"),
            "channel": {"id": "C1", "name": "team", "is_im": im, "is_mpim": false}});
        let p1 = json!({"ok": true, "messages": {"matches": [m("2", "two", false)],
            "paging": {"count": 1, "page": 1, "pages": 2}}});
        let p2 = json!({"ok": true, "messages": {"matches": [m("1", "one", true)],
            "paging": {"count": 1, "page": 2, "pages": 2}}});
        let (c, rec, _) = setup(vec![ok(p1), ok(p2)]);
        let got = c.search("deploy", 10).await.unwrap();
        assert_eq!(got.len(), 2);
        let seen = rec.requests();
        assert_eq!(seen.len(), 2);
        assert!(has(&seen[0], "page", "1") && has(&seen[1], "page", "2"));
        assert!(has(&seen[0], "query", "deploy") && has(&seen[0], "count", "10"));
        assert!(has(&seen[0], "sort", "timestamp") && has(&seen[0], "sort_dir", "desc"));
        assert_eq!(got[0].channel_name, "team");
        assert_eq!(got[0].channel_id, "C1");
        assert!(!got[0].is_im && got[1].is_im);
        assert_eq!(got[0].permalink, "https://acme.slack.com/archives/C1/p2");
        assert_eq!(got[0].message.bot_name.as_deref(), Some("bob"));
        assert_eq!(got[0].message.text, "two");
    }

    #[tokio::test]
    async fn search_stops_at_max_without_another_page() {
        let m = json!({"ts": "2", "text": "x", "channel": {"id": "C1", "name": "team"}});
        let p1 = json!({"ok": true, "messages": {"matches": [m.clone(), m],
            "paging": {"pages": 4}}});
        let (c, rec, _) = setup(vec![ok(p1)]);
        assert_eq!(c.search("x", 2).await.unwrap().len(), 2);
        assert_eq!(rec.requests().len(), 1);
    }

    fn search_request(query: &str, count: usize, page: usize) -> Request {
        Client::read(
            "search.messages",
            vec![
                ("query".into(), query.into()),
                ("count".into(), count.to_string()),
                ("sort".into(), "timestamp".into()),
                ("sort_dir".into(), "desc".into()),
                ("page".into(), page.to_string()),
            ],
        )
    }

    #[tokio::test]
    async fn observed_search_distinguishes_exact_max_overflow_and_unread_pages() {
        for (rows, max, pages, truncated) in [
            (0, 2, 1, false),
            (1, 2, 1, false),
            (2, 2, 1, false),
            (3, 2, 1, true),
            (2, 2, 3, true),
            (3, 2, 3, true),
        ] {
            let matches: Vec<_> = (0..rows).map(|n| msg(&n.to_string(), "row")).collect();
            let expected: Vec<_> = matches.iter().take(max).map(found).collect();
            let body = json!({"ok": true, "messages": {"matches": matches,
                "paging": {"pages": pages}}});
            let (plain, plain_rec, _) = setup(vec![ok(body.clone())]);
            let (c, rec, _) = setup(vec![ok(body)]);
            let got = c.search_observed("x", max).await.unwrap();
            assert_eq!(got.matches, expected);
            assert_eq!(plain.search("x", max).await.unwrap(), got.matches);
            assert_eq!(
                got.evidence,
                ListEvidence {
                    truncated,
                    incomplete_pages: 0
                }
            );
            assert_eq!(rec.requests(), [search_request("x", max, 1)]);
            assert_eq!(plain_rec.requests(), rec.requests());
        }
    }

    #[tokio::test]
    async fn observed_search_five_page_bound_requires_numeric_unread_pages_or_overflow() {
        for (pages, max, extra_row, truncated) in [
            (5, 1000, false, false),
            (5, 5, false, false),
            (6, 1000, false, true),
            (6, 5, false, true),
            (5, 5, true, true),
        ] {
            let replies = (1..=MAX_SEARCH_PAGES)
                .map(|page| {
                    let mut matches = vec![msg(&page.to_string(), "row")];
                    if extra_row && page == MAX_SEARCH_PAGES {
                        matches.push(msg("overflow", "discarded"));
                    }
                    ok(json!({"ok": true, "messages": {"matches": matches,
                    "paging": {"pages": pages}}}))
                })
                .collect();
            let (c, rec, _) = setup(replies);
            let got = c.search_observed("x", max).await.unwrap();
            assert_eq!(got.matches.len(), MAX_SEARCH_PAGES);
            assert_eq!(got.matches.last().unwrap().message.ts, "5");
            assert_eq!(
                got.evidence,
                ListEvidence {
                    truncated,
                    incomplete_pages: 0
                }
            );
            assert_eq!(
                rec.requests(),
                (1..=MAX_SEARCH_PAGES)
                    .map(|page| search_request("x", max.min(100), page))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[tokio::test]
    async fn observed_search_empty_nonfinal_page_is_incomplete_without_bound_evidence() {
        for empty_page in [1, MAX_SEARCH_PAGES] {
            let replies = (1..=empty_page)
                .map(|page| {
                    let matches = if page == empty_page {
                        vec![]
                    } else {
                        vec![msg("row", "kept")]
                    };
                    ok(json!({"ok": true, "messages": {"matches": matches,
                    "paging": {"pages": 10}}}))
                })
                .collect();
            let (c, rec, _) = setup(replies);
            let got = c.search_observed("x", 100).await.unwrap();
            assert_eq!(got.matches.len(), empty_page - 1);
            assert_eq!(
                got.evidence,
                ListEvidence {
                    truncated: false,
                    incomplete_pages: 1
                }
            );
            assert_eq!(rec.requests().len(), empty_page);
        }
    }

    #[tokio::test]
    async fn observed_search_missing_or_malformed_matches_stop_without_truncation() {
        let mut bodies = vec![
            json!({"ok": true}),
            json!({"ok": true, "messages": []}),
            json!({"ok": true, "messages": {"paging": {"pages": 10}}}),
            json!({"ok": true, "messages": {"paging": false}}),
        ];
        for malformed in [
            Value::Null,
            json!(false),
            json!(1),
            json!("rows"),
            json!({}),
        ] {
            bodies.push(json!({"ok": true, "messages": malformed}));
            bodies.push(json!({"ok": true, "messages": {"matches": malformed,
                "paging": {"pages": 10}}}));
            bodies.push(json!({"ok": true, "messages": {"matches": malformed,
                "paging": {"pages": "bad"}}}));
        }
        for body in bodies {
            let (plain, plain_rec, _) = setup(vec![ok(body.clone())]);
            let (c, rec, _) = setup(vec![ok(body.clone())]);
            let got = c.search_observed("x", 10).await.unwrap();
            assert!(got.matches.is_empty(), "{body}");
            assert_eq!(plain.search("x", 10).await.unwrap(), got.matches);
            assert_eq!(
                got.evidence,
                ListEvidence {
                    truncated: false,
                    incomplete_pages: 1
                },
                "{body}"
            );
            assert_eq!(rec.requests(), [search_request("x", 10, 1)]);
            assert_eq!(plain_rec.requests(), rec.requests());
        }
    }

    #[tokio::test]
    async fn observed_search_paging_absence_keeps_default_but_malformed_presence_is_incomplete() {
        let mut cases = vec![
            (
                json!({"ok": true, "messages": {"matches": [msg("1", "row")]}}),
                false,
            ),
            (
                json!({"ok": true, "messages": {"matches": [msg("1", "row")], "paging": {}}}),
                false,
            ),
        ];
        for malformed in [Value::Null, json!(false), json!(1), json!([]), json!("bad")] {
            cases.push((
                json!({"ok": true, "messages": {"matches": [msg("1", "row")],
                "paging": malformed}}),
                true,
            ));
        }
        for malformed in [
            Value::Null,
            json!(false),
            json!(-1),
            json!(1.5),
            json!([]),
            json!({}),
            json!("2"),
        ] {
            cases.push((
                json!({"ok": true, "messages": {"matches": [msg("1", "row")],
                "paging": {"pages": malformed}}}),
                true,
            ));
        }
        for (body, incomplete) in cases {
            for max in [1, 10] {
                let (c, rec, _) = setup(vec![ok(body.clone())]);
                let got = c.search_observed("x", max).await.unwrap();
                assert_eq!(got.matches, [found(&msg("1", "row"))]);
                assert_eq!(
                    got.evidence,
                    ListEvidence {
                        truncated: false,
                        incomplete_pages: usize::from(incomplete)
                    },
                    "{body}"
                );
                assert_eq!(rec.requests(), [search_request("x", max, 1)]);
            }
        }
    }

    #[tokio::test]
    async fn observed_search_contradictory_numeric_pages_preserve_rows_and_requests() {
        for (requested_page, reported_pages, nonempty, incomplete) in [
            (1, 0, true, true),
            (1, 0, false, false),
            (1, 1, true, false),
            (2, 0, true, true),
            (2, 0, false, true),
            (2, 1, true, true),
            (2, 1, false, true),
            (2, 2, true, false),
            (2, 2, false, false),
        ] {
            let mut bodies = Vec::new();
            let mut expected = Vec::new();
            for _ in 1..requested_page {
                let row = msg("first", "kept");
                expected.push(found(&row));
                bodies.push(json!({"ok": true, "messages": {"matches": [row],
                    "paging": {"pages": 3}}}));
            }
            let matches = if nonempty {
                vec![msg("last", "kept")]
            } else {
                vec![]
            };
            expected.extend(matches.iter().map(found));
            bodies.push(json!({"ok": true, "messages": {"matches": matches,
                "paging": {"pages": reported_pages}}}));
            let (plain, plain_rec, _) = setup(bodies.iter().cloned().map(ok).collect());
            let (c, rec, _) = setup(bodies.into_iter().map(ok).collect());
            let got = c.search_observed("x", 10).await.unwrap();
            assert_eq!(got.matches, expected);
            assert_eq!(plain.search("x", 10).await.unwrap(), expected);
            assert_eq!(
                got.evidence,
                ListEvidence {
                    truncated: false,
                    incomplete_pages: usize::from(incomplete),
                },
                "requested page {requested_page}, reported pages {reported_pages}, nonempty {nonempty}"
            );
            let expected_requests: Vec<_> = (1..=requested_page)
                .map(|page| search_request("x", 10, page))
                .collect();
            assert_eq!(rec.requests(), expected_requests);
            assert_eq!(plain_rec.requests(), expected_requests);
        }
    }

    #[tokio::test]
    async fn observed_search_absent_later_paging_keeps_default_without_contradiction() {
        for paging in [None, Some(json!({}))] {
            for nonempty in [false, true] {
                let first = msg("first", "kept");
                let last = if nonempty {
                    vec![msg("last", "kept")]
                } else {
                    vec![]
                };
                let mut expected = vec![found(&first)];
                expected.extend(last.iter().map(found));
                let mut terminal = json!({"ok": true, "messages": {"matches": last}});
                if let Some(paging) = &paging {
                    terminal["messages"]["paging"] = paging.clone();
                }
                let bodies = [
                    json!({"ok": true, "messages": {"matches": [first],
                    "paging": {"pages": 3}}}),
                    terminal,
                ];
                let (plain, plain_rec, _) = setup(bodies.iter().cloned().map(ok).collect());
                let (c, rec, _) = setup(bodies.into_iter().map(ok).collect());
                let got = c.search_observed("x", 10).await.unwrap();
                assert_eq!(got.matches, expected);
                assert_eq!(plain.search("x", 10).await.unwrap(), expected);
                assert_eq!(got.evidence, ListEvidence::default());
                let requests = [search_request("x", 10, 1), search_request("x", 10, 2)];
                assert_eq!(rec.requests(), requests);
                assert_eq!(plain_rec.requests(), requests);
            }
        }
    }

    #[tokio::test]
    async fn observed_search_malformed_last_paging_never_invents_unread_pages() {
        for paging in [Value::Null, json!({"pages": "6"}), json!({})] {
            let mut replies: Vec<_> = (1..MAX_SEARCH_PAGES)
                .map(|_| {
                    ok(
                        json!({"ok": true, "messages": {"matches": [msg("row", "kept")],
                    "paging": {"pages": 6}}}),
                    )
                })
                .collect();
            replies.push(ok(
                json!({"ok": true, "messages": {"matches": [msg("last", "kept")],
                "paging": paging}}),
            ));
            let (c, rec, _) = setup(replies);
            let got = c.search_observed("x", 100).await.unwrap();
            assert_eq!(got.matches.len(), MAX_SEARCH_PAGES);
            assert_eq!(
                got.evidence,
                ListEvidence {
                    truncated: false,
                    incomplete_pages: usize::from(paging != json!({}))
                }
            );
            assert_eq!(rec.requests().len(), MAX_SEARCH_PAGES);
        }
    }

    #[tokio::test]
    async fn observed_search_preserves_every_permissively_parsed_found_row() {
        let matches = vec![
            Value::Null,
            json!(false),
            json!(1),
            json!([]),
            json!("row"),
            json!({"ts": 1, "text": false, "channel": 42}),
            msg("last", "kept"),
        ];
        let expected: Vec<_> = matches.iter().map(found).collect();
        let body = json!({"ok": true, "messages": {"matches": matches}});
        let (plain, plain_rec, _) = setup(vec![ok(body.clone())]);
        let (c, rec, _) = setup(vec![ok(body)]);
        let got = c.search_observed("x", expected.len()).await.unwrap();
        assert_eq!(got.matches, expected);
        assert_eq!(
            plain.search("x", expected.len()).await.unwrap(),
            got.matches
        );
        assert_eq!(got.evidence, ListEvidence::default());
        assert_eq!(plain_rec.requests(), rec.requests());
    }

    #[tokio::test]
    async fn observed_search_wrapper_preserves_requests_order_and_rate_limit_retries() {
        let replies = || {
            vec![
                limited(Some(7)),
                ok(
                    json!({"ok": true, "messages": {"matches": [msg("3", "first"), null], "paging": {"pages": 2}}}),
                ),
                limited(Some(500)),
                ok(
                    json!({"ok": true, "messages": {"matches": [msg("1", "last")], "paging": {"pages": 2}}}),
                ),
            ]
        };
        let query = "in:team \"ship 🚀\"";
        let (plain, plain_rec, plain_waits) = setup(replies());
        let (c, rec, waits) = setup(replies());
        let got = c.search_observed(query, 300).await.unwrap();
        assert_eq!(
            got.matches,
            [
                found(&msg("3", "first")),
                found(&Value::Null),
                found(&msg("1", "last"))
            ]
        );
        assert_eq!(plain.search(query, 300).await.unwrap(), got.matches);
        assert_eq!(got.evidence, ListEvidence::default());
        let expected: Vec<_> = [1, 1, 2, 2]
            .into_iter()
            .map(|page| search_request(query, 100, page))
            .collect();
        assert_eq!(rec.requests(), expected);
        assert_eq!(plain_rec.requests(), expected);
        assert_eq!(*waits.lock().unwrap(), [7, 60]);
        assert_eq!(*plain_waits.lock().unwrap(), [7, 60]);
    }

    #[tokio::test]
    async fn observed_search_zero_bound_makes_no_requests_or_unseen_row_claims() {
        let (c, rec, waits) = setup(vec![]);
        let got = c.search_observed("x", 0).await.unwrap();
        assert!(got.matches.is_empty());
        assert_eq!(got.evidence, ListEvidence::default());
        assert!(c.search("x", 0).await.unwrap().is_empty());
        assert!(rec.requests().is_empty());
        assert!(waits.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn observed_search_late_errors_return_no_partial_success() {
        for kind in ["transport", "provider", "http", "json", "rate_limit"] {
            let replies = || {
                let mut replies = vec![ok(json!({"ok": true, "messages": {
                    "matches": [msg("1", "partial")], "paging": {"pages": 3}}}))];
                match kind {
                    "transport" => replies.push(Err(anyhow!("network stopped"))),
                    "provider" => replies.push(ok(
                        json!({"ok": false, "error": "missing_scope", "needed": "search:read"}),
                    )),
                    "http" => replies.extend((0..=MAX_RETRIES).map(|_| {
                        Ok(Reply {
                            status: 503,
                            ..Reply::default()
                        })
                    })),
                    "json" => replies.push(Ok(Reply {
                        status: 200,
                        body: "not JSON".into(),
                        ..Reply::default()
                    })),
                    "rate_limit" => replies.extend((0..=MAX_RETRIES).map(|_| limited(None))),
                    _ => unreachable!(),
                }
                replies
            };
            let (plain, plain_rec, plain_waits) = setup(replies());
            let (c, rec, waits) = setup(replies());
            let error = c.search_observed("x", 10).await.unwrap_err().to_string();
            assert_eq!(plain.search("x", 10).await.unwrap_err().to_string(), error);
            assert!(!error.is_empty());
            assert_eq!(plain_rec.requests(), rec.requests());
            let seen = rec.requests();
            assert_eq!(seen[0], search_request("x", 10, 1));
            assert!(
                seen[1..]
                    .iter()
                    .all(|req| *req == search_request("x", 10, 2))
            );
            assert_eq!(
                seen.len(),
                if matches!(kind, "rate_limit" | "http") {
                    MAX_RETRIES as usize + 2
                } else {
                    2
                }
            );
            assert_eq!(*plain_waits.lock().unwrap(), *waits.lock().unwrap());
            assert_eq!(
                waits.lock().unwrap().len(),
                if matches!(kind, "rate_limit" | "http") {
                    MAX_RETRIES as usize
                } else {
                    0
                }
            );
        }
    }

    fn groups_request() -> Request {
        Client::read(
            "usergroups.list",
            vec![("include_users".into(), "true".into())],
        )
    }

    #[tokio::test]
    async fn observed_groups_valid_membership_does_not_validate_names_handles_or_id_lengths() {
        for groups in [
            json!([]),
            json!([
                {"id": "s", "users": ["u"]},
                {"id": "long-group-id-without-provider-shape", "name": false, "handle": [], "users": []},
                {"id": " ", "name": "", "handle": "", "users": [" ", "U1", "U1"]}
            ]),
        ] {
            let body = json!({"ok": true, "usergroups": groups});
            let (plain, plain_rec, _) = setup(vec![ok(body.clone())]);
            let (c, rec, _) = setup(vec![ok(body)]);
            let got = c.usergroups_observed().await.unwrap();
            assert_eq!(plain.usergroups().await.unwrap(), got.groups);
            assert!(!got.incomplete_collection);
            assert_eq!(got.incomplete_entries, 0);
            assert_eq!(rec.requests(), [groups_request()]);
            assert_eq!(plain_rec.requests(), rec.requests());
            if !got.groups.is_empty() {
                assert_eq!(
                    got.groups[0],
                    UserGroup {
                        id: "s".into(),
                        users: vec!["u".into()],
                        ..UserGroup::default()
                    }
                );
                assert_eq!(got.groups[1].name, "");
                assert_eq!(got.groups[1].handle, "");
                assert_eq!(got.groups[2].users, [" ", "U1", "U1"]);
            }
        }
    }

    #[tokio::test]
    async fn observed_groups_missing_or_non_array_collection_is_incomplete() {
        let mut bodies = vec![json!({"ok": true})];
        bodies.extend(
            [
                Value::Null,
                json!(false),
                json!(1),
                json!("groups"),
                json!({}),
            ]
            .into_iter()
            .map(|groups| json!({"ok": true, "usergroups": groups})),
        );
        for body in bodies {
            let (plain, plain_rec, _) = setup(vec![ok(body.clone())]);
            let (c, rec, _) = setup(vec![ok(body.clone())]);
            let got = c.usergroups_observed().await.unwrap();
            assert!(got.groups.is_empty(), "{body}");
            assert_eq!(plain.usergroups().await.unwrap(), got.groups);
            assert!(got.incomplete_collection, "{body}");
            assert_eq!(got.incomplete_entries, 0);
            assert_eq!(rec.requests(), [groups_request()]);
            assert_eq!(plain_rec.requests(), rec.requests());
        }
    }

    #[tokio::test]
    async fn observed_groups_count_each_malformed_membership_row_once_without_changing_rows() {
        let mut rows = vec![
            Value::Null,
            json!(false),
            json!(1),
            json!("group"),
            json!([]),
            json!({}),
            json!({"users": []}),
            json!({"id": "g"}),
            json!({"id": "", "users": [null, "", false]}),
            json!({"id": "g", "users": ["u", "", false, null, "u"]}),
        ];
        for malformed in [
            Value::Null,
            json!(false),
            json!(1),
            json!([]),
            json!({}),
            json!(""),
        ] {
            rows.push(json!({"id": malformed, "users": []}));
        }
        for malformed in [
            Value::Null,
            json!(false),
            json!(1),
            json!("users"),
            json!({}),
        ] {
            rows.push(json!({"id": "g", "users": malformed}));
        }
        for malformed in [
            Value::Null,
            json!(false),
            json!(1),
            json!([]),
            json!({}),
            json!(""),
        ] {
            rows.push(json!({"id": "g", "users": [malformed]}));
        }
        let incomplete_entries = rows.len();
        rows.push(json!({"id": "ok", "users": []}));
        let expected: Vec<_> = rows
            .iter()
            .map(|g| UserGroup {
                id: text_of(g, "id"),
                handle: text_of(g, "handle"),
                name: text_of(g, "name"),
                users: strings(g.get("users")),
            })
            .collect();
        let body = json!({"ok": true, "usergroups": rows});
        let (plain, plain_rec, _) = setup(vec![ok(body.clone())]);
        let (c, rec, _) = setup(vec![ok(body)]);
        let got = c.usergroups_observed().await.unwrap();
        assert_eq!(got.groups, expected);
        assert_eq!(plain.usergroups().await.unwrap(), got.groups);
        assert!(!got.incomplete_collection);
        assert_eq!(got.incomplete_entries, incomplete_entries);
        assert_eq!(got.groups[8].users, [""]);
        assert_eq!(got.groups[9].users, ["u", "", "u"]);
        assert_eq!(rec.requests(), [groups_request()]);
        assert_eq!(plain_rec.requests(), rec.requests());
    }

    #[tokio::test]
    async fn observed_groups_wrapper_preserves_one_list_call_and_read_retries() {
        let replies = || {
            vec![
                limited(Some(3)),
                limited(Some(500)),
                ok(json!({"ok": true,
            "usergroups": [{"id": "g", "users": ["u"]}], "response_metadata": {"next_cursor": "ignored"}})),
            ]
        };
        let (plain, plain_rec, plain_waits) = setup(replies());
        let (c, rec, waits) = setup(replies());
        let got = c.usergroups_observed().await.unwrap();
        assert_eq!(plain.usergroups().await.unwrap(), got.groups);
        assert!(!got.incomplete_collection);
        assert_eq!(got.incomplete_entries, 0);
        assert_eq!(
            rec.requests(),
            [groups_request(), groups_request(), groups_request()]
        );
        assert_eq!(plain_rec.requests(), rec.requests());
        assert_eq!(*waits.lock().unwrap(), [3, 60]);
        assert_eq!(*plain_waits.lock().unwrap(), [3, 60]);
    }

    #[tokio::test]
    async fn observed_groups_errors_preserve_wrapper_errors_and_requests() {
        for kind in ["transport", "provider", "http", "json", "rate_limit"] {
            let replies = || match kind {
                "transport" => vec![Err(anyhow!("network stopped"))],
                "provider" => vec![ok(
                    json!({"ok": false, "error": "missing_scope", "needed": "usergroups:read"}),
                )],
                "http" => (0..=MAX_RETRIES)
                    .map(|_| {
                        Ok(Reply {
                            status: 503,
                            ..Reply::default()
                        })
                    })
                    .collect(),
                "json" => vec![Ok(Reply {
                    status: 200,
                    body: "not JSON".into(),
                    ..Reply::default()
                })],
                "rate_limit" => (0..=MAX_RETRIES).map(|_| limited(None)).collect(),
                _ => unreachable!(),
            };
            let (plain, plain_rec, plain_waits) = setup(replies());
            let (c, rec, waits) = setup(replies());
            let error = c.usergroups_observed().await.unwrap_err().to_string();
            assert_eq!(plain.usergroups().await.unwrap_err().to_string(), error);
            assert!(!error.is_empty());
            let count = if matches!(kind, "rate_limit" | "http") {
                MAX_RETRIES as usize + 1
            } else {
                1
            };
            assert_eq!(rec.requests(), vec![groups_request(); count]);
            assert_eq!(plain_rec.requests(), rec.requests());
            assert_eq!(*plain_waits.lock().unwrap(), *waits.lock().unwrap());
            assert_eq!(
                waits.lock().unwrap().len(),
                if matches!(kind, "rate_limit" | "http") {
                    MAX_RETRIES as usize
                } else {
                    0
                }
            );
        }
    }

    #[tokio::test]
    async fn usergroups_carry_their_users() {
        let body = json!({"ok": true, "usergroups": [
            {"id": "S1", "handle": "review-squad", "name": "Review squad", "users": ["U1", "U2"]}]});
        let (c, rec, _) = setup(vec![ok(body)]);
        let got = c.usergroups().await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].handle, "review-squad");
        assert_eq!(got[0].users, ["U1", "U2"]);
        assert!(has(&rec.requests()[0], "include_users", "true"));
    }

    #[tokio::test]
    async fn user_prefers_the_profile() {
        let body = json!({"ok": true, "user": {"id": "U1", "name": "alice", "real_name": "Alice A",
            "profile": {"display_name": "ally", "real_name": "Alice Anders"}}});
        let (c, _, _) = setup(vec![ok(body)]);
        let u = c.user("U1").await.unwrap();
        assert_eq!(u.display_name, "ally");
        assert_eq!(u.real_name, "Alice Anders");
        assert_eq!(u.shown(), "ally");
    }

    #[test]
    fn shown_falls_back() {
        let mut u = User {
            id: "U2".into(),
            name: "bob".into(),
            real_name: "Bob B".into(),
            display_name: String::new(),
            tz_offset: 0,
        };
        assert_eq!(u.shown(), "Bob B");
        u.real_name.clear();
        assert_eq!(u.shown(), "bob");
        u.name.clear();
        assert_eq!(u.shown(), "U2");
    }

    #[test]
    fn write_requests_are_exact() {
        let r = post_request("C1", "hello", None);
        assert_eq!(r.method, "chat.postMessage");
        assert!(r.write && r.params.is_empty());
        assert_eq!(
            r.body.unwrap(),
            json!({"channel": "C1", "text": "hello", "unfurl_links": false, "unfurl_media": false})
        );
        let r = post_request("C1", "hello", Some("1.5"));
        assert_eq!(
            r.body.unwrap(),
            json!({"channel": "C1", "text": "hello", "thread_ts": "1.5",
                "unfurl_links": false, "unfurl_media": false})
        );
        let r = react_request("C1", "1.5", "eyes");
        assert_eq!(r.method, "reactions.add");
        assert!(r.write);
        assert_eq!(r.body.unwrap(), json!({"channel": "C1", "timestamp": "1.5", "name": "eyes"}));
    }

    #[tokio::test]
    async fn post_sends_the_built_request_and_reads_the_reply() {
        let (c, rec, _) = setup(vec![ok(json!({"ok": true, "channel": "C1", "ts": "9.0"}))]);
        let p = c.post("C1", "hi", Some("1.0")).await.unwrap();
        assert_eq!(p, Posted { channel: "C1".into(), ts: "9.0".into() });
        assert_eq!(rec.requests(), vec![post_request("C1", "hi", Some("1.0"))]);
    }

    #[test]
    fn message_round_trips_through_serde() {
        let m = Message {
            ts: "1.0".into(),
            thread_ts: Some("1.0".into()),
            user: Some("U1".into()),
            bot_name: None,
            subtype: Some("thread_broadcast".into()),
            text: "hi".into(),
            reply_count: 2,
            reply_users: vec!["U2".into()],
            latest_reply: Some("2.0".into()),
            reactions: vec![Reaction { name: "eyes".into(), count: 1, users: vec!["U2".into()] }],
            files: vec![File {
                name: "a.png".into(),
                url: "https://acme.slack.com/files/U1/F1/a.png".into(),
                image: true,
                src: "https://files.slack.com/files-tmb/T1-F1/a_720.png".into(),
                size: Some((1440, 900)),
            }],
            shared: vec![Shared { author: "Bob".into(), text: "hi".into(), url: "https://acme.slack.com/archives/C1/p1".into() }],
            edited: true,
            cards: vec![Card { color: "danger".into(), title: "[FIRING] X".into(), fields: vec![("env".into(), "prod".into())], ..Card::default() }],
            blocks: vec![Block { kind: "header".into(), texts: vec!["X failed".into()] }],
        };
        // The token goes to Slack's own hosts only.
        assert!(slack_host("https://files.slack.com/files-pri/T1-F1/a.png", crate::SLACK_API));
        assert!(slack_host("https://acme.slack.com/x", crate::SLACK_API));
        assert!(!slack_host("https://slack.com.evil.example/x", crate::SLACK_API));
        assert!(!slack_host("https://y.example/shot.png", crate::SLACK_API));
        assert!(!slack_host("http://files.slack.com/a.png", crate::SLACK_API));
        // A picture: the biggest thumbnail up to 1024 wide, with its size.
        let f = json!({"url_private": "https://files.slack.com/F9/big.png", "original_w": 4000, "original_h": 3000,
            "thumb_360": "https://files.slack.com/t360", "thumb_360_w": 360, "thumb_360_h": 270,
            "thumb_720": "https://files.slack.com/t720", "thumb_720_w": 720, "thumb_720_h": 540});
        assert_eq!(picture_of(&f), ("https://files.slack.com/t720".to_string(), Some((720, 540))));
        let f = json!({"url_private": "https://files.slack.com/F9/small.png", "original_w": 40, "original_h": 30});
        assert_eq!(picture_of(&f), ("https://files.slack.com/F9/small.png".to_string(), Some((40, 30))));
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<Message>(&s).unwrap(), m);
        let f = Found { channel_id: "C1".into(), message: m, ..Found::default() };
        let s = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<Found>(&s).unwrap(), f);
    }

    #[test]
    fn the_token_goes_to_the_configured_api_host_and_nowhere_else() {
        let gov = "https://slack-gov.com";
        assert!(slack_host("https://slack-gov.com/api/auth.test", gov));
        assert!(slack_host("https://files.slack-gov.com/F1", gov));
        // Slack's own hosts still count; others, lookalikes and plain http do not.
        assert!(slack_host("https://files.slack.com/F1", gov));
        assert!(!slack_host("https://evilslack-gov.com/F1", gov));
        assert!(!slack_host("https://slack-gov.com.evil.example/F1", gov));
        assert!(!slack_host("http://files.slack-gov.com/F1", gov));
        assert!(!slack_host("https://example.com/F1", gov));
        assert!(!slack_host("https://user:pw@slack-gov.com/F1", gov));
        // The default api adds nothing to the slack.com rule.
        assert!(!slack_host("https://slack-gov.com/F1", crate::SLACK_API));
        // A local stand-in matches itself: scheme, host and port.
        let local = "http://127.0.0.1:8123";
        assert!(slack_host("http://127.0.0.1:8123/files/F1.png", local));
        assert!(!slack_host("http://127.0.0.1:8124/files/F1.png", local));
        assert!(!slack_host("https://127.0.0.1:8123/files/F1.png", local));
        assert!(!slack_host("http://sub.127.0.0.1:8123/F1", local));
        assert!(!slack_host("http://localhost:8123/F1", local));
        assert!(!slack_host("http://127.0.0.1:8123/F1", "not a url"));
    }

    #[tokio::test]
    async fn calls_go_to_the_configured_api() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let seen = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).unwrap();
            let body = r#"{"ok":true,"team_id":"T1","user_id":"U1","user":"alice","team":"Acme","url":"http://x/"}"#;
            write!(
                sock,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            String::from_utf8_lossy(&buf[..n]).to_string()
        });
        let client = Client::http(&base, "xoxp-fake").unwrap();
        let me = client.auth_test().await.unwrap();
        assert_eq!(me.team_id, "T1");
        let head = seen.join().unwrap();
        assert!(head.starts_with("GET /api/auth.test"), "{head}");
        assert!(head.to_lowercase().contains("authorization: bearer xoxp-fake"), "{head}");
    }

    #[test]
    fn an_empty_message_serializes_small() {
        assert_eq!(serde_json::to_string(&Message::default()).unwrap(), "{}");
        let m = Message { ts: "1.0".into(), text: "hi".into(), ..Message::default() };
        assert_eq!(serde_json::to_string(&m).unwrap(), r#"{"ts":"1.0","text":"hi"}"#);
        assert_eq!(serde_json::from_str::<Message>("{}").unwrap(), Message::default());
    }
}
