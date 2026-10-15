//! Pure action collection plus the bounded, runner-owned SQLite writer.
//! No raw input, key, message payload or copied display label enters this layer.
//! Explicit work commands may carry bounded, deliberately authored SemanticLabel
//! annotations. Their request bodies and Debug representations are never logged.
//! UI history reads share the ordered queue but never audit or mutate the journal.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use crate::history::ReadError;
use crate::journal::work_map::{EventCursor, EventFilter, EventPage, Limits, Snapshot};
use crate::journal::{
    ActionDescriptor, ActionId, Journal, Metadata, Origin, Outcome, Reason, SessionId,
    SessionOrigin, WriteMode,
};

/// A reducer-produced record. The runner alone performs the database work.
#[derive(Clone, Debug)]
pub enum JournalRecord {
    Begin {
        id: ActionId,
        descriptor: ActionDescriptor,
        parent: Option<ActionId>,
    },
    Outcome {
        id: ActionId,
        outcome: Outcome,
        metadata: Metadata,
    },
    /// An explicit recording gap; never silently pretend the trace is complete.
    Gap { dropped: u64 },
}

const PENDING_LIMIT: usize = 4096;
const WRITER_LIMIT: usize = 2048;
const HISTORY_LIMITS: Limits = Limits {
    records: 50,
    bytes: 262_144,
};

pub(crate) struct MenuTrace {
    pub action: ActionId,
    pub parent: Option<ActionId>,
    pub semantic: Option<ActionId>,
}

/// Pure, opt-in reducer state. Unit-test hosts leave it disabled.
#[derive(Default)]
pub(crate) struct Trace {
    pub enabled: bool,
    pub current: Option<ActionId>,
    pub menu: Option<MenuTrace>,
    pub semantic: Option<ActionId>,
    pub delivering_task: Option<ActionId>,
    pub seed: Option<ActionId>,
    pub mode: Option<WriteMode>,
    next_id: u64,
    pending: Vec<JournalRecord>,
    overflow: u64,
}

impl Trace {
    fn push(&mut self, record: JournalRecord) {
        if self.pending.len() < PENDING_LIMIT {
            self.pending.push(record);
        } else {
            self.overflow = self.overflow.saturating_add(1);
        }
    }

    pub fn begin(
        &mut self,
        mut descriptor: ActionDescriptor,
        parent: Option<ActionId>,
    ) -> Option<ActionId> {
        if !self.enabled {
            return None;
        }
        // The runner chooses a namespace once. Reduction only increments
        // this counter: no clock, randomness, filesystem or database calls.
        let seed = self.seed.as_ref()?;
        self.next_id = self.next_id.checked_add(1)?;
        if self.next_id >= (1u64 << 48) {
            self.overflow = self.overflow.saturating_add(1);
            return None;
        }
        let id = ActionId::parse(&format!("{}{:012x}", &seed.as_str()[..24], self.next_id)).ok()?;
        descriptor.metadata.mode = self.mode;
        self.push(JournalRecord::Begin {
            id: id.clone(),
            descriptor,
            parent,
        });
        Some(id)
    }

    pub fn host(&mut self, kind: &'static str, parent: Option<ActionId>) -> Option<ActionId> {
        self.begin(ActionDescriptor::new(kind, Origin::Host), parent)
    }

    pub fn record(&mut self, id: Option<ActionId>, outcome: Outcome, reason: Option<Reason>) {
        if outcome == Outcome::Requested {
            return;
        } // Begin already records it.
        if let Some(id) = id {
            if self
                .pending
                .iter()
                .rev()
                .find_map(|record| match record {
                    JournalRecord::Outcome {
                        id: existing,
                        outcome,
                        ..
                    } if existing == &id => Some(*outcome),
                    _ => None,
                })
                .is_some_and(|previous| previous == outcome || previous.is_terminal())
            {
                return;
            }
            self.push(JournalRecord::Outcome {
                id,
                outcome,
                metadata: Metadata {
                    reason,
                    ..Default::default()
                },
            });
        }
    }

    pub fn terminal(&self, action: &ActionId) -> bool {
        self.pending.iter().any(|record| {
            matches!(record,
            JournalRecord::Outcome { id, outcome, .. } if id == action && outcome.is_terminal())
        })
    }

    pub fn drain(&mut self) -> Vec<JournalRecord> {
        let mut records = std::mem::take(&mut self.pending);
        let dropped = std::mem::take(&mut self.overflow);
        if dropped > 0 {
            records.push(JournalRecord::Gap { dropped });
        }
        records
    }
}

