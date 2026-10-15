//! Transient dispatch identity for read results. This prevents late application;
//! cache ownership is separately established by current-session authentication.

use std::collections::{BTreeMap, BTreeSet};

use radar_core::journal::{ActionDescriptor, Origin, Outcome, Reason, StableCode};
use radar_core::plugin::Effect;

use crate::{SlackPlugin, client::Me};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ReadStamp(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CheckStamp {
    read: ReadStamp,
    request: u64,
}

#[derive(Debug, Default)]
pub(crate) struct ReadContext {
    epoch: u64,
    next_check: u64,
    checks: BTreeMap<String, u64>,
    /// Channel additions are independent intents, not replaceable status checks.
    additions: BTreeSet<u64>,
    /// Explicit check-all targets, leased to exactly one workspace request.
    channel_checks: Option<(CheckStamp, Vec<String>)>,
}

impl SlackPlugin {
    pub(crate) fn read_stamp(&self) -> ReadStamp {
        ReadStamp(self.read_context.epoch)
    }

    pub(crate) fn is_current_read(&self, stamp: ReadStamp) -> bool {
        stamp == self.read_stamp()
    }

    pub(crate) fn begin_check(&mut self, key: &str) -> CheckStamp {
        if key == "workspace" {
            self.read_context.channel_checks = None;
        }
        self.read_context.next_check = self
            .read_context
            .next_check
            .checked_add(1)
            .expect("Slack check sequence exhausted");
        let request = self.read_context.next_check;
        if key == "add" {
            self.read_context.additions.insert(request);
        } else {
            self.read_context.checks.insert(key.into(), request);
        }
        CheckStamp {
            read: self.read_stamp(),
            request,
        }
    }

    pub(crate) fn defer_channel_checks(&mut self, stamp: CheckStamp, ids: Vec<String>) {
        self.read_context.channel_checks = Some((stamp, ids));
    }

    pub(crate) fn take_channel_checks(&mut self, stamp: CheckStamp) -> Vec<String> {
        if self.is_current_read(stamp.read)
            && self
                .read_context
                .channel_checks
                .as_ref()
                .is_some_and(|(owner, _)| *owner == stamp)
        {
            return self.read_context.channel_checks.take().unwrap().1;
        }
        Vec::new()
    }

    pub(crate) fn complete_check(&mut self, key: &str, stamp: CheckStamp) -> bool {
        if !self.is_current_read(stamp.read) {
            return false;
        }
        if key == "add" {
            return self.read_context.additions.remove(&stamp.request);
        }
        if self.read_context.checks.get(key) != Some(&stamp.request) {
            return false;
        }
        self.read_context.checks.remove(key);
        true
    }

    pub(crate) fn invalidate_reads(&mut self) {
        self.read_context.epoch = self
            .read_context
            .epoch
            .checked_add(1)
            .expect("Slack read generation exhausted");
        self.read_context.checks.clear();
        self.read_context.additions.clear();
        self.read_context.channel_checks = None;
        self.checks.clear();
        self.sync.reading = 0;
        self.sync.started = None;
        self.sync.last_ok = None;
        self.sync.errors.clear();
        self.sync.groups_at = 0;
        self.sync.channel_reads.clear();
        self.sync.for_you_read = None;
        self.threads.clear();
        self.pages
            .retain(|page| matches!(page, crate::page::Page::Draft));
        crate::page::invalidate_links(self);
        self.shown.borrow_mut().clear();
        self.opened_once.borrow_mut().clear();
        let _ = self.tree.replace(Default::default());
        self.logged_refusals.clear();
        self.changed();
        // Authored editors survive, with old read stamps that cannot send.
    }

    /// Compare configured references only; never resolve or fingerprint secrets.
    pub(crate) fn replace_token(&mut self, token: String) {
        if self.cfg.token != token {
            self.cfg.token = token;
            self.revoke_session();
        }
    }

    /// Another API address is another place to ask who the token is.
    pub(crate) fn replace_api(&mut self, api: String) {
        if self.cfg.api != api {
            self.cfg.api = api;
            self.revoke_session();
        }
    }

    pub(crate) fn revoke_session(&mut self) {
        self.park_account();
        self.invalidate_reads();
    }

    pub(crate) fn install_me(&mut self, me: Me) -> bool {
        self.activate_account(me)
    }
}

/// Separate result-application evidence; never overrides the task's real result.
/// Generic reads have no captured safe scope and must not borrow the new account.
pub(crate) fn skipped(scope: Option<&ActionDescriptor>) -> Effect {
    let mut action = scope.cloned().unwrap_or_else(|| {
        ActionDescriptor::new(
            "slack.read.apply.skipped.stale",
            Origin::Plugin("slack".into()),
        )
    });
    action.kind = StableCode::new("slack.read.apply.skipped.stale");
    action.metadata.count = Some(1);
    action.metadata.truncated = false;
    action.metadata.reason = Some(Reason::Unspecified);
    Effect::Observe {
        action,
        outcome: Outcome::Blocked,
    }
}
