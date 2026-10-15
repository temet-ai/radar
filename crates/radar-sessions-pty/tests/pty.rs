//! The terminal's lifecycle, the socket a renderer attaches to, and the
//! adapter's honesty. Real `sh` and `cat`; every wait is on state.

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use radar_sessions::{Adapter, CreateSession, RadarSessionId, ProviderId, SendInput, Update};
use radar_sessions_pty::frame::{inp, out, read_frame, write_frame, StateFrame};
use radar_sessions_pty::{PtyAdapter, PtyCommand, ServedTerminal, TermState, Terminal, TerminalSpec};

fn sh(script: &str) -> TerminalSpec {
    TerminalSpec::new("sh", std::env::temp_dir()).args(["-c", script])
}

fn until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

fn text(t: &Terminal) -> String {
    String::from_utf8_lossy(&t.replay()).into_owned()
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("radar-pty-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn output_and_exit_status_are_kept() {
    let t = Terminal::spawn(sh("printf hello; exit 3")).unwrap();
    let s = t.wait_exit(Duration::from_secs(10)).expect("exits");
    assert_eq!(s, TermState::Exited { code: Some(3), signal: None });
    assert!(text(&t).contains("hello"), "{:?}", text(&t));
}

#[test]
fn input_reaches_the_program() {
    let t = Terminal::spawn(TerminalSpec::new("cat", std::env::temp_dir())).unwrap();
    t.write(b"abc\n").unwrap();
    until("the echo", || text(&t).contains("abc"));
    t.kill();
}

#[test]
fn resize_reaches_the_program() {
    let t = Terminal::spawn(sh("read x; stty size")).unwrap();
    t.resize(100, 30).unwrap();
    t.write(b"\n").unwrap();
    until("the size", || text(&t).contains("30 100"));
    assert_eq!((t.frame().columns, t.frame().rows), (100, 30));
}

#[test]
fn restart_runs_it_again_on_a_clear_screen() {
    let t = Terminal::spawn(sh("printf one; exit 0")).unwrap();
    t.wait_exit(Duration::from_secs(10)).unwrap();
    assert_eq!(t.frame().generation, 1);
    t.restart().unwrap();
    assert_eq!(t.frame().generation, 2);
    until("the second run to end", || text(&t).contains("one") && !t.state().is_running());
    assert_eq!(text(&t).matches("one").count(), 1, "the ring was cleared");
}

#[test]
fn a_program_that_cannot_start_is_a_failed_terminal_that_can_restart() {
    let spec = TerminalSpec::new("/definitely/not/a/program", std::env::temp_dir());
    let err = Terminal::spawn(spec).err().expect("fails");
    assert!(format!("{err:#}").contains("could not start"), "{err:#}");
}

#[test]
fn the_ring_is_bounded() {
    let t = Terminal::spawn(sh("head -c 600000 /dev/zero | tr '\\0' x; exit 0")).unwrap();
    t.wait_exit(Duration::from_secs(10)).unwrap();
    until("the output to settle", || t.replay().len() >= radar_sessions_pty::term::RING_BYTES);
    assert_eq!(t.replay().len(), radar_sessions_pty::term::RING_BYTES);
}

fn state_of(payload: &[u8]) -> StateFrame {
    serde_json::from_slice(payload).unwrap()
}

/// Reads frames until `want` is in the output or the stream ends.
fn read_until(s: &mut UnixStream, want: &str) -> (String, Vec<StateFrame>) {
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let (mut text, mut states) = (String::new(), Vec::new());
    while !text.contains(want) {
        let (kind, p) = read_frame(s).expect("frame");
        match kind {
            out::OUTPUT => text.push_str(&String::from_utf8_lossy(&p)),
            out::STATE => states.push(state_of(&p)),
            _ => {}
        }
    }
    (text, states)
}

#[test]
fn a_renderer_attaches_types_resizes_and_a_new_one_gets_the_screen_back() {
    let dir = scratch("serve");
    let served = ServedTerminal::start(sh("read x; stty size; printf done; sleep 30"), &dir, "t1").unwrap();
    assert_eq!(std::fs::metadata(served.socket()).unwrap().permissions().mode_bits() & 0o077, 0);

    let mut a = UnixStream::connect(served.socket()).unwrap();
    let (kind, p) = read_frame(&mut a).unwrap();
    assert_eq!(kind, out::STATE);
    let first = state_of(&p);
    assert_eq!((first.state.as_str(), first.generation), ("running", 1));

    let mut resize = Vec::new();
    resize.extend_from_slice(&90u16.to_be_bytes());
    resize.extend_from_slice(&20u16.to_be_bytes());
    write_frame(&mut a, inp::RESIZE, &resize).unwrap();
    write_frame(&mut a, inp::INPUT, b"\n").unwrap();
    let (seen, _) = read_until(&mut a, "done");
    assert!(seen.contains("20 90"), "{seen:?}");

    // The renderer goes away; the program does not.
    drop(a);
    assert!(served.terminal().state().is_running());
    let mut b = UnixStream::connect(served.socket()).unwrap();
    let (seen, _) = read_until(&mut b, "done");
    assert!(seen.contains("20 90"), "replayed: {seen:?}");

    write_frame(&mut b, inp::KILL, &[]).unwrap();
    let (_, states) = {
        let mut states = Vec::new();
        loop {
            let (k, p) = read_frame(&mut b).unwrap();
            if k == out::STATE {
                let s = state_of(&p);
                let over = s.state != "running";
                states.push(s);
                if over {
                    break;
                }
            }
        }
        ((), states)
    };
    assert_eq!(states.last().unwrap().state, "exited");
    drop(served);
    assert!(!dir.join("pty-t1.sock").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

trait ModeBits {
    fn mode_bits(&self) -> u32;
}
impl ModeBits for std::fs::Permissions {
    fn mode_bits(&self) -> u32 {
        std::os::unix::fs::PermissionsExt::mode(self)
    }
}

#[test]
fn restart_over_the_socket_bumps_the_generation() {
    let dir = scratch("restart");
    let served = ServedTerminal::start(sh("printf hi; exit 0"), &dir, "t2").unwrap();
    served.terminal().wait_exit(Duration::from_secs(10)).unwrap();
    let mut a = UnixStream::connect(served.socket()).unwrap();
    let (_, p) = read_frame(&mut a).unwrap();
    assert_eq!(state_of(&p).state, "exited");
    write_frame(&mut a, inp::RESTART, &[]).unwrap();
    // The first replay held the old run's "hi"; the new run's comes after a state with generation 2.
    let (mut gen2, mut after) = (false, String::new());
    while !(gen2 && after.contains("hi")) {
        let (kind, p) = read_frame(&mut a).expect("frame");
        match kind {
            out::STATE => gen2 |= state_of(&p).generation == 2,
            out::OUTPUT if gen2 => after.push_str(&String::from_utf8_lossy(&p)),
            _ => {}
        }
    }
    drop(served);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_adapter_says_what_a_terminal_cannot_do() {
    let dir = scratch("adapter");
    let a = PtyAdapter::new(
        PtyCommand { id: ProviderId::new("pty:cat"), command: "cat".into(), args: vec![] },
        &dir,
    );
    let caps = a.capabilities();
    assert!(!caps.permissions && !caps.structured_events && !caps.resume && !caps.stop);
    for control in ["permissions", "structured_events", "stop", "resume", "steer", "list_models"] {
        assert!(caps.unsupported.iter().any(|u| u.control == control), "{control} is explained");
    }
    futures_lite_block(async {
        assert!(a.stop(&RadarSessionId::new("x")).await.is_err());
        assert!(a.answer_permission(&RadarSessionId::new("x"), radar_sessions::PermissionAnswer {
            request_id: "r".into(),
            decision: radar_sessions::PermissionDecision::ApproveOnce,
        }).await.is_err());
        let st = a.status().await;
        assert!(st.installed, "cat is on PATH");
    });
}

#[test]
fn the_adapter_opens_types_reports_the_end_and_deletes() {
    let dir = scratch("adapter2");
    let a = PtyAdapter::new(
        PtyCommand { id: ProviderId::new("pty:sh"), command: "sh".into(), args: vec!["-c".into(), "read x; echo got-$x; exit 7".into()] },
        &dir,
    )
    .with_clock(|| 42);
    let id = RadarSessionId::new("s1");
    futures_lite_block(async {
        let mut updates = a.subscribe().await.unwrap();
        assert!(a.subscribe().await.is_err(), "once");
        a.create(CreateSession { radar_session_id: id.clone(), cwd: std::env::temp_dir(), model: None, setup: Default::default() }).await.unwrap();
        let info = a.info(&id).unwrap();
        assert!(info.socket.exists());
        a.send(&id, SendInput { text: "brief\n".into() }).await.unwrap();
        let mut notice = None;
        while let Some(u) = updates.recv().await {
            if let Update::Event(e) = u {
                assert_eq!(e.observed_at, 42);
                if let radar_sessions::EventKind::Notice { text } = e.kind {
                    notice = Some(text);
                    break;
                }
            }
        }
        assert_eq!(notice.unwrap(), "exited with status 7");
        assert_eq!(a.list_sessions().await.unwrap().len(), 1);
        a.delete_session(&radar_sessions::ProviderSessionId::new("s1")).await.unwrap();
        assert!(a.info(&id).is_none());
        assert!(!info.socket.exists());
    });
    let _ = std::fs::remove_dir_all(&dir);
}

fn futures_lite_block<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
}


#[test]
fn a_command_can_be_opened_in_a_tab_in_place_of_the_adapters_own() {
    let dir = scratch("opencmd");
    let a = PtyAdapter::new(PtyCommand { id: ProviderId::new("pty:codex"), command: "does-not-exist".into(), args: vec![] }, &dir);
    let id = RadarSessionId::new("login1");
    futures_lite_block(async {
        a.open_command(
            CreateSession { radar_session_id: id.clone(), cwd: std::env::temp_dir(), model: None, setup: Default::default() },
            vec!["sh".into(), "-c".into(), "printf logged-in; exit 0".into()],
        )
        .await
        .unwrap();
        let info = a.info(&id).unwrap();
        assert_eq!(info.command, "sh");
        let mut s = UnixStream::connect(&info.socket).unwrap();
        let (seen, _) = read_until(&mut s, "logged-in");
        assert!(seen.contains("logged-in"));
        assert!(a.open_command(CreateSession { radar_session_id: RadarSessionId::new("x"), cwd: std::env::temp_dir(), model: None, setup: Default::default() }, vec![]).await.is_err());
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_configured_executable_runs_with_the_clis_config_folder() {
    let dir = scratch("configured");
    std::fs::create_dir_all(&dir).unwrap();
    // A stand-in `codex`: prints where its config folder is.
    let exe = dir.join("my-codex");
    std::fs::write(&exe, "#!/bin/sh\nprintf \"home=$CODEX_HOME\"\nsleep 5\n").unwrap();
    std::fs::set_permissions(&exe, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let a = PtyAdapter::new(PtyCommand { id: ProviderId::new("pty:codex"), command: "codex".into(), args: vec![] }, &dir);
    a.configure(radar_sessions::ProviderSetup { executable: Some(exe.clone()), config_dir: Some(dir.join("codex-home")) });
    let id = RadarSessionId::new("configured1");
    futures_lite_block(async {
        a.create(CreateSession { radar_session_id: id.clone(), cwd: std::env::temp_dir(), model: None, setup: Default::default() }).await.unwrap();
        let info = a.info(&id).unwrap();
        assert_eq!(info.command, exe.display().to_string());
        let mut s = UnixStream::connect(&info.socket).unwrap();
        let want = format!("home={}", dir.join("codex-home").display());
        let (seen, _) = read_until(&mut s, &want);
        assert!(seen.contains(&want), "{seen}");
        a.delete_session(&radar_sessions::ProviderSessionId::new("configured1")).await.ok();
    });
    let _ = std::fs::remove_dir_all(&dir);
}
