//! Completed CLI provider observations. The host timestamps receipt of the
//! envelope; these facts do not survive a process killed before completion.

use std::collections::BTreeMap;

use radar_core::journal::{ActionDescriptor, Origin, Outcome, Reason, SubjectRole, SubjectSnapshot};
use radar_core::plugin::{CliExecution, CliObservations, ObservedCliTask};
use radar_core::time;
use serde_json::{Value, json};

use crate::client::{Me, MessagesRead};
use crate::model::{ts_key, ts_secs};
use crate::sync_history::NamesEvidence;
use crate::{
    Client, Settings, SlackPlugin, cache, config, model, mrkdwn, sync_history, view,
};

pub(crate) fn run(plugin: &SlackPlugin, matches: &clap::ArgMatches) -> ObservedCliTask {
    #[cfg(feature = "test-support")]
    if plugin.offline_fixture {
        return Box::pin(async {
            CliExecution::new(
                Err(anyhow::anyhow!(
                    "offline test fixture cannot run provider commands"
                )),
                CliObservations::default(),
            )
        });
    }
    let cfg = plugin.cfg.clone();
    let data = plugin.cache_snapshot();
    let m = matches.clone();
    let now = time::now_secs();
    #[cfg(any(test, feature = "test-support"))]
    let via = plugin.via.clone();
    Box::pin(async move {
        let mut evidence = Evidence::default();
        let result = async {
            match m.subcommand() {
                Some(("cache-recovery", _)) => data.recovery(),
                Some((command @ ("list" | "catchup"), m)) => {
                    let api = cfg.resolved_api().inspect_err(|_| {
                        evidence.refused(Reason::Unavailable);
                    })?;
                    let token = cfg.resolved_token().map_err(|e| {
                        evidence.refused(Reason::Unavailable);
                        anyhow::anyhow!("{e:#}: run radar, press , for settings, then set it under Slack, connection")
                    })?;
                    #[cfg(any(test, feature = "test-support"))]
                    let client = match via.0 {
                        Some(transport) => {
                            let wait: crate::client::Wait = std::sync::Arc::new(|_| Box::pin(async {}));
                            Ok(Client::new(transport).with_wait(wait))
                        }
                        None => Client::http(&api, &token),
                    };
                    #[cfg(not(any(test, feature = "test-support")))]
                    let client = Client::http(&api, &token);
                    let c = client.inspect_err(|_| {
                        evidence.refused(Reason::Unavailable);
                    })?;
                    if command == "list" {
                        let view = m.get_one::<String>("view").cloned();
                        let filter = m.get_one::<String>("filter").cloned();
                        list_inner(&c, cfg, data, view, filter, m.get_flag("json"), now, &mut evidence).await
                    } else {
                        let only = m.get_one::<String>("channel").cloned();
                        catchup_inner(&c, cfg, data, only, m.get_flag("json"), now, &mut evidence).await
                    }
                }
                _ => anyhow::bail!("unknown slack command"),
            }
        }.await;
        CliExecution::new(result, evidence.observations)
    })
}

#[derive(Default)]
struct Evidence {
    observations: CliObservations,
}

impl Evidence {
    fn count(&mut self, mut action: ActionDescriptor, n: usize, outcome: Outcome) {
        action.metadata.count = Some(u32::try_from(n).unwrap_or(u32::MAX));
        action.metadata.truncated |= n > u32::MAX as usize;
        self.observations.observe(action, outcome);
    }

    fn unscoped(kind: &'static str) -> ActionDescriptor {
        ActionDescriptor::new(kind, Origin::Plugin("slack".into()))
    }

    fn refused(&mut self, reason: Reason) {
        let mut action = Self::unscoped("slack.cli.refused");
        action.metadata.reason = Some(reason);
        self.count(action, 1, Outcome::Blocked);
    }

    fn auth(&mut self, result: &anyhow::Result<Me>) {
        let mut action = Self::unscoped("slack.cli.auth");
        if result.is_err() {
            action.metadata.reason = Some(Reason::Unspecified);
        }
        self.count(
            action,
            1,
            if result.is_ok() {
                Outcome::Succeeded
            } else {
                Outcome::Failed
            },
        );
    }

