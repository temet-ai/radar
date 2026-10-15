//! One-use, session-local ownership for settings replies. Configured identity is
//! compared without resolving credentials; no request/incarnation enters history.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{Check, GitlabPlugin, settings_history::Kind, write_context::ConnectionStamp};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum Target {
    Auth,
    Base,
    Source(String),
}

impl Target {
    pub(crate) fn key(&self) -> String {
        match self {
            Self::Auth => "auth".into(),
            Self::Base => "base".into(),
            Self::Source(path) => format!("group:{path}"),
        }
    }

    pub(crate) fn kind(&self) -> Kind {
        match self {
            Self::Auth => Kind::Auth,
            Self::Base => Kind::Base,
            Self::Source(_) => Kind::Source,
        }
    }

    pub(crate) fn payload(&self) -> String {
        serde_json::to_string(self).expect("settings check target JSON")
    }

    pub(crate) fn from_payload(value: &str) -> Option<Self> {
        serde_json::from_str(value).ok()
    }

    pub(crate) fn menu(&self) -> String {
        format!("ask-check:{}", self.payload())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Request {
    id: u64,
    connection: ConnectionStamp,
    base: Option<u64>,
    membership: Option<u64>,
}

#[derive(Debug, Default)]
pub(crate) struct Owners {
    // Never reset when a target, connection or membership leaves.
    next: u64,
    pub(crate) pending: BTreeMap<Target, Request>,
    base: Option<String>,
    base_incarnation: u64,
    next_membership: u64,
    watched: BTreeMap<String, u64>,
}

/// Observe every identity transition, not just the value at reply time. In
/// particular present -> absent -> present and unwatched -> watched -> unwatched
/// retire the old owner even when no intervening check was dispatched.
pub(crate) fn reconcile(p: &mut GitlabPlugin) {
    let watched: BTreeSet<_> = p.cfg.sources.iter().map(|s| p.cfg.group_path(s)).collect();
    let connection = p.connection_stamp();
    let owners = &mut p.check_owners;
    if owners.base.as_ref() != Some(&p.cfg.base) {
        owners.base_incarnation = owners
            .base_incarnation
            .checked_add(1)
            .expect("GitLab base incarnation exhausted");
        owners.base = Some(p.cfg.base.clone());
    }
    owners.watched.retain(|path, _| watched.contains(path));
    for path in watched {
        owners.watched.entry(path).or_insert_with(|| {
            owners.next_membership = owners
                .next_membership
                .checked_add(1)
                .expect("GitLab source membership sequence exhausted");
            owners.next_membership
        });
    }
    owners.pending.retain(|target, request| {
        let current = request.connection == connection
            && match target {
                Target::Auth => true,
                Target::Base => request.base == Some(owners.base_incarnation),
                Target::Source(path) => request.membership == owners.watched.get(path).copied(),
            };
        if !current && matches!(p.checks.get(&target.key()), Some((Check::Running, _))) {
            p.checks.remove(&target.key());
        }
        current
    });
}

pub(crate) fn begin(p: &mut GitlabPlugin, target: &Target) -> Request {
    reconcile(p);
    let connection = p.connection_stamp();
    let owners = &mut p.check_owners;
    owners.next = owners
        .next
        .checked_add(1)
        .expect("GitLab settings request sequence exhausted");
    let request = Request {
        id: owners.next,
        connection,
        base: matches!(target, Target::Base).then_some(owners.base_incarnation),
        membership: match target {
            Target::Source(path) => owners.watched.get(path).copied(),
            _ => None,
        },
    };
    owners.pending.insert(target.clone(), request);
    request
}

/// Consume only the exact current owner; a late or duplicate reply cannot remove
/// a newer request for the same target, or mutate its Running/terminal status.
pub(crate) fn claim(p: &mut GitlabPlugin, target: &Target, request: Request) -> bool {
    reconcile(p);
    if p.check_owners.pending.get(target) != Some(&request) {
        return false;
    }
    p.check_owners.pending.remove(target);
    true
}

#[cfg(test)]
mod tests;
