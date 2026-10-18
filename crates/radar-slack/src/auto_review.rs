//! Local, account-scoped intake. First activation starts now, never at the
//! beginning of a channel. Repeated reads cannot enqueue the same message.
use crate::{SlackPlugin, agent_context};
use radar_core::agent_context::ReviewRequest;
use radar_core::plugin::Effect;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    since: i64,
    handled: BTreeSet<String>,
}

pub(crate) fn poll(p: &mut SlackPlugin) -> Vec<Effect> {
    if !p.cfg.review.auto_review || !p.authenticated() {
        return vec![];
    }
    let Some(channel) = p
        .cfg
        .channels
        .iter()
        .find(|c| c.alias == p.cfg.review.channel && c.watched)
        .map(|c| c.id.clone())
    else {
        return vec![];
    };
    let scope = format!("{channel}/{}", p.cfg.review.group);
    if !p.data.auto_reviews.contains_key(&scope) {
        p.data.auto_reviews.insert(
            scope.clone(),
            Checkpoint {
                since: p.now,
                handled: BTreeSet::new(),
            },
        );
        return vec![Effect::PersistCache];
    }
    let mut checkpoint = p.data.auto_reviews[&scope].clone();
    checkpoint.since = checkpoint
        .since
        .max(p.now - p.cfg.days.min(30) as i64 * 86400);
    checkpoint
        .handled
        .retain(|ts| seconds(ts) >= checkpoint.since);
    let Some(data) = p.data.channels.get(&channel) else {
        return vec![];
    };
    let mut requests = vec![];
    for message in data.messages.iter().chain(data.replies.values().flatten()) {
        if seconds(&message.ts) <= checkpoint.since || checkpoint.handled.contains(&message.ts) {
            continue;
        }
        let text = message.text.to_lowercase();
        let addressed = if p.cfg.review.group.is_empty() {
            !agent_context::merge_request_links(&crate::mrkdwn::links(&message.text)).is_empty()
                || text
                    .split(|c: char| !c.is_alphabetic())
                    .any(|w| matches!(w, "review" | "ptal"))
        } else {
            [">", "|"].iter().any(|end| {
                message
                    .text
                    .contains(&format!("<!subteam^{}{end}", p.cfg.review.group))
            })
        };
        if !addressed {
            continue;
        }
        let links = agent_context::thread_merge_requests(p, &channel, &message.ts);
        if links.is_empty() {
            continue;
        }
        if let Some(origin) = agent_context::key(p, &channel, &message.ts) {
            requests.push((
                message.ts.clone(),
                ReviewRequest {
                    automatic: true,
                    origin,
                    links: links.into_iter().take(agent_context::MAX_LINKS).collect(),
                    changes: vec![],
                },
            ));
        }
    }
    // One message per tick; the dock serializes background reviews.
    requests.sort_by(|a, b| a.0.cmp(&b.0));
    let Some((ts, request)) = requests.into_iter().next() else {
        return vec![];
    };
    checkpoint.handled.insert(ts);
    p.data.auto_reviews.insert(scope, checkpoint);
    vec![Effect::StartReview(request), Effect::PersistCache]
}
fn seconds(ts: &str) -> i64 {
    ts.split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    #[test]
    fn new_requests_queue_once_across_refresh_and_restore_without_replaying_history() {
        let mut p = demo::sample();
        p.cfg.review.auto_review = true;
        assert!(
            poll(&mut p)
                .iter()
                .all(|e| !matches!(e, Effect::StartReview(_)))
        );
        let ts = format!("{}.000001", p.now + 1);
        let text = format!(
            "<!subteam^{}> please review <https://git.example/ankh-morpork/watch/-/merge_requests/12>",
            p.cfg.review.group
        );
        p.data
            .channels
            .get_mut(demo::TEAM)
            .unwrap()
            .messages
            .push(demo::msg(&ts, "UVIMES", &text));
        p.now += 2;
        let fx = poll(&mut p);
        assert!(
            matches!(&fx[0],Effect::StartReview(r) if r.automatic && r.links.len() == 1 && r.origin.id.ends_with(&ts))
        );
        assert!(poll(&mut p).is_empty());
        p.data = serde_json::from_value(serde_json::to_value(&p.data).unwrap()).unwrap();
        assert!(poll(&mut p).is_empty());
        p.session_owner = None;
        assert!(poll(&mut p).is_empty());
    }
    #[test]
    fn other_channels_and_other_groups_do_not_trigger() {
        let mut p = demo::sample();
        p.cfg.review.auto_review = true;
        poll(&mut p);
        let ts = format!("{}.000001", p.now + 1);
        let text = format!(
            "<!subteam^{}extra> review https://github.com/ankh-morpork/watch/pull/12",
            p.cfg.review.group
        );
        p.data
            .channels
            .get_mut(demo::TEAM)
            .unwrap()
            .messages
            .push(demo::msg(&ts, "UVIMES", &text));
        p.data
            .channels
            .get_mut(demo::SVC)
            .unwrap()
            .messages
            .push(demo::msg(&ts, "UVIMES", &text.replace("extra", "")));
        p.now += 2;
        assert!(poll(&mut p).is_empty());
    }
}
