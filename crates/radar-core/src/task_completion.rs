//! Shared asynchronous effect boundaries for the runner and scratch scenarios.
//! Plugin tasks preserve causality and isolate panics; local work/history use the
//! supplied ordered writer sink. Callers own event delivery and writer lifetime.

use crate::host::Event;
use crate::journal::{ActionId, Metadata, Outcome, Reason};
use crate::journal_runtime::{JournalRecord, Sink};
use crate::plugin::Task;

pub(crate) struct Completion {
    /// Submit these before delivering the event, just as the live runner does.
    pub records: Vec<JournalRecord>,
    pub event: Event,
}

/// Execute local work through the production ordered writer. Work requests are
/// already audited by that writer when appropriate; delivery must not invent a
/// tracking envelope or terminal outcome around the typed response.
pub(crate) async fn complete_work(
    journal: Option<Sink>,
    plugin: usize,
    id: u64,
    request: crate::work_api::Request,
    mode: crate::writes::Mode,
) -> Completion {
    let result = match journal {
        Some(journal) => journal.work(request, mode).await,
        None => Err(crate::work_api::Error::Disabled),
    };
    Completion {
        records: Vec::new(),
        event: Event::Msg {
            plugin,
            msg: Box::new(crate::source_context::WorkReply { id, result }),
        },
    }
}

/// History uses the same ordered writer queue without auditing its own reads.
pub(crate) async fn complete_history(
    journal: Option<Sink>,
    request: crate::history::Request,
) -> Completion {
    let result = match journal {
        Some(journal) => {
            journal
                .history(request.filter, request.cursor, request.snapshot)
                .await
        }
        None => Err(crate::history::ReadError::Disabled),
    };
    Completion {
        records: Vec::new(),
        event: Event::History {
            id: request.id,
            result: result.map(Box::new),
        },
    }
}

pub(crate) fn tracked_event(
    event: Event,
    action: Option<ActionId>,
    semantic: Option<ActionId>,
) -> Event {
    match action {
        Some(action) => Event::Tracked {
            action,
            semantic,
            event: Box::new(event),
        },
        None => event,
    }
}

