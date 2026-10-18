//! Latency tracing for the renderer pipe (`RADAR_TRACE_LATENCY=1`): what each
//! snapshot answers (a renderer intent, a host event), how long the host
//! took from that cause to the snapshot ready to send, and when a session
//! started from the dock can first be written to. Lines go to radar.log
//! (`latency host: ...`); the snapshot carries a [`SnapshotTrace`] so the
//! renderer logs the rest of the way to the frame it paints
//! (`ink: latency ...`). Off, the bridge holds `None` and does nothing.

use std::collections::BTreeSet;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::dock::DockView;
use crate::host::Event;
use crate::view::Intent;

use super::SnapshotTrace;

/// The variable that turns tracing on (`1`).
pub const VAR: &str = "RADAR_TRACE_LATENCY";

/// Whether `RADAR_TRACE_LATENCY` asks for tracing.
pub fn wanted(value: Option<&str>) -> bool {
    value.map(str::trim) == Some("1")
}

#[derive(Debug, Clone)]
struct Cause {
    word: String,
    at: Instant,
    epoch_ms: f64,
}

impl Cause {
    fn now(word: String) -> Self {
        Self {
            word,
            at: Instant::now(),
            epoch_ms: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0),
        }
    }
}

/// What one snapshot sent cost the host, for its log line.
#[derive(Debug, Default, Clone, Copy)]
pub struct Costs {
    /// Building the snapshot (the host's views, the dock).
    pub build_ms: f64,
    /// Deciding it changed.
    pub compare_ms: f64,
    /// Measuring it against the line bound.
    pub measure_ms: f64,
    pub bytes: usize,
    /// The dock's transcript items, and how many of them were sent.
    pub items: usize,
    pub items_sent: usize,
}

#[derive(Debug, Default)]
pub struct Trace {
    /// The oldest change not yet answered by a snapshot sent.
    pending: Option<Cause>,
    /// The newest renderer intent not yet answered.
    seq: Option<u64>,
    /// Tabs seen in a snapshot sent.
    tabs: BTreeSet<String>,
    /// A tab that appeared and cannot be written to yet, with what made it
    /// and the intent that did.
    starting: Option<(String, Cause, Option<u64>)>,
    /// A message sent from the composer and not in the transcript yet:
    /// the send, its intent and the transcript's length when it was sent.
    sending: Option<(Cause, u64, usize)>,
}

impl Trace {
    /// A renderer intent arrived; the renderer holds `items` transcript
    /// items.
    pub fn intent(&mut self, seq: u64, intent: &Intent, items: usize) {
        let value = serde_json::to_value(intent).unwrap_or_default();
        let mut word = format!("intent {}", value["intent"].as_str().unwrap_or("?"));
        if let Some(d) = value["dock"].as_str() {
            word.push(' ');
            word.push_str(d);
        }
        if word == "intent dock send" {
            self.sending = Some((Cause::now(word.clone()), seq, items));
        }
        self.pending.get_or_insert_with(|| Cause::now(word));
        self.seq = Some(seq);
    }

    /// One of the host's own events arrived (a tick, a task's message).
    pub fn event(&mut self, event: &Event) {
        let word = match event {
            Event::Intent(_) => return,
            Event::Tick(_) => "event tick",
            Event::Wake(_) => "event wake",
            Event::Msg { .. } => "event msg",
            Event::Tracked { .. } => "event tracked",
            Event::Notice(_) | Event::ErrorNotice(_) => "event notice",
            _ => "event",
        };
        self.pending.get_or_insert_with(|| Cause::now(word.into()));
    }

    /// Nothing showed a change: what was pending is answered by the screen
    /// as it is.
    pub fn unchanged(&mut self) {
        self.pending = None;
        self.seq = None;
    }

    /// A snapshot is about to be sent with `dock`: what it answers, logged
    /// with `costs`, and the session-ready line when a tab started from
    /// the dock can now be written to.
    pub fn sent(&mut self, cursor: u64, dock: Option<&DockView>, costs: Costs) -> Option<SnapshotTrace> {
        let cause = self.pending.take().unwrap_or_else(|| Cause::now("host".into()));
        let seq = self.seq.take();
        let host_ms = cause.at.elapsed().as_secs_f64() * 1000.0;
        crate::log::info(format!(
            "latency host: cursor {cursor} answers {}{}: {host_ms:.2} ms since (build {:.2}, compare {:.2}, measure {:.2}), {} bytes, transcript {} items, {} sent",
            cause.word,
            seq.map(|s| format!(" (seq {s})")).unwrap_or_default(),
            costs.build_ms,
            costs.compare_ms,
            costs.measure_ms,
            costs.bytes,
            costs.items,
            costs.items_sent,
        ));
        let mut trace = SnapshotTrace { cause: cause.word.clone(), seq, at_ms: cause.epoch_ms, host_ms, bytes: costs.bytes as u64 };
        if let Some(dock) = dock {
            if let Some(active) = &dock.active
                && !self.tabs.contains(active)
                && !ready(dock)
            {
                self.starting = Some((active.clone(), cause, seq));
            }
            self.tabs.extend(dock.tabs.iter().map(|t| t.id.clone()));
            if let Some((id, started, by)) = &self.starting
                && dock.active.as_ref() == Some(id)
                && ready(dock)
            {
                let ms = started.at.elapsed().as_secs_f64() * 1000.0;
                crate::log::info(format!("latency host: session {id} ready {ms:.1} ms after {}", started.word));
                trace = SnapshotTrace { cause: format!("session ready ({})", started.word), seq: *by, at_ms: started.epoch_ms, host_ms: ms, bytes: trace.bytes };
                self.starting = None;
            }
        }
        // The message sent shows: the transcript grew past what the renderer
        // held when it was sent.
        if let Some((sent, seq, before)) = &self.sending
            && costs.items > *before
        {
            let ms = ms_since(sent.at);
            crate::log::info(format!("latency host: the message sent shows {ms:.1} ms after intent dock send"));
            trace = SnapshotTrace { cause: "message shown (intent dock send)".into(), seq: Some(*seq), at_ms: sent.epoch_ms, host_ms: ms, bytes: trace.bytes };
            self.sending = None;
        }
        Some(trace)
    }
}

/// The active session takes a message.
fn ready(dock: &DockView) -> bool {
    dock.composer.can_send && dock.composer.disabled.is_none()
}

/// Milliseconds since `at`.
pub fn ms_since(at: Instant) -> f64 {
    at.elapsed().as_secs_f64() * 1000.0
}
