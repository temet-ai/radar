//! Exact, keyed provider scripts for hosted scenarios. Only the transport is
//! replaced: credential sourcing is synthetic; provider response parsing,
//! request builders and reducers remain production code.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub use crate::client::{Method, Req, Resp};
use crate::{
    GitlabPlugin,
    client::{Sent, Transport},
};

/// Full connection identity plus the complete wire request. Authentication is
/// deliberately synthetic and never retained in the request ledger.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub server: String,
    pub request: Req,
}

impl Request {
    pub fn new(server: impl Into<String>, request: Req) -> Self {
        Self {
            server: server.into(),
            request,
        }
    }
}

/// A unique scenario label plus the complete expected provider request.
pub struct ExpectedRequest {
    key: String,
    request: Request,
    reply: Resp,
    held: bool,
}

impl ExpectedRequest {
    pub fn json(key: impl Into<String>, request: Request, body: serde_json::Value) -> Self {
        Self::reply(
            key,
            request,
            Resp {
                status: 200,
                body: body.to_string().into_bytes(),
                ..Default::default()
            },
        )
    }

    pub fn reply(key: impl Into<String>, request: Request, reply: Resp) -> Self {
        Self {
            key: key.into(),
            request,
            reply,
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

impl std::fmt::Debug for HostedFixture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostedFixture")
    }
}

impl HostedFixture {
    /// A production parser/request builder over this server-scoped script.
    pub fn client(&self, server: &str) -> crate::Client {
        crate::Client::over_at(
            Arc::new(ScopedTransport {
                fixture: self.clone(),
                server: server.to_owned(),
            }),
            server,
        )
    }

    pub(crate) fn blocked(&self, operation: &str) {
        self.0
            .lock()
            .unwrap()
            .violations
            .push(format!("blocked external I/O: {operation}"));
    }

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

struct ScopedTransport {
    fixture: HostedFixture,
    server: String,
}

impl Transport for ScopedTransport {
    fn send(&self, req: &Req) -> Sent<'_> {
        let request = Request::new(self.server.clone(), req.clone());
        Box::pin(async move {
            let (reply, gate, index) = {
                let mut s = self.fixture.0.lock().unwrap();
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
                        .push(format!("unexpected request: {}", request.request.path));
                    return Err(format!(
                        "unexpected hosted request: {}",
                        request.request.path
                    ));
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
                    .map_err(|_| "hosted release gate dropped".to_string())?;
            }
            self.fixture.0.lock().unwrap().ledger[index].completed = true;
            Ok(reply)
        })
    }
}

/// Starts empty and unauthenticated. Populate it only via the real host/config,
/// refresh tasks and result ingestion; there is never an HTTP fallback.
pub fn hosted_plugin(
    expected: Vec<ExpectedRequest>,
) -> anyhow::Result<(GitlabPlugin, HostedFixture)> {
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
    let plugin = GitlabPlugin {
        now: radar_core::plugin::DEMO_NOW,
        section_rows: [0, 0, 0, 1],
        scripted_fixture: true,
        fixture: Some(fixture.clone()),
        env: Some(vec![]),
        ..Default::default()
    };
    Ok((plugin, fixture))
}

/// Exact request recipe for a previously unseen group, one page of open merge
/// requests and one page of projects. Other states, pagination and refreshes
/// must declare their own exact requests; this does not auto-answer them.
pub fn initial_group_requests(server: &str, path: &str) -> Vec<Request> {
    crate::client::initial_group_requests(path)
        .into_iter()
        .map(|request| Request::new(server, request))
        .collect()
}

/// An owned, pure merge-request value. No plugin, time, environment or I/O.
#[derive(Clone, Debug)]
pub struct MrBuilder(crate::model::Mr);

impl Default for MrBuilder {
    fn default() -> Self {
        Self(crate::model::Mr {
            iid: 12,
            title: "Repair the Watch bell".into(),
            draft: false,
            web_url: "https://gitlab.example.test/ankh-morpork/watch/-/merge_requests/12".into(),
            updated_at: "2026-01-01T12:00:00Z".into(),
            source_branch: "watch-bell".into(),
            target_branch: "main".into(),
            head_sha: "1234567890abcdef1234567890abcdef12345678".into(),
            notes: 0,
            conflicts: false,
            merge_status: "MERGEABLE".into(),
            rebase: false,
            checks: None,
            approved: false,
            approvals_required: 0,
            approvals_left: 0,
            author: crate::model::Person {
                username: "vimes".into(),
                name: "Sam Vimes".into(),
            },
            reviewers: vec![],
            approved_by: vec![],
            pipeline: None,
            pipeline_url: None,
            additions: 0,
            deletions: 0,
            files: 0,
            stages: vec![],
            duration_secs: None,
            threads: 0,
            threads_resolved: 0,
            last_comment: None,
            project_path: "ankh-morpork/watch".into(),
            project_name: "Watch".into(),
            state: crate::model::MrState::Opened,
            ended_at: String::new(),
            labels: vec![],
            merge_ongoing: false,
            merge_error: String::new(),
            merge_watch: crate::model::MergeWatch::None,
        })
    }
}

impl MrBuilder {
    /// Replace stable identity and its derived link together.
    pub fn identity(mut self, server: &str, project: &str, iid: u64) -> Self {
        self.0.iid = iid;
        self.0.project_path = project.into();
        self.0.project_name = project.rsplit('/').next().unwrap_or(project).into();
        self.0.web_url = format!(
            "{}/{project}/-/merge_requests/{iid}",
            server.trim_end_matches('/')
        );
        self
    }
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.0.title = title.into();
        self
    }
    pub fn draft(mut self, draft: bool) -> Self {
        self.0.draft = draft;
        self
    }
    pub fn build(self) -> crate::model::Mr {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_core::{Plugin, plugin::Effect};
    use serde_json::json;
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };

