//! Exact, keyed provider scripts for hosted scenarios. Only the transport is
//! replaced: credential sourcing is synthetic; provider response parsing,
//! request builders and reducers remain production code.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub use crate::client::{Reply, Request};
use crate::{
    SlackPlugin,
    client::{Fut, Transport},
};

/// A unique scenario label plus the complete expected provider request.
pub struct ExpectedRequest {
    key: String,
    request: Request,
    delivery: Delivery,
    held: bool,
}

enum Delivery {
    Reply(Reply),
    AcceptedPostResponseLost { server_ts: String },
}

impl ExpectedRequest {
    pub fn json(key: impl Into<String>, request: Request, body: serde_json::Value) -> Self {
        Self::reply(
            key,
            request,
            Reply {
                status: 200,
                body: body.to_string(),
                ..Default::default()
            },
        )
    }

    pub fn reply(key: impl Into<String>, request: Request, reply: Reply) -> Self {
        Self {
            key: key.into(),
            request,
            delivery: Delivery::Reply(reply),
            held: false,
        }
    }

    /// The synthetic server accepts the exact post, but the client loses its
    /// response. A configured expectation alone is not acceptance evidence.
    pub fn accepted_post_then_lost_response(
        key: impl Into<String>,
        request: Request,
        server_ts: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let server_ts = server_ts.into();
        let body = request.body.as_ref();
        let nonempty = |name: &str| {
            body.and_then(|v| v.get(name))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
        };
        let timestamp = |ts: &str| {
            ts.split_once('.').is_some_and(|(seconds, fraction)| {
                !seconds.is_empty()
                    && !fraction.is_empty()
                    && seconds
                        .bytes()
                        .chain(fraction.bytes())
                        .all(|b| b.is_ascii_digit())
            })
        };
        anyhow::ensure!(
            request.method == "chat.postMessage"
                && request.write
                && request.params.is_empty()
                && nonempty("channel")
                && nonempty("text")
                && body
                    .and_then(|v| v.get("thread_ts"))
                    .is_none_or(|v| v.as_str().is_some_and(timestamp))
                && timestamp(&server_ts),
            "accepted-post response loss requires a valid post request and server timestamp"
        );
        Ok(Self {
            key: key.into(),
            request,
            delivery: Delivery::AcceptedPostResponseLost { server_ts },
            held: false,
        })
    }

    /// Hold this response until the scenario explicitly releases its key.
    pub fn held(mut self) -> Self {
        self.held = true;
        self
    }
}

/// Immutable synthetic server evidence created only by an exact matched call.
/// Callers get copies; no client result, plugin reducer or cache is changed.
#[derive(Clone, Debug, PartialEq)]
pub struct AcceptedPost {
    key: String,
    request: Request,
    server_ts: String,
}

impl AcceptedPost {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn request(&self) -> &Request {
        &self.request
    }

    pub fn server_ts(&self) -> &str {
        &self.server_ts
    }

    /// Provider-shaped data for a subsequent scripted read. Its content comes
    /// from the actual matched request, not a separately invented success.
    pub fn message(&self) -> serde_json::Value {
        let body = self.request.body.as_ref().expect("validated post body");
        let mut message = serde_json::json!({"ts": self.server_ts, "text": body["text"]});
        if let Some(thread) = body.get("thread_ts") {
            message["thread_ts"] = thread.clone();
        }
        message
    }
}

#[derive(Debug)]
struct ResponseLost;

impl std::fmt::Display for ResponseLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("synthetic transport response lost")
    }
}

impl std::error::Error for ResponseLost {}

#[derive(Clone, Debug, PartialEq)]
pub struct RequestRecord {
    /// None means an unexpected call, retained even when its error is swallowed.
    pub key: Option<String>,
    pub request: Request,
    pub completed: bool,
}

#[derive(Default)]
struct Script {
    expected: Vec<ExpectedRequest>,
    ledger: Vec<RequestRecord>,
    accepted_posts: Vec<AcceptedPost>,
    held: BTreeMap<String, tokio::sync::oneshot::Sender<()>>,
    violations: Vec<String>,
}

#[derive(Clone)]
pub struct HostedFixture(Arc<Mutex<Script>>);

impl HostedFixture {
    pub fn ledger(&self) -> Vec<RequestRecord> {
        self.0.lock().unwrap().ledger.clone()
    }

    pub fn accepted_posts(&self) -> Vec<AcceptedPost> {
        self.0.lock().unwrap().accepted_posts.clone()
    }