enum Work {
    Record {
        record: Box<JournalRecord>,
        observed_at_ms: i64,
    },
    Command {
        command: Box<crate::work_cli::Command>,
        mode: crate::writes::Mode,
        observed_at_ms: i64,
        reply: tokio::sync::oneshot::Sender<Result<crate::work_cli::Reply, crate::work_cli::Error>>,
    },
    /// UI discovery does not audit itself or produce further writer work.
    History {
        filter: Box<EventFilter>,
        cursor: Option<Box<EventCursor>>,
        snapshot: Option<Snapshot>,
        reply: tokio::sync::oneshot::Sender<Result<EventPage, ReadError>>,
    },
    MapRequest {
        request: Box<crate::work_api::Request>,
        mode: crate::writes::Mode,
        observed_at_ms: i64,
        reply:
            tokio::sync::oneshot::Sender<Result<crate::work_api::Response, crate::work_api::Error>>,
    },
    Finish,
}

/// Clones can enqueue from task threads without blocking the event loop.
#[derive(Clone)]
pub(crate) struct Sink {
    sender: mpsc::SyncSender<Work>,
    warning: Arc<Mutex<Option<&'static str>>>,
    dropped: Arc<AtomicU64>,
    accepting: Arc<Mutex<bool>>,
}

impl Sink {
    /// Local work is ordered after all earlier records. Read requests do not
    /// audit themselves; mutation requests use the audited CLI implementation.
    pub(crate) async fn work(
        &self,
        request: crate::work_api::Request,
        mode: crate::writes::Mode,
    ) -> Result<crate::work_api::Response, crate::work_api::Error> {
        let mutation = request.is_mutation();
        let (reply, receive) = tokio::sync::oneshot::channel();
        {
            let accepting = self
                .accepting
                .lock()
                .map_err(|_| crate::work_api::Error::Unavailable)?;
            if !*accepting {
                return Err(crate::work_api::Error::Unavailable);
            }
            self.sender
                .try_send(Work::MapRequest {
                    request: Box::new(request),
                    mode,
                    observed_at_ms: observed_now(),
                    reply,
                })
                .map_err(|_| crate::work_api::Error::Unavailable)?;
        }
        receive.await.map_err(|_| {
            if mutation {
                crate::work_api::Error::OutcomeUnknown
            } else {
                crate::work_api::Error::Unavailable
            }
        })?
    }

    /// Read after all earlier queued records, using a stable bounded snapshot.
    /// No audit action or recording gap is created by this request.
    pub(crate) async fn history(
        &self,
        filter: EventFilter,
        cursor: Option<EventCursor>,
        snapshot: Option<Snapshot>,
    ) -> Result<EventPage, ReadError> {
        let (reply, receive) = tokio::sync::oneshot::channel();
        {
            let accepting = self.accepting.lock().map_err(|_| ReadError::Unavailable)?;
            if !*accepting {
                return Err(ReadError::Unavailable);
            }
            self.sender
                .try_send(Work::History {
                    filter: Box::new(filter),
                    cursor: cursor.map(Box::new),
                    snapshot,
                    reply,
                })
                .map_err(|_| ReadError::Unavailable)?;
        }
        receive.await.map_err(|_| ReadError::Unavailable)?
    }

    pub(crate) async fn command(
        &self,
        command: crate::work_cli::Command,
        mode: crate::writes::Mode,
    ) -> Result<crate::work_cli::Reply, crate::work_cli::Error> {
        let applied = matches!(&command, crate::work_cli::Command::Apply(_));
        let (reply, receive) = tokio::sync::oneshot::channel();
        {
            let accepting = self
                .accepting
                .lock()
                .map_err(|_| crate::work_cli::Error::Unavailable)?;
            if !*accepting {
                return Err(crate::work_cli::Error::Unavailable);
            }
            self.sender
                .try_send(Work::Command {
                    command: Box::new(command),
                    mode,
                    observed_at_ms: observed_now(),
                    reply,
                })
                .map_err(|_| crate::work_cli::Error::Unavailable)?;
        }
        receive.await.map_err(|_| {
            if applied {
                crate::work_cli::Error::OutcomeUnknown
            } else {
                crate::work_cli::Error::Unavailable
            }
        })?
    }

