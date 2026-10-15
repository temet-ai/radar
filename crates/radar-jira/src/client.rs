//! Jira Cloud REST v3: search by JQL paged by `nextPageToken`, a ticket with
//! its detail fields, comments, web links and development panel
//! (`dev-status`), who-am-I, the fields list, a JQL check, an approximate
//! count, a ticket's transitions and a project's issue types; and the
//! writes: make a transition, comment, create a ticket, assign it. A write's
//! request is built by a plain function (`*_req`) so tests read it exactly.
//!
//! Basic auth with the account's email and an API token. A read is tried
//! up to three times when Jira answers 429 or 5xx (waiting as it asks, a
//! few seconds at most; asked to wait longer, it gives up at once with
//! [`Busy`], and the sync waits as asked); a request marked as a write is sent once, so a
//! slow answer can never post a comment twice. 401, 403 and 404 come back
//! as sentences saying what to do. Keys are checked before they go into a
//! path.

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use radar_core::log;
use serde_json::{Value, json};

use crate::config::{is_project_key, is_ticket_key};
use crate::rate::{Limits, Rate};

/// Whether `url` is on the site at `base` (same scheme and host), the only
/// place the site's credentials are sent.
pub fn on_site(url: &str, base: &str) -> bool {
    match (reqwest::Url::parse(url), reqwest::Url::parse(base)) {
        (Ok(u), Ok(b)) => u.scheme() == "https" && u.scheme() == b.scheme() && u.host_str() == b.host_str() && u.port() == b.port(),
        _ => false,
    }
}

/// A picture's bytes: an attachment's content with the site's account and
/// token (`creds`: the site's address, email, token), sent only to the site
/// itself (Jira answers with a redirect to its media store, followed
/// without them); any other address without them.
pub async fn picture(src: &str, creds: Option<(String, String, String)>) -> Result<Vec<u8>> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("radar")
        .build()?;
    let mut req = http.get(src);
    if let Some((base, email, token)) = &creds
        && on_site(src, base)
    {
        req = req.basic_auth(email, Some(token));
    }
    let resp = req.send().await?;
    let status = resp.status();
    if !status.is_success() {
        bail!("Jira answered {status}");
    }
    Ok(resp.bytes().await?.to_vec())
}
use crate::model::{self, Comment, Detail, Link, Me, Ticket};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
}

/// A request, path and query relative to the site.
#[derive(Debug, Clone, PartialEq)]
pub struct Req {
    pub method: Method,
    pub path: String,
    pub body: Option<Value>,
}

/// An answer: status, how long Jira asked to wait, what it said about its
/// rate limits, the body.
#[derive(Debug, Clone, PartialEq)]
pub struct Resp {
    pub status: u16,
    pub retry_after: Option<u64>,
    pub rate: Rate,
    pub body: String,
}

pub type Sent<'a> = Pin<Box<dyn Future<Output = Result<Resp, String>> + Send + 'a>>;

/// What carries requests: HTTP, or recorded replies in tests.
pub trait Transport: Send + Sync {
    fn send(&self, req: &Req) -> Sent<'_>;
}

struct Http {
    http: reqwest::Client,
    base: String,
    auth: String,
}

impl Transport for Http {
    fn send(&self, req: &Req) -> Sent<'_> {
        let url = format!("{}{}", self.base, req.path);
        let (method, body) = (req.method, req.body.clone());
        Box::pin(async move {
            let mut r = match method {
                Method::Get => self.http.get(&url),
                Method::Post => self.http.post(&url),
                Method::Put => self.http.put(&url),
            }
            .header("Authorization", &self.auth)
            .header("Accept", "application/json");
            if let Some(b) = body {
                r = r.json(&b);
            }
            let resp = r.send().await.map_err(|e| {
                let host = crate::config::host_of(&self.base);
                if e.is_timeout() {
                    format!("Jira at {host} did not answer in time")
                } else {
                    format!("Jira unreachable at {host}")
                }
            })?;
            let status = resp.status().as_u16();
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse().ok());
            let rate = Rate::from_headers(|name| {
                resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
            });
            let body = match resp.text().await {
                Ok(body) => body,
                // A successful status alone does not confirm a complete read or
                // write acknowledgement. Never turn a lost body into blank success.
                Err(_) if (200..=299).contains(&status) => {
                    return Err("Jira response body could not be read.".into());
                }
                // Preserve status-specific refusals and read retry policy even
                // when Jira's optional error text could not be collected.
                Err(_) => String::new(),
            };
            Ok(Resp {
                status,
                retry_after,
                rate,
                body,
            })
        })
    }
}

#[derive(Clone)]
pub struct Client {
    t: Arc<dyn Transport>,
    /// The first wait between tries; doubled each time.
    pause: Duration,
    /// Where every answer's rate-limit word is kept, and for which site.
    limits: Option<(String, Limits)>,
}

/// How many times a read is tried.
const TRIES: usize = 3;
/// The longest radar waits between tries, whatever Jira asks.
const LONGEST_WAIT: u64 = 10;
/// Tickets a view fetches per site at most (pages of 100).
pub const MAX_TICKETS: usize = 400;

/// The observed fields used by the permissive settings parsers, never cached.
/// Blank strings are incomplete but retain their original display value.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CheckEvidence {
    pub incomplete_fields: u32,
    pub defaulted_fields: u32,
}

impl CheckEvidence {
    fn fields(value: &Value, fields: &[&str]) -> Self {
        let mut evidence = Self::default();
        for field in fields {
            let text = value.get(*field).and_then(Value::as_str);
            evidence.incomplete_fields += u32::from(text.is_none_or(|s| s.trim().is_empty()));
            evidence.defaulted_fields += u32::from(text.is_none());
        }
        evidence
    }
}

#[derive(Debug)]
pub(crate) struct CheckRead<T> {
    pub value: T,
    pub evidence: CheckEvidence,
}

/// Classification at the branch that actually knows whether a read was attempted.
/// Keep the original error for unchanged public wrappers and UI, never the journal.
#[derive(Debug)]
pub(crate) enum CheckError {
    Setup(anyhow::Error),
    InvalidProject(anyhow::Error),
    Read(anyhow::Error),
}

impl CheckError {
    pub(crate) fn error(&self) -> &anyhow::Error {
        match self {
            Self::Setup(error) | Self::InvalidProject(error) | Self::Read(error) => error,
        }
    }

    pub(crate) fn into_error(self) -> anyhow::Error {
        match self {
            Self::Setup(error) | Self::InvalidProject(error) | Self::Read(error) => error,
        }
    }
}

/// Evidence from one bounded search, kept out of the provider cache.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ListEvidence {
    /// Actual rows or a next page were left unread at the existing bound.
    pub truncated: bool,
    /// Returned rows that could not be represented as tickets.
    pub omitted: usize,
    /// Missing list data or a non-final page with no usable continuation.
    /// Counts observed pages, not the unknown number of missing tickets.
    pub incomplete_pages: usize,
}

impl ListEvidence {
    fn add(&mut self, more: ListEvidence) {
        self.truncated |= more.truncated;
        self.omitted += more.omitted;
        self.incomplete_pages += more.incomplete_pages;
    }
}