    const SERVER: &str = "https://gitlab.example.test";
    fn auth() -> Request {
        Request::new(
            SERVER,
            Req::new(
                Method::Post,
                "/api/graphql",
                Some(json!({
                    "query": "query { currentUser { username name } }", "variables": {}
                })),
            ),
        )
    }
    fn me() -> serde_json::Value {
        json!({"data":{"currentUser":{"username":"vimes","name":"Sam Vimes"}}})
    }
    fn configure(p: &mut GitlabPlugin) {
        p.configure(Some(&toml::from_str::<toml::Value>(&format!(
            "url = '{SERVER}'\ntoken = 'glab'\n[[source]]\ngroup = 'ankh-morpork/watch'\ncode = 'WATCH'\n"
        )).unwrap())).unwrap();
    }

    #[test]
    fn builder_is_owned_stable_and_identity_replacement_is_complete() {
        let first = MrBuilder::default().build();
        assert_eq!(first, MrBuilder::default().build());
        let next = MrBuilder::default()
            .identity(SERVER, "ankh-morpork/semaphore", 27)
            .title("Repair the clacks")
            .draft(true)
            .build();
        assert_eq!(first.iid, 12);
        assert_eq!(next.iid, 27);
        assert_eq!(next.project_name, "semaphore");
        assert_eq!(
            next.web_url,
            format!("{SERVER}/ankh-morpork/semaphore/-/merge_requests/27")
        );
        assert!(next.draft);
    }

    #[tokio::test]
    async fn injection_survives_config_restore_connection_retirement_and_demo() {
        let (mut p, f) = hosted_plugin(vec![ExpectedRequest::json("auth", auth(), me())]).unwrap();
        configure(&mut p);
        p.restore(json!({"data":{}, "me":"vimes"}));
        p.replace_connection(SERVER.into(), "env:NEVER_RESOLVE_GITLAB_FIXTURE".into());
        crate::pages::invalidate(&mut p);
        crate::why::reset(&mut p);
        #[cfg(feature = "demo")]
        {
            p.demo();
            configure(&mut p);
        }
        assert_eq!(
            p.provider()
                .run(|c| async move { c.check_user().await })
                .await
                .unwrap(),
            "signed in as vimes (Sam Vimes)"
        );
        f.assert_finished();
        assert!(
            p.provider()
                .run(|c| async move { c.check_user().await })
                .await
                .is_err()
        );
        assert!(f.check_finished().is_err());
    }