    fn identity(&mut self, me: &Me, accepted: bool) {
        let mut action = Self::unscoped("slack.cli.identity");
        // Cache acceptance remains the existing authority. History may omit
        // scope for a valid identity that cannot fit its own safe key bounds.
        if accepted {
            if let Some(key) = sync_history::auth_workspace(&me.team_id) {
                action
                    .subjects
                    .push(SubjectSnapshot::new(key, SubjectRole::Result));
            } else {
                action.metadata.redacted = true;
                action.metadata.omitted_subjects = 1;
            }
        } else {
            action.metadata.reason = Some(Reason::InvalidInput);
        }
        self.count(
            action,
            1,
            if accepted {
                Outcome::Succeeded
            } else {
                Outcome::Partial
            },
        );
    }

    fn history(&mut self, team: &str, channel: &str, result: &anyhow::Result<MessagesRead>) {
        let scope = |kind| sync_history::action(kind, team, channel);
        let Ok(read) = result else {
            let mut action = scope("slack.cli.history.errors");
            action.metadata.reason = Some(Reason::Unspecified);
            self.count(action, 1, Outcome::Failed);
            return;
        };
        let outcome = if read.evidence.truncated || read.evidence.incomplete_pages != 0 {
            Outcome::Partial
        } else {
            Outcome::Succeeded
        };
        let mut action = scope("slack.cli.history.messages");
        action.metadata.truncated = read.evidence.truncated;
        self.count(action, read.messages.len(), outcome);
        if read.evidence.truncated {
            let mut action = scope("slack.cli.history.lists.truncated");
            action.metadata.truncated = true;
            self.count(action, 1, Outcome::Partial);
        }
        if read.evidence.incomplete_pages != 0 {
            self.count(
                scope("slack.cli.history.pages.incomplete"),
                read.evidence.incomplete_pages,
                Outcome::Partial,
            );
        }
    }

    fn names(&mut self, team: &str, channel: &str, names: NamesEvidence) {
        // Counts only. A name lookup failure is optional enrichment and never
        // rewrites a successful required messages read or its CLI output.
        let scope = |kind| sync_history::action(kind, team, channel);
        if names.attempted != 0 {
            self.count(
                scope("slack.cli.names.attempted"),
                names.attempted,
                if names.errors == 0 {
                    Outcome::Succeeded
                } else {
                    Outcome::Partial
                },
            );
            self.count(
                scope("slack.cli.names.returned"),
                names.attempted.saturating_sub(names.errors),
                Outcome::Succeeded,
            );
        }
        if names.errors != 0 {
            let mut action = scope("slack.cli.names.fallback");
            action.metadata.reason = Some(Reason::Unspecified);
            self.count(action, names.errors, Outcome::Partial);
        }
    }
}

async fn authenticate(
    c: &Client,
    data: cache::AccountCache,
    evidence: &mut Evidence,
) -> anyhow::Result<model::Data> {
    let auth = c.auth_test().await;
    evidence.auth(&auth);
    let me = auth?;
    evidence.identity(&me, cache::AccountOwner::of(&me).is_some());
    data.authenticated_data(me)
}

// Preserve the test-facing legacy entry points; all production calls enter the
// observation envelope, including callers using the legacy Plugin method.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn list(
    c: &Client,
    cfg: Settings,
    data: impl Into<cache::AccountCache>,
    view: Option<String>,
    filter: Option<String>,
    json: bool,
    now: i64,
) -> anyhow::Result<String> {
    list_inner(
        c,
        cfg,
        data,
        view,
        filter,
        json,
        now,
        &mut Evidence::default(),
    )
    .await
}

#[cfg(test)]
pub(crate) async fn catchup(
    c: &Client,
    cfg: Settings,
    data: impl Into<cache::AccountCache>,
    only: Option<String>,
    json: bool,
    now: i64,
) -> anyhow::Result<String> {
    catchup_inner(c, cfg, data, only, json, now, &mut Evidence::default()).await
}

