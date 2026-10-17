//! Bounded, renderer-free scenarios using the real host and task completion path.
//!
//! This is a test harness, not an I/O sandbox: injected plugins must use explicit
//! fake transports. Spawned futures can execute arbitrary Rust. Desktop effects
//! are captured only and config/cache writes stay in memory. History and work
//! requests use the production writer only with an explicit scratch journal;
//! otherwise they fail closed. Credential scans always fail closed. No clock or
//! environment is read to advance the host; callers dispatch ticks and release
//! fake replies explicitly.
//! Dispatch and `inspect` scope the host's thread-local write permissions. Spawned
//! futures must capture any needed permissions before spawning, not consult ambient
//! thread-local mode: Tokio may poll them later or on another thread.

use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use tokio::task::JoinSet;

use crate::host::{Event, Host, HostEffect};
use crate::journal::{ActionId, Metadata, Outcome, Reason, SessionOrigin};
use crate::journal_runtime::Recording;
use crate::task_completion::{
    Completion, complete_history, complete_task, complete_work, tracked_event,
};

pub use crate::host::JournalRecord;

/// Captured requests, never invoked and never reported as successful.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopEffect {
    Open(String),
    OpenApp { link: String, web: String },
    Copy(String),
    CopySelection(String),
}

/// A copied observation of this driver's work, not a purity or I/O guarantee.
/// No global state or count of hosts constructed elsewhere is consulted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScenarioCosts {
    /// Events passed to `Host::update` by this driver, including completions.
    /// Calls rejected because the scenario finished or failed do not count.
    pub dispatched_events: u64,
    /// Top-level entries returned by those updates, including Journal and
    /// Tracked envelopes. A Tracked effect's inner effect is not another entry.
    /// All returned entries count even if performing an earlier one fails.
    pub returned_effect_envelopes: u64,
    /// Cumulative completion tasks started by this driver for Spawn, Work and
    /// History. Settling does not reset it. This excludes child tasks inside
    /// futures, detached plugin jobs and the journal writer's own work.
    pub spawned_completion_tasks: u64,
    /// Lifetime calls to `shell::render` on this owned host, including calls
    /// before the host entered this driver and through raw shell helpers.
    /// Neutral views, part-only buffers, Ink and other hosts are not counted.
    pub shell_frames: u64,
}

/// Restore thread-local permissions even if inspection or reduction panics.
/// This guard is used only in synchronous scopes, never across an await.
struct ModeScope((crate::writes::Mode, Option<String>));

impl ModeScope {
    fn enter(host: &Host) -> Self {
        let previous = crate::writes::mode();
        host.apply_mode();
        Self(previous)
    }
}

impl Drop for ModeScope {
    fn drop(&mut self) {
        crate::writes::set_mode(self.0.0, self.0.1.take());
    }
}

/// One isolated host, with no production runner or global storage paths.
/// Dropping it aborts owned completion futures. A work mutation already queued
/// to the real writer can still commit while the scratch recording is drained;
/// cancellation does not roll back queued or durable journal operations.
pub struct Scenario {
    host: Host,
    tasks: JoinSet<Completion>,
    pending: VecDeque<Event>,
    records: Vec<JournalRecord>,
    desktop: Vec<DesktopEffect>,
    configs: Vec<String>,
    caches: Vec<(&'static str, serde_json::Value)>,
    recording: Option<Recording>,
    finished: bool,
    failure: Option<String>,
    dispatched_events: u64,
    returned_effect_envelopes: u64,
    spawned_completion_tasks: u64,
}

impl Scenario {
    pub fn new(mut host: Host) -> Self {
        host.enable_scenario_journal();
        Self {
            host,
            tasks: JoinSet::new(),
            pending: VecDeque::new(),
            records: Vec::new(),
            desktop: Vec::new(),
            configs: Vec::new(),
            caches: Vec::new(),
            recording: None,
            finished: false,
            failure: None,
            dispatched_events: 0,
            returned_effect_envelopes: 0,
            spawned_completion_tasks: 0,
        }
    }

