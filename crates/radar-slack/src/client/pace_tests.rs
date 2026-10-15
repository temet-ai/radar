use std::sync::atomic::AtomicUsize;

use serde_json::json;

use super::*;
use crate::client::{Client, Fut, Recorded, Reply, Request, Transport};

fn user(id: &str) -> anyhow::Result<Reply> {
    Ok(Reply {
        status: 200,
        body: json!({"ok": true, "user": {"id": id, "name": id.to_lowercase(),
            "profile": {"display_name": format!("name of {id}")}}})
        .to_string(),
        ..Default::default()
    })
}

#[tokio::test]
async fn all_gives_results_in_the_order_given_whatever_finishes_first() {
    let slow = |n: u32| async move {
        for _ in 0..n {
            tokio::task::yield_now().await;
        }
        n
    };
    assert_eq!(all([slow(3), slow(0), slow(2)]).await, [3, 0, 2]);
    assert!(all(Vec::<std::future::Ready<u8>>::new()).await.is_empty());
}

#[tokio::test]
async fn a_name_wanted_by_many_tasks_is_asked_for_once_in_a_round() {
    let rec = Recorded::new(vec![user("U1"), user("U2"), user("U1")]);
    let c = Client::new(rec.clone());
    let stop = Stop::default();
    let got = all([
        c.user_name("U1", &stop),
        c.user_name("U1", &stop),
        c.user_name("U2", &stop),
        c.user_name("U1", &stop),
    ])
    .await;
    let name = |s: &str| Some(Ok(format!("name of {s}")));
    assert_eq!(got, [name("U1"), name("U1"), name("U2"), name("U1")]);
    assert_eq!(rec.requests().len(), 2);
    // The next round asks again.
    c.pace.new_round();
    assert_eq!(c.user_name("U1", &stop).await, name("U1"));
    assert_eq!(rec.requests().len(), 3);
}

/// Answers slowly, and counts how many calls are in flight.
#[derive(Default)]
struct Slow {
    active: AtomicUsize,
    most: AtomicUsize,
}

impl Transport for Slow {
    fn send<'a>(&'a self, req: &'a Request) -> Fut<'a, anyhow::Result<Reply>> {
        Box::pin(async move {
            let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.most.fetch_max(now, Ordering::SeqCst);
            for _ in 0..3 {
                tokio::task::yield_now().await;
            }
            self.active.fetch_sub(1, Ordering::SeqCst);
            user(&req.params[0].1)
        })
    }
}

#[tokio::test]
async fn names_are_asked_a_few_at_a_time_and_no_more() {
    let slow = Arc::new(Slow::default());
    let c = Client::new(slow.clone());
    let stop = Stop::default();
    let ids: Vec<String> = (0..12).map(|i| format!("U{i}")).collect();
    let got = all(ids.iter().map(|id| c.user_name(id, &stop))).await;
    assert!(got.iter().all(|g| matches!(g, Some(Ok(_)))));
    assert_eq!(slow.most.load(Ordering::SeqCst), IN_FLIGHT);
}

#[tokio::test]
async fn replies_are_taken_a_few_at_a_time_too() {
    let c = Client::new(Arc::new(Slow::default()));
    let active = AtomicUsize::new(0);
    let most = AtomicUsize::new(0);
    all((0..10).map(|_| async {
        let _turn = c.replies_turn().await;
        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
        most.fetch_max(now, Ordering::SeqCst);
        tokio::task::yield_now().await;
        active.fetch_sub(1, Ordering::SeqCst);
    }))
    .await;
    assert_eq!(most.load(Ordering::SeqCst), IN_FLIGHT);
}

#[tokio::test]
async fn a_failure_leaves_the_calls_not_yet_sent_out() {
    let rec = Recorded::new(vec![Ok(Reply {
        status: 200,
        body: json!({"ok": false, "error": "user_not_found"}).to_string(),
        ..Default::default()
    })]);
    let c = Client::new(rec.clone());
    let stop = Stop::default();
    let ids = ["U1", "U2", "U3", "U4", "U5", "U6"];
    let got = all(ids.iter().map(|id| async {
        let got = c.user_name(id, &stop).await;
        if matches!(got, Some(Err(_))) {
            stop.set();
        }
        got
    }))
    .await;
    assert!(matches!(got[0], Some(Err(_))));
    assert!(got[1..].iter().all(Option::is_none));
    assert_eq!(rec.requests().len(), 1);
}

#[tokio::test]
async fn a_rate_limit_is_waited_out_by_every_call_to_that_method() {
    let limited = Reply {
        status: 429,
        retry_after: Some(3),
        ..Default::default()
    };
    let rec = Recorded::new(vec![Ok(limited), user("U1"), user("U2")]);
    let sent_when_wait_ended = Arc::new(AtomicUsize::new(usize::MAX));
    let waits = Arc::new(Mutex::new(Vec::new()));
    let wait: crate::client::Wait = {
        let (rec, sent, waits) = (rec.clone(), sent_when_wait_ended.clone(), waits.clone());
        Arc::new(move |secs| {
            let (rec, sent, waits) = (rec.clone(), sent.clone(), waits.clone());
            Box::pin(async move {
                waits.lock().unwrap().push(secs);
                for _ in 0..3 {
                    tokio::task::yield_now().await;
                }
                sent.store(rec.requests().len(), Ordering::SeqCst);
            })
        })
    };
    let c = Client::new(rec.clone()).with_wait(wait);
    let got = all([c.user("U1"), c.user("U2")]).await;
    assert!(got.iter().all(Result::is_ok));
    // The second call did not go out while the first waited.
    assert_eq!(sent_when_wait_ended.load(Ordering::SeqCst), 1);
    assert_eq!(*waits.lock().unwrap(), [3]);
    assert_eq!(rec.requests().len(), 3);
}
