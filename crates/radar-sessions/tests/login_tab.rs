//! Codex and Claude Code sign in by their own login command, run in a
//! terminal tab (the PTY adapter's `open_command`); when the tab's program
//! ends, the status is read again. Nothing here reads a token.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use radar_core::view::{Intent, Target};
use radar_core::{Pane, Plugin};
use radar_sessions::SessionsPlugin;
use radar_sessions::adapter::*;
use radar_sessions::event::{RadarSessionId, PermissionAnswer, ProviderId, ProviderSessionId};
use radar_sessions::fake::FakeAdapter;

/// The fake provider under another id; `signed_out` sets its status, and a
/// terminal adapter also records the commands it was asked to open.
struct Wrap {
    id: &'static str,
    inner: FakeAdapter,
    signed_out: bool,
    opened: Arc<Mutex<Vec<Vec<String>>>>,
    exited: Arc<AtomicBool>,
    status_reads: Arc<AtomicUsize>,
}

impl Adapter for Wrap {
    fn id(&self) -> ProviderId {
        ProviderId::new(self.id)
    }
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn status(&self) -> BoxFuture<'_, AdapterStatus> {
        self.status_reads.fetch_add(1, Ordering::SeqCst);
        let mut status = self.inner.status();
        let signed_out = self.signed_out && !self.exited.load(Ordering::SeqCst);
        Box::pin(async move {
            let mut s = status.as_mut().await;
            if signed_out {
                s.auth = AuthStatus::SignedOut;
            }
            s
        })
    }
    fn list_models(&self) -> BoxFuture<'_, anyhow::Result<Vec<ModelInfo>>> {
        self.inner.list_models()
    }
    fn subscribe(&self) -> BoxFuture<'_, anyhow::Result<Updates>> {
        self.inner.subscribe()
    }
    fn create(&self, req: CreateSession) -> BoxFuture<'_, anyhow::Result<ProviderSessionId>> {
        self.inner.create(req)
    }
    fn send(&self, session: &RadarSessionId, input: SendInput) -> BoxFuture<'_, anyhow::Result<()>> {
        self.inner.send(session, input)
    }
    fn stop(&self, session: &RadarSessionId) -> BoxFuture<'_, anyhow::Result<()>> {
        self.inner.stop(session)
    }
    fn resume(&self, req: ResumeSession) -> BoxFuture<'_, anyhow::Result<()>> {
        self.inner.resume(req)
    }
    fn list_sessions(&self) -> BoxFuture<'_, anyhow::Result<Vec<ProviderSessionInfo>>> {
        self.inner.list_sessions()
    }
    fn delete_session(&self, session: &ProviderSessionId) -> BoxFuture<'_, anyhow::Result<()>> {
        self.inner.delete_session(session)
    }
    fn terminal(&self, _session: &RadarSessionId) -> Option<TerminalSurface> {
        if !self.id.starts_with("pty:") {
            return None;
        }
        let state = if self.exited.load(Ordering::SeqCst) { TerminalStatus::Exited(0) } else { TerminalStatus::Running };
        Some(TerminalSurface { socket: PathBuf::new(), command: "login".into(), cwd: ".".into(), columns: 80, rows: 24, state })
    }
    fn open_command(&self, _req: CreateSession, argv: Vec<String>) -> BoxFuture<'_, anyhow::Result<ProviderSessionId>> {
        self.opened.lock().unwrap().push(argv);
        Box::pin(async { Ok(ProviderSessionId::new("tab-1")) })
    }
    fn answer_permission(&self, session: &RadarSessionId, answer: PermissionAnswer) -> BoxFuture<'_, anyhow::Result<()>> {
        self.inner.answer_permission(session, answer)
    }
}

fn tick_until(p: &mut SessionsPlugin, what: &str, done: impl Fn() -> bool) {
    // The plugin's tasks run on this thread, only here: a bound on steps, not on time.
    for n in 0..256i64 {
        p.tick(n);
        if done() {
            return;
        }
        p.run_tasks();
    }
    panic!("never reached: {what}");
}

#[test]
fn signing_in_to_codex_runs_its_login_command_in_a_tab_then_reads_the_status_again() {
    let opened = Arc::new(Mutex::new(Vec::new()));
    let exited = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));
    let wrap = |id, signed_out| Wrap {
        id,
        inner: FakeAdapter::new(),
        signed_out,
        opened: opened.clone(),
        exited: exited.clone(),
        status_reads: reads.clone(),
    };
    let mut p = SessionsPlugin::on_this_thread();
    p.add_adapter(Box::new(wrap("codex", true)));
    p.add_adapter(Box::new(wrap("pty:codex", false)));
    tick_until(&mut p, "the first status read", || reads.load(Ordering::SeqCst) > 0);

    p.intent("agents", Pane::Detail, &Intent::Select { target: Target::new("cli:codex") });
    // The status arrives on a later tick; Activate does nothing until the row reads "signed out".
    for n in 0..256i64 {
        p.tick(n);
        p.intent("agents", Pane::Detail, &Intent::Activate { target: Target::new("cli:codex") });
        if !opened.lock().unwrap().is_empty() {
            break;
        }
        p.run_tasks();
    }
    tick_until(&mut p, "the login command in a tab", || !opened.lock().unwrap().is_empty());
    let argv = opened.lock().unwrap()[0].clone();
    assert_eq!(argv.first().map(String::as_str), Some("codex"));
    assert!(argv.iter().any(|a| a == "login"), "codex's own login command: {argv:?}");

    let before = reads.load(Ordering::SeqCst);
    exited.store(true, Ordering::SeqCst);
    tick_until(&mut p, "the status read again after the tab ended", || reads.load(Ordering::SeqCst) > before);
}
