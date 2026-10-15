//! Actual response collection against bounded, synthetic-only loopback HTTP.
//! Production client factories and credential resolution are never invoked.

use super::*;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionId, Outcome};
use radar_core::plugin::{Effect, Msg, Task};
use radar_core::{Host, Plugin, writes};

use crate::{JiraMsg, JiraPlugin, demo, detail_key, job, start, sync_history, write};

const ERROR: &str = "Jira response body could not be read.";
const EMAIL: &str = "EMAIL_CANARY@example.invalid";
const TOKEN: &str = "TOKEN_CANARY";
const BODY: &str = "BODY_CANARY";
const JQL: &str = "summary ~ QUERY_CANARY";

struct Reply {
    status: u16,
    body: String,
    missing: usize,
    retry_after: Option<&'static str>,
    headers: Vec<(&'static str, &'static str)>,
}

impl Reply {
    fn complete(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.into(),
            missing: 0,
            retry_after: None,
            headers: vec![],
        }
    }

    fn truncated(status: u16) -> Self {
        Self {
            missing: 7,
            ..Self::complete(status, BODY)
        }
    }

    fn retry_after(mut self, value: &'static str) -> Self {
        self.retry_after = Some(value);
        self
    }

    fn header(mut self, name: &'static str, value: &'static str) -> Self {
        self.headers.push((name, value));
        self
    }

    fn wire(&self) -> Vec<u8> {
        let mut retry = self
            .retry_after
            .map(|v| format!("Retry-After: {v}\r\n"))
            .unwrap_or_default();
        for (name, value) in &self.headers {
            retry.push_str(&format!("{name}: {value}\r\n"));
        }
        format!(
            "HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\n{retry}Connection: close\r\n\r\n{}",
            self.status,
            self.body.len() + self.missing,
            self.body,
        )
        .into_bytes()
    }
}

struct Fixture {
    base: String,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl Fixture {
    fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stopped = stop.clone();
        let asked = requests.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut replies = VecDeque::from(replies);
            // Keep serving/counting extras until the client finishes. Exhausting
            // the script must not hide an unexpected retry as a refused socket.
            while !stopped.load(Ordering::SeqCst) {
                if Instant::now() >= deadline {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                match listener.accept() {
                    Ok((mut stream, peer)) => {
                        assert!(peer.ip().is_loopback());
                        stream.set_read_timeout(Some(Duration::from_millis(25)))?;
                        stream.set_write_timeout(Some(Duration::from_millis(100)))?;
                        let request = read_request(&mut stream, deadline, &stopped)?;
                        asked.lock().unwrap().push(request);
                        let reply = replies
                            .pop_front()
                            .unwrap_or_else(|| Reply::complete(503, ""));
                        stream.write_all(&reply.wire())?;
                        stream.flush()?;
                        // Close with fewer bytes than Content-Length: send()
                        // sees headers, then text() fails while collecting.
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::park_timeout(Duration::from_millis(1));
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        });
        Self {
            base,
            stop,
            requests,
            worker: Some(worker),
        }
    }

    fn client(&self) -> Client {
        Client::over(Arc::new(Http {
            base: self.base.clone(),
            auth: format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!("{EMAIL}:{TOKEN}"))
            ),
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(1))
                .build()
                .unwrap(),
        }))
    }

    fn finish(mut self) -> Vec<Vec<u8>> {
        self.stop.store(true, Ordering::SeqCst);
        let worker = self.worker.take().unwrap();
        worker.thread().unpark();
        worker.join().expect("fixture thread").expect("fixture I/O");
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            // Deadline and socket timeout also bound cleanup after a panic.
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

fn get() -> Req {
    Req {
        method: Method::Get,
        path: "/PATH_CANARY?token=QUERY_CANARY".into(),
        body: None,
    }
}

fn safe_error(error: &str) {
    assert_eq!(error, ERROR);
    assert!(!error.contains("CANARY"));
    assert!(!error.contains("127.0.0.1"));
    assert!(!error.contains("Basic"));
}

fn unreadable_read(method: Method) {
    let fixture = Fixture::new(vec![Reply::truncated(200)]);
    let client = fixture.client();
    let error = block(async {
        match method {
            Method::Get => client
                .tickets_observed(JQL, &|_| None, &[])
                .await
                .map(|_| Value::Null),
            Method::Post => client.check_jql(JQL).await.map(|_| Value::Null),
            Method::Put => unreachable!(),
        }
    })
    .unwrap_err();
    safe_error(&format!("{error:#}"));
    let asked = fixture.finish();
    assert_eq!(asked.len(), 1);
    let request = String::from_utf8_lossy(&asked[0]);
    assert!(request.starts_with(if method == Method::Get {
        "GET "
    } else {
        "POST "
    }));
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("{EMAIL}:{TOKEN}"));
    assert!(request.contains(&format!("Basic {auth}")));
    assert!(request.contains("QUERY_CANARY"));
}

