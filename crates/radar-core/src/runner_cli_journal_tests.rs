//! Completion envelopes use scratch journals and fake providers only.
use super::*;
use crate::journal::{Journal, Metadata, Query, WriteMode};
use crate::plugin::{CliObservations, CliTask, MAX_CLI_OBSERVATIONS, ObservedCliTask};

const OUTPUT: &str = "{\"text\":\"SECRET output\",\"count\":1}\n";

struct LegacyProbe {
    fail: bool,
}

impl Plugin for LegacyProbe {
    fn id(&self) -> &'static str {
        "probe"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn run_cli(&self, _: &clap::ArgMatches) -> CliTask {
        let fail = self.fail;
        Box::pin(async move { output(fail) })
    }
}

fn output(fail: bool) -> Result<String> {
    if fail {
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "SECRET error").into())
    } else {
        Ok(OUTPUT.into())
    }
}

struct ObservedProbe {
    fail: bool,
    count: usize,
    pending: bool,
}

impl Plugin for ObservedProbe {
    fn id(&self) -> &'static str {
        "probe"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn run_cli(&self, _: &clap::ArgMatches) -> CliTask {
        panic!("the runner must dispatch the observed CLI method")
    }
    fn run_cli_observed(&self, _: &clap::ArgMatches) -> ObservedCliTask {
        let (fail, count, pending) = (self.fail, self.count, self.pending);
        Box::pin(async move {
            let mut observations = CliObservations::default();
            for i in 0..count {
                observations.observe(
                    ActionDescriptor::new("probe.read", Origin::Host).with_metadata(Metadata {
                        mode: Some(WriteMode::Write),
                        count: Some(i as u32),
                        ..Default::default()
                    }),
                    if i % 2 == 0 {
                        JournalOutcome::Succeeded
                    } else {
                        JournalOutcome::Partial
                    },
                );
            }
            if pending {
                std::future::pending::<()>().await;
            }
            CliExecution::new(output(fail), observations)
        })
    }
}

fn matches() -> clap::ArgMatches {
    clap::Command::new("probe")
        .try_get_matches_from(["probe"])
        .unwrap()
}

#[tokio::test]
async fn cli_observed_default_preserves_legacy_output_and_error_type() {
    let matches = matches();
    let execution = LegacyProbe { fail: false }.run_cli_observed(&matches).await;
    assert_eq!(execution.result.unwrap(), OUTPUT);
    assert!(execution.observations.is_empty());
    assert_eq!(execution.observations.omitted(), 0);
    let execution = LegacyProbe { fail: true }.run_cli_observed(&matches).await;
    let error = execution.result.unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(error.to_string(), "SECRET error");
    assert!(execution.observations.is_empty());
}

#[tokio::test]
async fn cli_observed_disabled_journal_preserves_result() {
    for fail in [false, true] {
        let result = run_plugin_cli_recorded(
            &ObservedProbe {
                fail,
                count: 2,
                pending: false,
            },
            &matches(),
            None,
            None,
            crate::writes::Mode::ReadOnly,
        )
        .await;
        if fail {
            assert_eq!(result.unwrap_err().to_string(), "SECRET error");
        } else {
            assert_eq!(result.unwrap(), OUTPUT);
        }
    }
}

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("radar-cli-envelope-{}", ActionId::new().as_str())))
    }
    fn path(&self) -> PathBuf {
        self.0.join("actions.sqlite3")
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
        let _ = std::fs::remove_file(&self.0);
    }
}

#[tokio::test]
async fn cli_observed_unavailable_journal_preserves_result() {
    let temp = TestDir::new();
    std::fs::write(&temp.0, "not a directory").unwrap();
    let recording = Recording::open_at(&temp.path(), SessionOrigin::Cli).ok();
    assert!(recording.is_none());
    let result = run_plugin_cli_recorded(
        &ObservedProbe {
            fail: false,
            count: 1,
            pending: false,
        },
        &matches(),
        recording.as_ref().map(|recording| &recording.sink),
        None,
        crate::writes::Mode::ReadOnly,
    )
    .await;
    assert_eq!(result.unwrap(), OUTPUT);
}