/// Await one task under the same panic boundary used by the live runner.
/// Starting the action remains the caller's responsibility. Successful futures
/// produce no terminal record here: the host must interpret their typed Report
/// before treating an otherwise opaque result as Unknown.
pub(crate) async fn complete_task(
    plugin: usize,
    task: Task,
    action: Option<ActionId>,
    semantic: Option<ActionId>,
) -> Completion {
    // Tokio detaches a dropped JoinHandle. A cancelled runner/scenario waiter
    // must instead cancel its child future, so it cannot outlive its owner.
    struct AbortOnDrop(tokio::task::AbortHandle);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let task = tokio::spawn(task);
    let _abort = AbortOnDrop(task.abort_handle());
    match task.await {
        Ok(msg) => Completion {
            records: Vec::new(),
            event: tracked_event(Event::Msg { plugin, msg }, action, semantic),
        },
        Err(_) => Completion {
            // No trustworthy domain result survives a panic/cancellation.
            // Never retain the JoinError, panic text, or panic payload.
            records: action
                .into_iter()
                .chain(semantic)
                .map(|id| JournalRecord::Outcome {
                    id,
                    outcome: Outcome::Unknown,
                    metadata: Metadata {
                        reason: Some(Reason::Interrupted),
                        ..Default::default()
                    },
                })
                .collect(),
            event: Event::ErrorNotice("background action stopped; its outcome is unknown".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::Msg;

    fn id(number: u64) -> ActionId {
        ActionId::parse(&format!("00000000-0000-0000-0000-{number:012x}")).unwrap()
    }

    fn result(value: &'static str) -> Task {
        Box::pin(async move { Box::new(value) as Msg })
    }

    fn assert_message(event: Event, expected_plugin: usize, expected_value: &str) {
        let Event::Msg { plugin, msg } = event else {
            panic!("expected a plugin message");
        };
        assert_eq!(plugin, expected_plugin);
        assert_eq!(*msg.downcast::<&'static str>().unwrap(), expected_value);
    }

    #[tokio::test]
    async fn disabled_journal_completions_preserve_real_error_and_request_ids() {
        let work = complete_work(
            None,
            7,
            52,
            crate::work_api::Request::Items(Default::default()),
            crate::writes::Mode::ReadOnly,
        )
        .await;
        assert!(work.records.is_empty());
        let Event::Msg { plugin, msg } = work.event else {
            panic!("expected an untracked plugin response");
        };
        assert_eq!(plugin, 7);
        let reply = msg.downcast::<crate::source_context::WorkReply>().unwrap();
        assert_eq!(reply.id, 52);
        assert!(matches!(
            reply.result,
            Err(crate::work_api::Error::Disabled)
        ));
        let history = complete_history(
            None,
            crate::history::Request {
                id: 53,
                filter: Default::default(),
                cursor: None,
                snapshot: None,
            },
        )
        .await;
        assert!(history.records.is_empty());
        assert!(matches!(
            history.event,
            Event::History {
                id: 53,
                result: Err(crate::history::ReadError::Disabled),
            }
        ));
    }

    #[tokio::test]
    async fn untracked_result_preserves_plugin_and_typed_message() {
        let completion = complete_task(7, result("watch"), None, None).await;
        assert!(completion.records.is_empty());
        assert_message(completion.event, 7, "watch");
    }

    #[tokio::test]
    async fn tracked_result_preserves_action_and_semantic_without_claiming_success() {
        for semantic in [None, Some(id(2))] {
            let completion = complete_task(3, result("vimes"), Some(id(1)), semantic.clone()).await;
            assert!(completion.records.is_empty());
            let Event::Tracked {
                action,
                semantic: actual_semantic,
                event,
            } = completion.event
            else {
                panic!("expected tracked completion");
            };
            assert_eq!(action, id(1));
            assert_eq!(actual_semantic, semantic);
            assert_message(*event, 3, "vimes");
        }
    }

    #[tokio::test]
    async fn semantic_without_action_does_not_introduce_a_tracking_envelope() {
        let completion = complete_task(4, result("watch"), None, Some(id(2))).await;
        assert!(completion.records.is_empty());
        assert_message(completion.event, 4, "watch");
    }

    #[tokio::test]
    async fn panicked_task_marks_each_context_unknown_and_discards_payload() {
        for (action, semantic) in [
            (None, None),
            (Some(id(1)), None),
            (None, Some(id(2))),
            (Some(id(1)), Some(id(2))),
        ] {
            let expected_ids: Vec<_> = action.iter().chain(semantic.iter()).cloned().collect();
            let completion = complete_task(
                3,
                Box::pin(async { panic!("SECRET panic payload") }),
                action,
                semantic,
            )
            .await;
            assert_eq!(completion.records.len(), expected_ids.len());
            assert!(!format!("{:?}", completion.records).contains("SECRET"));
            for (record, expected_id) in completion.records.iter().zip(expected_ids) {
                let JournalRecord::Outcome {
                    id,
                    outcome,
                    metadata,
                } = record
                else {
                    panic!("expected an outcome record");
                };
                assert_eq!(*id, expected_id);
                assert_eq!(*outcome, Outcome::Unknown);
                assert_eq!(metadata.reason, Some(Reason::Interrupted));
                assert_eq!(
                    *metadata,
                    Metadata {
                        reason: Some(Reason::Interrupted),
                        ..Default::default()
                    },
                );
            }
            assert!(matches!(completion.event, Event::ErrorNotice(note)
                if note == "background action stopped; its outcome is unknown"));
        }
    }

    #[tokio::test]
    async fn cancelling_waiter_drops_nested_future() {
        struct Probe(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for Probe {
            fn drop(&mut self) {
                if let Some(sender) = self.0.take() {
                    let _ = sender.send(());
                }
            }
        }
        let (started, ready) = tokio::sync::oneshot::channel();
        let (dropped, observed) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(complete_task(
            0,
            Box::pin(async move {
                let _probe = Probe(Some(dropped));
                started.send(()).unwrap();
                std::future::pending::<Msg>().await
            }),
            None,
            None,
        ));
        ready.await.unwrap();
        waiter.abort();
        assert!(matches!(waiter.await, Err(error) if error.is_cancelled()));
        observed.await.unwrap();
    }

    #[tokio::test]
    async fn reverse_completion_keeps_each_tasks_own_causality() {
        let (release, wait) = tokio::sync::oneshot::channel();
        let first = tokio::spawn(complete_task(
            0,
            Box::pin(async move {
                wait.await.unwrap();
                Box::new("first") as Msg
            }),
            Some(id(1)),
            Some(id(2)),
        ));
        let second = complete_task(0, result("second"), Some(id(3)), Some(id(4))).await;
        assert!(!first.is_finished());
        release.send(()).unwrap();
        let first = first.await.unwrap();
        for (completion, expected_action, expected_semantic, value) in [
            (second, id(3), id(4), "second"),
            (first, id(1), id(2), "first"),
        ] {
            assert!(completion.records.is_empty());
            let Event::Tracked {
                action,
                semantic,
                event,
            } = completion.event
            else {
                panic!("expected tracked completion");
            };
            assert_eq!(action, expected_action);
            assert_eq!(semantic, Some(expected_semantic));
            assert_message(*event, 0, value);
        }
    }
}