#[test]
fn truncated_successful_get_is_an_error_without_retry() {
    unreadable_read(Method::Get);
}

#[test]
fn truncated_successful_read_only_post_is_an_error_without_retry() {
    unreadable_read(Method::Post);
}

#[test]
fn truncated_development_panel_is_an_error_instead_of_default_links() {
    let fixture = Fixture::new(vec![Reply::truncated(200)]);
    let error = block(fixture.client().code_observed("10012")).unwrap_err();
    safe_error(&error.to_string());
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn collected_bodies_keep_legacy_json_blank_no_content_and_malformed_semantics() {
    let fixture = Fixture::new(vec![
        Reply::complete(200, r#"{"count":7}"#),
        Reply::complete(200, ""),
        Reply::complete(200, " \n\t "),
        Reply::complete(204, ""),
        Reply::complete(200, "{"),
        Reply::complete(200, ""),
    ]);
    let client = fixture.client();
    block(async {
        assert_eq!(
            client.request(get(), "fixture", true).await.unwrap(),
            json!({"count":7})
        );
        for _ in 0..3 {
            assert_eq!(
                client.request(get(), "fixture", true).await.unwrap(),
                Value::Null
            );
        }
        let error = client.request(get(), "fixture", true).await.unwrap_err();
        assert!(error.to_string().starts_with("Jira's answer was not JSON:"));
        assert!(
            client.detail("ABC-12", None).await.is_err(),
            "detail parser still rejects Null"
        );
    });
    assert_eq!(fixture.finish().len(), 6);
}

#[test]
fn fully_collected_transport_preserves_status_retry_after_and_exact_body() {
    for status in [200, 201, 204, 299, 400, 401, 403, 404, 429, 503] {
        let body = if status == 204 {
            ""
        } else {
            " \n{\"count\":7}\t "
        };
        let fixture = Fixture::new(vec![Reply::complete(status, body).retry_after("17")]);
        let response = block(fixture.client().t.send(&get())).unwrap();
        assert_eq!(
            response,
            Resp {
                status,
                retry_after: Some(17),
                rate: Rate::default(),
                body: body.into()
            }
        );
        assert_eq!(fixture.finish().len(), 1);
    }
}

#[test]
fn unreadable_non_success_keeps_status_retry_after_and_empty_fallback() {
    for status in [401, 403, 404, 429, 503] {
        let fixture = Fixture::new(vec![Reply::truncated(status).retry_after("17")]);
        let response = block(fixture.client().t.send(&get())).unwrap();
        assert_eq!(
            response,
            Resp {
                status,
                retry_after: Some(17),
                rate: Rate::default(),
                body: String::new()
            }
        );
        assert_eq!(fixture.finish().len(), 1);
    }
    for status in [401, 403, 404] {
        let fixture = Fixture::new(vec![Reply::truncated(status)]);
        let error = block(fixture.client().request(get(), "ticket ABC-12", true)).unwrap_err();
        let expected = answer(
            Resp {
                status,
                retry_after: None,
                rate: Rate::default(),
                body: String::new(),
            },
            "ticket ABC-12",
            1,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), expected.to_string());
        assert!(!format!("{error:#}").contains("CANARY"));
        assert_eq!(
            fixture.finish().len(),
            1,
            "no auth renewal or extra attempt"
        );
    }
}

#[test]
fn status_retries_remain_three_for_reads_once_for_writes_and_can_succeed() {
    for status in [429, 503] {
        for retry in [true, false] {
            let fixture = Fixture::new(
                (0..4)
                    .map(|_| Reply::truncated(status).retry_after("5"))
                    .collect(),
            );
            let req = if retry {
                get()
            } else {
                comment_req("ABC-12", BODY).unwrap()
            };
            let error = block(fixture.client().request(req, "ticket ABC-12", retry)).unwrap_err();
            let count = if retry { TRIES } else { 1 };
            let expected = answer(
                Resp {
                    status,
                    retry_after: None,
                    rate: Rate::default(),
                    body: String::new(),
                },
                "ticket ABC-12",
                count,
            )
            .unwrap_err();
            assert_eq!(error.to_string(), expected.to_string());
            assert_eq!(fixture.finish().len(), count);
        }
        // Asked to wait longer than a read waits: one try, and the wait said.
        let fixture = Fixture::new(vec![Reply::truncated(status).retry_after("999")]);
        let error = block(fixture.client().request(get(), "ticket ABC-12", true)).unwrap_err();
        assert_eq!(Busy::wait(&error), Some(999));
        assert!(error.to_string().ends_with("; it asks radar to wait 999s"), "{error}");
        assert_eq!(fixture.finish().len(), 1);
    }
    let fixture = Fixture::new(vec![
        Reply::truncated(429).retry_after("5"),
        Reply::truncated(503).retry_after("not-a-number"),
        Reply::complete(200, r#"{"count":7}"#),
    ]);
    assert_eq!(block(fixture.client().count(JQL)).unwrap(), 7);
    assert_eq!(fixture.finish().len(), TRIES);
}

#[test]
fn fully_collected_non_success_still_uses_jiras_words() {
    let body = r#"{"errorMessages":["readable refusal"]}"#;
    let fixture = Fixture::new(vec![Reply::complete(403, body)]);
    let error = block(fixture.client().request(get(), "ticket ABC-12", true)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "this account may not see the ticket ABC-12 (403): readable refusal"
    );
    assert_eq!(fixture.finish().len(), 1);
}

enum Begin {
    Read,
    Write(write::Change),
    Start(Box<start::Start>),
}

struct Probe {
    plugin: Arc<Mutex<JiraPlugin>>,
    client: Client,
}

impl Plugin for Probe {
    fn id(&self) -> &'static str {
        "jira"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn on_msg(&mut self, message: Msg) -> Vec<Effect> {
        let mut p = self.plugin.lock().unwrap();
        let begin = match message.downcast::<Begin>() {
            Ok(begin) => *begin,
            Err(message) => return p.on_msg(message),
        };
        if matches!(begin, Begin::Read) {
            let client = self.client.clone();
            p.sync.in_flight.insert(job("acme", JQL));
            return vec![sync_history::spawn(
                sync_history::action(&p.cfg.sites[0]),
                "acme".into(),
                JQL.into(),
                async move {
                    client
                        .tickets_observed(JQL, &|_| None, &[])
                        .await
                        .map(|list| sync_history::Synced { list, ..Default::default() })
                        .map_err(|error| error.to_string())
                },
            )];
        }
        let (change, effects) = match begin {
            Begin::Write(change) => (change.clone(), write::send(&mut p, change)),
            Begin::Start(plan) => {
                let change = plan.assign.clone().unwrap();
                p.starting = Some(*plan);
                (change, start::on_menu(&mut p, "j-start", "yes").unwrap())
            }
            Begin::Read => unreachable!(),
        };
        let dispatch = p.sent[&detail_key(change.site(), change.about())]
            .dispatch
            .clone()
            .unwrap();
        // Real dispatch allocates recovery and ownership. Drop its unpolled
        // production factory and run exactly that request on the private Http.
        effects
            .into_iter()
            .map(|effect| match effect {
                Effect::Spawn(_) => {
                    let client = self.client.clone();
                    let change = change.clone();
                    let dispatch = dispatch.clone();
                    Effect::Spawn(Box::pin(async move {
                        let result = client
                            .write(change.request().unwrap(), "ticket ABC-12")
                            .await
                            .map_err(|e| e.to_string());
                        Box::new(JiraMsg::Wrote {
                            dispatch,
                            change,
                            result,
                        }) as Msg
                    }))
                }
                effect => effect,
            })
            .collect()
    }
}

struct Run {
    host: Host,
    plugin: Arc<Mutex<JiraPlugin>>,
    records: Vec<JournalRecord>,
}

impl Run {
    fn new(client: Client) -> Self {
        let mut p = demo::sample();
        p.cfg.sites.truncate(1);
        p.cfg.views.truncate(1);
        p.cfg.views[0].jql = JQL.into();
        p.cfg.views[0].when.clear();
        p.data.get_mut("acme").unwrap().found = [(JQL.into(), vec!["ABC-12".into()])].into();
        p.sync.last_ok = Some(p.now - 30);
        p.sync
            .errors
            .insert("acme".into(), "previous failure".into());
        for key in p.data["acme"].tickets.keys() {
            p.sync.code_tried.insert(detail_key("acme", key), p.now);
        }
        let plugin = Arc::new(Mutex::new(p));
        let mut host = Host::new(
            vec![Box::new(Probe {
                plugin: plugin.clone(),
                client,
            })],
            demo::NOW,
        )
        .unwrap();
        host.enable_journal();
        host.set_env_mode(Some((writes::Mode::Edit, "RADAR_MODE".into())));
        Self {
            host,
            plugin,
            records: vec![],
        }
    }

    fn event(&mut self, event: Event) -> Vec<HostEffect> {
        let effects = self.host.update(event);
        self.records
            .extend(effects.iter().filter_map(|effect| match effect {
                HostEffect::Journal(record) => Some(record.clone()),
                _ => None,
            }));
        effects
    }

    fn launch(&mut self, begin: Begin) -> (ActionId, ActionId, Task) {
        let effects = self.event(Event::Msg {
            plugin: 0,
            msg: Box::new(begin),
        });
        let mut tasks = effects.into_iter().filter_map(|effect| match effect {
            HostEffect::Tracked {
                action,
                semantic: Some(semantic),
                effect,
            } => match *effect {
                HostEffect::Spawn { task, .. } => Some((action, semantic, task)),
                _ => None,
            },
            _ => None,
        });
        let task = tasks.next().expect("tracked HTTP task");
        assert!(tasks.next().is_none());
        task
    }

    fn complete(&mut self, action: ActionId, semantic: ActionId, task: Task) -> Vec<HostEffect> {
        self.event(Event::Tracked {
            action,
            semantic: Some(semantic),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: block(task),
            }),
        })
    }

    fn terminal(&self, target: &ActionId) -> Vec<Outcome> {
        self.records
            .iter()
            .filter_map(|record| match record {
                JournalRecord::Outcome { id, outcome, .. }
                    if id == target && outcome.is_terminal() =>
                {
                    Some(*outcome)
                }
                _ => None,
            })
            .collect()
    }

    fn safe_history(&self) {
        let history = format!("{:?}", self.records);
        assert!(!history.contains("CANARY"));
        assert!(!history.contains(ERROR));
        assert!(!history.contains("127.0.0.1"));
        let auth = base64::engine::general_purpose::STANDARD.encode(format!("{EMAIL}:{TOKEN}"));
        assert!(!history.contains(&auth));
    }
}

