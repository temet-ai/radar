//! Transient connection identity for post-write refreshes and dedicated MR reads.
//! Original results, held data and authored text keep their existing handling.

use radar_core::journal::{ActionDescriptor, Origin, Outcome};
use radar_core::plugin::Effect;

use crate::GitlabPlugin;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConnectionStamp(u64);

impl ConnectionStamp {
    pub(crate) fn generation(self) -> u64 {
        self.0
    }
}

impl GitlabPlugin {
    pub(crate) fn connection_stamp(&self) -> ConnectionStamp {
        self.connection
    }

    pub(crate) fn is_current_connection(&self, stamp: ConnectionStamp) -> bool {
        self.connection == stamp
    }

    /// Compare configured values only. Never resolve or fingerprint credentials.
    /// A generation, rather than value equality on completion, also fences A-B-A.
    pub(crate) fn replace_connection(&mut self, url: String, token: String) {
        if self.cfg.url != url || self.cfg.token != token {
            self.connection.0 = self
                .connection
                .0
                .checked_add(1)
                .expect("GitLab connection generation exhausted");
            crate::pages::invalidate(self);
            crate::pages::invalidate_logs(self);
            crate::runs_ownership::invalidate(self);
            self.cfg.url = url;
            self.cfg.token = token;
        }
        crate::settings_checks::reconcile(self);
    }
}

/// Application evidence, never a replacement for the original write's outcome.
/// Do not borrow the newer connection's identity or include configured values.
pub(crate) fn skipped() -> Effect {
    let mut action = ActionDescriptor::new(
        "gitlab.write.refresh.skipped.stale",
        Origin::Plugin("gitlab".into()),
    );
    action.metadata.count = Some(1);
    Effect::Observe {
        action,
        outcome: Outcome::Blocked,
    }
}

#[cfg(test)]
mod tests;
