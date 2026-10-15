//! Runner dispatch preserves the requester and uses the one ordered writer.
use super::*;
use crate::source_context::WorkReply;
use crate::work_api::{Error, ItemsRequest, Request, Response};

async fn reply(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<Event>,
    expected_plugin: usize,
    expected_id: u64,
) -> Result<Response, Error> {
    let Event::Msg { plugin, msg } = rx.recv().await.unwrap() else {
        panic!("expected requester-local work reply");
    };
    assert_eq!(plugin, expected_plugin);
    let reply = *msg.downcast::<WorkReply>().unwrap();
    assert_eq!(reply.id, expected_id);
    reply.result
}

#[tokio::test]
async fn work_without_a_writer_returns_safe_disabled_to_the_exact_requester() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    perform(
        HostEffect::Work {
            plugin: 4,
            id: 73,
            request: Box::new(Request::Items(ItemsRequest::default())),
            mode: crate::writes::Mode::ReadOnly,
        },
        &tx,
        None,
    );
    assert_eq!(reply(&mut rx, 4, 73).await.unwrap_err(), Error::Disabled);
}

#[cfg(unix)]
#[tokio::test]
async fn work_runner_orders_requests_and_reads_without_self_auditing() {
    let directory =
        std::env::temp_dir().join(format!("radar-work-runner-{}", ActionId::new().as_str()));
    let path = directory.join("actions.sqlite3");
    let mut recording = Recording::open_at(&path, SessionOrigin::Tui).unwrap();
    let action = recording.sink.begin("test.source", Origin::Host, None);
    recording
        .sink
        .outcome(&action, JournalOutcome::Succeeded, None);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    perform(
        HostEffect::Work {
            plugin: 2,
            id: 9,
            request: Box::new(Request::Items(ItemsRequest::default())),
            mode: crate::writes::Mode::ReadOnly,
        },
        &tx,
        Some(&recording.sink),
    );
    let Response::Items(page) = reply(&mut rx, 2, 9).await.unwrap() else {
        panic!("expected items");
    };
    assert_eq!(page.page.snapshot.event_sequence(), 2);
    assert!(page.page.items.is_empty());
    let events = recording
        .sink
        .history(Default::default(), None, None)
        .await
        .unwrap();
    assert_eq!(events.events.len(), 2);
    assert!(
        events
            .events
            .iter()
            .all(|entry| entry.event.action_id == action)
    );
    recording.finish();
    std::fs::remove_dir_all(directory).unwrap();
}