/// What `radar slack list` prints, read through `c`: the last `days` of each
/// watched channel (or the one `view` names), newest first, the rows that
/// have every word of `filter`. JSON: `id`, `channel`, `updated` (RFC 3339),
/// `author` as `{name, handle}`, `text`, `replies` and `url`.
#[allow(clippy::too_many_arguments)]
async fn list_inner(
    c: &Client,
    cfg: Settings,
    data: impl Into<cache::AccountCache>,
    view: Option<String>,
    filter: Option<String>,
    json: bool,
    now: i64,
    evidence: &mut Evidence,
) -> anyhow::Result<String> {
    let chans: Vec<&config::Channel> = match &view {
        Some(name) => {
            let name = name.trim_start_matches('#');
            let found = cfg
                .channels
                .iter()
                .find(|c| c.alias.eq_ignore_ascii_case(name) || c.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| {
                    evidence.refused(Reason::InvalidInput);
                    anyhow::anyhow!("no channel called {name} in settings")
                })?;
            vec![found]
        }
        None => cfg
            .watched()
            .filter(|c| c.kind != config::Kind::Alerts)
            .collect(),
    };
    if chans.is_empty() {
        evidence.refused(Reason::Unavailable);
        anyhow::bail!("no watched channels: add one under channels in settings");
    }
    let mut data = authenticate(c, data.into(), evidence).await?;
    let team = data
        .me
        .as_ref()
        .map(|me| me.team_id.as_str())
        .unwrap_or_default();
    let workspace = data.me.as_ref().map(|m| m.url.clone()).unwrap_or_default();
    let words: Vec<String> = filter
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    // Who wrote what: the name shown and the handle, asked once each.
    let mut people: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut rows = Vec::new();
    for ch in chans {
        let since = format!("{}.000000", now - cfg.days as i64 * 86_400);
        let read = c.history_observed(&ch.id, Some(&since), 300).await;
        evidence.history(team, &ch.id, &read);
        let msgs = read?.messages;
        let mut names = NamesEvidence::default();
        for m in msgs {
            let who = match &m.user {
                Some(u) => {
                    if !people.contains_key(u) {
                        names.attempted += 1;
                        let one = match c.user(u).await {
                            Ok(user) => (user.shown(), user.name),
                            Err(_) => {
                                names.errors += 1;
                                (data.name(u), String::new())
                            }
                        };
                        data.users.insert(u.clone(), one.0.clone());
                        people.insert(u.clone(), one);
                    }
                    people[u].clone()
                }
                None => (
                    m.bot_name.clone().unwrap_or_else(|| "someone".into()),
                    String::new(),
                ),
            };
            let names = model::Namer {
                data: &data,
                cfg: &cfg,
            };
            let text = mrkdwn::plain(&m.text, &names);
            let hay = format!("{} {} {} {}", ch.alias, who.0, who.1, text).to_lowercase();
            if !words.iter().all(|w| hay.contains(w)) {
                continue;
            }
            rows.push((ts_key(&m.ts), ch.alias.clone(), ch.id.clone(), m, who, text));
        }
        evidence.names(team, &ch.id, names);
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    if json {
        let out: Vec<Value> = rows
            .iter()
            .map(|(_, alias, id, m, who, text)| {
                json!({
                    "id": m.ts, "channel": alias, "updated": time::to_rfc3339(ts_secs(&m.ts)),
                    "author": {"name": who.0, "handle": who.1}, "text": text,
                    "replies": m.reply_count, "url": model::permalink(&workspace, id, &m.ts, None),
                })
            })
            .collect();
        return Ok(serde_json::to_string_pretty(&out)? + "\n");
    }
    let mut text = String::new();
    for (_, alias, _, m, who, body) in &rows {
        let first = body.lines().next().unwrap_or_default();
        let replies = if m.reply_count > 0 {
            format!("  · {}", view::replies(m.reply_count))
        } else {
            String::new()
        };
        text.push_str(&format!(
            "#{alias} {:<6} {:<16} {first}{replies}\n",
            time::ago(ts_secs(&m.ts), now),
            radar_core::ui::fit(&who.0, 16),
        ));
    }
    Ok(text)
}

/// What `radar slack catchup` prints, read through `c`.
async fn catchup_inner(
    c: &Client,
    cfg: Settings,
    data: impl Into<cache::AccountCache>,
    only: Option<String>,
    json: bool,
    now: i64,
    evidence: &mut Evidence,
) -> anyhow::Result<String> {
    let chans: Vec<&config::Channel> = match &only {
        Some(name) => {
            let name = name.trim_start_matches('#');
            let c = cfg
                .channels
                .iter()
                .find(|c| c.alias.eq_ignore_ascii_case(name) || c.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| {
                    evidence.refused(Reason::InvalidInput);
                    anyhow::anyhow!("no channel called {name} in settings")
                })?;
            vec![c]
        }
        None => cfg.watched().collect(),
    };
    if chans.is_empty() {
        evidence.refused(Reason::Unavailable);
        anyhow::bail!("no watched channels: add one under channels in settings");
    }
    let mut data = authenticate(c, data.into(), evidence).await?;
    let team = data
        .me
        .as_ref()
        .map(|me| me.team_id.as_str())
        .unwrap_or_default();
    let workspace = data.me.as_ref().map(|m| m.url.clone()).unwrap_or_default();
    let mut out_json = Vec::new();
    let mut text = String::new();
    for ch in chans {
        let since = data
            .marks
            .get(&ch.id)
            .cloned()
            .unwrap_or_else(|| format!("{}.000000", now - cfg.days as i64 * 86_400));
        let read = c.history_observed(&ch.id, Some(&since), 300).await;
        evidence.history(team, &ch.id, &read);
        let mut msgs = read?.messages;
        let mut names = NamesEvidence::default();
        msgs.sort_by_key(|m| ts_key(&m.ts));
        for m in &msgs {
            for u in m
                .user
                .iter()
                .cloned()
                .chain(mrkdwn::mentions(&m.text).users)
            {
                if let std::collections::btree_map::Entry::Vacant(entry) = data.users.entry(u) {
                    names.attempted += 1;
                    match c.user(entry.key()).await {
                        Ok(user) => {
                            entry.insert(user.shown());
                        }
                        Err(_) => names.errors += 1,
                    }
                }
            }
        }
        evidence.names(team, &ch.id, names);
        let names = model::Namer {
            data: &data,
            cfg: &cfg,
        };
        let marked = data.marks.contains_key(&ch.id);
        if json {
            for m in &msgs {
                out_json.push(json!({
                    "channel": ch.alias, "ts": m.ts, "updated": time::to_rfc3339(ts_secs(&m.ts)),
                    "author": {"name": data.author(m), "handle": ""}, "text": mrkdwn::plain(&m.text, &names),
                    "replies": m.reply_count, "links": mrkdwn::links(&m.text),
                    "url": model::permalink(&workspace, &ch.id, &m.ts, None),
                }));
            }
            continue;
        }
        text.push_str(&format!(
            "#{} · {} new {}\n",
            ch.alias,
            msgs.len(),
            if marked {
                format!("since you caught up ({})", view::when(&since))
            } else {
                format!("in the last {} days (no caught-up mark yet)", cfg.days)
            }
        ));
        for m in &msgs {
            let first = mrkdwn::first_line(&m.text, &names);
            let replies = if m.reply_count > 0 {
                format!("  · {}", view::replies(m.reply_count))
            } else {
                String::new()
            };
            text.push_str(&format!(
                "  {:<12} {:<16} {}{replies}\n",
                view::when(&m.ts),
                radar_core::ui::fit(&data.author(m), 16),
                first
            ));
        }
        text.push('\n');
    }
    if json {
        return Ok(serde_json::to_string_pretty(&out_json)? + "\n");
    }
    Ok(text)
}

#[cfg(test)]
mod tests;
