//! How reads that run at the same time share Slack: a few in flight per
//! method, one lookup per person however many tasks ask, and a wait that
//! every caller of a method keeps once Slack has said "slow down".
//!
//! Slack limits calls per method per minute (`users.info` Tier 4,
//! `conversations.replies` Tier 3) and allows short bursts, so radar bounds how
//! many are in flight and honours `Retry-After` for all of them, rather than
//! spacing every call out and making a cold start crawl.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;

use tokio::sync::{Mutex as AsyncMutex, OnceCell, Semaphore, SemaphorePermit};

/// Calls to one method in flight at once, at most.
pub(crate) const IN_FLIGHT: usize = 4;

type Name = Arc<OnceCell<Result<String, String>>>;

#[derive(Debug)]
pub(crate) struct Pace {
    names: Semaphore,
    replies: Semaphore,
    /// Held by the caller waiting out a 429 on a method; every other call to
    /// the method passes through it before it is sent.
    pauses: Mutex<BTreeMap<&'static str, Arc<AsyncMutex<()>>>>,
    /// The name of each person asked for in this round of reading.
    lookups: Mutex<BTreeMap<String, Name>>,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            names: Semaphore::new(IN_FLIGHT),
            replies: Semaphore::new(IN_FLIGHT),
            pauses: Mutex::default(),
            lookups: Mutex::default(),
        }
    }
}

impl Pace {
    pub(crate) fn pause(&self, method: &'static str) -> Arc<AsyncMutex<()>> {
        self.pauses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(method)
            .or_default()
            .clone()
    }

    /// A new round of reading asks everyone again, once.
    pub(crate) fn new_round(&self) {
        self.lookups.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub(crate) fn lookup(&self, id: &str) -> Name {
        self.lookups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(id.to_string())
            .or_default()
            .clone()
    }

    pub(crate) async fn name_turn(&self) -> SemaphorePermit<'_> {
        self.names.acquire().await.expect("the semaphore is never closed")
    }

    pub(crate) async fn replies_turn(&self) -> SemaphorePermit<'_> {
        self.replies.acquire().await.expect("the semaphore is never closed")
    }
}

/// Set by the first failure of a batch: calls not yet sent are left out.
#[derive(Debug, Default)]
pub(crate) struct Stop(AtomicBool);

impl Stop {
    pub(crate) fn set(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub(crate) fn is_set(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Every future to its end, polled together and in order, results in the
/// order given. With a [`Pace`] turn inside each, that is "at most N at once,
/// the first N in order".
pub(crate) async fn all<F: Future>(futures: impl IntoIterator<Item = F>) -> Vec<F::Output> {
    let mut running: Vec<Option<Pin<Box<F>>>> =
        futures.into_iter().map(|f| Some(Box::pin(f))).collect();
    let mut done: Vec<Option<F::Output>> = running.iter().map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (slot, out) in running.iter_mut().zip(done.iter_mut()) {
            if let Some(f) = slot {
                match f.as_mut().poll(cx) {
                    Poll::Ready(v) => {
                        *out = Some(v);
                        *slot = None;
                    }
                    Poll::Pending => pending = true,
                }
            }
        }
        if pending { Poll::Pending } else { Poll::Ready(()) }
    })
    .await;
    done.into_iter().flatten().collect()
}

#[cfg(test)]
#[path = "pace_tests.rs"]
mod tests;
