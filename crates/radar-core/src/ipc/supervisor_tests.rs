//! The supervisor against a fake renderer: a real child process (this
//! test binary, run as the test `fake_renderer`) that speaks the protocol
//! over the socket and behaves as `RADAR_FAKE_RENDERER` says.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::unbounded_channel;

use super::fixture::{self, ASK};
use super::supervisor::{Ended, RESTORE, RestartPolicy, Supervisor, find_bundle, find_renderer, socket_path};
use super::*;
use crate::host::Event;
use crate::view::Intent;

/// Not a test of its own: the fake renderer, when a supervisor test
/// starts this binary with `RADAR_FAKE_RENDERER` set.
#[test]
fn fake_renderer() {
    let Ok(mode) = std::env::var("RADAR_FAKE_RENDERER") else {
        return;
    };
    let socket = std::env::var("RADAR_IPC_SOCKET").expect("the host names its socket");
    let mut out = UnixStream::connect(socket).unwrap();
    let mut read = BufReader::new(out.try_clone().unwrap());
    let mut next = || -> ToRenderer {
        let mut line = String::new();
        read.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line:?}"))
    };
    let mut send = |m: ToHost| out.write_all(&encode(&m)).unwrap();
    // The run counter: a file the test names, one line per start.
    let run = std::env::var("RADAR_FAKE_COUNT")
        .map(|path| {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .unwrap();
            f.write_all(b"run\n").unwrap();
            std::fs::read_to_string(&path).unwrap().lines().count()
        })
        .unwrap_or(1);
    let protocol = if mode == "old" { PROTOCOL + 1 } else { PROTOCOL };
    let view = if mode == "oldview" {
        crate::view::VERSION + 1
    } else {
        crate::view::VERSION
    };
    send(ToHost::Hello {
        protocol,
        view,
        capabilities: Capabilities {
            columns: 80,
            rows: 24,
            mouse: true,
            true_colour: false,
            image: ImageProtocol::None,
            plain_text: false,
        },
    });
    if mode == "stuck-once" && run == 1 {
        // Reads nothing more: the host's queue to it fills.
        std::thread::sleep(Duration::from_secs(30));
        std::process::exit(1);
    }
    if mode == "slow" {
        // Reads every line, acknowledges each snapshot late, and writes
        // down what it saw; leaves after the notice "last".
        let seen = std::env::var("RADAR_FAKE_SEEN").unwrap();
        let mut seen = std::fs::File::create(seen).unwrap();
        loop {
            match next() {
                ToRenderer::Snapshot(s) => {
                    writeln!(seen, "snapshot {}", s.cursor).unwrap();
                    std::thread::sleep(Duration::from_millis(150));
                    send(ToHost::Ack { cursor: s.cursor });
                }
                ToRenderer::Question(q) => writeln!(seen, "question {}", q.menu.title).unwrap(),
                ToRenderer::Notice(n) => {
                    writeln!(seen, "notice {}", n.text).unwrap();
                    if n.text == "last" {
                        send(ToHost::Bye);
                    }
                }
                ToRenderer::Shutdown { .. } => std::process::exit(0),
                _ => {}
            }
        }
    }
    if mode == "old" || mode == "oldview" {
        match next() {
            ToRenderer::Refuse { message, .. } => {
                eprintln!("{message}");
                std::process::exit(3);
            }
            other => panic!("{other:?}"),
        }
    }
    assert!(matches!(next(), ToRenderer::Welcome { .. }));
    // A question still open from before a restart may come first.
    let s = loop {
        match next() {
            ToRenderer::Snapshot(s) => break s,
            ToRenderer::Look(_) | ToRenderer::Question(_) | ToRenderer::Notice(_) => {}
            other => panic!("{other:?}"),
        }
    };
    send(ToHost::Ack { cursor: s.cursor });
    let crash = mode == "crash" || (mode == "crash-once" && run == 1);
    if crash {
        // Mid-run: the user acted, then the renderer died before the
        // answer.
        send(ToHost::Intent {
            seq: 1,
            cursor: s.cursor,
            intent: Intent::Invoke {
                action: ASK.into(),
            },
        });
        std::thread::sleep(Duration::from_millis(50));
        std::process::exit(1);
    }
    send(ToHost::Bye);
    loop {
        if let ToRenderer::Shutdown { .. } = next() {
            std::process::exit(0);
        }
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("radar-ipc-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn supervisor(mode: &'static str, dir: &Path) -> Supervisor<Vec<u8>> {
    let count = dir.join("runs");
    let seen = dir.join("seen");
    Supervisor::new(
        Box::new(move || {
            let mut c = tokio::process::Command::new(std::env::current_exe().unwrap());
            c.args([
                "--exact",
                "ipc::supervisor_tests::fake_renderer",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("RADAR_FAKE_RENDERER", mode)
            .env("RADAR_FAKE_COUNT", &count)
            .env("RADAR_FAKE_SEEN", &seen)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
            c
        }),
        dir.join("ink.sock"),
        Vec::new(),
    )
}

async fn run(sup: &mut Supervisor<Vec<u8>>, bridge: &mut Bridge) -> Ended {
    run_with(sup, bridge, vec![]).await
}

/// Run with these host events waiting (they are handled once a renderer
/// is connected, as fast as the host can).
async fn run_with(sup: &mut Supervisor<Vec<u8>>, bridge: &mut Bridge, events: Vec<Event>) -> Ended {
    let (tx, mut rx) = unbounded_channel::<Event>();
    for e in events {
        tx.send(e).unwrap();
    }
    let mut fx = Vec::new();
    sup.run(bridge, &mut rx, &mut |e| fx.push(e), &mut |e, _| Some(e))
        .await
        .unwrap()
}

fn restores(sup: &Supervisor<Vec<u8>>) -> usize {
    String::from_utf8_lossy(&sup.terminal).matches(RESTORE).count()
}

#[tokio::test]
async fn a_renderer_that_says_goodbye_ends_the_run_with_the_terminal_put_back() {
    let dir = scratch("bye");
    let mut sup = supervisor("bye", &dir);
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    assert_eq!(run(&mut sup, &mut bridge).await, Ended::Quit);
    assert_eq!(sup.started.len(), 1);
    assert_eq!(restores(&sup), 1);
}

#[tokio::test]
async fn a_renderer_of_another_protocol_is_refused_and_ratatui_takes_over() {
    let dir = scratch("old");
    let mut sup = supervisor("old", &dir);
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    let Ended::FallBack(why) = run(&mut sup, &mut bridge).await else {
        panic!()
    };
    assert!(why.contains("protocol") && why.contains("RADAR_RENDERER=ratatui"), "{why}");
    assert_eq!(sup.started.len(), 1, "a refused renderer is not restarted");
    assert_eq!(restores(&sup), 1);
    // Visible: in the status line the Ratatui renderer opens with, and on
    // stderr.
    assert_eq!(bridge.host().message.as_ref().map(|m| m.text.as_str()), Some(why.as_str()));
    let mut err = Vec::new();
    super::supervisor::say_fall_back(&mut err, &why);
    assert_eq!(String::from_utf8(err).unwrap(), format!("radar: {why}\n"));
}

#[tokio::test]
async fn a_renderer_reading_other_views_is_refused_too() {
    let dir = scratch("oldview");
    let mut sup = supervisor("oldview", &dir);
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    let Ended::FallBack(why) = run(&mut sup, &mut bridge).await else {
        panic!()
    };
    assert!(why.contains("views of version") && why.contains("RADAR_RENDERER=ratatui"), "{why}");
    assert_eq!(sup.started.len(), 1);
    assert_eq!(bridge.host().message.as_ref().map(|m| m.text.as_str()), Some(why.as_str()));
}

/// C2(c) over the socket: a renderer that acknowledges each snapshot
/// 150 ms late, while the host changes 20 times as fast as it can, sees
/// every notice in order and the question, and fewer snapshots than
/// changes (progress coalesced).
#[tokio::test]
async fn a_slow_renderer_process_loses_no_question_or_notice() {
    let dir = scratch("slow");
    let mut sup = supervisor("slow", &dir);
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    let mut events = Vec::new();
    for n in 0..20 {
        events.push(Event::Notice(format!("step {n}")));
        events.push(Event::Tick(n));
        if n == 10 {
            events.push(Event::Intent(Intent::Invoke {
                action: ASK.into(),
            }));
        }
    }
    events.push(Event::Notice("last".into()));
    assert_eq!(run_with(&mut sup, &mut bridge, events).await, Ended::Quit);
    let seen = std::fs::read_to_string(dir.join("seen")).unwrap();
    let notices: Vec<_> = seen.lines().filter_map(|l| l.strip_prefix("notice ")).collect();
    let mut want: Vec<String> = (0..20).map(|n| format!("step {n}")).collect();
    want.push("last".into());
    assert_eq!(notices, want, "{seen}");
    let questions: Vec<_> = seen.lines().filter(|l| l.starts_with("question ")).collect();
    assert_eq!(questions, ["question save it?"], "{seen}");
    let snapshots = seen.lines().filter(|l| l.starts_with("snapshot ")).count();
    assert!((1..20).contains(&snapshots), "coalesced: {snapshots} snapshots\n{seen}");
}

/// A renderer that stops reading fills the host's queue to it: it is
/// ended as crashed, once its queue has stayed full for `stalled_after`,
/// and started again. The renderer started again reads every line of the
/// same flood, so it is not ended: a queue that is full only until the
/// writer's next turn is not a renderer that stopped reading.
#[tokio::test]
async fn a_renderer_that_stops_reading_is_ended_and_started_again() {
    let dir = scratch("stuck");
    let mut sup = supervisor("stuck-once", &dir);
    sup.stalled_after = Duration::from_millis(300);
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    let long = "x".repeat(MAX_NOTICE);
    let events = (0..2000)
        .map(|n| Event::Notice(format!("{n} {long}")))
        .collect();
    assert_eq!(run_with(&mut sup, &mut bridge, events).await, Ended::Quit);
    // Why first: the one renderer ended was ended for its full queue.
    assert_eq!(sup.stopped.len(), 1, "{:?}", sup.stopped);
    assert!(sup.stopped[0].contains("stopped reading"), "{:?}", sup.stopped);
    assert_eq!(sup.started.len(), 2, "ended once and started again");
    assert_eq!(restores(&sup), 2);
}

/// C2(c): the renderer dies mid-run: the terminal is put back, it starts
/// again, and the host's state (the question it asked) and journal are
/// as they were.
#[tokio::test]
async fn a_renderer_that_crashes_once_is_restarted_and_the_host_keeps_its_state() {
    let dir = scratch("crash-once");
    let mut sup = supervisor("crash-once", &dir);
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    assert_eq!(run(&mut sup, &mut bridge).await, Ended::Quit);
    assert_eq!(sup.started.len(), 2, "started again after the crash");
    assert_eq!(restores(&sup), 2, "the terminal put back after each");
    assert_eq!(
        bridge.host().menu_view().map(|m| m.title),
        Some("save it?".into()),
        "the question asked before the crash is still open"
    );
    // The journal holds what the intent recorded and nothing more: the
    // crash, the restart and the goodbye add no action and no outcome.
    let recorded = fixture::shape(bridge.records());
    let mut headless = crate::headless::Headless::new(fixture::host(), 80, 24);
    headless.send(Intent::Invoke {
        action: ASK.into(),
    });
    assert_eq!(recorded, fixture::shape(headless.records()));
    assert!(recorded.iter().any(|l| l.contains(ASK)), "{recorded:#?}");
}

#[tokio::test]
async fn a_renderer_that_keeps_crashing_falls_back_to_ratatui_after_three_restarts() {
    let dir = scratch("crash");
    let mut sup = supervisor("crash", &dir);
    let mut bridge = Bridge::new(fixture::host(), 80, 24);
    let Ended::FallBack(why) = run(&mut sup, &mut bridge).await else {
        panic!()
    };
    assert!(why.contains("Ratatui"), "{why}");
    assert_eq!(sup.started.len(), 4, "the first start and three restarts");
    assert_eq!(restores(&sup), 4);
    // The host is whole for the Ratatui renderer.
    let host = bridge.into_host();
    assert!(host.menu_view().is_some());
}

#[test]
fn restarts_are_counted_in_a_moving_minute() {
    let mut policy = RestartPolicy::default();
    let t = Instant::now();
    assert!(policy.restart(t));
    assert!(policy.restart(t + Duration::from_secs(1)));
    assert!(policy.restart(t + Duration::from_secs(2)));
    assert!(!policy.restart(t + Duration::from_secs(3)), "a fourth in the minute");
    assert!(policy.restart(t + Duration::from_secs(61)), "the first has left the window");
}

#[test]
fn the_bundle_is_found_by_variable_then_install_then_checkout() {
    let dir = scratch("bundle");
    let named = dir.join("named.js");
    std::fs::write(&named, "").unwrap();
    let var = |path: Option<PathBuf>| move |k: &str| (k == "RADAR_INK_BUNDLE").then(|| path.clone()).flatten().map(|p| p.display().to_string());
    assert_eq!(find_bundle(&var(Some(named.clone())), None), Some(named.clone()));
    // A named bundle that is not there is not looked past.
    assert_eq!(find_bundle(&var(Some(dir.join("gone.js"))), None), None);

    let bin = dir.join("prefix/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join("radar");
    assert_eq!(find_bundle(&var(None), Some(&exe)), None);
    let installed = dir.join("prefix/share/radar/ink");
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::write(installed.join("radar-ink.js"), "").unwrap();
    assert!(find_bundle(&var(None), Some(&exe)).is_some_and(|p| p.ends_with("share/radar/ink/radar-ink.js")));

    let checkout = dir.join("checkout");
    let built = checkout.join("target/fleet/debug");
    std::fs::create_dir_all(&built).unwrap();
    std::fs::create_dir_all(checkout.join("ink/dist")).unwrap();
    std::fs::write(checkout.join("Cargo.toml"), "").unwrap();
    std::fs::write(checkout.join("ink/dist/radar-ink.js"), "").unwrap();
    assert_eq!(
        find_bundle(&var(None), Some(&built.join("radar"))),
        Some(checkout.join("ink/dist/radar-ink.js"))
    );
}

#[test]
fn the_runtime_starts_the_renderer_unless_a_bundle_is_named() {
    let dir = scratch("runtime");
    let runtime = dir.join("radar-js");
    let bundle = dir.join("named.js");
    std::fs::write(&runtime, "").unwrap();
    std::fs::write(&bundle, "").unwrap();
    std::fs::write(dir.join("node"), "").unwrap();
    let env = |named: bool| {
        let (runtime, bundle, path) = (runtime.display().to_string(), bundle.display().to_string(), dir.display().to_string());
        move |k: &str| match k {
            "RADAR_RUNTIME" => Some(runtime.clone()),
            "RADAR_INK_BUNDLE" if named => Some(bundle.clone()),
            "PATH" => Some(path.clone()),
            _ => None,
        }
    };
    assert_eq!(find_renderer(&env(false), None), Ok((runtime.clone(), vec!["ink".into()])));
    assert_eq!(find_renderer(&env(true), None), Ok((dir.join("node"), vec![bundle.clone().into()])));
    // No runtime, no bundle: it says so.
    assert!(find_renderer(&|_: &str| None, None).is_err());
}

#[test]
fn the_socket_lives_in_the_state_folder_unless_its_path_is_too_long() {
    assert_eq!(
        socket_path(Some("/state".into()), 7),
        PathBuf::from("/state/radar/ink-7.sock")
    );
    let long = PathBuf::from(format!("/{}", "x".repeat(120)));
    assert!(socket_path(Some(long), 7).starts_with(std::env::temp_dir()));
}
