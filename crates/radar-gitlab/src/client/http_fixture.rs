//! Real HTTP body collection over bounded, synthetic-only loopback responses.
//! No production client, configured tokens, proxies, redirects or live providers.

use super::*;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use radar_core::Plugin;
use radar_core::journal::Outcome;
use radar_core::plugin::Effect;

const ERROR: &str = "GitLab response body could not be read.";
const TOKEN: &str = "SYNTHETIC_TOKEN_CANARY";
const BODY: &[u8] = b"BODY_CANARY";
const PROJECT: &str = "acme/team-a/api";
const JOB: u64 = 41;

struct Reply {
    status: u16,
    body: Vec<u8>,
    missing: usize,
    range: Option<&'static str>,
    /// Header lines, each ending in `\r\n`.
    headers: &'static str,
}
impl Reply {
    fn complete(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
            missing: 0,
            range: None,
            headers: "",
        }
    }
    fn truncated(status: u16) -> Self {
        Self {
            missing: 7,
            range: Some("bytes 100-110/200"),
            ..Self::complete(status, BODY)
        }
    }
    fn wire(&self) -> Vec<u8> {
        let range = self
            .range
            .map(|r| format!("Content-Range: {r}\r\n"))
            .unwrap_or_default();
        let mut bytes = format!(
            "HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\n{range}{}Connection: close\r\n\r\n",
            self.status,
            self.body.len() + self.missing,
            self.headers,
        )
        .into_bytes();
        bytes.extend(&self.body);
        bytes
    }
}

struct Fixture {
    base: String,
    stopped: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
    worker: Option<JoinHandle<io::Result<()>>>,
}
impl Fixture {
    fn new(replies: Vec<Reply>) -> Self {
        let listener =
            TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("loopback bind");
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stopped = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = stopped.clone();
        let asked = requests.clone();
        let worker = thread::spawn(move || {
            for reply in replies {
                let deadline = Instant::now() + Duration::from_secs(2);
                let mut stream = loop {
                    if stop.load(Ordering::SeqCst) {
                        return Ok(());
                    }
                    if Instant::now() >= deadline {
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                    match listener.accept() {
                        Ok((stream, peer)) => {
                            assert!(peer.ip().is_loopback());
                            break stream;
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            thread::park_timeout(Duration::from_millis(1));
                        }
                        Err(e) => return Err(e),
                    }
                };
                stream.set_read_timeout(Some(Duration::from_millis(50)))?;
                stream.set_write_timeout(Some(Duration::from_millis(100)))?;
                let request = read_request(&mut stream, deadline, &stop)?;
                asked.lock().unwrap().push(request);
                stream.write_all(&reply.wire())?;
                stream.flush()?;
                // Closing before the advertised length makes bytes(), not send(), fail.
            }
            Ok(())
        });
        Self {
            base,
            stopped,
            requests,
            worker: Some(worker),
        }
    }
    fn client(&self) -> Client {
        self.client_with(TOKEN.into())
    }
    fn client_with(&self, token: String) -> Client {
        Client {
            base: self.base.clone(),
            t: Arc::new(Http {
                base: self.base.clone(),
                token,
                http: reqwest::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_secs(1))
                    .build()
                    .unwrap(),
            }),
            pacing: Arc::default(),
        }
    }
    fn finish(mut self) -> Vec<Vec<u8>> {
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("fixture thread")
            .expect("fixture IO");
        self.requests.lock().unwrap().clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            // The read/accept deadlines also bound cleanup during a failed assertion.
            let _ = worker.join();
        }
    }
}

fn read_request(
    stream: &mut TcpStream,
    deadline: Instant,
    stop: &AtomicBool,
) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    loop {
        if stop.load(Ordering::SeqCst) || Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            if end > 16 * 1024 {
                return Err(io::ErrorKind::InvalidData.into());
            }
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if length > 32 * 1024 {
                return Err(io::ErrorKind::InvalidData.into());
            }
            if bytes.len() >= end + 4 + length {
                return Ok(bytes);
            }
        } else if bytes.len() > 16 * 1024 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let mut chunk = [0; 2048];
        match stream.read(&mut chunk) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e),
        }
    }
}