#[derive(Debug, Default)]
pub(crate) struct TicketsRead {
    pub tickets: Vec<Ticket>,
    pub evidence: ListEvidence,
}

/// How a view's list is asked for: all of it, or only what changed in the
/// last `minutes` (and, with `sweep`, the whole view's keys beside it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ask {
    Full,
    Changed { minutes: i64, sweep: bool },
}

/// A view's list as read: the tickets read in full; whether they are only
/// what changed (to merge into what is held); and, from a sweep, the whole
/// view's keys in Jira's order.
#[derive(Debug, Default)]
pub(crate) struct ViewRead {
    pub list: TicketsRead,
    pub changed_only: bool,
    pub swept: Option<Vec<String>>,
}

/// Development-panel evidence from the returned attempt, never cached.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CodeEvidence {
    /// The existing invalid-id guard returned without making a request.
    pub skipped_invalid_id: bool,
    /// Returned pull-request rows omitted by the legacy URL-string parser.
    pub omitted_rows: usize,
    /// Responses with missing/malformed expected collections or host counts.
    /// Each response counts once, not once per malformed nested collection.
    pub incomplete_responses: usize,
}

#[derive(Debug, Default)]
pub(crate) struct CodeRead {
    pub links: Vec<model::CodeLink>,
    pub evidence: CodeEvidence,
}

/// Exact collection/row losses in the existing link parsers, not URL validation.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct LinkEvidence {
    pub omitted_rows: usize,
    pub incomplete_collection: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CommentEvidence {
    pub incomplete_collection: bool,
    /// The existing fallback was used; no remote total was learned.
    pub total_defaulted: bool,
}

#[derive(Debug)]
pub(crate) struct CommentsRead {
    pub comments: Vec<Comment>,
    pub total: usize,
    pub evidence: CommentEvidence,
}

#[derive(Debug)]
pub(crate) struct WebLinksRead {
    pub links: Vec<Link>,
    pub evidence: LinkEvidence,
}

/// Evidence carried only by a successful detail task. Fatal pipeline errors
/// still discard earlier results; these fields never enter `Detail` or cache.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct DetailEvidence {
    pub children: ListEvidence,
    pub children_count: usize,
    pub children_failed: bool,
    pub issue_links: LinkEvidence,
    pub issue_link_count: usize,
    pub web: LinkEvidence,
    pub web_link_count: usize,
    pub web_failed: bool,
    pub comments: CommentEvidence,
    pub code: CodeEvidence,
    pub code_failed: bool,
}

#[derive(Debug, Default)]
pub(crate) struct DetailRead {
    pub detail: Detail,
    pub evidence: DetailEvidence,
}

struct SearchRead {
    issues: Vec<Value>,
    evidence: ListEvidence,
}

/// `s` made safe for a query string.
pub fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Jira's own words from an error body: `errorMessages` and `errors`.
fn jira_says(body: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return String::new();
    };
    let mut parts: Vec<String> = v
        .get("errorMessages")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if let Some(errors) = v.get("errors").and_then(Value::as_object) {
        parts.extend(errors.values().filter_map(Value::as_str).map(str::to_string));
    }
    parts.join("; ")
}

