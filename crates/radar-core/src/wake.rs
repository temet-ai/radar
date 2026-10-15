//! A part that hears from its own background work (an agent streaming text) wakes the
//! host now instead of waiting for the next 200 ms tick.
//!
//! The host gives each part a [`Waker`] ([`crate::Plugin::set_waker`]). A part calls
//! [`Waker::wake`] from any thread when it has something new; the runner's [`pump`] turns
//! the wakes into [`crate::host::Event::Wake`], at most one per [`COALESCE`], and the host
//! hands each woken part [`crate::Plugin::woken`]. A flood of wakes (one per streamed
//! token) is one event per window, so the renderer gets one snapshot per window, not one
//! per token; the first wake after a quiet spell goes out at once.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::sync::mpsc::UnboundedSender;

/// The shortest time between two wake events: a burst of wakes inside it is one event.
pub const COALESCE: Duration = Duration::from_millis(40);

#[derive(Default)]
struct Inner {
    woken: Mutex<BTreeSet<usize>>,
    notify: Notify,
}

/// The host's end: which parts woke, and a way to wait for the next wake.
#[derive(Clone, Default)]
pub struct WakeHub {
    inner: Arc<Inner>,
}

impl WakeHub {
    /// The waker of the part at `plugin`.
    pub fn waker(&self, plugin: usize) -> Waker {
        Waker { inner: self.inner.clone(), plugin }
    }

    /// The parts that woke since the last call, in order; they are forgotten.
    pub fn take(&self) -> Vec<usize> {
        std::mem::take(&mut *self.inner.woken.lock().expect("wakes")).into_iter().collect()
    }

    /// Resolves when a part has woken since the last time this resolved (a wake that
    /// came while nobody waited counts).
    pub async fn notified(&self) {
        self.inner.notify.notified().await;
    }
}

/// A part's end: cheap to clone, usable from any thread.
#[derive(Clone)]
pub struct Waker {
    inner: Arc<Inner>,
    plugin: usize,
}

impl Waker {
    /// Something new for the part: the host asks it soon ([`crate::Plugin::woken`]).
    pub fn wake(&self) {
        self.inner.woken.lock().expect("wakes").insert(self.plugin);
        // One stored permit at most: many wakes before the pump looks are one.
        self.inner.notify.notify_one();
    }
}

impl std::fmt::Debug for Waker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Waker({})", self.plugin)
    }
}

/// Turns wakes into `event()`s on `tx` until it closes: one at once, then none for
/// `every`, then one more if anything woke meanwhile.
pub async fn pump<E>(hub: WakeHub, tx: UnboundedSender<E>, every: Duration, event: impl Fn() -> E) {
    loop {
        hub.notified().await;
        if tx.send(event()).is_err() {
            return;
        }
        tokio::time::sleep(every).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;
    use tokio::time::{Instant, advance};

    async fn settle() {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }

    #[test]
    fn the_hub_says_which_parts_woke_once() {
        let hub = WakeHub::default();
        let (a, b) = (hub.waker(0), hub.waker(3));
        b.wake();
        a.wake();
        b.wake();
        assert_eq!(hub.take(), vec![0, 3]);
        assert!(hub.take().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_wake_goes_out_at_once_and_a_flood_is_one_event_per_window() {
        let hub = WakeHub::default();
        let waker = hub.waker(1);
        let (tx, mut rx) = unbounded_channel();
        let t0 = Instant::now();
        tokio::spawn(pump(hub.clone(), tx, COALESCE, Instant::now));
        settle().await;
        assert!(rx.try_recv().is_err(), "nothing woke");

        waker.wake();
        settle().await;
        assert_eq!(rx.try_recv().ok(), Some(t0), "the first wake is not delayed");

        // A token every 5 ms for 200 ms: one event per 40 ms window, not forty.
        let mut sent = 0;
        for _ in 0..40 {
            waker.wake();
            advance(Duration::from_millis(5)).await;
            settle().await;
            while rx.try_recv().is_ok() {
                sent += 1;
            }
        }
        assert!((4..=6).contains(&sent), "{sent} events for 40 wakes in 200 ms");

        // Quiet again: the last wake of the flood still goes out, then nothing.
        advance(COALESCE).await;
        settle().await;
        while rx.try_recv().is_ok() {}
        advance(Duration::from_secs(1)).await;
        settle().await;
        assert!(rx.try_recv().is_err(), "no wake, no event");
    }

    #[tokio::test(start_paused = true)]
    async fn the_pump_ends_when_the_host_is_gone() {
        let hub = WakeHub::default();
        let (tx, rx) = unbounded_channel::<()>();
        drop(rx);
        let pump = tokio::spawn(pump(hub.clone(), tx, COALESCE, || ()));
        hub.waker(0).wake();
        settle().await;
        assert!(pump.is_finished());
    }
}
