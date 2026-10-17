//! Exact, keyed provider scripts for hosted scenarios. Only the transport is
//! replaced: credential resolution is bypassed; provider response parsing,
//! request builders and reducers remain production code.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub use crate::client::wire::{Request, Response as Reply};
use crate::{
    GithubPlugin,
    client::{ResponseFuture, Transport},
    model::{ApiError, ApiErrorKind},
};

/// A unique scenario label plus the complete expected provider request.
pub struct ExpectedRequest {
    key: String,
    request: Request,
    reply: Result<Reply, ApiError>,
    held: bool,
}

impl ExpectedRequest {
    pub fn json(key: impl Into<String>, request: Request, body: serde_json::Value) -> Self {
        Self::reply(
            key,
            request,
            Reply {
                status: 200,
                body: Ok(body.to_string().into_bytes()),
                headers: Default::default(),
            },
        )
    }

    pub fn reply(key: impl Into<String>, request: Request, reply: Reply) -> Self {
        Self {
            key: key.into(),
            request,
            reply: Ok(reply),
            held: false,
        }
    }

    /// Fail at the transport boundary, through the real client's error path.
    pub fn failure(key: impl Into<String>, request: Request, error: ApiError) -> Self {
        Self {
            key: key.into(),
            request,
            reply: Err(error),
            held: false,
        }
    }

    /// Hold this response until the scenario explicitly releases its key.
    pub fn held(mut self) -> Self {
        self.held = true;
        self
    }
}

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
    held: BTreeMap<String, tokio::sync::oneshot::Sender<()>>,
    violations: Vec<String>,
}

#[derive(Clone)]
pub struct HostedFixture(Arc<Mutex<Script>>);

impl HostedFixture {
    pub fn ledger(&self) -> Vec<RequestRecord> {
        self.0.lock().unwrap().ledger.clone()
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
    fn send(&self, request: Request) -> ResponseFuture<'_> {
        Box::pin(async move {
            let (reply, gate, index) = {
                let mut s = self.0.lock().unwrap();
                // Match the entire wire request, not just the method. The first
                // still-unused equal expectation wins for repeated reads.
                let found = s.expected.iter().position(|e| e.request == request);
                let index = s.ledger.len();
                let Some(found) = found else {
                    s.ledger.push(RequestRecord {
                        key: None,
                        request: request.clone(),
                        completed: true,
                    });
                    s.violations
                        .push(format!("unexpected request: {}", request.url));
                    return Err(ApiError::new(ApiErrorKind::InvalidTarget));
                };
                let e = s.expected.remove(found);
                s.ledger.push(RequestRecord {
                    key: Some(e.key.clone()),
                    request: request.clone(),
                    completed: false,
                });
                let gate = if e.held {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    s.held.insert(e.key, tx);
                    Some(rx)
                } else {
                    None
                };
                (e.reply, gate, index)
            };
            if let Some(gate) = gate {
                gate.await
                    .map_err(|_| ApiError::new(ApiErrorKind::Unavailable))?;
            }
            self.0.lock().unwrap().ledger[index].completed = true;
            reply
        })
    }
}

