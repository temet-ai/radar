//! Session-local configured-connection incarnations and accepted write ownership.
//! Compare only configured values; never resolve, fingerprint, persist or print
//! credentials. Rotation behind an unchanged environment reference is not detected.

use std::collections::BTreeMap;
use std::fmt;

use radar_core::journal::{ActionDescriptor, Origin, Outcome};
use radar_core::plugin::Effect;

use crate::JiraPlugin;
use crate::config::Site;

#[derive(PartialEq, Eq)]
struct Configured {
    url: String,
    email: String,
    token: String,
}

impl From<&Site> for Configured {
    fn from(site: &Site) -> Self {
        Self {
            url: site.url.clone(),
            email: site.email.clone(),
            token: site.token.clone(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Connections {
    next: u64,
    sites: BTreeMap<String, (Configured, u64)>,
}

// The retained equality inputs can contain literal secrets. Do not derive Debug.
impl fmt::Debug for Connections {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connections")
            .field("count", &self.sites.len())
            .finish_non_exhaustive()
    }
}

impl Connections {
    pub(crate) fn reconcile(&mut self, sites: &[Site]) -> Vec<String> {
        let before: BTreeMap<_, _> = self
            .sites
            .iter()
            .map(|(name, (_, generation))| (name.clone(), *generation))
            .collect();
        self.sites
            .retain(|name, _| sites.iter().any(|s| &s.name == name));
        for site in sites {
            let configured = Configured::from(site);
            if self
                .sites
                .get(&site.name)
                .is_some_and(|(old, _)| old == &configured)
            {
                continue;
            }
            self.next = self
                .next
                .checked_add(1)
                .expect("Jira connection sequence exhausted");
            self.sites
                .insert(site.name.clone(), (configured, self.next));
        }
        before
            .keys()
            .chain(self.sites.keys())
            .filter(|name| before.get(*name).copied() != self.generation(name))
            .cloned()
            .collect()
    }

    /// The sites met so far.
    pub(crate) fn names(&self) -> Vec<String> {
        self.sites.keys().cloned().collect()
    }

    pub(crate) fn generation(&self, site: &str) -> Option<u64> {
        self.sites.get(site).map(|(_, generation)| *generation)
    }
}

/// Allocated only for a write that passed the final dispatch guards. The action
/// contains sanitized scope, never the connection's configured values.
#[derive(Debug, Clone)]
pub(crate) struct Dispatch {
    pub id: u64,
    pub generation: u64,
    pub action: ActionDescriptor,
}

pub(crate) fn reconcile(p: &mut JiraPlugin) {
    let known = p.connections.names();
    let changed = p.connections.reconcile(&p.cfg.sites);
    // A connection that changed or went while radar ran takes the pages it
    // kept with it (one met for the first time, as after a restart, keeps
    // the cache it came with).
    for name in changed.iter().filter(|n| known.contains(n)) {
        if let Some(data) = p.data.get_mut(name) {
            data.details.clear();
        }
    }
    crate::page::invalidate(p, &changed);
    crate::settings_checks::reconcile(p);
}

pub(crate) fn current(p: &mut JiraPlugin, site: &str, dispatch: &Dispatch) -> bool {
    reconcile(p);
    p.connections.generation(site) == Some(dispatch.generation)
}

pub(crate) fn skipped(dispatch: &Dispatch) -> Effect {
    let mut action = ActionDescriptor::new(
        "jira.write.follow_up.skipped",
        Origin::Plugin("jira".into()),
    );
    action.subjects.clone_from(&dispatch.action.subjects);
    action.metadata.redacted = dispatch.action.metadata.redacted;
    action.metadata.omitted_subjects = dispatch.action.metadata.omitted_subjects;
    action.metadata.count = Some(1);
    Effect::Observe {
        action,
        outcome: Outcome::Blocked,
    }
}

#[cfg(test)]
mod tests;