fn block<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn successful_status_with_truncated_body_is_a_fixed_error_not_empty_success() {
    for status in [200, 206] {
        let fixture = Fixture::new(vec![Reply::truncated(status)]);
        let error = block(
            fixture
                .client()
                .send(Req::new(Method::Get, "/BODY_CANARY", None)),
        )
        .unwrap_err();
        assert_eq!(format!("{error:#}"), ERROR);
        assert!(!is_rejected(&error));
        let asked = fixture.finish();
        assert_eq!(asked.len(), 1);
        assert!(String::from_utf8_lossy(&asked[0]).contains(TOKEN));
    }
}

#[test]
fn completed_bodies_preserve_status_range_and_bytes_including_empty() {
    for status in [200, 206, 401, 403, 416, 500] {
        for body in [Vec::new(), b"complete\0\xff BODY_CANARY".to_vec()] {
            let mut reply = Reply::complete(status, body.clone());
            reply.range = Some("bytes 100-110/200");
            let fixture = Fixture::new(vec![reply]);
            let got = block(
                fixture
                    .client()
                    .send(Req::new(Method::Get, "/fixture", None)),
            )
            .unwrap();
            assert_eq!(
                got,
                Resp {
                    status,
                    body,
                    content_range: Some("bytes 100-110/200".into()),
                    ..Default::default()
                }
            );
            assert_eq!(fixture.finish().len(), 1);
        }
    }
}

#[test]
fn unreadable_error_body_keeps_401_rejection_and_403_refusal_across_read_apis() {
    for status in [401, 403] {
        let fixture = Fixture::new((0..3).map(|_| Reply::truncated(status)).collect());
        let client = fixture.client();
        let errors = block(async {
            [
                client.job(PROJECT, JOB).await.unwrap_err(),
                client.trace(PROJECT, JOB, None).await.unwrap_err(),
                client
                    .graphql("query { currentUser { username } }", json!({}))
                    .await
                    .unwrap_err(),
            ]
        });
        for error in errors {
            assert_eq!(is_rejected(&error), status == 401);
            assert!(error.to_string().contains(&status.to_string()));
            assert!(!format!("{error:#}").contains("CANARY"));
        }
        assert_eq!(fixture.finish().len(), 3);
    }
}