/// Starts empty and unauthenticated. Empty configured tokens select public
/// request paths; nonempty references select authenticated request paths (for
/// example `/user` rather than `/rate_limit`). Both modes omit authorization
/// headers entirely: this tests request selection and parsing, not real wire
/// authentication. Neither mode reads env, runs gh, or constructs native HTTP.
/// Configure still validates its normal syntax; fixtures do not bypass it.
/// Populate it only via the real host/config,
/// refresh tasks and result ingestion; there is never an HTTP fallback.
pub fn hosted_plugin(
    expected: Vec<ExpectedRequest>,
) -> anyhow::Result<(GithubPlugin, HostedFixture)> {
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
    let plugin = GithubPlugin {
        now: radar_core::plugin::DEMO_NOW,
        via: crate::client::Via {
            fixture: Some(fixture.clone()),
        },
        ..Default::default()
    };
    Ok((plugin, fixture))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Settings, model::RepositoryId};
    use radar_core::{Plugin, plugin::Effect};
    use serde_json::json;
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };

    const API: &str = "https://api.github.com";
    const REPO: &str = "ankh-morpork/watch";
    const PULLS: &str = "https://api.github.com/repos/ankh-morpork/watch/pulls?state=open&sort=updated&direction=desc&per_page=100&page=1";

    fn user() -> ExpectedRequest {
        ExpectedRequest::json(
            "user",
            Request::get(format!("{API}/user")),
            json!({"id":12,"login":"vimes"}),
        )
    }
    fn repository() -> ExpectedRequest {
        ExpectedRequest::json(
            "repository",
            Request::get(format!("{API}/repos/{REPO}")),
            json!({
                "id":52, "full_name":REPO, "name":"watch", "owner":{"login":"ankh-morpork"},
                "html_url":"https://github.com/ankh-morpork/watch", "description":"The Watch",
                "default_branch":"main", "private":false, "archived":false
            }),
        )
    }
    fn configure(p: &mut GithubPlugin, token: &str) {
        p.configure(Some(
            &toml::from_str::<toml::Value>(&format!(
                "token = '{token}'\n[[groups]]\nname = 'The Watch'\nrepositories = ['{REPO}']"
            ))
            .unwrap(),
        ))
        .unwrap();
    }
    fn client(p: &GithubPlugin) -> crate::client::Client {
        p.via
            .client(&p.cfg.ids(), &p.cfg.token, &p.cfg.api)
            .unwrap()
    }

    #[tokio::test]
    async fn exact_read_is_gated_and_records_completion() {
        let (p, f) = hosted_plugin(vec![user().held()]).unwrap();
        let c = p.via.client(&[], "gh", API).unwrap();
        let mut call = Box::pin(c.check_connection());
        assert!(matches!(
            call.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        assert_eq!(f.ledger()[0].key.as_deref(), Some("user"));
        assert!(!f.ledger()[0].completed);
        assert!(f.check_finished().is_err());
        f.release("user").unwrap();
        assert_eq!(call.await.unwrap().as_deref(), Some("vimes"));
        f.assert_finished();
        assert!(f.ledger()[0].completed);
        assert!(f.ledger()[0].request.authorization.is_none());
    }

    #[tokio::test]
    async fn auth_is_synthetic_before_reference_validation_or_resolution() {
        for token in ["gh", "env:NEVER_RESOLVE_GITHUB_FIXTURE", "env:", "x"] {
            let (p, f) = hosted_plugin(vec![user()]).unwrap();
            assert_eq!(
                p.via
                    .client(&[], token, API)
                    .unwrap()
                    .check_connection()
                    .await
                    .unwrap()
                    .as_deref(),
                Some("vimes")
            );
            f.assert_finished();
        }
        let (p, f) = hosted_plugin(vec![ExpectedRequest::json(
            "public",
            Request::get(format!("{API}/rate_limit")),
            json!({"resources":{}}),
        )])
        .unwrap();
        assert_eq!(client(&p).check_connection().await.unwrap(), None);
        f.assert_finished();
    }

    #[tokio::test]
    async fn injection_survives_configuration_reset_restore_and_demo() {
        let (mut p, f) = hosted_plugin(vec![user()]).unwrap();
        configure(&mut p, "env:NEVER_RESOLVE_GITHUB_FIXTURE");
        p.apply_config(Settings {
            token: "gh".into(),
            ..p.cfg.clone()
        });
        p.restore(json!({}));
        p.configure(None).unwrap();
        p.demo();
        assert_eq!(p.setup_finds(), Default::default());
        assert!(p.pictures("github").is_none());
        assert!(p.pictures("https://github.com/never.png").is_none());
        assert_eq!(
            client(&p).check_connection().await.unwrap().as_deref(),
            Some("vimes")
        );
        f.assert_finished();
        assert!(client(&p).check_connection().await.is_err());
        assert!(f.check_finished().is_err());
    }

    #[tokio::test]
    async fn setup_check_uses_real_parser_and_reducer() {
        let (mut p, f) = hosted_plugin(vec![user()]).unwrap();
        configure(&mut p, "gh");
        for effect in p.check_connection() {
            if let Effect::SpawnAction { task, .. } = effect {
                p.on_msg(task.await);
            } else {
                panic!("unexpected setup effect");
            }
        }
        assert_eq!(p.login.as_deref(), Some("vimes"));
        f.assert_finished();
    }

    // Plugin integration: task results pass through the real reducer, without
    // pretending to exercise host completion or persistence (root scenarios do).
    #[tokio::test]
    async fn public_refresh_uses_production_requests_parsers_and_reducer() {
        let (mut p, f) = hosted_plugin(vec![
            repository(),
            ExpectedRequest::json("pulls", Request::get(PULLS), json!([])),
        ])
        .unwrap();
        configure(&mut p, "");
        let mut effects: std::collections::VecDeque<_> = p.refresh_now().into();
        while let Some(effect) = effects.pop_front() {
            if let Effect::SpawnAction { task, .. } = effect {
                effects.extend(p.on_msg(task.await));
            }
        }
        f.assert_finished();
        assert_eq!(
            p.repos[0]
                .repository
                .as_ref()
                .unwrap()
                .description
                .as_deref(),
            Some("The Watch")
        );
        assert!(p.repos[0].pulls.as_ref().unwrap().completion.complete);
        assert!(!p.busy("github"));
    }

    #[tokio::test]
    async fn cli_reads_use_the_same_injection_and_production_parsers() {
        let (mut p, f) = hosted_plugin(vec![
            repository(),
            ExpectedRequest::json("pulls", Request::get(PULLS), json!([])),
        ])
        .unwrap();
        configure(&mut p, "gh");
        let matches = p
            .cli()
            .unwrap()
            .try_get_matches_from(["github", "list", "--json"])
            .unwrap();
        p.run_cli(&matches).await.unwrap();
        f.assert_finished();
        assert_eq!(f.ledger().len(), 2);
    }

    #[tokio::test]
    async fn unexpected_optional_read_is_sticky_even_when_returned_as_incomplete_data() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "link",
            "<https://api.github.com/repos/ankh-morpork/watch/pulls?page=2>; rel=\"next\""
                .parse()
                .unwrap(),
        );
        let (mut p, f) = hosted_plugin(vec![ExpectedRequest::reply(
            "page-one",
            Request::get(PULLS),
            Reply {
                status: 200,
                headers,
                body: Ok(b"[]".to_vec()),
            },
        )])
        .unwrap();
        configure(&mut p, "gh");
        let collection = client(&p)
            .open_pulls(&RepositoryId::parse(REPO).unwrap())
            .await
            .unwrap();
        assert!(!collection.completion.complete);
        assert!(
            f.check_finished()
                .unwrap_err()
                .to_string()
                .contains("unexpected request")
        );
        assert_eq!(f.ledger()[1].key, None);
    }

    #[tokio::test]
    async fn graphql_probe_uses_real_query_and_response_parser() {
        let body = serde_json::to_vec(&json!({
            "query":"query($o0:String!,$n0:String!){r0:repository(owner:$o0,name:$n0){nameWithOwner pullRequests(states:OPEN,first:1,orderBy:{field:UPDATED_AT,direction:DESC}){totalCount nodes{updatedAt}}}}",
            "variables":{"o0":"ankh-morpork","n0":"watch"}
        })).unwrap();
        let (mut p, f) = hosted_plugin(vec![ExpectedRequest::json("probe", Request::graphql(format!("{API}/graphql"), body), json!({"data":{"r0":{"nameWithOwner":REPO,"pullRequests":{"totalCount":0,"nodes":[]}}}}))]).unwrap();
        configure(&mut p, "gh");
        let result = client(&p)
            .probe(&[(RepositoryId::parse(REPO).unwrap(), false)])
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].marker.as_ref().unwrap().open, 0);
        f.assert_finished();
    }

    #[tokio::test]
    async fn exact_headers_and_body_mismatches_do_not_consume_expectations() {
        let expected = Request::graphql(
            format!("{API}/graphql"),
            br#"{"query":"query { viewer { login } }"}"#.to_vec(),
        );
        let (_, f) = hosted_plugin(vec![ExpectedRequest::json(
            "query",
            expected.clone(),
            json!({"data":{}}),
        )])
        .unwrap();
        let mut wrong = expected.clone();
        wrong
            .headers
            .insert("if-none-match", "changed".parse().unwrap());
        assert!(f.send(wrong).await.is_err());
        let mut wrong = expected.clone();
        wrong.graphql = Some(br#"{"query":"query { viewer { id } }"}"#.to_vec());
        assert!(f.send(wrong).await.is_err());
        assert_eq!(f.0.lock().unwrap().expected.len(), 1);
        f.send(expected).await.unwrap();
        assert!(f.check_finished().is_err());
    }

    #[tokio::test]
    async fn declared_failures_still_complete_but_unexpected_and_unused_calls_fail() {
        let (p, f) = hosted_plugin(vec![ExpectedRequest::failure(
            "network",
            Request::get(format!("{API}/user")),
            ApiError::new(ApiErrorKind::Network),
        )])
        .unwrap();
        assert_eq!(
            p.via
                .client(&[], "gh", API)
                .unwrap()
                .check_connection()
                .await
                .unwrap_err()
                .kind,
            ApiErrorKind::Network
        );
        f.assert_finished();
        let (_, unused) = hosted_plugin(vec![user()]).unwrap();
        assert!(
            unused
                .check_finished()
                .unwrap_err()
                .to_string()
                .contains("unused")
        );
        assert!(hosted_plugin(vec![user(), user()]).is_err());
    }

    #[tokio::test]
    async fn cancelled_and_premature_release_are_sticky() {
        let (p, f) = hosted_plugin(vec![user().held()]).unwrap();
        let c = p.via.client(&[], "gh", API).unwrap();
        let mut call = Box::pin(c.check_connection());
        assert!(matches!(
            call.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        drop(call);
        assert!(f.release("user").is_err());
        assert!(f.check_finished().is_err());
        let (_, f) = hosted_plugin(vec![user().held()]).unwrap();
        assert!(f.release("user").is_err());
        assert!(f.check_finished().is_err());
    }
}