    /// Release an already-started request. Unknown, duplicate or premature
    /// releases are contract violations, not silently ignored scheduling hints.
    pub fn release(&self, key: &str) -> anyhow::Result<()> {
        let mut s = self.0.lock().unwrap();
        let Some(sender) = s.held.remove(key) else {
            s.violations
                .push(format!("release without a held request: {key}"));
            anyhow::bail!("release without a held request: {key}");
        };
        if sender.send(()).is_err() {
            s.violations
                .push(format!("held request was cancelled: {key}"));
            anyhow::bail!("held request was cancelled: {key}");
        }
        Ok(())
    }

    pub fn check_finished(&self) -> anyhow::Result<()> {
        let s = self.0.lock().unwrap();
        anyhow::ensure!(
            s.violations.is_empty(),
            "hosted transport violations: {:?}",
            s.violations
        );
        let unused: Vec<_> = s.expected.iter().map(|e| e.key.as_str()).collect();
        anyhow::ensure!(unused.is_empty(), "unused hosted requests: {unused:?}");
        anyhow::ensure!(
            s.ledger.iter().all(|r| r.completed),
            "hosted requests remain in flight"
        );
        anyhow::ensure!(s.held.is_empty(), "hosted responses remain held");
        Ok(())
    }

    pub fn assert_finished(&self) {
        self.check_finished().unwrap();
    }
}

impl Transport for HostedFixture {
    fn blocked_external_io(&self, operation: &str) {
        self.0
            .lock()
            .unwrap()
            .violations
            .push(format!("blocked external I/O: {operation}"));
    }

    fn send<'a>(&'a self, request: &'a Request) -> Fut<'a, anyhow::Result<Reply>> {
        Box::pin(async move {
            let (delivery, gate, index) = {
                let mut s = self.0.lock().unwrap();
                // Match the entire wire request, not just the method. The first
                // still-unused equal expectation wins for repeated reads.
                let found = s.expected.iter().position(|e| e.request == *request);
                let index = s.ledger.len();
                let Some(found) = found else {
                    s.ledger.push(RequestRecord {
                        key: None,
                        request: request.clone(),
                        completed: true,
                    });
                    s.violations
                        .push(format!("unexpected request: {}", request.method));
                    anyhow::bail!("unexpected hosted request: {}", request.method);
                };
                let e = s.expected.remove(found);
                s.ledger.push(RequestRecord {
                    key: Some(e.key.clone()),
                    request: request.clone(),
                    completed: false,
                });
                if let Delivery::AcceptedPostResponseLost { server_ts } = &e.delivery {
                    s.accepted_posts.push(AcceptedPost {
                        key: e.key.clone(),
                        request: request.clone(),
                        server_ts: server_ts.clone(),
                    });
                }
                let gate = if e.held {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    s.held.insert(e.key, tx);
                    Some(rx)
                } else {
                    None
                };
                (e.delivery, gate, index)
            };
            if let Some(gate) = gate {
                gate.await
                    .map_err(|_| anyhow::anyhow!("hosted release gate dropped"))?;
            }
            self.0.lock().unwrap().ledger[index].completed = true;
            match delivery {
                Delivery::Reply(reply) => Ok(reply),
                Delivery::AcceptedPostResponseLost { .. } => Err(ResponseLost.into()),
            }
        })
    }
}

