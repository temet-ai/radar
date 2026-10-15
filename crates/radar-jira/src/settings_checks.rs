//! One-use ownership of settings site/project replies. Targets and generations
//! are transient; no credentials, provider identities or menu IDs enter history.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{Check, JiraPlugin, settings_history::Kind};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum Target {
    Site(String),
    Project { site: String, project: String },
}

impl Target {
    pub(crate) fn project(site: &str, project: &str) -> Self {
        Self::Project {
            site: site.into(),
            project: project.into(),
        }
    }

    pub(crate) fn site(&self) -> &str {
        match self {
            Self::Site(site) | Self::Project { site, .. } => site,
        }
    }

    pub(crate) fn kind(&self) -> Kind {
        match self {
            Self::Site(_) => Kind::Site,
            Self::Project { .. } => Kind::Project,
        }
    }

    pub(crate) fn menu(&self) -> String {
        format!(
            "ask-check:{}",
            serde_json::to_string(self).expect("settings target JSON")
        )
    }

    pub(crate) fn from_menu(value: &str) -> Option<Self> {
        serde_json::from_str(value).ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Request {
    id: u64,
    generation: u64,
    membership: Option<u64>,
}

#[derive(Debug, Default)]
pub(crate) struct Owners {
    // Never reset when a target, connection or membership is removed.
    next: u64,
    pub(crate) pending: BTreeMap<Target, Request>,
    next_membership: u64,
    // Current watched pairs only. An absent -> present transition retires an
    // unwatched owner immediately, so absence cannot revive it after an ABA.
    watched: BTreeMap<Target, u64>,
}

pub(crate) fn reconcile(p: &mut JiraPlugin) {
    let watched: BTreeSet<_> = p
        .cfg
        .sites
        .iter()
        .flat_map(|site| {
            site.projects
                .iter()
                .map(|project| Target::project(&site.name, &project.key))
        })
        .collect();
    let owners = &mut p.check_owners;
    owners.watched.retain(|target, _| watched.contains(target));
    for target in watched {
        owners.watched.entry(target).or_insert_with(|| {
            owners.next_membership = owners
                .next_membership
                .checked_add(1)
                .expect("Jira project membership sequence exhausted");
            owners.next_membership
        });
    }
    owners.pending.retain(|target, request| {
        let current = p.connections.generation(target.site()) == Some(request.generation)
            && owners.watched.get(target).copied() == request.membership;
        if !current && matches!(p.checks.get(target), Some((Check::Running, _))) {
            p.checks.remove(target);
        }
        current
    });
}

pub(crate) fn begin(p: &mut JiraPlugin, target: &Target) -> Request {
    crate::write_context::reconcile(p);
    let owners = &mut p.check_owners;
    owners.next = owners
        .next
        .checked_add(1)
        .expect("Jira settings request sequence exhausted");
    let request = Request {
        id: owners.next,
        generation: p
            .connections
            .generation(target.site())
            .expect("configured check site"),
        membership: owners.watched.get(target).copied(),
    };
    owners.pending.insert(target.clone(), request);
    request
}

/// Claim once before any application mutation. A stale/duplicate delivery can
/// never clear the pending owner of a newer request for the same typed target.
pub(crate) fn claim(p: &mut JiraPlugin, target: &Target, request: Request) -> bool {
    crate::write_context::reconcile(p);
    if p.check_owners.pending.get(target) != Some(&request) {
        return false;
    }
    p.check_owners.pending.remove(target);
    true
}