    pub fn submit(&self, record: JournalRecord) {
        let Ok(accepting) = self.accepting.lock() else {
            self.warn("action recording degraded: writer synchronization failed");
            return;
        };
        // Shutdown seals the queue before its final marker. Late task results
        // belong to the now-ended session and cannot race that marker.
        if !*accepting {
            return;
        }
        if let JournalRecord::Gap { dropped } = record {
            self.dropped.fetch_add(dropped, Ordering::Relaxed);
            self.warn(
                "action recording degraded: observation records omitted; some actions are missing",
            );
            return;
        }
        let work = Work::Record {
            record: Box::new(record),
            observed_at_ms: observed_now(),
        };
        if let Err(error) = self.sender.try_send(work) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            self.warn(match error {
                mpsc::TrySendError::Full(_) => {
                    "action recording degraded: writer queue full; some actions are missing"
                }
                mpsc::TrySendError::Disconnected(_) => {
                    "action recording degraded: journal writer stopped"
                }
            });
        }
    }

    fn warn(&self, message: &'static str) {
        if let Ok(mut warning) = self.warning.lock() {
            *warning = Some(message);
        }
    }

    pub fn take_warning(&self) -> Option<&'static str> {
        self.warning.lock().ok()?.take()
    }

    pub fn begin(&self, kind: &'static str, origin: Origin, parent: Option<ActionId>) -> ActionId {
        self.begin_descriptor(ActionDescriptor::new(kind, origin), parent)
    }

    pub fn begin_descriptor(
        &self,
        descriptor: ActionDescriptor,
        parent: Option<ActionId>,
    ) -> ActionId {
        let id = ActionId::new();
        self.submit(JournalRecord::Begin {
            id: id.clone(),
            descriptor,
            parent,
        });
        id
    }

    pub fn outcome(&self, id: &ActionId, outcome: Outcome, reason: Option<Reason>) {
        self.submit(JournalRecord::Outcome {
            id: id.clone(),
            outcome,
            metadata: Metadata {
                reason,
                ..Default::default()
            },
        });
    }
}

/// Owns the only writing connection and flushes it at normal shutdown.
pub(crate) struct Recording {
    pub sink: Sink,
    worker: Option<JoinHandle<()>>,
}

impl Recording {
    pub fn open(origin: SessionOrigin) -> anyhow::Result<Self> {
        Self::open_at(&Journal::default_path()?, origin)
    }

    pub(crate) fn open_at(path: &std::path::Path, origin: SessionOrigin) -> anyhow::Result<Self> {
        let mut journal = Journal::open(path)?;
        let session = journal.start_session_with(origin)?;
        let (sender, receiver) = mpsc::sync_channel(WRITER_LIMIT);
        let warning = Arc::new(Mutex::new(None));
        let sink = Sink {
            sender,
            warning,
            dropped: Arc::new(AtomicU64::new(0)),
            accepting: Arc::new(Mutex::new(true)),
        };
        let status = sink.warning.clone();
        let dropped = sink.dropped.clone();
        let worker = std::thread::spawn(move || {
            let mut write_errors = 0u64;
            while let Ok(work) = receiver.recv() {
                // A UI read must not flush pending gaps or otherwise mutate the
                // journal. The next writing operation still persists those gaps.
                let read = matches!(&work, Work::History { .. })
                    || matches!(&work, Work::MapRequest { request, .. } if !request.is_mutation());
                if !read
                    && flush_gap(&mut journal, &session, &dropped, &mut write_errors).is_err()
                    && let Ok(mut warning) = status.lock()
                {
                    *warning = Some("action recording degraded: recording gap could not be saved");
                }
                let result = match work {
                    Work::Record {
                        record,
                        observed_at_ms,
                    } => persist(&mut journal, &session, *record, observed_at_ms),
                    Work::Command {
                        command,
                        mode,
                        observed_at_ms,
                        reply,
                    } => {
                        let execution = crate::work_cli::execute(
                            &mut journal,
                            &session,
                            *command,
                            mode,
                            observed_at_ms,
                        );
                        let degraded = execution.recording_degraded;
                        let _ = reply.send(execution.result);
                        if degraded {
                            Err(anyhow::anyhow!("work command recording degraded"))
                        } else {
                            Ok(())
                        }
                    }
                    Work::History {
                        filter,
                        cursor,
                        snapshot,
                        reply,
                    } => {
                        let page = journal
                            .query_events_at(
                                &filter,
                                HISTORY_LIMITS,
                                cursor.as_deref(),
                                snapshot.as_ref(),
                            )
                            .map_err(|_| ReadError::Unavailable);
                        let _ = reply.send(page);
                        // Query/receiver failures are not missing action writes.
                        Ok(())
                    }
                    Work::MapRequest {
                        request,
                        mode,
                        observed_at_ms,
                        reply,
                    } => {
                        let execution = crate::work_api::execute(
                            &mut journal,
                            &session,
                            *request,
                            mode,
                            observed_at_ms,
                        );
                        let _ = reply.send(execution.result);
                        if execution.recording_degraded {
                            Err(anyhow::anyhow!("work command recording degraded"))
                        } else {
                            Ok(())
                        }
                    }
                    Work::Finish => {
                        if journal
                            .finish_session(&session)
                            .and_then(|_| {
                                flush_gap(&mut journal, &session, &dropped, &mut write_errors)
                            })
                            .and_then(|_| journal.flush())
                            .is_err()
                        {
                            write_errors = write_errors.saturating_add(1);
                            let _ = flush_gap(&mut journal, &session, &dropped, &mut write_errors);
                            if let Ok(mut warning) = status.lock() {
                                *warning = Some("action recording degraded: shutdown flush failed");
                            }
                        }
                        break;
                    }
                };
                if result.is_err() {
                    write_errors = write_errors.saturating_add(1);
                    if let Ok(mut warning) = status.lock() {
                        *warning = Some(
                            "action recording degraded: journal write failed; some actions are missing",
                        );
                    }
                }
            }
        });
        Ok(Self {
            sink,
            worker: Some(worker),
        })
    }