fn no_follow_up(effects: &[HostEffect]) {
    for effect in effects {
        match effect {
            HostEffect::Tracked { effect, .. } => no_follow_up(std::slice::from_ref(effect)),
            HostEffect::Spawn { .. } | HostEffect::Open(_) => {
                panic!("unexpected success follow-up")
            }
            _ => {}
        }
    }
}

#[test]
fn unreadable_search_records_failed_and_preserves_held_cache_and_last_success() {
    let fixture = Fixture::new(vec![Reply::truncated(200)]);
    let mut run = Run::new(fixture.client());
    let (held, last_ok) = {
        let p = run.plugin.lock().unwrap();
        (p.snapshot(), p.sync.last_ok)
    };
    let (action, semantic, task) = run.launch(Begin::Read);
    let effects = run.complete(action, semantic.clone(), task);
    assert_eq!(run.terminal(&semantic), [Outcome::Failed]);
    no_follow_up(&effects);
    let p = run.plugin.lock().unwrap();
    assert_eq!(p.snapshot(), held);
    assert_eq!(p.sync.last_ok, last_ok);
    assert_eq!(p.sync.errors["acme"], ERROR);
    assert!(p.sync.in_flight.is_empty());
    run.safe_history();
    assert_eq!(fixture.finish().len(), 1);
}