#[test]
fn legacy_416_default_and_readable_error_messages_are_unchanged() {
    let fixture = Fixture::new(vec![
        Reply::truncated(416),
        Reply::complete(403, br#"{"message":"readable refusal"}"#),
        Reply::complete(403, br#"{"message":"readable refusal"}"#),
        Reply::complete(403, br#"{"errors":[{"message":"readable refusal"}]}"#),
    ]);
    let client = fixture.client();
    block(async {
        let got = client.trace_observed(PROJECT, JOB, None).await.unwrap();
        assert_eq!(got.part, TracePart::default());
        assert_eq!(got.evidence.supplied_bytes, 0);
        assert_eq!(
            got.evidence.interpretation,
            TraceInterpretation::Legacy416Default
        );
        for error in [
            client.job(PROJECT, JOB).await.unwrap_err(),
            client.trace(PROJECT, JOB, None).await.unwrap_err(),
            client.graphql("query {}", json!({})).await.unwrap_err(),
        ] {
            assert!(error.to_string().contains("readable refusal"));
            assert!(!is_rejected(&error));
        }
    });
    assert_eq!(fixture.finish().len(), 4);
}

#[derive(Default)]
struct Tokens {
    renewals: AtomicUsize,
    reads: AtomicUsize,
}
impl crate::auth::Tokens for Tokens {
    fn token(&self) -> anyhow::Result<String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(TOKEN.into())
    }
    fn renew(&self) -> anyhow::Result<bool> {
        self.renewals.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
}

#[test]
fn only_verified_401_renews_once_never_a_success_status_body_error() {
    for (statuses, renewals, completed_final) in [
        (vec![401, 200], 1, true),
        (vec![401, 200], 1, false),
        (vec![401, 401], 1, false),
        (vec![403], 0, false),
        (vec![200], 0, false),
        (vec![206], 0, false),
    ] {
        let fixture = Fixture::new(
            statuses
                .iter()
                .map(|status| {
                    if completed_final && *status == 200 {
                        Reply::complete(*status, BODY)
                    } else {
                        Reply::truncated(*status)
                    }
                })
                .collect(),
        );
        let tokens = Tokens::default();
        let result = block(crate::auth::with_token(&tokens, |token| {
            let client = fixture.client_with(token);
            async move { client.trace(PROJECT, JOB, None).await }
        }));
        assert_eq!(result.is_ok(), completed_final);
        if completed_final {
            assert_eq!(result.unwrap().text.as_bytes(), BODY);
        } else if statuses
            .last()
            .is_some_and(|status| (200..300).contains(status))
        {
            assert_eq!(result.unwrap_err().to_string(), ERROR);
        }
        assert_eq!(tokens.renewals.load(Ordering::SeqCst), renewals);
        assert_eq!(tokens.reads.load(Ordering::SeqCst), statuses.len());
        assert_eq!(fixture.finish().len(), statuses.len());
    }
}

fn metadata() -> Vec<u8> {
    json!({"id":JOB, "name":"BODY_CANARY", "stage":"test", "status":"failed"})
        .to_string()
        .into_bytes()
}

#[test]
fn combined_body_failures_preserve_held_metadata_log_time_and_page_state() {
    for tail_fails in [false, true] {
        let replies = if tail_fails {
            vec![Reply::complete(200, metadata()), Reply::truncated(206)]
        } else {
            vec![Reply::truncated(200)]
        };
        let fixture = Fixture::new(replies);
        let result = block(crate::log_history::read(&fixture.client(), PROJECT, JOB))
            .map_err(|e| format!("{e:#}"));
        assert_eq!(result.as_ref().unwrap_err(), ERROR);
        let mut p = crate::demo::sample();
        let key = crate::log_ownership::JobKey::new(PROJECT, JOB);
        let info = parse_job(&json!({"id":JOB, "name":"held job"}), PROJECT);
        p.jobs.insert(key.clone(), info.clone());
        p.logs.insert(
            key.clone(),
            crate::pages::Fetched {
                value: Some(crate::joblog::JobLog::new(TracePart {
                    text: "old needle\nheld tail\n".into(),
                    start: 100,
                    total: 200,
                })),
                loading: true,
                error: None,
                at: 123,
            },
        );
        p.pages.clear();
        let subject = crate::join::job(&p, PROJECT, JOB);
        drop(p.show(&subject)); // The held/loading log needs no new task.
        assert_eq!(p.pages.len(), 1);
        let held_log = format!("{:?}", p.logs[&key].value);
        let text = &p.logs[&key].value.as_ref().unwrap().text;
        p.pages[0].log.search(text, "needle");
        p.pages[0].log.goto(text, 1);
        let held_page = format!("{:?}", p.pages[0].log);
        let request = crate::log_ownership::direct(&mut p, key.clone());
        let effects = p.on_msg(Box::new(crate::GlMsg::JobLog { request, result }));
        assert_eq!(p.jobs[&key], info);
        assert_eq!(format!("{:?}", p.logs[&key].value), held_log);
        assert_eq!(p.logs[&key].at, 123);
        assert!(!p.logs[&key].loading);
        assert_eq!(p.logs[&key].error.as_deref(), Some(ERROR));
        assert_eq!(format!("{:?}", p.pages[0].log), held_page);
        assert!(matches!(effects[0], Effect::Report(Outcome::Failed)));
        assert!(
            matches!(&effects[1], Effect::Observe { action, outcome: Outcome::Failed } if action.kind.as_str() == "gitlab.job.log.errors" && action.metadata.count == Some(1))
        );
        assert_eq!(
            effects.len(),
            2,
            "no intermediate metadata or invented body progress"
        );
        assert!(!format!("{effects:?}").contains("CANARY"));
        assert_eq!(fixture.finish().len(), if tail_fails { 2 } else { 1 });
    }
}

#[test]
fn truncated_write_sends_once_is_unknown_and_keeps_original_comment_uncertain() {
    let fixture = Fixture::new(vec![Reply::truncated(200)]);
    let mut p = crate::demo::sample();
    let original = "  original text\n日本語\n  ";
    let change = crate::write::Change::Note {
        project: PROJECT.into(),
        iid: 263,
        text: original.into(),
    };
    let review = crate::comment_recovery::Review::capture(&p, &change);
    let operation = p.recovery.reserve(&change, &review, true).unwrap();
    let result = block(crate::write::send_with(&fixture.client(), &change));
    assert_eq!(result, Err(ERROR.into()));
    let effects = crate::write::on_wrote(
        &mut p,
        crate::write::WroteMsg {
            operation,
            connection: review.connection,
            change: change.clone(),
            result,
        },
    );
    assert!(matches!(effects[0], Effect::Report(Outcome::Unknown)));
    assert_eq!(effects.len(), 2, "no retry or remote-check effect");
    let record = &p.recovery.records[&operation];
    assert_eq!(record.original, original);
    assert_eq!(record.status, crate::comment_recovery::Status::Uncertain);
    let expected = format!(
        "Could not confirm the result of asking GitLab to post a comment on !263: {ERROR} Check GitLab before trying again."
    );
    assert!(
        matches!(&effects[1], Effect::Notice(words) if words.tone == radar_core::view::chrome::NoticeTone::Error && words.text == format!("✗ {expected} · c has your text"))
    );
    assert_eq!(
        p.acting[&change.key()].state,
        crate::mr_actions::Acting::Failed(expected.clone())
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    let request = String::from_utf8_lossy(&requests[0]);
    assert!(
        request.starts_with("POST /api/v4/projects/acme%2Fteam-a%2Fapi/merge_requests/263/notes ")
    );
    let body = request.split_once("\r\n\r\n").unwrap().1;
    assert_eq!(
        serde_json::from_str::<Value>(body).unwrap(),
        json!({"body":original})
    );
}

#[test]
fn complete_empty_write_remains_a_status_only_success_for_post_and_put() {
    for change in [
        crate::write::Change::Note {
            project: PROJECT.into(),
            iid: 263,
            text: "synthetic".into(),
        },
        crate::write::Change::Resolve {
            project: PROJECT.into(),
            iid: 263,
            discussion: "abc".into(),
            place: "fixture".into(),
            resolve: true,
        },
    ] {
        let fixture = Fixture::new(vec![Reply::complete(200, Vec::new())]);
        assert!(block(crate::write::send_with(&fixture.client(), &change)).is_ok());
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[test]
fn action_completion_notice_and_page_distinguish_unknown_from_known_failure_and_block() {
    for outcome in [Outcome::Unknown, Outcome::Failed, Outcome::Blocked] {
        let mut p = crate::demo::sample();
        let connection = p.connection_stamp();
        let effects = crate::mr_actions::on_msg(
            &mut p,
            crate::mr_actions::ActMsg {
                connection,
                key: format!("{PROJECT}!263"),
                what: "rebase",
                result: Err("fixture reason".into()),
                outcome,
            },
        );
        let words = if outcome == Outcome::Unknown {
            "Could not confirm the result of asking GitLab to rebase !263: fixture reason Check GitLab before trying again."
        } else {
            "GitLab did not rebase !263: fixture reason"
        };
        assert!(matches!(effects[0], Effect::Report(got) if got == outcome));
        assert!(matches!(&effects[1], Effect::Notice(got) if got.tone == radar_core::view::chrome::NoticeTone::Error && got.text == format!("✗ {words}")));
        let acting = &p.acting[&format!("{PROJECT}!263")];
        let shown = match &acting.state {
            crate::mr_actions::Acting::Failed(shown) | crate::mr_actions::Acting::Done(shown) => {
                shown.clone()
            }
            crate::mr_actions::Acting::Running => String::new(),
        };
        assert!(shown.contains(words) || words.contains(&shown));
        assert!(
            !shown.contains("GitLab did not rebase: GitLab"),
            "no doubled wrapper"
        );
        assert_eq!(
            shown.matches("GitLab did not").count(),
            usize::from(outcome != Outcome::Unknown)
        );
    }
}

#[test]
fn real_host_records_failed_combined_http_read_without_payload_or_component_progress() {
    use radar_core::Host;
    use radar_core::host::{Event, HostEffect, JournalRecord};
    use radar_core::plugin::Msg;

    struct Begin(Client);
    struct Reads(crate::GitlabPlugin);
    impl Plugin for Reads {
        fn id(&self) -> &'static str {
            "gitlab"
        }
        fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
            Ok(())
        }
        fn config(&self) -> Option<toml::Value> {
            None
        }
        fn on_msg(&mut self, message: Msg) -> Vec<Effect> {
            let message = match message.downcast::<Begin>() {
                Ok(begin) => {
                    let key = crate::log_ownership::JobKey::new(PROJECT, JOB);
                    self.0.logs.entry(key.clone()).or_default().loading = true;
                    let request = crate::log_ownership::direct(&mut self.0, key);
                    return vec![crate::log_history::spawn(request, async move {
                        crate::log_history::read(&begin.0, PROJECT, JOB)
                            .await
                            .map_err(|e| format!("{e:#}"))
                    })];
                }
                Err(message) => message,
            };
            self.0.on_msg(message)
        }
    }
    fn records(effects: &[HostEffect]) -> Vec<JournalRecord> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                HostEffect::Journal(record) => Some(record.clone()),
                _ => None,
            })
            .collect()
    }

    let fixture = Fixture::new(vec![
        Reply::complete(200, metadata()),
        Reply::truncated(206),
    ]);
    let mut host = Host::new(vec![Box::new(Reads(crate::demo::sample()))], 1).unwrap();
    host.enable_journal();
    let initial = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin(fixture.client())),
    });
    let mut all = records(&initial);
    let (task_id, semantic, task) = initial
        .into_iter()
        .find_map(|effect| match effect {
            HostEffect::Tracked {
                action,
                semantic: Some(semantic),
                effect,
            } => match *effect {
                HostEffect::Spawn { task, .. } => Some((action, semantic, task)),
                _ => None,
            },
            _ => None,
        })
        .expect("tracked combined read");
    let completion = host.update(Event::Tracked {
        action: task_id,
        semantic: Some(semantic.clone()),
        event: Box::new(Event::Msg {
            plugin: 0,
            msg: block(task),
        }),
    });
    all.extend(records(&completion));
    let terminal: Vec<_> = all
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Outcome { id, outcome, .. }
                if id == &semantic && outcome.is_terminal() =>
            {
                Some(*outcome)
            }
            _ => None,
        })
        .collect();
    assert_eq!(terminal, [Outcome::Failed]);
    let actions: Vec<_> = all
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Begin { descriptor, .. }
                if descriptor.kind.as_str().starts_with("gitlab.job.log") =>
            {
                Some(descriptor)
            }
            _ => None,
        })
        .collect();
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0].kind.as_str(), "gitlab.job.log");
    assert_eq!(actions[1].kind.as_str(), "gitlab.job.log.errors");
    assert_eq!(actions[1].metadata.count, Some(1));
    struct RemoveDir(std::path::PathBuf);
    impl Drop for RemoveDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = RemoveDir(std::env::temp_dir().join(format!(
        "radar-gitlab-http-history-{}",
        radar_core::journal::ActionId::new().as_str()
    )));
    let mut journal = radar_core::journal::Journal::open(&dir.0.join("actions.sqlite3")).unwrap();
    let session = journal.start_session().unwrap();
    for record in all {
        match record {
            JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } => {
                journal
                    .begin_with_id(&session, &id, descriptor, parent.as_ref())
                    .unwrap();
            }
            JournalRecord::Outcome {
                id,
                outcome,
                metadata,
            } => {
                journal.record(&id, outcome, metadata).unwrap();
            }
            JournalRecord::Gap { .. } => panic!("unexpected gap"),
        }
    }
    journal.finish_session(&session).unwrap();
    let events = journal
        .query(&radar_core::journal::Query {
            limit: 100,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.action_id == semantic && event.outcome.is_terminal())
            .map(|event| event.outcome)
            .collect::<Vec<_>>(),
        [Outcome::Failed]
    );
    let encoded = serde_json::to_string(&events).unwrap();
    for forbidden in [
        "CANARY",
        ERROR,
        "127.0.0.1",
        "metadata.responses",
        "body.supplied",
        "body.accepted",
    ] {
        assert!(!encoded.contains(forbidden), "{encoded}");
    }
    assert_eq!(fixture.finish().len(), 2);
}