impl Client {
    pub fn new(base: &str, email: &str, token: &str) -> Self {
        log::secret(token);
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{email}:{token}"))
        );
        log::secret(&auth);
        let http = Http {
            http: reqwest::Client::builder()
                .user_agent("radar")
                .timeout(Duration::from_secs(30))
                .build()
                .expect("http client"),
            base: base.trim_end_matches('/').to_string(),
            auth,
        };
        Self {
            t: Arc::new(http),
            pause: Duration::from_secs(1),
            limits: None,
        }
    }

    /// A client over another transport (recorded replies), not waiting
    /// between tries.
    #[cfg(test)]
    pub fn over(t: Arc<dyn Transport>) -> Self {
        Self {
            t,
            pause: Duration::ZERO,
            limits: None,
        }
    }

    /// Keep what each answer says about Jira's rate limits for `site`.
    pub(crate) fn limited(mut self, site: &str, limits: &Limits) -> Self {
        self.limits = Some((site.to_string(), limits.clone()));
        self
    }

    /// For a configured site, noting its rate limits in `limits`.
    pub(crate) fn tracked(site: &crate::config::Site, limits: &Limits) -> Result<Self> {
        Ok(Self::for_site(site)?.limited(&site.name, limits))
    }

    /// For a configured site: its address, email and token resolved.
    pub fn for_site(site: &crate::config::Site) -> Result<Self> {
        let base = site.base_url()?;
        let email = crate::config::resolve(&site.email, "email")?;
        let token = crate::config::resolve(&site.token, "API token")?;
        Ok(Self::new(&base, &email, &token))
    }

    /// Send `req`; `what` names the thing asked for, for a 404. Reads
    /// (`retry`) are tried again on 429 and 5xx.
    pub async fn request(&self, req: Req, what: &str, retry: bool) -> Result<Value> {
        let tries = if retry { TRIES } else { 1 };
        let mut wait = self.pause;
        for attempt in 1..=tries {
            let resp = self.t.send(&req).await.map_err(|e| anyhow!(e))?;
            if let Some((site, limits)) = &self.limits {
                limits.note(site, &resp);
            }
            // Asked to wait longer than radar waits inside one read: give up
            // now and let the sync back off as long as Jira asked.
            let again = (resp.status == 429 || resp.status >= 500)
                && resp.retry_after.is_none_or(|s| s <= LONGEST_WAIT);
            if again && attempt < tries {
                let asked = resp.retry_after.map(Duration::from_secs);
                let pause = if self.pause.is_zero() {
                    Duration::ZERO
                } else {
                    asked.unwrap_or(wait)
                };
                log::warn(format!(
                    "jira answered {} to {}; trying again in {}s",
                    resp.status,
                    req.path.split('?').next().unwrap_or(""),
                    pause.as_secs()
                ));
                tokio::time::sleep(pause).await;
                wait *= 2;
                continue;
            }
            return answer(resp, what, attempt);
        }
        unreachable!("the last try returns")
    }

    async fn get(&self, path: String, what: &str) -> Result<Value> {
        self.request(
            Req {
                method: Method::Get,
                path,
                body: None,
            },
            what,
            true,
        )
        .await
    }

    /// A POST that only reads (a JQL check, a count): retried like a GET.
    async fn post_read(&self, path: &str, body: Value, what: &str) -> Result<Value> {
        self.request(
            Req {
                method: Method::Post,
                path: path.into(),
                body: Some(body),
            },
            what,
            true,
        )
        .await
    }

    pub async fn myself(&self) -> Result<Me> {
        Ok(self.myself_observed().await?.value)
    }

    pub(crate) async fn myself_observed(&self) -> Result<CheckRead<Me>> {
        let v = self.get("/rest/api/3/myself".into(), "account").await?;
        Ok(CheckRead {
            value: Me::from_json(&v),
            evidence: CheckEvidence::fields(&v, &["accountId", "displayName"]),
        })
    }

    /// Every issue a JQL query finds, newest page by page, up to `max`.
    pub async fn search(&self, jql: &str, fields: &[String], max: usize) -> Result<Vec<Value>> {
        Ok(self.search_observed(jql, fields, max).await?.issues)
    }

    /// The same request loop with transient, count-only completeness evidence.
    async fn search_observed(&self, jql: &str, fields: &[String], max: usize) -> Result<SearchRead> {
        self.search_pages(jql, fields, max, 100).await
    }

    /// A search in pages of `page` (Jira may send fewer).
    async fn search_pages(&self, jql: &str, fields: &[String], max: usize, page: usize) -> Result<SearchRead> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        let mut evidence = ListEvidence::default();
        loop {
            let mut path = format!(
                "/rest/api/3/search/jql?jql={}&fields={}&maxResults={page}",
                enc(jql),
                enc(&fields.join(","))
            );
            if let Some(t) = &token {
                path.push_str(&format!("&nextPageToken={}", enc(t)));
            }
            let v = self.get(path, "search").await?;
            let issues = v
                .get("issues")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let n = issues.len();
            out.extend(issues);
            token = v
                .get("nextPageToken")
                .and_then(Value::as_str)
                .map(str::to_string);
            let last = v.get("isLast").and_then(Value::as_bool).unwrap_or(token.is_none());
            let usable_token = token.as_deref().is_some_and(|token| !token.trim().is_empty());
            if v.get("issues").and_then(Value::as_array).is_none()
                || v.get("isLast").is_some_and(|value| !value.is_boolean())
                || v.get("nextPageToken").is_some_and(|value| !value.is_null() && !value.is_string())
                || (!last && (!usable_token || n == 0))
            {
                evidence.incomplete_pages += 1;
            }
            // Reaching exactly the bound is not a cutoff when this was the last
            // page. Preserve the original stop conditions and request count.
            evidence.truncated |= out.len() > max
                || (out.len() >= max && !last && usable_token && n != 0);
            if last || token.is_none() || n == 0 || out.len() >= max {
                break;
            }
        }
        out.truncate(max);
        Ok(SearchRead { issues: out, evidence })
    }

    /// A search's issues as tickets.
    pub async fn tickets(
        &self,
        jql: &str,
        ac: &(dyn Fn(&str) -> Option<String> + Sync),
        ac_fields: &[String],
    ) -> Result<Vec<Ticket>> {
        let issues = self
            .search(jql, &model::list_fields(ac_fields), MAX_TICKETS)
            .await?;
        Ok(issues
            .iter()
            .filter_map(|i| Ticket::from_issue(i, ac))
            .collect())
    }

    pub(crate) async fn tickets_observed(
        &self,
        jql: &str,
        ac: &(dyn Fn(&str) -> Option<String> + Sync),
        ac_fields: &[String],
    ) -> Result<TicketsRead> {
        let SearchRead { issues, mut evidence } = self
            .search_observed(jql, &model::list_fields(ac_fields), MAX_TICKETS)
            .await?;
        let tickets: Vec<_> = issues.iter().filter_map(|issue| Ticket::from_issue(issue, ac)).collect();
        evidence.omitted = issues.len() - tickets.len();
        Ok(TicketsRead { tickets, evidence })
    }

    /// A view's list as `ask` says: all of it; or only the tickets changed
    /// within its window, and, with `sweep`, the whole view's keys and
    /// `updated` (one small request) so tickets that left it go, and those
    /// that came in or changed unseen (not as `held` has them) are read by key.
    pub(crate) async fn view_list(
        &self,
        jql: &str,
        ask: &Ask,
        held: &BTreeMap<String, String>,
        ac: &(dyn Fn(&str) -> Option<String> + Sync),
        ac_fields: &[String],
    ) -> Result<ViewRead> {
        let Ask::Changed { minutes, sweep } = *ask else {
            let list = self.tickets_observed(jql, ac, ac_fields).await?;
            return Ok(ViewRead { list, changed_only: false, swept: None });
        };
        let mut list = self
            .tickets_observed(&crate::view::changed_since(jql, minutes), ac, ac_fields)
            .await?;
        if !sweep {
            return Ok(ViewRead { list, changed_only: true, swept: None });
        }
        let SearchRead { issues, evidence } = self
            .search_pages(jql, &["updated".into()], MAX_TICKETS, MAX_TICKETS)
            .await?;
        list.evidence.add(evidence);
        let read: HashSet<String> = list.tickets.iter().map(|t| t.key.clone()).collect();
        let (mut keys, mut unseen) = (Vec::new(), Vec::new());
        for issue in &issues {
            let Some(key) = issue.get("key").and_then(Value::as_str).filter(|k| is_ticket_key(k)) else {
                list.evidence.omitted += 1;
                continue;
            };
            let updated = issue.pointer("/fields/updated").and_then(Value::as_str).unwrap_or_default();
            if !read.contains(key) && (updated.is_empty() || held.get(key).is_none_or(|u| u != updated)) {
                unseen.push(key.to_string());
            }
            keys.push(key.to_string());
        }
        for chunk in unseen.chunks(100) {
            let more = self
                .tickets_observed(&format!("key in ({})", chunk.join(", ")), ac, ac_fields)
                .await?;
            list.evidence.add(more.evidence);
            list.tickets.extend(more.tickets);
        }
        Ok(ViewRead { list, changed_only: true, swept: Some(keys) })
    }

    /// A ticket's `updated` alone: the cheapest way to ask whether its page
    /// is out of date.
    pub(crate) async fn updated_of(&self, key: &str) -> Result<String> {
        if !is_ticket_key(key) {
            bail!("\"{key}\" is not a ticket key like ABC-12");
        }
        let issue = self
            .get(format!("/rest/api/3/issue/{key}?fields=updated"), &format!("ticket {key}"))
            .await?;
        issue
            .pointer("/fields/updated")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| anyhow!("Jira's answer for {key} has no updated time in it"))
    }

    /// One ticket with every field and the fields' names.
    pub async fn issue(&self, key: &str) -> Result<Value> {
        if !is_ticket_key(key) {
            bail!("\"{key}\" is not a ticket key like ABC-12");
        }
        self.get(
            format!("/rest/api/3/issue/{key}?fields=*all&expand=names"),
            &format!("ticket {key}"),
        )
        .await
    }

    /// A page of comments, newest first, and how many there are.
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn comments(&self, key: &str, start: usize, max: usize) -> Result<(Vec<Comment>, usize)> {
        let got = self.comments_observed(key, start, max).await?;
        Ok((got.comments, got.total))
    }

    pub(crate) async fn comments_observed(&self, key: &str, start: usize, max: usize) -> Result<CommentsRead> {
        if !is_ticket_key(key) {
            bail!("\"{key}\" is not a ticket key like ABC-12");
        }
        let v = self
            .get(
                format!(
                    "/rest/api/3/issue/{key}/comment?startAt={start}&maxResults={max}&orderBy=-created"
                ),
                &format!("ticket {key}"),
            )
            .await?;
        let comments: Vec<Comment> = v
            .get("comments")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(Comment::from_json).collect())
            .unwrap_or_default();
        let total = v
            .get("total")
            .and_then(Value::as_u64)
            .map_or(start + comments.len(), |n| n as usize);
        let evidence = CommentEvidence {
            incomplete_collection: v.get("comments").and_then(Value::as_array).is_none(),
            total_defaulted: v.get("total").and_then(Value::as_u64).is_none(),
        };
        Ok(CommentsRead { comments, total, evidence })
    }

    // Preserve the plain client wrapper while page reads consume its evidence.
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn web_links(&self, key: &str) -> Result<Vec<Link>> {
        Ok(self.web_links_observed(key).await?.links)
    }

    pub(crate) async fn web_links_observed(&self, key: &str) -> Result<WebLinksRead> {
        if !is_ticket_key(key) {
            bail!("\"{key}\" is not a ticket key like ABC-12");
        }
        let v = self
            .get(format!("/rest/api/3/issue/{key}/remotelink"), &format!("ticket {key}"))
            .await?;
        let links = model::web_links(&v);
        let evidence = LinkEvidence {
            omitted_rows: v.as_array().map_or(0, |rows| rows.len() - links.len()),
            incomplete_collection: !v.is_array(),
        };
        Ok(WebLinksRead { links, evidence })
    }

    /// Everything a ticket's page shows: the ticket, its sprint, parent,
    /// children, links, web links and the newest comments.
    pub async fn detail(&self, key: &str, ac_field: Option<String>) -> Result<Detail> {
        Ok(self.detail_observed(key, ac_field).await?.detail)
    }

    pub(crate) async fn detail_observed(&self, key: &str, ac_field: Option<String>) -> Result<DetailRead> {
        let issue = self.issue(key).await?;
        let ac = |_: &str| ac_field.clone();
        let ticket = Ticket::from_issue(&issue, &ac)
            .ok_or_else(|| anyhow!("Jira's answer for {key} has no ticket in it"))?;
        let children_q = format!("parent = {key} ORDER BY created ASC");
        let fields: Vec<String> = ac_field.iter().cloned().collect();
        let (children, web, comments) = tokio::join!(
            self.tickets_observed(&children_q, &ac, &fields),
            self.web_links_observed(key),
            self.comments_observed(key, 0, COMMENTS_PAGE),
        );
        let mut evidence = DetailEvidence::default();
        let mut links = Vec::new();
        if let Some(p) = &ticket.parent {
            links.push(Link {
                relation: "parent".into(),
                title: p.summary.clone(),
                target: model::Target::Ticket {
                    key: p.key.clone(),
                    status: p.status.clone(),
                    category: p.category,
                },
            });
        }
        match children {
            Ok(c) => {
                evidence.children_count = c.tickets.len();
                evidence.children = c.evidence;
                links.extend(c.tickets.iter().map(|t| model::ticket_link("child", t)));
            }
            Err(e) => {
                evidence.children_failed = true;
                log::warn(format!("jira children of {key}: {e:#}"));
            }
        }
        let issue_links = model::issue_links(&issue);
        let collection = issue.pointer("/fields/issuelinks");
        evidence.issue_link_count = issue_links.len();
        evidence.issue_links = LinkEvidence {
            omitted_rows: collection.and_then(Value::as_array).map_or(0, |rows| rows.len() - issue_links.len()),
            // This optional field may legitimately be absent.
            incomplete_collection: collection.is_some_and(|value| !value.is_array()),
        };
        links.extend(issue_links);
        match web {
            Ok(w) => {
                evidence.web_link_count = w.links.len();
                evidence.web = w.evidence;
                links.extend(w.links);
            }
            Err(e) => {
                evidence.web_failed = true;
                log::warn(format!("jira web links of {key}: {e:#}"));
            }
        }
        let CommentsRead { mut comments, total: comments_total, evidence: comment_evidence } = comments?;
        evidence.comments = comment_evidence;
        // A comment's pictures are the ticket's attachments too.
        let files = crate::adf::attachments(issue.get("fields").unwrap_or(&Value::Null));
        for c in &mut comments {
            crate::adf::attach(&mut c.body, &files);
        }
        // The development panel is extra: a page without it still shows.
        let code = match self.code_observed(&ticket.id).await {
            Ok(c) => {
                evidence.code = c.evidence;
                c.links
            }
            Err(e) => {
                evidence.code_failed = true;
                log::warn(format!("jira development panel of {key}: {e:#}"));
                vec![]
            }
        };
        Ok(DetailRead {
            detail: Detail {
                sprint: model::sprint_of(&issue),
                ticket,
                links,
                comments,
                comments_total,
                code,
            },
            evidence,
        })
    }

    /// The merge requests Jira's development panel links to a ticket, by
    /// its number: the summary says which code hosts have some, the detail
    /// of each lists them.
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn code(&self, issue_id: &str) -> Result<Vec<model::CodeLink>> {
        Ok(self.code_observed(issue_id).await?.links)
    }

    /// The same request loop and permissive parser, with count-only evidence.
    /// This endpoint has no pagination loop here; a successful observation is
    /// not a claim that Jira or its linked code hosts returned all remote work.
    pub(crate) async fn code_observed(&self, issue_id: &str) -> Result<CodeRead> {
        if issue_id.is_empty() || !issue_id.chars().all(|c| c.is_ascii_digit()) {
            return Ok(CodeRead {
                evidence: CodeEvidence { skipped_invalid_id: true, ..Default::default() },
                ..Default::default()
            });
        }
        let v = self
            .get(
                format!("/rest/dev-status/latest/issue/summary?issueId={issue_id}"),
                "development panel",
            )
            .await?;
        let by_host = v
            .pointer("/summary/pullrequest/byInstanceType")
            .and_then(Value::as_object);
        let mut evidence = CodeEvidence {
            incomplete_responses: usize::from(by_host.is_none_or(|hosts| {
                hosts.values().any(|h| h.get("count").and_then(Value::as_u64).is_none())
            })),
            ..Default::default()
        };
        let hosts: Vec<String> = by_host
            .map(|o| {
                o.iter()
                    .filter(|(_, h)| h.get("count").and_then(Value::as_u64).unwrap_or(0) > 0)
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut out = Vec::new();
        for host in hosts {
            let v = self
                .get(
                    format!(
                        "/rest/dev-status/latest/issue/detail?issueId={issue_id}&applicationType={}&dataType=pullrequest",
                        enc(&host)
                    ),
                    "development panel",
                )
                .await?;
            let details = v.get("detail").and_then(Value::as_array);
            let mut incomplete = details.is_none();
            for detail in details.into_iter().flatten() {
                match detail.get("pullRequests").and_then(Value::as_array) {
                    Some(rows) => {
                        evidence.omitted_rows += rows
                            .iter()
                            .filter(|row| row.get("url").and_then(Value::as_str).is_none())
                            .count();
                    }
                    None => incomplete = true,
                }
            }
            evidence.incomplete_responses += usize::from(incomplete);
            out.extend(model::code_links(&v));
        }
        Ok(CodeRead { links: out, evidence })
    }

    /// The moves a ticket can make now, in Jira's order.
    pub async fn transitions(&self, key: &str) -> Result<Vec<model::Transition>> {
        if !is_ticket_key(key) {
            bail!("\"{key}\" is not a ticket key like ABC-12");
        }
        let v = self
            .get(format!("/rest/api/3/issue/{key}/transitions"), &format!("ticket {key}"))
            .await?;
        Ok(v.get("transitions")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(model::Transition::from_json).collect())
            .unwrap_or_default())
    }

    /// The kinds of ticket a project takes, sub-tasks left out.
    pub async fn issue_types(&self, project: &str) -> Result<Vec<model::IssueType>> {
        if !is_project_key(project) {
            bail!("\"{project}\" is not a project key like ABC");
        }
        let v = self
            .get(
                format!("/rest/api/3/issue/createmeta/{project}/issuetypes"),
                &format!("project {project}"),
            )
            .await?;
        let list = v
            .get("issueTypes")
            .or_else(|| v.get("values"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(list
            .iter()
            .filter_map(|t| {
                Some(model::IssueType {
                    id: t.get("id")?.as_str()?.to_string(),
                    name: t.get("name")?.as_str()?.to_string(),
                    subtask: t.get("subtask").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .filter(|t| !t.subtask)
            .collect())
    }

    /// Send a write, once: a slow or failed answer is shown, never retried.
    pub async fn write(&self, req: Req, what: &str) -> Result<Value> {
        log::info(format!(
            "jira write: {:?} {}",
            req.method,
            req.path.split('?').next().unwrap_or("")
        ));
        self.request(req, what, false).await
    }

    /// Every field: (id, name).
    pub async fn fields(&self) -> Result<Vec<(String, String)>> {
        let v = self.get("/rest/api/3/field".into(), "fields").await?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|f| {
                        Some((
                            f.get("id")?.as_str()?.to_string(),
                            f.get("name")?.as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// About how many tickets a query finds.
    pub async fn count(&self, jql: &str) -> Result<u64> {
        let v = self
            .post_read(
                "/rest/api/3/search/approximate-count",
                json!({ "jql": jql }),
                "count",
            )
            .await?;
        Ok(v.get("count").and_then(Value::as_u64).unwrap_or(0))
    }

    /// What is wrong with a query, as Jira says; empty when it is fine.
    pub async fn check_jql(&self, jql: &str) -> Result<Vec<String>> {
        let v = self
            .post_read(
                "/rest/api/3/jql/parse?validation=strict",
                json!({ "queries": [jql] }),
                "query check",
            )
            .await?;
        Ok(v.pointer("/queries/0/errors")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// A project's name, which also proves it exists and can be seen.
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn project(&self, key: &str) -> Result<String> {
        self.project_observed(key)
            .await
            .map(|got| got.value)
            .map_err(CheckError::into_error)
    }

    pub(crate) async fn project_observed(
        &self,
        key: &str,
    ) -> Result<CheckRead<String>, CheckError> {
        if !is_project_key(key) {
            return Err(CheckError::InvalidProject(anyhow!(
                "\"{key}\" is not a project key like ABC"
            )));
        }
        let v = self
            .get(
                format!("/rest/api/3/project/{key}"),
                &format!("project {key}"),
            )
            .await
            .map_err(CheckError::Read)?;
        Ok(CheckRead {
            value: v
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(key)
                .to_string(),
            evidence: CheckEvidence::fields(&v, &["name"]),
        })
    }

    /// The projects this account looked at lately: (key, name).
    pub async fn recent_projects(&self) -> Result<Vec<(String, String)>> {
        let v = self
            .get("/rest/api/3/project/recent?maxResults=20".into(), "projects")
            .await?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        Some((
                            p.get("key")?.as_str()?.to_string(),
                            p.get("name")?.as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }
}

/// Comments fetched per page.
pub const COMMENTS_PAGE: usize = 50;

// --- the writes, as requests ----------------------------------------------------

fn checked(key: &str) -> Result<()> {
    if is_ticket_key(key) {
        Ok(())
    } else {
        bail!("\"{key}\" is not a ticket key like ABC-12")
    }
}

/// Move a ticket by one of its transitions.
pub fn transition_req(key: &str, transition: &str) -> Result<Req> {
    checked(key)?;
    if transition.is_empty() || !transition.chars().all(|c| c.is_ascii_digit()) {
        bail!("\"{transition}\" is not a transition");
    }
    Ok(Req {
        method: Method::Post,
        path: format!("/rest/api/3/issue/{key}/transitions"),
        body: Some(json!({ "transition": { "id": transition } })),
    })
}

/// A comment, its text sent as ADF.
pub fn comment_req(key: &str, text: &str) -> Result<Req> {
    checked(key)?;
    if text.trim().is_empty() {
        bail!("an empty comment is not sent");
    }
    Ok(Req {
        method: Method::Post,
        path: format!("/rest/api/3/issue/{key}/comment"),
        body: Some(json!({ "body": crate::adf::from_text(text) })),
    })
}

/// Give a ticket to an account.
pub fn assign_req(key: &str, account_id: &str) -> Result<Req> {
    checked(key)?;
    if account_id.trim().is_empty() {
        bail!("no account to assign to");
    }
    Ok(Req {
        method: Method::Put,
        path: format!("/rest/api/3/issue/{key}/assignee"),
        body: Some(json!({ "accountId": account_id })),
    })
}

/// A new ticket: its project, type, summary and description (ADF; left
/// out when empty).
pub fn create_req(project: &str, type_id: &str, summary: &str, description: &str) -> Result<Req> {
    if !is_project_key(project) {
        bail!("\"{project}\" is not a project key like ABC");
    }
    if summary.trim().is_empty() {
        bail!("a ticket needs a summary");
    }
    let mut fields = json!({
        "project": { "key": project },
        "issuetype": { "id": type_id },
        "summary": summary.trim(),
    });
    if !description.trim().is_empty() {
        fields["description"] = crate::adf::from_text(description);
    }
    Ok(Req {
        method: Method::Post,
        path: "/rest/api/3/issue".into(),
        body: Some(json!({ "fields": fields })),
    })
}

/// An answer as a value, or as a sentence saying what went wrong.
/// Jira limiting requests (429) or failing (5xx): what radar says, and how
/// long Jira asked it to wait, so the next sync waits at least that long.
#[derive(Debug)]
pub(crate) struct Busy {
    text: String,
    pub retry_after: Option<u64>,
}

impl Busy {
    fn error(mut text: String, retry_after: Option<u64>) -> anyhow::Error {
        if let Some(s) = retry_after.filter(|s| *s > LONGEST_WAIT) {
            text.push_str(&format!("; it asks radar to wait {s}s"));
        }
        anyhow::Error::new(Self { text, retry_after })
    }

    /// How long the failure `e` asked radar to wait, if it did.
    pub(crate) fn wait(e: &anyhow::Error) -> Option<u64> {
        e.chain().find_map(|c| c.downcast_ref::<Self>()).and_then(|b| b.retry_after)
    }
}

impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::error::Error for Busy {}

fn answer(resp: Resp, what: &str, tries: usize) -> Result<Value> {
    let says = jira_says(&resp.body);
    let said = |s: &str| if says.is_empty() { s.to_string() } else { format!("{s}: {says}") };
    match resp.status {
        200..=299 => {
            if resp.body.trim().is_empty() {
                return Ok(Value::Null);
            }
            serde_json::from_str(&resp.body).map_err(|e| anyhow!("Jira's answer was not JSON: {e}"))
        }
        400 => bail!("{}", said("Jira refused the request (400)")),
        401 => bail!(
            "Jira turned the email and API token down (401): check them under settings, Jira, connection"
        ),
        403 => bail!(
            "{}",
            said(&format!("this account may not see the {what} (403)"))
        ),
        404 => bail!("Jira has no {what}, or this account cannot see it (404)"),
        429 => Err(Busy::error(
            format!(
                "Jira is limiting requests (429){}",
                if tries > 1 { format!(", still after {tries} tries") } else { String::new() }
            ),
            resp.retry_after,
        )),
        s if s >= 500 => Err(Busy::error(
            format!("Jira answered {s}{}", if tries > 1 { format!(", {tries} times") } else { String::new() }),
            resp.retry_after,
        )),
        s => bail!("{}", said(&format!("Jira answered {s}"))),
    }
}

#[cfg(test)]
pub mod recorded {
    //! Recorded Jira replies, played back in order; the requests are kept.

    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Replay {
        pub replies: Mutex<std::collections::VecDeque<Resp>>,
        pub asked: Mutex<Vec<Req>>,
    }

    impl Replay {
        pub fn new(replies: Vec<(u16, Value)>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(
                    replies
                        .into_iter()
                        .map(|(status, body)| Resp {
                            status,
                            retry_after: None,
                            rate: Rate::default(),
                            body: if body.is_null() { String::new() } else { body.to_string() },
                        })
                        .collect(),
                ),
                asked: Mutex::default(),
            })
        }

        /// Replies that carry a `Retry-After` and rate-limit headers too.
        pub fn saying(replies: Vec<Resp>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies.into()),
                asked: Mutex::default(),
            })
        }

        pub fn paths(&self) -> Vec<String> {
            self.asked.lock().unwrap().iter().map(|r| r.path.clone()).collect()
        }
    }

    impl Transport for Replay {
        fn send(&self, req: &Req) -> Sent<'_> {
            self.asked.lock().unwrap().push(req.clone());
            let next = self.replies.lock().unwrap().pop_front();
            Box::pin(async move { next.ok_or_else(|| "no recorded reply left".to_string()) })
        }
    }

    pub fn block<T>(f: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(f)
    }
}

#[cfg(test)]
mod http_tests;

#[cfg(test)]
mod tests {
    use super::recorded::{Replay, block};
    use super::*;

    #[test]
    fn the_sites_credentials_go_only_to_the_site() {
        let base = "https://acme.atlassian.net";
        assert!(on_site("https://acme.atlassian.net/rest/api/3/attachment/content/10001", base));
        assert!(!on_site("https://api.media.atlassian.com/file/x/binary", base));
        assert!(!on_site("https://acme.atlassian.net.evil.example/x", base));
        assert!(!on_site("http://acme.atlassian.net/x", base));
        assert!(!on_site("not a url", base));
    }

    fn issue(key: &str) -> Value {
        json!({ "key": key, "fields": {
            "summary": format!("summary of {key}"),
            "issuetype": { "name": "Task" },
            "status": { "name": "To Do", "statusCategory": { "key": "new" } },
            "project": { "key": "ABC" },
            "updated": "2026-10-06T09:00:00.000+0000"
        }})
    }

    #[test]
    fn search_follows_next_page_tokens_until_the_last_page() {
        let r = Replay::new(vec![
            (200, json!({ "issues": [issue("ABC-1"), issue("ABC-2")], "nextPageToken": "p2", "isLast": false })),
            (200, json!({ "issues": [issue("ABC-3")], "isLast": true })),
        ]);
        let c = Client::over(r.clone());
        let none = |_: &str| None;
        let got = block(c.tickets("assignee = currentUser() ORDER BY updated DESC", &none, &[])).unwrap();
        let keys: Vec<&str> = got.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys, ["ABC-1", "ABC-2", "ABC-3"]);
        let paths = r.paths();
        assert_eq!(paths.len(), 2);
        assert!(paths[0].starts_with("/rest/api/3/search/jql?jql=assignee%20%3D%20currentUser%28%29"), "{}", paths[0]);
        assert!(paths[0].contains("&fields=summary%2Cissuetype"), "{}", paths[0]);
        assert!(!paths[0].contains("nextPageToken"));
        assert!(paths[1].ends_with("&nextPageToken=p2"), "{}", paths[1]);
    }

    #[test]
    fn a_read_is_tried_again_on_429_and_5xx_then_given_up() {
        let r = Replay::new(vec![
            (429, json!({})),
            (503, Value::Null),
            (200, json!({ "accountId": "acc-alice", "displayName": "Alice Example" })),
        ]);
        let me = block(Client::over(r.clone()).myself()).unwrap();
        assert_eq!(me.name, "Alice Example");
        assert_eq!(r.paths().len(), 3);

        let r = Replay::new(vec![(502, Value::Null), (502, Value::Null), (502, Value::Null), (200, json!({}))]);
        let e = block(Client::over(r.clone()).myself()).unwrap_err();
        assert_eq!(e.to_string(), "Jira answered 502, 3 times");
        assert_eq!(r.paths().len(), 3, "three tries, no more");

        let r = Replay::new(vec![(429, json!({})); 3]);
        let e = block(Client::over(r).myself()).unwrap_err();
        assert_eq!(e.to_string(), "Jira is limiting requests (429), still after 3 tries");
    }

    #[test]
    fn a_long_retry_after_is_not_waited_out_inside_a_read_but_said() {
        let r = Arc::new(Replay {
            replies: std::sync::Mutex::new(
                [Resp { status: 429, retry_after: Some(60), rate: Default::default(), body: "{}".into() }].into(),
            ),
            asked: Default::default(),
        });
        let e = block(Client::over(r.clone()).myself()).unwrap_err();
        assert_eq!(r.paths().len(), 1, "no second try into a limit");
        assert_eq!(Busy::wait(&e), Some(60));
        assert_eq!(e.to_string(), "Jira is limiting requests (429); it asks radar to wait 60s");
        let e = anyhow!("not busy");
        assert_eq!(Busy::wait(&e), None);
    }

    fn at(key: &str, updated: &str) -> Value {
        let mut i = issue(key);
        i["fields"]["updated"] = json!(updated);
        i
    }

    #[test]
    fn a_changed_only_read_asks_the_updated_window_and_nothing_else() {
        let r = Replay::new(vec![(200, json!({ "issues": [], "isLast": true }))]);
        let got = block(Client::over(r.clone()).view_list(
            "project = ABC ORDER BY updated DESC",
            &Ask::Changed { minutes: 4, sweep: false },
            &BTreeMap::new(),
            &|_| None,
            &[],
        ))
        .unwrap();
        assert!(got.changed_only && got.swept.is_none() && got.list.tickets.is_empty());
        let paths = r.paths();
        assert_eq!(paths.len(), 1, "nothing changed: one small request");
        assert!(
            paths[0].contains(&enc("(project = ABC) AND updated >= -4m ORDER BY updated DESC")),
            "{}",
            paths[0]
        );
    }

    #[test]
    fn a_sweep_lists_the_whole_views_keys_and_reads_only_the_unseen_by_key() {
        let held: BTreeMap<String, String> = [
            ("ABC-1".to_string(), "2026-10-06T09:00:00.000+0000".to_string()),
            ("ABC-2".to_string(), "2026-10-06T09:00:00.000+0000".to_string()),
            ("ABC-5".to_string(), "2026-10-01T09:00:00.000+0000".to_string()),
        ]
        .into();
        let r = Replay::new(vec![
            // Changed in the window: ABC-3.
            (200, json!({ "issues": [issue("ABC-3")], "isLast": true })),
            // The whole view: ABC-2 left; ABC-4 came in unchanged; ABC-5 changed unseen.
            (200, json!({ "issues": [
                at("ABC-3", "2026-10-06T09:00:00.000+0000"),
                at("ABC-1", "2026-10-06T09:00:00.000+0000"),
                at("ABC-4", "2026-09-01T09:00:00.000+0000"),
                at("ABC-5", "2026-10-02T09:00:00.000+0000"),
            ], "isLast": true })),
            (200, json!({ "issues": [issue("ABC-4"), issue("ABC-5")], "isLast": true })),
        ]);
        let got = block(Client::over(r.clone()).view_list(
            "project = ABC ORDER BY updated DESC",
            &Ask::Changed { minutes: 4, sweep: true },
            &held,
            &|_| None,
            &[],
        ))
        .unwrap();
        assert_eq!(got.swept.unwrap(), ["ABC-3", "ABC-1", "ABC-4", "ABC-5"]);
        let keys: Vec<&str> = got.list.tickets.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys, ["ABC-3", "ABC-4", "ABC-5"], "ABC-1 is held as it is: not read again");
        let paths = r.paths();
        assert_eq!(paths.len(), 3);
        assert!(paths[1].contains(&format!("jql={}&fields=updated&", enc("project = ABC ORDER BY updated DESC"))), "{}", paths[1]);
        assert!(paths[2].contains(&enc("key in (ABC-4, ABC-5)")), "{}", paths[2]);
    }

    #[test]
    fn myself_keeps_the_account_id_of_a_recorded_reply() {
        let r = Replay::new(vec![(
            200,
            json!({
                "self": "https://acme.atlassian.net/rest/api/3/user?accountId=712020:abcd",
                "accountId": "712020:abcd",
                "emailAddress": "alice@example.com",
                "displayName": "Alice Example",
                "active": true,
                "timeZone": "Europe/London"
            }),
        )]);
        let me = block(Client::over(r.clone()).myself()).unwrap();
        assert_eq!((me.account_id.as_str(), me.name.as_str()), ("712020:abcd", "Alice Example"));
        assert_eq!(r.paths(), ["/rest/api/3/myself"]);
    }

    #[test]
    fn a_write_is_never_tried_twice() {
        let r = Replay::new(vec![(503, Value::Null), (201, json!({ "id": "1" }))]);
        let req = Req {
            method: Method::Post,
            path: "/rest/api/3/issue/ABC-12/comment".into(),
            body: Some(json!({ "body": {} })),
        };
        let e = block(Client::over(r.clone()).request(req, "ticket ABC-12", false)).unwrap_err();
        assert_eq!(e.to_string(), "Jira answered 503");
        assert_eq!(r.paths().len(), 1, "sent once");
    }

    #[test]
    fn refusals_are_said_in_words() {
        let say = |status: u16, body: Value| {
            let r = Replay::new(vec![(status, body)]);
            block(Client::over(r).issue("ABC-12")).unwrap_err().to_string()
        };
        assert!(say(401, Value::Null).contains("check them under settings, Jira, connection"));
        assert_eq!(
            say(404, json!({ "errorMessages": ["Issue does not exist"] })),
            "Jira has no ticket ABC-12, or this account cannot see it (404)"
        );
        assert_eq!(
            say(403, json!({ "errorMessages": ["No permission"] })),
            "this account may not see the ticket ABC-12 (403): No permission"
        );
        let r = Replay::new(vec![(400, json!({ "errorMessages": ["Field 'sprnt' does not exist."] }))]);
        let none = |_: &str| None;
        let e = block(Client::over(r).tickets("sprnt = 1", &none, &[])).unwrap_err();
        assert_eq!(e.to_string(), "Jira refused the request (400): Field 'sprnt' does not exist.");
    }

    #[test]
    fn keys_are_checked_before_anything_is_sent() {
        let r = Replay::new(vec![]);
        let c = Client::over(r.clone());
        assert!(block(c.issue("ABC-1/../../x")).is_err());
        assert!(block(c.comments("abc-1", 0, 5)).is_err());
        assert!(block(c.project("A B")).is_err());
        assert!(r.paths().is_empty(), "nothing reached Jira");
    }

    #[test]
    fn a_ticket_page_brings_children_links_web_links_and_comments() {
        let mut main = issue("ABC-12");
        main["fields"]["parent"] = json!({ "key": "ABC-1", "fields": { "summary": "Dashboard" } });
        main["fields"]["customfield_10050"] = json!("Given a request, it is counted");
        main["fields"]["customfield_10020"] = json!([{ "name": "Sprint 41", "state": "active" }]);
        main["names"] = json!({ "customfield_10020": "Sprint" });
        let r = Replay::new(vec![
            (200, main),
            (200, json!({ "issues": [issue("ABC-13")], "isLast": true })),
            (200, json!([{ "object": { "url": "https://example.com/doc", "title": "Design" } }])),
            (200, json!({ "comments": [
                { "author": { "displayName": "Bob Example" }, "created": "2026-10-05T10:00:00.000+0000",
                  "body": { "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Looks right." }] }] } }
            ], "total": 51 })),
        ]);
        let d = block(Client::over(r.clone()).detail("ABC-12", Some("customfield_10050".into()))).unwrap();
        assert_eq!(radar_core::ui::rich::plain(&d.ticket.ac, 80), "Given a request, it is counted");
        assert_eq!(d.sprint.as_deref(), Some("Sprint 41"));
        let rel: Vec<&str> = d.links.iter().map(|l| l.relation.as_str()).collect();
        assert_eq!(rel, ["parent", "child", "web link"]);
        assert_eq!(radar_core::ui::rich::plain(&d.comments[0].body, 80), "Looks right.");
        assert_eq!(d.comments_total, 51);
        let paths = r.paths();
        assert!(paths[0].starts_with("/rest/api/3/issue/ABC-12?fields=*all"));
        assert!(paths.iter().any(|p| p.contains("jql=parent%20%3D%20ABC-12")));
        assert!(paths.iter().any(|p| p.ends_with("comment?startAt=0&maxResults=50&orderBy=-created")));
    }

    #[test]
    fn jql_checks_and_counts_read_jiras_answer() {
        let r = Replay::new(vec![
            (200, json!({ "queries": [{ "query": "x", "errors": ["The field 'sprnt' does not exist."] }] })),
            (200, json!({ "queries": [{ "query": "y" }] })),
            (200, json!({ "count": 37 })),
        ]);
        let c = Client::over(r.clone());
        assert_eq!(block(c.check_jql("sprnt = 1")).unwrap(), ["The field 'sprnt' does not exist."]);
        assert!(block(c.check_jql("sprint = 1")).unwrap().is_empty());
        assert_eq!(block(c.count("project = ABC")).unwrap(), 37);
        let asked = r.asked.lock().unwrap();
        assert_eq!(asked[0].method, Method::Post);
        assert_eq!(asked[0].body, Some(json!({ "queries": ["sprnt = 1"] })));
    }

    #[test]
    fn each_write_is_the_request_jira_documents() {
        let r = transition_req("ABC-12", "31").unwrap();
        assert_eq!((r.method, r.path.as_str()), (Method::Post, "/rest/api/3/issue/ABC-12/transitions"));
        assert_eq!(r.body, Some(json!({ "transition": { "id": "31" } })));
        let r = comment_req("ABC-12", "Fixed in **!263**.").unwrap();
        assert_eq!((r.method, r.path.as_str()), (Method::Post, "/rest/api/3/issue/ABC-12/comment"));
        assert_eq!(
            r.body.unwrap()["body"]["content"][0]["content"][1],
            json!({ "type": "text", "text": "!263", "marks": [{ "type": "strong" }] })
        );
        let r = assign_req("ABC-12", "acc-alice").unwrap();
        assert_eq!((r.method, r.path.as_str()), (Method::Put, "/rest/api/3/issue/ABC-12/assignee"));
        assert_eq!(r.body, Some(json!({ "accountId": "acc-alice" })));
        let r = create_req("ABC", "10001", " Count requests ", "Per day.").unwrap();
        assert_eq!((r.method, r.path.as_str()), (Method::Post, "/rest/api/3/issue"));
        let f = &r.body.as_ref().unwrap()["fields"];
        assert_eq!(f["project"], json!({ "key": "ABC" }));
        assert_eq!(f["issuetype"], json!({ "id": "10001" }));
        assert_eq!(f["summary"], "Count requests");
        assert_eq!(f["description"]["type"], "doc");
        assert!(create_req("ABC", "10001", "No description", " ").unwrap().body.unwrap()["fields"].get("description").is_none());
        // Refused before anything is built.
        assert!(transition_req("ABC-12/../x", "31").is_err());
        assert!(transition_req("ABC-12", "31; drop").is_err());
        assert!(comment_req("ABC-12", "  \n").is_err());
        assert!(assign_req("ABC-12", "").is_err());
        assert!(create_req("abc", "1", "x", "").is_err());
        assert!(create_req("ABC", "1", " ", "").is_err());
    }

    #[test]
    fn a_failed_write_is_said_once_and_a_good_one_reads_jiras_answer() {
        for status in [429, 500, 502] {
            let r = Replay::new(vec![(status, json!({})), (204, Value::Null)]);
            let req = assign_req("ABC-12", "acc-alice").unwrap();
            assert!(block(Client::over(r.clone()).write(req, "ticket ABC-12")).is_err());
            assert_eq!(r.paths().len(), 1, "{status}: sent once");
        }
        let r = Replay::new(vec![(400, json!({ "errors": { "summary": "Summary is required." } }))]);
        let req = create_req("ABC", "1", "x", "").unwrap();
        let e = block(Client::over(r).write(req, "project ABC")).unwrap_err();
        assert_eq!(e.to_string(), "Jira refused the request (400): Summary is required.");
        let r = Replay::new(vec![(201, json!({ "id": "10200", "key": "ABC-31" }))]);
        let v = block(Client::over(r).write(create_req("ABC", "1", "x", "").unwrap(), "project ABC")).unwrap();
        assert_eq!(v["key"], "ABC-31");
        let r = Replay::new(vec![(204, Value::Null)]);
        assert_eq!(block(Client::over(r).write(transition_req("ABC-12", "31").unwrap(), "ticket ABC-12")).unwrap(), Value::Null);
    }

    #[test]
    fn transitions_types_and_the_development_panel_are_read() {
        let r = Replay::new(vec![
            (200, json!({ "transitions": [
                { "id": "21", "name": "Start review", "to": { "name": "In Review", "statusCategory": { "key": "indeterminate" } } },
                { "id": "31", "name": "Done", "to": { "name": "Done", "statusCategory": { "key": "done" } } },
                { "name": "no id" }
            ]})),
            (200, json!({ "issueTypes": [
                { "id": "1", "name": "Story", "subtask": false },
                { "id": "2", "name": "Sub-task", "subtask": true },
                { "id": "3", "name": "Bug" }
            ]})),
            (200, json!({ "summary": { "pullrequest": { "byInstanceType": {
                "GitLab": { "count": 2, "name": "GitLab" }, "bitbucket": { "count": 0 } } } } })),
            (200, json!({ "detail": [{ "pullRequests": [
                { "id": "!263", "name": "ABC-12: stats", "url": "https://gitlab.com/acme/team-a/api/-/merge_requests/263",
                  "status": "OPEN", "repositoryName": "api", "source": { "branch": "abc-12-stats" } },
                { "id": "acme/team-a/web!187", "name": "ABC-12: page", "url": "https://gitlab.com/acme/team-a/web/-/merge_requests/187",
                  "status": "MERGED", "repositoryName": "web" }
            ]}]})),
        ]);
        let c = Client::over(r.clone());
        let t = block(c.transitions("ABC-12")).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].to.as_str(), t[0].category), ("In Review", model::Category::Doing));
        let types = block(c.issue_types("ABC")).unwrap();
        let names: Vec<&str> = types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Story", "Bug"]);
        let code = block(c.code("10012")).unwrap();
        assert_eq!(code[0].id, "!263");
        assert_eq!((code[1].id.as_str(), code[1].status.as_str()), ("!187", "merged"), "the path left off");
        assert_eq!((code[0].status.as_str(), code[0].branch.as_str()), ("open", "abc-12-stats"));
        let paths = r.paths();
        assert_eq!(paths[2], "/rest/dev-status/latest/issue/summary?issueId=10012");
        assert_eq!(paths[3], "/rest/dev-status/latest/issue/detail?issueId=10012&applicationType=GitLab&dataType=pullrequest");
        assert!(block(c.code("12; x")).unwrap().is_empty(), "not a number: not asked");
        assert_eq!(r.paths().len(), 4);
    }

    #[test]
    fn query_strings_are_encoded() {
        assert_eq!(enc("a b\"c&d=é"), "a%20b%22c%26d%3D%C3%A9");
    }
}
