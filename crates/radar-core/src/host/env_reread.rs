//! Reading a part's `env:` settings again from the person's shell, so a
//! token exported after radar started is used without a restart: before the
//! chip's `refresh now`, and about once a minute while the part is signed
//! out (it syncs again as soon as a value changed).

use super::{Host, HostEffect};
use crate::plugin::Effect;
use crate::view::chrome::HealthState;

/// How often a signed-out part's variables are read again, in seconds.
const SIGNED_OUT_EVERY: i64 = 60;

impl Host {
    /// The part's `env:` settings when the shell can be asked for them.
    fn rereadable(&self, p: usize) -> Option<Vec<String>> {
        if crate::system::sandboxed() {
            return None;
        }
        let settings = self.plugins[p].env_settings();
        (!crate::env::names_in(settings.iter().map(String::as_str)).is_empty()).then_some(settings)
    }

    /// `refresh now` from a part's chip: its variables first, then the sync.
    pub(super) fn refresh_part(&mut self, p: usize) -> Vec<HostEffect> {
        match self.rereadable(p) {
            Some(settings) => self.route(p, vec![Effect::Spawn(crate::env::reread_task(settings, true))], 0),
            None => {
                let fx = self.plugins[p].refresh_now();
                self.route(p, fx, 0)
            }
        }
    }

    /// On a tick: each signed-out part's variables, at most once a minute.
    pub(super) fn reread_signed_out(&mut self, now: i64) -> Vec<HostEffect> {
        let mut out = Vec::new();
        for p in 0..self.plugins.len() {
            let signed_out = self.plugins[p]
                .health()
                .is_some_and(|h| h.state(now) == HealthState::SignedOut);
            if !signed_out {
                self.env_reread.remove(&p);
                continue;
            }
            if self.env_reread.get(&p).is_some_and(|at| now - at < SIGNED_OUT_EVERY) {
                continue;
            }
            let Some(settings) = self.rereadable(p) else { continue };
            self.env_reread.insert(p, now);
            out.extend(self.route(p, vec![Effect::Spawn(crate::env::reread_task(settings, false))], 0));
        }
        out
    }

    /// A reread task's answer: sync the part when it asked to.
    pub(super) fn on_reread(&mut self, p: usize, reread: crate::env::Reread) -> Vec<HostEffect> {
        if !reread.refresh {
            return vec![];
        }
        crate::log::info(format!("{}: read its env: settings again; syncing", self.plugins[p].id()));
        let fx = self.plugins[p].refresh_now();
        self.route(p, fx, 0)
    }
}