    /// Use the production SQLite writer at a fresh, caller-owned scratch path.
    /// Session IDs and storage timestamps come from the production writer; only
    /// host action IDs and explicitly dispatched host time are deterministic.
    /// The caller must provide an existing dedicated scratch directory, keep
    /// exclusive control of it, and clean it up. The database path is atomically
    /// reserved with create-new semantics (0600 on Unix); existing files and
    /// symlinks are refused. This prevents concurrent precreation, not malicious
    /// replacement inside a directory another process controls. Failed setup may
    /// leave the reserved file for caller cleanup. No default path, credentials,
    /// plugin cache directory or config directory is used.
    pub fn with_journal(host: Host, path: &Path) -> Result<Self> {
        ensure!(path.is_absolute(), "scenario journal path must be absolute");
        let parent = path
            .parent()
            .context("scenario journal path needs a directory")?;
        let directory = std::fs::symlink_metadata(parent)
            .context("scenario journal requires an existing caller-owned scratch directory")?;
        ensure!(
            directory.is_dir() && !directory.file_type().is_symlink(),
            "scenario journal requires a non-symlink scratch directory"
        );
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            ensure!(
                directory.permissions().mode() & 0o1000 == 0,
                "scenario journal requires a dedicated scratch directory"
            );
            options.mode(0o600);
        }
        let _reservation = match options.open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                bail!("scenario journal must use a fresh scratch path");
            }
            Err(error) => return Err(error).context("cannot reserve fresh scenario journal"),
        };
        let recording = Recording::open_at(path, SessionOrigin::Tui)?;
        let mut scenario = Self::new(host);
        scenario.recording = Some(recording);
        Ok(scenario)
    }

    /// Raw state access. Use `inspect` for views or other operations consulting
    /// thread-local write permissions when multiple hosts share a test thread.
    pub fn host(&self) -> &Host {
        &self.host
    }
    /// Inspect this host under its own write permissions, restoring the caller's
    /// thread-local mode afterward. The closure must remain synchronous.
    pub fn inspect<R>(&self, inspect: impl FnOnce(&Host) -> R) -> R {
        let _scope = ModeScope::enter(&self.host);
        inspect(&self.host)
    }

    pub fn records(&self) -> &[JournalRecord] {
        &self.records
    }
    pub fn desktop_effects(&self) -> &[DesktopEffect] {
        &self.desktop
    }
    pub fn config_writes(&self) -> &[String] {
        &self.configs
    }
    pub fn cache_writes(&self) -> &[(&'static str, serde_json::Value)] {
        &self.caches
    }
    pub fn pending_tasks(&self) -> usize {
        self.tasks.len()
    }
    pub fn is_idle(&self) -> bool {
        self.tasks.is_empty() && self.pending.is_empty()
    }

    /// Observe cumulative counts without driving the host or clearing failure.
    /// Event/effect/task counts begin when this driver is constructed; frames
    /// cover the owned host's lifetime. Scheduling-dependent event totals are
    /// diagnostic, not elapsed time or evidence that all plugin work is idle.
    pub fn costs(&self) -> ScenarioCosts {
        ScenarioCosts {
            dispatched_events: self.dispatched_events,
            returned_effect_envelopes: self.returned_effect_envelopes,
            spawned_completion_tasks: self.spawned_completion_tasks,
            shell_frames: self.host.scenario_shell_frames.get(),
        }
    }

    /// Deliver one event and start its tasks concurrently. Follow-up events are
    /// queued for bounded settling rather than recursively draining forever.
    pub fn dispatch(&mut self, event: Event) -> Result<()> {
        self.check_active()?;
        let _scope = ModeScope::enter(&self.host);
        self.dispatched_events += 1;
        let effects = self.host.update(event);
        self.returned_effect_envelopes += effects.len() as u64;
        for effect in effects {
            if let Err(error) = self.perform(effect, None, None) {
                self.failure = Some(error.to_string());
                return Err(error);
            }
        }
        Ok(())
    }

    /// Set host time, in its normal Unix-seconds unit. Does not advance Tokio's
    /// timer clock or fabricate a transport response.
    pub fn advance_time(&mut self, now: i64) -> Result<()> {
        self.dispatch(Event::Tick(now))
    }

    /// Drive at most `max_steps` scheduling turns until the predicate holds.
    /// Predicates may inspect an external fake's ledger via their closure. A held
    /// reply therefore does not prevent another task from completing. Use a
    /// current-thread Tokio test runtime for reproducible cooperative scheduling.
    /// Budget exhaustion is an error even if no progress was possible.
    pub async fn settle_until(
        &mut self,
        max_steps: usize,
        mut condition: impl FnMut(&Self) -> bool,
    ) -> Result<()> {
        self.check_active()?;
        for _ in 0..max_steps {
            if condition(self) {
                return Ok(());
            }
            tokio::task::yield_now().await;
            // Bound each turn too: a continuously completing task stream must
            // not bypass the caller's budget.
            if let Some(completion) = self.tasks.try_join_next() {
                self.receive_completion(completion)?;
            }
            if let Some(event) = self.pending.pop_front() {
                self.dispatch(event)?;
            }
        }
        ensure!(
            condition(self),
            "scenario did not settle within {max_steps} steps ({} tasks, {} events pending)",
            self.tasks.len(),
            self.pending.len()
        );
        Ok(())
    }

    pub async fn settle(&mut self, max_steps: usize) -> Result<()> {
        self.settle_until(max_steps, Self::is_idle).await
    }

    /// Await real I/O completions, delivering at most `max_steps` events within
    /// one total timeout. Unlike cooperative `settle`, this lets the SQLite
    /// writer's OS thread finish without spinning through scheduling turns.
    /// The Tokio timeout is a safety bound, not simulated host time; synchronous
    /// reducers still must return promptly. Exhaustion leaves work pending so a
    /// caller may release a held response and retry. No outcome is fabricated.
    pub async fn settle_io(&mut self, max_steps: usize, timeout: Duration) -> Result<()> {
        self.check_active()?;
        let result = tokio::time::timeout(timeout, async {
            for _ in 0..max_steps {
                if self.is_idle() {
                    return Ok(());
                }
                // Let the timeout and newly spawned tasks run between bounded
                // deliveries, even when follow-up events are already queued.
                tokio::task::yield_now().await;
                if self.pending.is_empty()
                    && let Some(completion) = self.tasks.join_next().await
                {
                    self.receive_completion(completion)?;
                }
                if let Some(event) = self.pending.pop_front() {
                    self.dispatch(event)?;
                }
            }
            ensure!(
                self.is_idle(),
                "scenario I/O did not settle within {max_steps} steps ({} tasks, {} events pending)",
                self.tasks.len(),
                self.pending.len()
            );
            Ok(())
        })
        .await;
        match result {
            Ok(result) => result,
            Err(_) => bail!(
                "scenario I/O did not settle within {timeout:?} ({} tasks, {} events pending)",
                self.tasks.len(),
                self.pending.len()
            ),
        }
    }

    /// Flush the real writer, rejecting unfinished tasks and recording gaps.
    /// This closes the scenario: no further dispatch or settling is permitted.
    pub fn finish_journal(&mut self) -> Result<()> {
        self.check_active()?;
        ensure!(
            self.is_idle(),
            "settle pending work before finishing the journal"
        );
        self.finished = true;
        if let Some(recording) = &mut self.recording {
            recording.finish();
            if let Some(warning) = recording.sink.take_warning() {
                bail!("{warning}");
            }
        }
        ensure!(
            !self
                .records
                .iter()
                .any(|record| matches!(record, JournalRecord::Gap { .. })),
            "scenario journal contains a recording gap"
        );
        Ok(())
    }

    fn check_active(&self) -> Result<()> {
        ensure!(!self.finished, "scenario journal has been finished");
        if let Some(failure) = &self.failure {
            bail!("scenario previously failed: {failure}");
        }
        Ok(())
    }

    fn receive_completion(
        &mut self,
        completion: std::result::Result<Completion, tokio::task::JoinError>,
    ) -> Result<()> {
        let completion = match completion {
            Ok(completion) => completion,
            Err(_) => {
                self.failure = Some("scenario completion worker stopped".into());
                bail!("scenario completion worker stopped");
            }
        };
        for record in completion.records {
            self.record(record);
        }
        self.pending.push_back(completion.event);
        Ok(())
    }

    fn record(&mut self, record: JournalRecord) {
        if let Some(recording) = &self.recording {
            recording.sink.submit(record.clone());
        }
        self.records.push(record);
    }

    fn outcome(&mut self, action: Option<ActionId>, outcome: Outcome, reason: Option<Reason>) {
        if let Some(id) = action {
            self.record(JournalRecord::Outcome {
                id,
                outcome,
                metadata: Metadata {
                    reason,
                    ..Default::default()
                },
            });
        }
    }

    fn perform(
        &mut self,
        effect: HostEffect,
        action: Option<ActionId>,
        semantic: Option<ActionId>,
    ) -> Result<()> {
        match effect {
            HostEffect::Journal(record) => self.record(record),
            HostEffect::Tracked {
                action,
                semantic,
                effect,
            } => {
                self.outcome(Some(action.clone()), Outcome::Started, None);
                self.perform(*effect, Some(action), semantic)?;
            }
            HostEffect::Spawn { plugin, task } => {
                self.tasks
                    .spawn(complete_task(plugin, task, action, semantic));
                self.spawned_completion_tasks += 1;
            }
            HostEffect::Save(text) => {
                self.configs.push(text);
                self.outcome(action.clone(), Outcome::Succeeded, Some(Reason::Storage));
                self.pending
                    .push_back(tracked_event(Event::Saved(Ok(())), action, semantic));
            }
            HostEffect::WriteCache { plugin, value } => {
                self.caches.push((plugin, value));
                self.outcome(action, Outcome::Succeeded, Some(Reason::Storage));
            }
            HostEffect::Open(url) => self.desktop.push(DesktopEffect::Open(url)),
            HostEffect::OpenApp { link, web } => {
                self.desktop.push(DesktopEffect::OpenApp { link, web })
            }
            HostEffect::Copy(text) => self.desktop.push(DesktopEffect::Copy(text)),
            HostEffect::CopySelection(text) => {
                self.desktop.push(DesktopEffect::CopySelection(text))
            }
            HostEffect::Scan(_) => bail!("scenario refuses credential scanning"),
            HostEffect::Work {
                plugin,
                id,
                request,
                mode,
            } => {
                let Some(recording) = &self.recording else {
                    bail!("scenario work commands require an explicit scratch journal");
                };
                self.tasks.spawn(complete_work(
                    Some(recording.sink.clone()),
                    plugin,
                    id,
                    *request,
                    mode,
                ));
                self.spawned_completion_tasks += 1;
            }
            HostEffect::History(request) => {
                let Some(recording) = &self.recording else {
                    bail!("scenario history reads require an explicit scratch journal");
                };
                self.tasks
                    .spawn(complete_history(Some(recording.sink.clone()), *request));
                self.spawned_completion_tasks += 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::work_map::{ChangeId, ItemId, Mutation};
    use crate::journal::{Journal, Query, StableCode, WriteMode};
    use crate::plugin::Msg;
    use crate::source_context::WorkReply;
    use crate::work_api::{Batch, BatchState, Intent, Request, Response};
    use crate::writes::Mode;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    const IO_TIMEOUT: Duration = Duration::from_secs(2);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("radar-scenario-journal-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn journal(&self) -> PathBuf {
            self.0.join("actions.sqlite3")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct WorkProbe(Arc<Mutex<Vec<WorkReply>>>);

    impl crate::Plugin for WorkProbe {
        fn id(&self) -> &'static str {
            "watch-work"
        }

        fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
            Ok(())
        }

        fn config(&self) -> Option<toml::Value> {
            None
        }

        fn on_msg(&mut self, msg: Msg) -> Vec<crate::Effect> {
            match msg.downcast::<(u64, Request)>() {
                Ok(request) => {
                    let (id, request) = *request;
                    vec![crate::Effect::Work {
                        id,
                        request: Box::new(request),
                    }]
                }
                Err(msg) => {
                    self.0
                        .lock()
                        .unwrap()
                        .push(*msg.downcast::<WorkReply>().unwrap());
                    Vec::new()
                }
            }
        }
    }

    fn work_scenario(scratch: &Scratch, mode: Mode) -> (Scenario, Arc<Mutex<Vec<WorkReply>>>) {
        let replies = Arc::new(Mutex::new(Vec::new()));
        let mut host = Host::new(vec![Box::new(WorkProbe(replies.clone()))], 0).unwrap();
        host.mode_cfg = mode;
        host.mode_env = None;
        (
            Scenario::with_journal(host, &scratch.journal()).unwrap(),
            replies,
        )
    }

    fn create_item(n: u64) -> (ItemId, Batch) {
        let item = ItemId::parse(&format!("20000000-0000-4000-8000-{n:012x}")).unwrap();
        let change = ChangeId::parse(&format!("30000000-0000-4000-8000-{n:012x}")).unwrap();
        let batch = Batch::new(vec![Intent::asserted(
            change,
            Mutation::CreateItem {
                item_id: item.clone(),
                kind: StableCode::new("task"),
            },
        )]);
        (item, batch)
    }

    fn request(scenario: &mut Scenario, id: u64, request: Request) {
        scenario
            .dispatch(Event::Msg {
                plugin: 0,
                msg: Box::new((id, request)),
            })
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn scratch_work_apply_and_read_use_real_writer_and_reply_ids() {
        let scratch = Scratch::new();
        let (mut scenario, replies) = work_scenario(&scratch, Mode::Write);
        let (item, batch) = create_item(1);
        request(&mut scenario, 7, Request::Apply(batch.clone()));
        assert_eq!(scenario.pending_tasks(), 1);
        assert_eq!(scenario.costs().spawned_completion_tasks, 1);
        assert!(replies.lock().unwrap().is_empty());
        assert!(scenario.finish_journal().is_err());
        scenario.settle_io(16, IO_TIMEOUT).await.unwrap();
        let receipt = {
            let replies = replies.lock().unwrap();
            assert_eq!(replies[0].id, 7);
            let Ok(Response::Applied(receipt)) = &replies[0].result else {
                panic!("real writer did not return an apply receipt");
            };
            assert_eq!(receipt.changes.len(), 1);
            assert_eq!(receipt.changes[0].change_id, batch.changes[0].change_id);
            assert!(!receipt.changes[0].replayed);
            assert!(!receipt.recording_degraded);
            receipt.clone()
        };
        request(&mut scenario, 8, Request::Items(Default::default()));
        request(&mut scenario, 9, Request::InspectBatch(batch));
        scenario.settle_io(16, IO_TIMEOUT).await.unwrap();
        {
            let replies = replies.lock().unwrap();
            assert_eq!(replies.len(), 3);
            let items = replies.iter().find(|reply| reply.id == 8).unwrap();
            let Ok(Response::Items(items)) = &items.result else {
                panic!("real writer did not return an item page");
            };
            assert_eq!(items.page.items.len(), 1);
            assert_eq!(items.page.items[0].item_id, item);
            let inspected = replies.iter().find(|reply| reply.id == 9).unwrap();
            assert!(matches!(
                inspected.result,
                Ok(Response::BatchState(BatchState::AllPresent))
            ));
        }
        assert!(scenario.records().is_empty(), "no synthetic work outcomes");
        assert_eq!(scenario.pending_tasks(), 0);
        assert_eq!(scenario.costs().spawned_completion_tasks, 3);
        assert_eq!(scenario.costs().dispatched_events, 6);
        scenario.finish_journal().unwrap();
        let journal = Journal::open(&scratch.journal()).unwrap();
        let items = journal.query_items(&Default::default(), 16, None).unwrap();
        assert_eq!(items.items.len(), 1);
        assert_eq!(items.items[0].item_id, item);
        let events = journal.query(&Query::default()).unwrap();
        assert_eq!(events.len(), 3, "reads must not audit themselves");
        assert!(
            events
                .iter()
                .all(|event| event.action_id == receipt.attempt.action_id)
        );
        assert_eq!(
            events.iter().map(|event| event.outcome).collect::<Vec<_>>(),
            [Outcome::Requested, Outcome::Started, Outcome::Succeeded]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn scratch_work_captures_request_mode_before_later_poll() {
        let scratch = Scratch::new();
        let (mut scenario, replies) = work_scenario(&scratch, Mode::ReadOnly);
        let (blocked_item, blocked) = create_item(1);
        request(&mut scenario, 11, Request::Apply(blocked));
        // Neither changing this host nor the polling thread's ambient mode can
        // change the permissions captured when the request left the reducer.
        scenario.host.mode_cfg = Mode::Write;
        let previous = crate::writes::mode();
        crate::writes::set_mode(Mode::Write, None);
        scenario.settle_io(16, IO_TIMEOUT).await.unwrap();
        assert!(matches!(
            replies.lock().unwrap()[0].result,
            Err(crate::work_api::Error::PermissionDenied)
        ));
        let (allowed_item, allowed) = create_item(2);
        request(&mut scenario, 12, Request::Apply(allowed));
        scenario.host.mode_cfg = Mode::ReadOnly;
        crate::writes::set_mode(Mode::ReadOnly, None);
        scenario.settle_io(16, IO_TIMEOUT).await.unwrap();
        assert!(matches!(
            replies.lock().unwrap()[1].result,
            Ok(Response::Applied(_))
        ));
        scenario.finish_journal().unwrap();
        let journal = Journal::open(&scratch.journal()).unwrap();
        let items = journal.query_items(&Default::default(), 16, None).unwrap();
        assert_eq!(items.items.len(), 1);
        assert_eq!(items.items[0].item_id, allowed_item);
        assert_ne!(items.items[0].item_id, blocked_item);
        let events = journal.query(&Query::default()).unwrap();
        let requested: Vec<_> = events
            .iter()
            .filter(|event| event.outcome == Outcome::Requested)
            .collect();
        assert_eq!(requested.len(), 2);
        assert_eq!(requested[0].metadata.mode, Some(WriteMode::ReadOnly));
        assert_eq!(requested[1].metadata.mode, Some(WriteMode::Write));
        assert!(
            events
                .iter()
                .any(|event| event.action_id == requested[0].action_id
                    && event.outcome == Outcome::Blocked
                    && event.metadata.reason == Some(Reason::PermissionDenied))
        );
        assert!(
            !events
                .iter()
                .any(|event| event.action_id == requested[0].action_id
                    && event.outcome == Outcome::Succeeded)
        );
        crate::writes::set_mode(previous.0, previous.1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn scratch_history_reads_queued_records_without_audit_writes() {
        use crate::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let scratch = Scratch::new();
        let mut scenario =
            Scenario::with_journal(Host::new(Vec::new(), 0).unwrap(), &scratch.journal()).unwrap();
        // Pause is an actual host action. Its two records are queued without
        // flushing or waiting before the History request joins the same writer.
        scenario
            .dispatch(Event::Key(KeyEvent::new(
                KeyCode::Char('z'),
                KeyModifiers::NONE,
            )))
            .unwrap();
        assert!(scenario.host().paused().is_some());
        assert_eq!(scenario.records().len(), 2);
        // Opening and refreshing the actual host overlay goes through the real
        // History effect and its matching request ID, not injected Event pages.
        scenario
            .dispatch(Event::Key(KeyEvent::new(
                KeyCode::Char('H'),
                KeyModifiers::NONE,
            )))
            .unwrap();
        assert_eq!(scenario.pending_tasks(), 1);
        let loading = scenario.inspect(|host| host.chrome_view(120, 40).history.unwrap());
        assert!(loading.pending && loading.rows.is_empty());
        assert_eq!(scenario.costs().spawned_completion_tasks, 1);
        scenario.settle_io(16, IO_TIMEOUT).await.unwrap();
        let first = scenario.inspect(|host| host.chrome_view(120, 40).history.unwrap());
        assert!(!first.pending && first.error.is_none());
        assert_eq!(first.rows.len(), 2);
        assert!(first.rows.iter().all(|row| row.action == "ui.action"));
        assert_eq!(
            first
                .rows
                .iter()
                .map(|row| row.outcome.as_str())
                .collect::<Vec<_>>(),
            ["requested", "succeeded"]
        );
        scenario
            .dispatch(Event::Key(KeyEvent::new(
                KeyCode::Char('r'),
                KeyModifiers::NONE,
            )))
            .unwrap();
        scenario.settle_io(16, IO_TIMEOUT).await.unwrap();
        let refreshed = scenario.inspect(|host| host.chrome_view(120, 40).history.unwrap());
        assert!(!refreshed.pending && refreshed.error.is_none());
        assert_eq!(refreshed.boundary, first.boundary);
        assert_eq!(refreshed.rows.len(), first.rows.len());
        assert_eq!(
            scenario.records().len(),
            2,
            "history browsing is not activity"
        );
        assert_eq!(scenario.pending_tasks(), 0);
        assert_eq!(scenario.costs().spawned_completion_tasks, 2);
        assert_eq!(scenario.costs().shell_frames, 0);
        scenario.finish_journal().unwrap();
        let journal = Journal::open(&scratch.journal()).unwrap();
        assert_eq!(journal.query(&Query::default()).unwrap().len(), 2);
    }

    #[test]
    fn scratch_journal_rejects_existing_database() {
        let scratch = Scratch::new();
        let mut scenario =
            Scenario::with_journal(Host::new(Vec::new(), 0).unwrap(), &scratch.journal()).unwrap();
        scenario.finish_journal().unwrap();
        let before = std::fs::read(scratch.journal()).unwrap();
        let result = Scenario::with_journal(Host::new(Vec::new(), 0).unwrap(), &scratch.journal());
        assert!(matches!(result, Err(error) if error.to_string().contains("fresh scratch path")));
        assert_eq!(std::fs::read(scratch.journal()).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn scratch_journal_reservation_preserves_existing_files_and_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let scratch = Scratch::new();
        let existing = scratch.journal();
        std::fs::write(&existing, b"watch fixture must remain untouched").unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o640)).unwrap();
        let absent = scratch.0.join("absent.sqlite3");
        let linked = scratch.0.join("linked.sqlite3");
        let dangling = scratch.0.join("dangling.sqlite3");
        symlink(&existing, &linked).unwrap();
        symlink(&absent, &dangling).unwrap();
        for path in [&existing, &linked, &dangling] {
            let result = Scenario::with_journal(Host::new(Vec::new(), 0).unwrap(), path);
            assert!(
                matches!(result, Err(error) if error.to_string().contains("fresh scratch path"))
            );
        }
        assert_eq!(
            std::fs::read(&existing).unwrap(),
            b"watch fixture must remain untouched"
        );
        assert_eq!(
            std::fs::metadata(&existing).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(std::fs::read_link(&linked).unwrap(), existing);
        assert_eq!(std::fs::read_link(&dangling).unwrap(), absent);
        assert!(!absent.exists(), "dangling link target must not be created");
    }

    #[test]
    fn scratch_journal_requires_an_existing_scratch_parent() {
        let scratch = Scratch::new();
        let parent = scratch.0.join("missing");
        let result = Scenario::with_journal(
            Host::new(Vec::new(), 0).unwrap(),
            &parent.join("actions.sqlite3"),
        );
        assert!(
            matches!(result, Err(error) if error.to_string().contains("existing caller-owned scratch directory"))
        );
        assert!(!parent.exists());
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn io_settle_timeout_keeps_pending_work_and_can_resume() {
        let mut scenario = scenario();
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        scenario
            .perform(
                HostEffect::Spawn {
                    plugin: 0,
                    task: Box::pin(async move {
                        wait.await.unwrap();
                        Box::new(()) as Msg
                    }),
                },
                None,
                None,
            )
            .unwrap();
        let error = scenario
            .settle_io(16, Duration::from_millis(1))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("1 tasks, 0 events pending"));
        assert!(!scenario.is_idle());
        assert!(scenario.finish_journal().is_err());
        release.send(()).unwrap();
        scenario.settle_io(16, IO_TIMEOUT).await.unwrap();
        assert!(scenario.is_idle());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn io_settle_step_budget_is_enforced() {
        let mut scenario = scenario();
        scenario
            .perform(HostEffect::Save("watch = true".into()), None, None)
            .unwrap();
        let error = scenario.settle_io(0, IO_TIMEOUT).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("0 steps (0 tasks, 1 events pending)")
        );
        scenario.settle_io(1, IO_TIMEOUT).await.unwrap();
        assert!(scenario.is_idle());
    }

    struct Watch;
    impl crate::Plugin for Watch {
        fn id(&self) -> &'static str {
            "watch"
        }
        fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
            Ok(())
        }
        fn config(&self) -> Option<toml::Value> {
            None
        }
        fn on_msg(&mut self, msg: Msg) -> Vec<crate::Effect> {
            if msg.is::<bool>() {
                vec![crate::Effect::Work {
                    id: 1,
                    request: Box::new(crate::work_api::Request::Items(Default::default())),
                }]
            } else {
                Vec::new()
            }
        }
    }

    fn scenario() -> Scenario {
        Scenario::new(Host::new(vec![Box::new(Watch)], 0).unwrap())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cost_counts_are_owned_and_include_settled_tracked_tasks() {
        struct TaskProbe;
        impl crate::Plugin for TaskProbe {
            fn id(&self) -> &'static str {
                "watch-task-costs"
            }
            fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
                Ok(())
            }
            fn config(&self) -> Option<toml::Value> {
                None
            }
            fn on_msg(&mut self, msg: Msg) -> Vec<crate::Effect> {
                if msg.is::<bool>() {
                    vec![crate::Effect::Spawn(Box::pin(async {
                        Box::new(()) as Msg
                    }))]
                } else {
                    Vec::new()
                }
            }
        }
        let make = || Scenario::new(Host::new(vec![Box::new(TaskProbe)], 0).unwrap());
        let mut left = make();
        let mut right = make();
        assert_eq!(left.costs(), ScenarioCosts::default());
        assert_eq!(right.costs(), ScenarioCosts::default());
        for index in [0, 1, 0] {
            let scenario = if index == 0 { &mut left } else { &mut right };
            scenario
                .dispatch(Event::Msg {
                    plugin: 0,
                    msg: Box::new(true),
                })
                .unwrap();
        }
        let left_pending = left.costs();
        assert_eq!(
            left_pending,
            ScenarioCosts {
                dispatched_events: 2,
                // One Requested journal entry and one Tracked Spawn per event;
                // the inner Spawn is not another top-level envelope.
                returned_effect_envelopes: 4,
                spawned_completion_tasks: 2,
                shell_frames: 0,
            }
        );
        assert_eq!(
            right.costs(),
            ScenarioCosts {
                dispatched_events: 1,
                returned_effect_envelopes: 2,
                spawned_completion_tasks: 1,
                shell_frames: 0,
            }
        );
        assert_eq!(
            left.records()
                .iter()
                .filter(|record| matches!(
                    record,
                    JournalRecord::Outcome {
                        outcome: Outcome::Started,
                        ..
                    }
                ))
                .count(),
            2,
            "both tasks were started through their Tracked wrappers"
        );
        let right_pending = right.costs();
        left.settle(32).await.unwrap();
        assert!(left.is_idle());
        assert_eq!(left.pending_tasks(), 0);
        assert_eq!(
            left.costs(),
            ScenarioCosts {
                dispatched_events: 4,
                returned_effect_envelopes: 6,
                ..left_pending
            }
        );
        assert_eq!(right.costs(), right_pending);
        right.settle(32).await.unwrap();
        assert_eq!(
            right.costs(),
            ScenarioCosts {
                dispatched_events: 2,
                returned_effect_envelopes: 3,
                ..right_pending
            }
        );
        assert_eq!(left_pending.dispatched_events, 2, "snapshot stays copied");
        let finished = left.costs();
        left.finish_journal().unwrap();
        assert!(left.advance_time(1).is_err());
        assert_eq!(left.costs(), finished);
    }

    #[test]
    fn shell_frame_counts_cover_raw_helpers_and_ignore_neutral_views() {
        use crate::design::theme::Tier;
        use crate::ratatui::{Terminal, backend::TestBackend};
        use crate::theme::Theme;

        let mut host = Host::new(Vec::new(), 0).unwrap();
        host.load_config(Some("[theme]\nicons = 'off'\n")).unwrap();
        let _ = crate::shell::to_text(&host, 80, 24);
        let mut scenario = Scenario::new(host);
        assert_eq!(
            scenario.costs(),
            ScenarioCosts {
                shell_frames: 1,
                ..Default::default()
            },
            "frames include the owned host's pre-scenario lifetime"
        );
        let other = self::scenario();
        let _ = scenario.inspect(|host| host.chrome_view(80, 24));
        assert_eq!(scenario.costs().shell_frames, 1);
        let _ = crate::shell::to_text(scenario.host(), 80, 24);
        assert_eq!(scenario.costs().shell_frames, 2, "raw host access counts");

        let renders: [fn(&Host); 5] = [
            |host| {
                let _ = crate::shell::to_svg(host, 80, 24);
            },
            |host| {
                let _ = crate::shell::to_styles(host, 80, 24);
            },
            |host| {
                let _ = crate::shell::text_in(host, &Theme::new(Tier::Mono), 80, 24);
            },
            |host| {
                let _ = crate::shell::text_and_cursor(host, &Theme::new(Tier::Mono), 80, 24);
            },
            |host| {
                let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
                terminal
                    .draw(|frame| crate::shell::render(host, &Theme::new(Tier::Mono), frame))
                    .unwrap();
            },
        ];
        for render in renders {
            let before = scenario.costs();
            scenario.inspect(render);
            assert_eq!(
                scenario.costs(),
                ScenarioCosts {
                    shell_frames: before.shell_frames + 1,
                    ..before
                },
                "each actual shell helper invocation renders exactly once"
            );
            assert_eq!(other.costs(), ScenarioCosts::default());
        }
        // An existing overlay takes render's early return, which still counts.
        // This sets only renderer input; it does not claim an executed read.
        scenario.host.history = Some(crate::history::History::new(Default::default(), 1).0);
        let before = scenario.costs();
        let _ = scenario.inspect(|host| host.chrome_view(80, 24));
        assert_eq!(
            scenario.costs(),
            before,
            "neutral overlay view does not render"
        );
        let _ = scenario.inspect(|host| crate::shell::to_text(host, 80, 24));
        assert_eq!(scenario.costs().shell_frames, before.shell_frames + 1);
        assert_eq!(other.costs(), ScenarioCosts::default());
    }

    #[test]
    fn interleaved_hosts_scope_permissions_for_dispatch_and_inspection() {
        use crate::writes::{self, Change, Mode};
        let previous = writes::mode();
        struct PermissionProbe;
        impl crate::Plugin for PermissionProbe {
            fn id(&self) -> &'static str {
                "watch-permissions"
            }
            fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
                Ok(())
            }
            fn config(&self) -> Option<toml::Value> {
                None
            }
            fn on_msg(&mut self, _: Msg) -> Vec<crate::Effect> {
                vec![crate::Effect::Copy(
                    writes::not_sent(Change::Add).unwrap_or_else(|| "allowed".into()),
                )]
            }
        }
        let make = |mode| {
            let mut host = Host::new(vec![Box::new(PermissionProbe)], 0).unwrap();
            host.mode_cfg = mode;
            host.mode_env = None;
            host.apply_mode();
            Scenario::new(host)
        };
        let mut read = make(Mode::ReadOnly);
        let mut write = make(Mode::Write);
        let caller = (Mode::Edit, Some("fixture-caller".into()));
        writes::set_mode(caller.0, caller.1.clone());
        for index in [0, 1, 0] {
            let scenario = if index == 0 { &mut read } else { &mut write };
            scenario
                .dispatch(Event::Msg {
                    plugin: 0,
                    msg: Box::new(()),
                })
                .unwrap();
            assert_eq!(writes::mode(), caller);
        }
        assert!(
            matches!(&read.desktop_effects()[0], DesktopEffect::Copy(note) if note != "allowed")
        );
        assert!(
            matches!(&read.desktop_effects()[1], DesktopEffect::Copy(note) if note != "allowed")
        );
        assert_eq!(
            write.desktop_effects(),
            &[DesktopEffect::Copy("allowed".into())]
        );
        assert_eq!(read.inspect(|_| writes::mode().0), Mode::ReadOnly);
        assert_eq!(write.inspect(|_| writes::mode().0), Mode::Write);
        assert_eq!(writes::mode(), caller);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            read.inspect(|_| panic!("fixture inspection panic"));
        }));
        assert!(result.is_err());
        assert_eq!(writes::mode(), caller);
        writes::set_mode(previous.0, previous.1);
    }

    #[test]
    fn credential_scanning_is_refused() {
        let error = scenario()
            .perform(HostEffect::Scan(Vec::new()), None, None)
            .unwrap_err();
        assert_eq!(error.to_string(), "scenario refuses credential scanning");
    }

    #[test]
    fn work_and_history_are_not_fabricated() {
        let mut scenario = scenario();
        let work = HostEffect::Work {
            plugin: 0,
            id: 1,
            request: Box::new(crate::work_api::Request::Items(Default::default())),
            mode: crate::writes::Mode::ReadOnly,
        };
        assert!(
            scenario
                .perform(work, None, None)
                .unwrap_err()
                .to_string()
                .contains("work commands require an explicit scratch journal")
        );
        let history = HostEffect::History(Box::new(crate::history::Request {
            id: 1,
            filter: Default::default(),
            cursor: None,
            snapshot: None,
        }));
        assert!(
            scenario
                .perform(history, None, None)
                .unwrap_err()
                .to_string()
                .contains("history reads require an explicit scratch journal")
        );
        assert!(scenario.is_idle());
    }

    #[tokio::test]
    async fn unsupported_effect_failure_cannot_be_hidden_by_a_true_predicate() {
        let mut scenario = scenario();
        assert!(
            scenario
                .dispatch(Event::Msg {
                    plugin: 0,
                    msg: Box::new(true)
                })
                .is_err()
        );
        let failed = scenario.costs();
        assert_eq!(failed.dispatched_events, 1);
        assert_eq!(failed.returned_effect_envelopes, 1);
        assert_eq!(failed.spawned_completion_tasks, 0);
        assert!(
            scenario
                .settle_until(1, |_| true)
                .await
                .unwrap_err()
                .to_string()
                .contains("previously failed")
        );
        assert!(scenario.finish_journal().is_err());
        assert!(scenario.advance_time(1).is_err());
        assert_eq!(
            scenario.costs(),
            failed,
            "inspection does not clear failure"
        );
    }

    #[tokio::test]
    async fn stopped_completion_worker_is_a_sticky_failure() {
        let mut scenario = scenario();
        scenario
            .tasks
            .spawn(async { std::future::pending::<Completion>().await });
        scenario.tasks.abort_all();
        assert!(
            scenario
                .settle(32)
                .await
                .unwrap_err()
                .to_string()
                .contains("completion worker stopped")
        );
        assert!(scenario.settle_until(1, |_| true).await.is_err());
        assert!(scenario.settle_io(1, IO_TIMEOUT).await.is_err());
        assert!(scenario.finish_journal().is_err());
    }

    #[tokio::test]
    async fn held_task_exhausts_budget_without_blocking_ready_task() {
        let mut scenario = scenario();
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        scenario
            .perform(
                HostEffect::Spawn {
                    plugin: 0,
                    task: Box::pin(async move {
                        wait.await.unwrap();
                        Box::new(()) as Msg
                    }),
                },
                None,
                None,
            )
            .unwrap();
        scenario
            .perform(
                HostEffect::Spawn {
                    plugin: 0,
                    task: Box::pin(async { Box::new(()) as Msg }),
                },
                None,
                None,
            )
            .unwrap();
        scenario
            .settle_until(32, |s| s.pending_tasks() == 1)
            .await
            .unwrap();
        let error = scenario.settle(8).await.unwrap_err();
        assert!(error.to_string().contains("8 steps"));
        release.send(()).unwrap();
        scenario.settle(32).await.unwrap();
        assert!(scenario.is_idle());
    }

    #[tokio::test]
    async fn storage_and_desktop_effects_are_captured_per_driver() {
        let mut scenario = scenario();
        scenario
            .perform(HostEffect::Save("watch = true".into()), None, None)
            .unwrap();
        scenario
            .perform(
                HostEffect::WriteCache {
                    plugin: "watch",
                    value: serde_json::json!({"rows": 2}),
                },
                None,
                None,
            )
            .unwrap();
        scenario
            .perform(HostEffect::Copy("vimes".into()), None, None)
            .unwrap();
        scenario.settle(8).await.unwrap();
        assert_eq!(scenario.config_writes(), &["watch = true"]);
        assert_eq!(scenario.cache_writes()[0].0, "watch");
        assert_eq!(
            scenario.desktop_effects(),
            &[DesktopEffect::Copy("vimes".into())]
        );
        assert!(
            super::Scenario::new(Host::new(Vec::new(), 0).unwrap())
                .cache_writes()
                .is_empty()
        );
    }
}