    #[tokio::test]
    async fn full_refresh_uses_real_parsing_and_reducer_without_credentials() {
        let requests = initial_group_requests(SERVER, "ankh-morpork/watch");
        let responses = [
            json!({"data":{"group":{"id":"gid://gitlab/Group/12"},"project":null}}),
            json!({"data":{"currentUser":{"username":"vimes"},"group":{"mergeRequests":{
                "nodes":[{"iid":"12","title":"Repair the Watch bell","state":"opened",
                    "project":{"fullPath":"ankh-morpork/watch","name":"Watch"},
                    "author":{"username":"vimes","name":"Sam Vimes"}}],
                "pageInfo":{"hasNextPage":false}}}}}),
            json!({"data":{"group":{"projects":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}}),
        ];
        let expected = requests
            .into_iter()
            .zip(responses)
            .enumerate()
            .map(|(i, (request, body))| {
                ExpectedRequest::json(format!("refresh-{i}"), request, body)
            })
            .collect();
        let (mut p, f) = hosted_plugin(expected).unwrap();
        configure(&mut p);
        let effects = p.refresh();
        assert_eq!(effects.len(), 1);
        for effect in effects {
            let Effect::SpawnAction { task, .. } = effect else {
                panic!("expected real refresh task")
            };
            p.on_msg(task.await);
        }
        f.assert_finished();
        assert_eq!(p.me, "vimes");
        assert_eq!(
            p.data["ankh-morpork/watch"].mrs[0].title,
            "Repair the Watch bell"
        );
        assert!(p.sync.errors.is_empty());
    }

    #[tokio::test]
    async fn exact_write_is_gated_until_explicit_release() {
        let request = Request::new(
            SERVER,
            Req::new(
                Method::Put,
                "/api/v4/projects/ankh-morpork%2Fwatch/merge_requests/12/rebase",
                None,
            ),
        );
        let (mut p, f) = hosted_plugin(vec![
            ExpectedRequest::json("rebase", request.clone(), json!({})).held(),
        ])
        .unwrap();
        configure(&mut p);
        let client = p.provider().client().unwrap();
        let mut call =
            Box::pin(client.put_rest("projects/ankh-morpork%2Fwatch/merge_requests/12/rebase"));
        assert!(matches!(
            call.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        assert_eq!(
            f.ledger(),
            vec![RequestRecord {
                key: Some("rebase".into()),
                request,
                completed: false
            }]
        );
        assert!(f.check_finished().is_err());
        f.release("rebase").unwrap();
        assert_eq!(call.await.unwrap().0, 200);
        f.assert_finished();
    }

    #[tokio::test]
    async fn exact_matching_includes_server_body_method_path_and_range() {
        let expected = auth();
        let mut variants = Vec::new();
        let mut changed = expected.clone();
        changed.server = "https://other.example.test".into();
        variants.push(changed);
        let mut changed = expected.clone();
        changed.request.method = Method::Get;
        variants.push(changed);
        let mut changed = expected.clone();
        changed.request.path = "/api/v4/user".into();
        variants.push(changed);
        let mut changed = expected.clone();
        changed.request.body = Some(json!({"query":"different"}));
        variants.push(changed);
        let mut changed = expected.clone();
        changed.request.range = Some("bytes=0-4".into());
        variants.push(changed);
        for changed in variants {
            let (_, f) =
                hosted_plugin(vec![ExpectedRequest::json("auth", expected.clone(), me())]).unwrap();
            let transport = ScopedTransport {
                fixture: f.clone(),
                server: changed.server,
            };
            assert!(transport.send(&changed.request).await.is_err());
            assert_eq!(f.0.lock().unwrap().expected.len(), 1);
            assert!(f.check_finished().is_err());
            assert!(f.client(SERVER).check_user().await.is_ok());
            assert!(
                f.check_finished().is_err(),
                "violation must remain after expected read"
            );
        }
    }

    #[tokio::test]
    async fn separate_fixtures_do_not_share_scripts_or_violations() {
        let (_, a) = hosted_plugin(vec![ExpectedRequest::json("same", auth(), me())]).unwrap();
        let (_, b) = hosted_plugin(vec![ExpectedRequest::json("same", auth(), me())]).unwrap();
        a.client(SERVER).check_user().await.unwrap();
        a.assert_finished();
        assert!(b.ledger().is_empty());
        assert!(b.check_finished().is_err());
        b.client(SERVER).check_user().await.unwrap();
        b.assert_finished();
    }

    #[tokio::test]
    async fn unexpected_optional_read_is_sticky_even_when_parser_swallows_error() {
        let (_, f) = hosted_plugin(vec![]).unwrap();
        assert!(f.client(SERVER).token_scopes().await.is_none());
        assert!(f.check_finished().is_err());
        assert_eq!(f.ledger().len(), 1);
    }

    #[tokio::test]
    async fn rejection_never_renews_glab_or_retries_a_mutation() {
        let (mut p, f) = hosted_plugin(vec![ExpectedRequest::reply(
            "auth",
            auth(),
            Resp {
                status: 401,
                ..Default::default()
            },
        )])
        .unwrap();
        configure(&mut p);
        assert!(
            p.provider()
                .run(|c| async move { c.check_user().await })
                .await
                .is_err()
        );
        f.assert_finished();
        assert_eq!(f.ledger().len(), 1);
    }

    #[tokio::test]
    async fn unsupported_pictures_are_fail_closed_and_sticky() {
        let (mut p, f) = hosted_plugin(vec![]).unwrap();
        configure(&mut p);
        assert!(
            p.pictures("").unwrap()(format!("{SERVER}/uploads/bell.png"))
                .await
                .is_err()
        );
        assert!(f.check_finished().is_err());
    }

    #[tokio::test]
    async fn explicit_marker_with_missing_transport_cannot_fall_back() {
        let (mut p, _) = hosted_plugin(vec![]).unwrap();
        configure(&mut p);
        p.fixture = None;
        let err = p
            .provider()
            .run(|c| async move { c.check_user().await })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no transport"));
    }

    #[tokio::test]
    async fn cancelled_held_request_and_invalid_release_remain_failures() {
        let (_, f) =
            hosted_plugin(vec![ExpectedRequest::json("auth", auth(), me()).held()]).unwrap();
        let client = f.client(SERVER);
        let mut call = Box::pin(client.check_user());
        assert!(matches!(
            call.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        drop(call);
        assert!(f.release("auth").is_err());
        assert!(f.release("auth").is_err());
        assert!(f.check_finished().is_err());
        let (_, other) = hosted_plugin(vec![]).unwrap();
        assert!(other.release("missing").is_err());
        assert!(other.check_finished().is_err());
    }

    #[test]
    fn keys_must_be_nonempty_and_unique_and_unused_expectations_fail() {
        assert!(hosted_plugin(vec![ExpectedRequest::json("", auth(), me())]).is_err());
        assert!(
            hosted_plugin(vec![
                ExpectedRequest::json("auth", auth(), me()),
                ExpectedRequest::json("auth", auth(), me())
            ])
            .is_err()
        );
        let (_, f) = hosted_plugin(vec![ExpectedRequest::json("unused", auth(), me())]).unwrap();
        assert!(
            f.check_finished()
                .unwrap_err()
                .to_string()
                .contains("unused")
        );
    }
    #[tokio::test]
    async fn cli_uses_captured_injection_before_any_credentials() {
        let request = initial_group_requests(SERVER, "ankh-morpork/watch").remove(0);
        let (mut p, f) = hosted_plugin(vec![ExpectedRequest::reply(
            "lookup",
            request,
            Resp {
                status: 401,
                ..Default::default()
            },
        )])
        .unwrap();
        configure(&mut p);
        let command = p
            .cli()
            .unwrap()
            .try_get_matches_from(["gitlab", "list"])
            .unwrap();
        assert!(
            p.run_cli(&command)
                .await
                .unwrap_err()
                .to_string()
                .contains("401")
        );
        f.assert_finished();
    }

    struct ModeGuard((radar_core::writes::Mode, Option<String>));
    impl ModeGuard {
        fn new(mode: radar_core::writes::Mode) -> Self {
            let previous = radar_core::writes::mode();
            radar_core::writes::set_mode(mode, None);
            Self(previous)
        }
    }
    impl Drop for ModeGuard {
        fn drop(&mut self) {
            radar_core::writes::set_mode(self.0.0, self.0.1.take());
        }
    }

    #[tokio::test]
    async fn real_rebase_dispatch_keeps_permission_and_single_send_semantics() {
        let request = Request::new(
            SERVER,
            Req::new(
                Method::Put,
                "/api/v4/projects/ankh-morpork%2Fwatch/merge_requests/12/rebase",
                None,
            ),
        );
        let (mut p, f) = hosted_plugin(vec![ExpectedRequest::reply(
            "rebase",
            request,
            Resp {
                status: 401,
                body: b"{}".to_vec(),
                ..Default::default()
            },
        )])
        .unwrap();
        configure(&mut p);
        {
            let _mode = ModeGuard::new(radar_core::writes::Mode::ReadOnly);
            let effects = crate::mr_actions::start(&mut p, "ankh-morpork/watch", 12, "rebase");
            assert!(
                !effects
                    .iter()
                    .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
            );
        }
        assert!(f.ledger().is_empty());
        let effects = {
            let _mode = ModeGuard::new(radar_core::writes::Mode::Edit);
            crate::mr_actions::start(&mut p, "ankh-morpork/watch", 12, "rebase")
        };
        let _mode = ModeGuard::new(radar_core::writes::Mode::ReadOnly);
        let mut sent = 0;
        for effect in effects {
            if let Effect::Spawn(task) = effect {
                p.on_msg(task.await);
                sent += 1;
            }
        }
        assert_eq!(sent, 1);
        f.assert_finished();
        assert_eq!(f.ledger().len(), 1);
        assert!(matches!(
            &p.acting["ankh-morpork/watch!12"].state,
            crate::mr_actions::Acting::Failed(_)
        ));
    }
}