#[cfg(unix)]
#[tokio::test]
async fn cli_observations_are_sibling_children_even_when_command_fails() {
    let temp = TestDir::new();
    let mut recording = Recording::open_at(&temp.path(), SessionOrigin::Cli).unwrap();
    let session = recording.sink.begin("session.cli", Origin::Host, None);
    let command = recording.sink.begin(
        "cli.command",
        Origin::Plugin("probe".into()),
        Some(session.clone()),
    );
    recording
        .sink
        .outcome(&command, JournalOutcome::Started, None);
    let received_after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let result = run_plugin_cli_recorded(
        &ObservedProbe {
            fail: true,
            count: 2,
            pending: false,
        },
        &matches(),
        Some(&recording.sink),
        Some(&command),
        crate::writes::Mode::ReadOnly,
    )
    .await;
    assert_eq!(result.unwrap_err().to_string(), "SECRET error");
    recording
        .sink
        .outcome(&command, JournalOutcome::Failed, None);
    recording
        .sink
        .outcome(&session, JournalOutcome::Failed, None);
    recording.finish();
    assert_eq!(recording.sink.take_warning(), None);
    let journal = Journal::open(&temp.path()).unwrap();
    let events = journal
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    let children = events
        .iter()
        .filter(|event| event.kind.as_str() == "probe.read")
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 4);
    for event in &children {
        assert_eq!(event.parent_action_id.as_ref(), Some(&command));
        assert_eq!(event.root_action_id, session);
        assert_eq!(event.correlation_id, session);
        assert_eq!(event.origin, Origin::Plugin("probe".into()));
        assert!(event.observed_at_ms >= received_after);
        assert!(!event.observation_time_estimated);
        assert_eq!(event.metadata.duration_ms, None);
        assert_ne!(event.outcome, JournalOutcome::Started);
        assert_eq!(event.metadata.mode, Some(WriteMode::ReadOnly));
    }
    assert_ne!(children[0].action_id, children[2].action_id);
    assert_eq!(children[1].outcome, JournalOutcome::Succeeded);
    assert_eq!(children[1].metadata.count, Some(0));
    assert_eq!(children[3].outcome, JournalOutcome::Partial);
    assert_eq!(children[3].metadata.count, Some(1));
    assert!(!serde_json::to_string(&events).unwrap().contains("SECRET"));
}

#[cfg(unix)]
#[tokio::test]
async fn cli_observation_overflow_records_correlated_omission_and_gap() {
    let temp = TestDir::new();
    let mut recording = Recording::open_at(&temp.path(), SessionOrigin::Cli).unwrap();
    let command = recording
        .sink
        .begin("cli.command", Origin::Plugin("probe".into()), None);
    let result = run_plugin_cli_recorded(
        &ObservedProbe {
            fail: false,
            count: MAX_CLI_OBSERVATIONS + 3,
            pending: false,
        },
        &matches(),
        Some(&recording.sink),
        Some(&command),
        crate::writes::Mode::ReadOnly,
    )
    .await;
    assert_eq!(result.unwrap(), OUTPUT);
    recording
        .sink
        .outcome(&command, JournalOutcome::Succeeded, None);
    recording.finish();
    assert!(recording.sink.take_warning().is_some());
    let journal = Journal::open(&temp.path()).unwrap();
    let events = journal
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind.as_str() == "probe.read")
            .count(),
        MAX_CLI_OBSERVATIONS * 2
    );
    let omitted = events
        .iter()
        .filter(|event| event.kind.as_str() == "cli.observations.omitted")
        .collect::<Vec<_>>();
    assert_eq!(omitted.len(), 2);
    assert_eq!(omitted[0].parent_action_id.as_ref(), Some(&command));
    assert_eq!(omitted[0].origin, Origin::Host);
    assert!(omitted[0].metadata.truncated);
    assert_eq!(omitted[0].metadata.count, Some(3));
    assert_eq!(omitted[1].outcome, JournalOutcome::Partial);
    assert_eq!(omitted[1].metadata.count, Some(3));
    let gaps = journal
        .recording_gaps(&events[0].session_id, 0, 1000)
        .unwrap();
    assert_eq!(gaps.iter().map(|gap| gap.dropped_records).sum::<u64>(), 6);
}

#[cfg(unix)]
#[tokio::test]
async fn cli_interruption_does_not_fabricate_unreturned_provider_observations() {
    use std::future::{Future, poll_fn};
    use std::task::Poll;

    let temp = TestDir::new();
    let mut recording = Recording::open_at(&temp.path(), SessionOrigin::Cli).unwrap();
    let command = recording
        .sink
        .begin("cli.command", Origin::Plugin("probe".into()), None);
    let plugin = ObservedProbe {
        fail: false,
        count: 2,
        pending: true,
    };
    let matches = matches();
    let mut task = Box::pin(run_plugin_cli_recorded(
        &plugin,
        &matches,
        Some(&recording.sink),
        Some(&command),
        crate::writes::Mode::ReadOnly,
    ));
    poll_fn(|cx| {
        assert!(task.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(task);
    recording.finish();
    let journal = Journal::open(&temp.path()).unwrap();
    let events = journal
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    assert!(
        events
            .iter()
            .all(|event| event.kind.as_str() == "cli.command")
    );
    assert_eq!(events.last().unwrap().outcome, JournalOutcome::Unknown);
    assert_eq!(
        events.last().unwrap().metadata.reason,
        Some(Reason::NoTerminalObservation)
    );
}