fn changes() -> Vec<write::Change> {
    vec![
        write::Change::Comment {
            site: "acme".into(),
            key: "ABC-12".into(),
            text: "  TEXT_CANARY\n  ".into(),
        },
        write::Change::Create {
            site: "acme".into(),
            project: "ABC".into(),
            kind: model::IssueType {
                id: "1".into(),
                name: "Task".into(),
                subtask: false,
            },
            summary: " SUMMARY_CANARY ".into(),
            description: " DESCRIPTION_CANARY\n  ".into(),
        },
        write::Change::Assign {
            site: "acme".into(),
            key: "ABC-12".into(),
            me: Me {
                account_id: "ACCOUNT_CANARY".into(),
                name: "PERSON_CANARY".into(),
            },
            from: String::new(),
        },
        write::Change::Move {
            site: "acme".into(),
            key: "ABC-12".into(),
            to: model::Transition {
                id: "21".into(),
                name: "Start".into(),
                to: "In Progress".into(),
                category: model::Category::Doing,
            },
            from: "To Do".into(),
            from_category: model::Category::Todo,
        },
    ]
}

fn unreadable_write(change: write::Change) {
    let fixture = Fixture::new(vec![Reply::truncated(201)]);
    let mut run = Run::new(fixture.client());
    let begin = if matches!(change, write::Change::Assign { .. }) {
        Begin::Start(Box::new(start::Start {
            site: "acme".into(),
            key: "ABC-12".into(),
            assign: Some(change.clone()),
            mv: Some(changes().pop().unwrap()),
        }))
    } else {
        Begin::Write(change.clone())
    };
    let (action, semantic, task) = run.launch(begin);
    let original = {
        let p = run.plugin.lock().unwrap();
        let dispatch = p.sent[&detail_key(change.site(), change.about())]
            .dispatch
            .as_ref()
            .unwrap();
        if let Some(record) = p.recovery.records.values().next() {
            assert_eq!(p.recovery.records.len(), 1);
            assert!(p.recovery.owns(&change, dispatch));
            Some(record.original.clone())
        } else {
            None
        }
    };
    let effects = run.complete(action, semantic.clone(), task);
    assert_eq!(run.terminal(&semantic), [Outcome::Unknown]);
    no_follow_up(&effects);
    let p = run.plugin.lock().unwrap();
    assert!(p.start_next.is_none());
    let result = p.sent[&detail_key(change.site(), change.about())]
        .result
        .as_ref()
        .unwrap();
    let notice = result.as_ref().unwrap_err();
    assert!(notice.contains("Jira did not confirm"));
    assert!(notice.contains(ERROR));
    assert!(!notice.contains(BODY));
    if let Some(original) = original {
        assert_eq!(p.recovery.records.len(), 1);
        let record = p.recovery.records.values().next().unwrap();
        assert_eq!(record.original, original);
        assert_eq!(record.status, crate::write_recovery::Status::Uncertain);
        assert!(notice.contains("original text kept"));
    } else {
        assert!(notice.contains("check Jira before trying again"));
    }
    run.safe_history();
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn unreadable_comment_records_unknown_and_keeps_original_text() {
    unreadable_write(changes().remove(0));
}

#[test]
fn unreadable_create_records_unknown_and_keeps_both_original_fields() {
    unreadable_write(changes().remove(1));
}

#[test]
fn unreadable_assignment_records_unknown_and_never_releases_queued_move() {
    unreadable_write(changes().remove(2));
}

#[test]
fn unreadable_move_records_unknown_with_check_before_retry_notice() {
    unreadable_write(changes().remove(3));
}

#[test]
fn jiras_rate_limit_headers_are_read_on_success_429_and_503() {
    for status in [200, 429, 503] {
        let fixture = Fixture::new(vec![
            Reply::complete(status, "{}")
                .retry_after("30")
                .header("X-RateLimit-Limit", "1000")
                .header("X-RateLimit-Remaining", "37")
                .header("X-RateLimit-Reset", "2026-10-13T12:00:00Z")
                .header("RateLimit-Reason", "jira-quota-tenant-based"),
        ]);
        let response = block(fixture.client().t.send(&get())).unwrap();
        assert_eq!(
            response.rate,
            Rate {
                limit: Some(1000),
                remaining: Some(37),
                reset: radar_core::time::parse_rfc3339("2026-10-13T12:00:00Z"),
                reason: Some("jira-quota-tenant-based".into()),
            },
            "{status}"
        );
        assert_eq!(response.retry_after, Some(30));
        fixture.finish();
    }
}

#[test]
fn an_answer_with_no_rate_limit_headers_says_nothing_about_them() {
    let fixture = Fixture::new(vec![Reply::complete(200, "{}")]);
    assert!(block(fixture.client().t.send(&get())).unwrap().rate.is_empty());
    fixture.finish();
}