/// Starts empty and unauthenticated. Populate it only via the real host/config,
/// refresh tasks and result ingestion; there is never an HTTP fallback.
pub fn hosted_plugin(
    expected: Vec<ExpectedRequest>,
) -> anyhow::Result<(SlackPlugin, HostedFixture)> {
    let mut keys = std::collections::BTreeSet::new();
    for e in &expected {
        anyhow::ensure!(
            !e.key.is_empty() && keys.insert(e.key.clone()),
            "hosted request keys must be nonempty and unique"
        );
    }
    let fixture = HostedFixture(Arc::new(Mutex::new(Script {
        expected,
        ..Default::default()
    })));
    let mut plugin = SlackPlugin {
        now: radar_core::plugin::DEMO_NOW,
        section_rows: [0, 0, 0, 0, 1],
        ..Default::default()
    };
    plugin.scripted_fixture = true;
    plugin.via = crate::Via(Some(Arc::new(fixture.clone())));
    plugin.env = Some(Vec::new());
    plugin.app_running = Some(false);
    plugin.cfg.token = "xoxp-offline-synthetic-fixture".into();
    Ok((plugin, fixture))
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_core::Plugin;
    use serde_json::json;
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };

    fn auth() -> Request {
        Request {
            method: "auth.test",
            params: vec![],
            write: false,
            body: None,
        }
    }
    fn me() -> serde_json::Value {
        json!({"ok":true,"team_id":"TWATCH","user_id":"UVIMES"})
    }

    #[tokio::test]
    async fn exact_write_is_gated_and_ledger_keeps_completion() {
        let request = crate::client::react_request("CWATCH", "1791200000.000100", "eyes");
        let (p, f) = hosted_plugin(vec![
            ExpectedRequest::json("react", request.clone(), json!({"ok":true})).held(),
        ])
        .unwrap();
        let client = p.client().unwrap();
        let mut call = Box::pin(client.react("CWATCH", "1791200000.000100", "eyes"));
        assert!(matches!(
            call.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        assert_eq!(
            f.ledger(),
            vec![RequestRecord {
                key: Some("react".into()),
                request,
                completed: false
            }]
        );
        assert!(f.check_finished().is_err());
        f.release("react").unwrap();
        call.await.unwrap();
        f.assert_finished();
        assert!(f.ledger()[0].completed);
    }

    fn lost_post(request: Request) -> ExpectedRequest {
        ExpectedRequest::accepted_post_then_lost_response("post", request, "1791200000.000900")
            .unwrap()
            .held()
    }

    #[tokio::test]
    async fn accepted_post_receipt_precedes_lost_response_and_remains_independent() {
        let request =
            crate::client::post_request("CWATCH", "gate report", Some("1791200000.000100"));
        let (p, f) = hosted_plugin(vec![lost_post(request.clone())]).unwrap();
        assert!(
            f.accepted_posts().is_empty(),
            "a script is not acceptance evidence"
        );
        let client = p.client().unwrap();
        let mut call = Box::pin(client.post("CWATCH", "gate report", Some("1791200000.000100")));
        assert!(matches!(
            call.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        let observed = f.accepted_posts();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].key(), "post");
        assert_eq!(observed[0].request(), &request);
        assert_eq!(observed[0].server_ts(), "1791200000.000900");
        assert_eq!(
            observed[0].message(),
            json!({
                "ts":"1791200000.000900", "thread_ts":"1791200000.000100", "text":"gate report"
            })
        );
        assert!(!f.ledger()[0].completed);
        assert!(f.check_finished().is_err());
        f.release("post").unwrap();
        let error = call.await.unwrap_err().to_string();
        assert!(error.contains("response lost"));
        assert!(
            !error.contains("accepted"),
            "the client cannot know acceptance"
        );
        assert_eq!(f.accepted_posts(), observed);
        assert_eq!(f.ledger().len(), 1, "the client did not retry");
        assert!(f.ledger()[0].completed);
        f.assert_finished();
    }

    #[tokio::test]
    async fn mismatched_post_has_no_acceptance_and_does_not_consume_the_script() {
        let request =
            crate::client::post_request("CWATCH", "gate report", Some("1791200000.000100"));
        let (p, f) = hosted_plugin(vec![lost_post(request)]).unwrap();
        assert!(
            p.client()
                .unwrap()
                .post("CWATCH", "wrong text", Some("1791200000.000100"))
                .await
                .is_err()
        );
        assert!(f.accepted_posts().is_empty());
        assert_eq!(f.0.lock().unwrap().expected.len(), 1);
        assert!(
            f.check_finished()
                .unwrap_err()
                .to_string()
                .contains("unexpected request")
        );
    }

    #[tokio::test]
    async fn cancelled_accepted_post_remains_incomplete_and_release_failure_is_sticky() {
        let (p, f) = hosted_plugin(vec![lost_post(crate::client::post_request(
            "CWATCH",
            "gate report",
            None,
        ))])
        .unwrap();
        let client = p.client().unwrap();
        let mut call = Box::pin(client.post("CWATCH", "gate report", None));
        assert!(matches!(
            call.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        let observed = f.accepted_posts();
        assert_eq!(observed.len(), 1);
        drop(call);
        assert!(!f.ledger()[0].completed);
        assert!(f.check_finished().is_err());
        assert!(
            f.release("post")
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
        assert!(
            f.check_finished()
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
        assert!(!f.ledger()[0].completed);
        assert_eq!(f.accepted_posts(), observed);
    }

    #[test]
    fn accepted_post_fault_rejects_reads_reactions_and_malformed_posts_up_front() {
        let post = crate::client::post_request("CWATCH", "gate report", None);
        let mut missing_body = post.clone();
        missing_body.body = None;
        let mut not_write = post.clone();
        not_write.write = false;
        let mut invalid_thread = post.clone();
        invalid_thread.body.as_mut().unwrap()["thread_ts"] = json!(false);
        let mut invalid_text = post.clone();
        invalid_text.body.as_mut().unwrap()["text"] = json!(["gate report"]);
        let mut query = post.clone();
        query.params.push(("channel".into(), "CWATCH".into()));
        for invalid in [
            auth(),
            crate::client::react_request("CWATCH", "1791200000.000100", "eyes"),
            crate::client::post_request("", "gate report", None),
            crate::client::post_request("CWATCH", "", None),
            missing_body,
            not_write,
            invalid_thread,
            invalid_text,
            query,
        ] {
            assert!(
                ExpectedRequest::accepted_post_then_lost_response(
                    "bad",
                    invalid,
                    "1791200000.000900"
                )
                .is_err()
            );
        }
        for timestamp in ["", "later", "1791200000", ".1", "1.", "1.2.3"] {
            assert!(
                ExpectedRequest::accepted_post_then_lost_response("bad", post.clone(), timestamp)
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn unexpected_request_is_sticky_even_when_optional_profile_failure_is_swallowed() {
        let (p, f) = hosted_plugin(vec![ExpectedRequest::json("auth", auth(), me())]).unwrap();
        // whoami intentionally tolerates profile failure. The guard must not.
        assert!(p.client().unwrap().whoami().await.is_ok());
        assert!(
            f.check_finished()
                .unwrap_err()
                .to_string()
                .contains("unexpected request: users.info")
        );
        assert_eq!(f.ledger()[1].key, None);
    }

    #[tokio::test]
    async fn full_payload_mismatch_does_not_consume_expectation_or_enable_fallback() {
        let request = crate::client::react_request("CWATCH", "1791200000.000100", "eyes");
        let (p, f) = hosted_plugin(vec![ExpectedRequest::json(
            "react",
            request,
            json!({"ok":true}),
        )])
        .unwrap();
        let client = p.client().unwrap();
        assert!(
            client
                .react("CWATCH", "1791200000.000100", "wrong")
                .await
                .is_err()
        );
        assert_eq!(f.0.lock().unwrap().expected.len(), 1);
        client
            .react("CWATCH", "1791200000.000100", "eyes")
            .await
            .unwrap();
        assert!(f.check_finished().is_err());
        assert!(client.auth_test().await.is_err());
    }

    #[tokio::test]
    async fn injection_survives_config_token_reset_restore_and_demo_without_credentials() {
        let (mut p, f) = hosted_plugin(vec![ExpectedRequest::json("auth", auth(), me())]).unwrap();
        p.configure(Some(
            &toml::Value::try_from(crate::Settings {
                token: "env:NEVER_RESOLVE_HOSTED_TOKEN".into(),
                api: "env:NEVER_RESOLVE_HOSTED_API".into(),
                ..Default::default()
            })
            .unwrap(),
        ))
        .unwrap();
        p.replace_token("env:NEVER_RESOLVE_RESET_TOKEN".into());
        p.restore(json!({"data":{"me":{"team_id":"TFORGED","user_id":"UFORGED"}}}));
        assert!(!p.authenticated());
        assert!(p.vars().is_empty());
        assert_eq!(
            p.session_credential().unwrap(),
            "xoxp-offline-synthetic-fixture"
        );
        p.demo();
        assert_eq!(
            p.client().unwrap().auth_test().await.unwrap().team_id,
            "TWATCH"
        );
        f.assert_finished();
    }

    #[tokio::test]
    async fn blocked_picture_fetch_is_a_sticky_violation() {
        let (p, f) = hosted_plugin(vec![]).unwrap();
        assert!(
            p.pictures()("https://files.slack.com/never.png".into())
                .await
                .is_err()
        );
        assert!(
            f.check_finished()
                .unwrap_err()
                .to_string()
                .contains("picture fetch")
        );
    }

    #[test]
    fn opening_slack_never_inspects_or_launches_local_processes() {
        let (mut p, f) = hosted_plugin(vec![]).unwrap();
        p.demo();
        let effects = p.open_in_slack(
            "CWATCH",
            "1791200000.000100",
            None,
            "https://watch.example/".into(),
        );
        assert!(matches!(
            effects.as_slice(),
            [radar_core::plugin::Effect::Open(_)]
        ));
        f.assert_finished();
    }

    #[test]
    fn unused_and_duplicate_keys_are_rejected() {
        let (_, f) = hosted_plugin(vec![ExpectedRequest::json("unused", auth(), me())]).unwrap();
        assert!(
            f.check_finished()
                .unwrap_err()
                .to_string()
                .contains("unused")
        );
        assert!(
            hosted_plugin(vec![
                ExpectedRequest::json("same", auth(), me()),
                ExpectedRequest::json("same", auth(), me())
            ])
            .is_err()
        );
    }
}