    pub fn finish(&mut self) {
        if let Some(worker) = self.worker.take() {
            if let Ok(mut accepting) = self.sink.accepting.lock() {
                *accepting = false;
            }
            let send_failed = self.sink.sender.send(Work::Finish).is_err();
            let join_failed = worker.join().is_err();
            if send_failed || join_failed {
                self.sink.warn(
                    "action recording degraded: journal writer stopped before shutdown flush",
                );
            }
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.finish();
    }
}

fn observed_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().min(i64::MAX as u128) as i64
        })
}

fn flush_gap(
    journal: &mut Journal,
    session: &SessionId,
    dropped: &AtomicU64,
    write_errors: &mut u64,
) -> anyhow::Result<()> {
    let count = dropped.swap(0, Ordering::Relaxed);
    let errors = std::mem::take(write_errors);
    if count == 0 && errors == 0 {
        return Ok(());
    }
    if let Err(error) = journal.record_gap(session, count, errors, observed_now()) {
        // A transient failure must not erase the fact that recording failed.
        dropped.fetch_add(count, Ordering::Relaxed);
        *write_errors = errors;
        return Err(error);
    }
    Ok(())
}

fn persist(
    journal: &mut Journal,
    session: &SessionId,
    record: JournalRecord,
    observed_at_ms: i64,
) -> anyhow::Result<()> {
    match record {
        JournalRecord::Begin {
            id,
            descriptor,
            parent,
        } => journal.begin_with_id_at(session, &id, descriptor, parent.as_ref(), observed_at_ms),
        JournalRecord::Outcome {
            id,
            outcome,
            metadata,
        } => journal
            .record_at(&id, outcome, metadata, observed_at_ms)
            .map(|_| ()),
        JournalRecord::Gap { .. } => Ok(()),
    }
}

#[cfg(all(test, unix))]
#[path = "journal_runtime_history_tests.rs"]
mod history_tests;

#[cfg(all(test, unix))]
#[path = "journal_runtime_work_tests.rs"]
mod work_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_writer_reports_overflow_without_blocking_or_retaining_payloads() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let sink = Sink {
            sender,
            warning: Arc::new(Mutex::new(None)),
            dropped: Arc::new(AtomicU64::new(0)),
            accepting: Arc::new(Mutex::new(true)),
        };
        sink.begin("test.first", Origin::Host, None);
        sink.begin("test.second", Origin::Host, None);
        assert!(sink.take_warning().unwrap().contains("writer queue full"));
        assert_eq!(sink.dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn reducer_ids_are_deterministic_after_seed_and_pending_records_are_bounded() {
        let seed = ActionId::parse("12345678-1234-4234-8234-123456789abc").unwrap();
        let mut one = Trace {
            enabled: true,
            seed: Some(seed.clone()),
            ..Default::default()
        };
        let mut two = Trace {
            enabled: true,
            seed: Some(seed),
            ..Default::default()
        };
        assert_eq!(one.host("test.action", None), two.host("test.action", None));
        for _ in 0..PENDING_LIMIT + 20 {
            one.host("test.action", None);
        }
        let records = one.drain();
        assert_eq!(records.len(), PENDING_LIMIT + 1);
        assert!(matches!(records.last(), Some(JournalRecord::Gap { .. })));
    }
}