#[test]
fn the_retry_after_and_ratelimit_headers_are_read_from_the_wire() {
    use crate::backoff::Pacing;
    let seconds = Reply {
        headers: "Retry-After: 120\r\nRateLimit-Limit: 2000\r\nRateLimit-Remaining: 3\r\nRateLimit-Reset: 50\r\n",
        ..Reply::complete(503, "<html>down</html>")
    };
    let date = Reply {
        headers: "Retry-After: Mon, 01 Jan 2100 00:00:00 GMT\r\n",
        ..Reply::complete(429, "")
    };
    let fixture = Fixture::new(vec![seconds, date]);
    let client = fixture.client();
    let (before, a, b, after) = block(async {
        let before = radar_core::time::now_secs();
        let a = client.send(Req::new(Method::Get, "/a", None)).await.unwrap();
        let b = client.send(Req::new(Method::Get, "/b", None)).await.unwrap();
        (before, a, b, radar_core::time::now_secs())
    });
    let near = |at: Option<i64>, secs: i64| at.is_some_and(|at| (before + secs..=after + secs).contains(&at));
    assert!(near(a.pacing.retry_at, 120), "{:?}", a.pacing);
    assert!(near(a.pacing.reset_at, 50), "{:?}", a.pacing);
    assert_eq!((a.pacing.remaining, a.pacing.limit), (Some(3), Some(2000)));
    assert!(a.pacing.low());
    assert_eq!(
        b.pacing,
        Pacing {
            retry_at: Some(4_102_444_800),
            ..Pacing::default()
        }
    );
    assert_eq!(client.pacing(), b.pacing, "the latest answer's");
    fixture.finish();
}

#[test]
fn every_client_for_one_server_shares_its_connections() {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicBool::new(false));
    let server = {
        let (accepted, stopped) = (accepted.clone(), stopped.clone());
        thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        accepted.fetch_add(1, Ordering::SeqCst);
                        stream.set_nonblocking(false).unwrap();
                        stream.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
                        let stopped = stopped.clone();
                        thread::spawn(move || {
                            let deadline = Instant::now() + Duration::from_secs(10);
                            while read_request(&mut stream, deadline, &stopped).is_ok() {
                                let reply = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
                                if stream.write_all(reply).is_err() {
                                    break;
                                }
                            }
                        });
                    }
                    Err(_) => thread::sleep(Duration::from_millis(1)),
                }
            }
        })
    };
    // One runtime: a connection lives and dies with the one that opened it.
    block(async {
        for _ in 0..3 {
            let got = Client::new(&base, "t".into())
                .send(Req::new(Method::Get, "/x", None))
                .await
                .unwrap();
            assert_eq!(got.status, 200);
        }
    });
    stopped.store(true, Ordering::SeqCst);
    server.join().unwrap();
    assert_eq!(accepted.load(Ordering::SeqCst), 1, "one handshake, not one a request");
}
