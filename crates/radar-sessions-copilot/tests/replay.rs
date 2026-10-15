//! The Copilot adapter on a stream recorded once from the real CLI (1.0.95, protocol 3).
//! No process, no network, no wall clock: the clock is injected and timeouts are virtual.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use radar_sessions::adapter::{Catalog, CatalogEntry, ProviderSetup, SessionSetup, Updates};
use radar_sessions::{
    Adapter, AuthStatus, CreateSession, Durability, EventKind, RadarSessionId, PermissionAnswer,
    PermissionDecision, PermissionKind, PermissionTarget, ResumeSession, SendInput, Update,
};
use radar_sessions_copilot::adapter::{Clock, CopilotAdapter};
use radar_sessions_copilot::backend::{BackendStatus, StatusError};
use radar_sessions_copilot::replay::{Recording, ReplayBackend};

const RECORDING: &str = include_str!("fixtures/live-1.0.95.jsonl");

fn clock() -> Clock {
    Arc::new(|| 1_700_000_000)
}

fn adapter(turns: &[&str]) -> CopilotAdapter<ReplayBackend> {
    let rec = Recording::from_jsonl(RECORDING).expect("recording parses");
    CopilotAdapter::new(ReplayBackend::new(rec, turns), None, clock())
}

fn sid() -> RadarSessionId {
    RadarSessionId::new("00000000-0000-4000-8000-000000000001")
}

async fn open(a: &CopilotAdapter<ReplayBackend>) -> Updates {
    let updates = a.subscribe().await.unwrap();
    a.create(CreateSession {
        radar_session_id: sid(),
        cwd: PathBuf::from("/work/repo"),
        model: None,
        setup: Default::default(),
    })
    .await
    .unwrap();
    updates
}

/// Reads updates until one matches; answers permission requests as `decide` says.
async fn until(
    a: &CopilotAdapter<ReplayBackend>,
    updates: &mut Updates,
    mut decide: impl FnMut(&radar_sessions::PermissionRequest) -> PermissionDecision,
    stop: impl Fn(&EventKind) -> bool,
) -> Vec<radar_sessions::Event> {
    let mut seen = Vec::new();
    loop {
        let u = tokio::time::timeout(Duration::from_secs(5), updates.recv())
            .await
            .expect("an update arrives")
            .expect("the stream stays open");
        let Update::Event(ev) = u else { continue };
        if let EventKind::PermissionRequested { request } = &ev.kind {
            let decision = decide(request);
            a.answer_permission(
                &sid(),
                PermissionAnswer {
                    request_id: request.request_id.clone(),
                    decision,
                },
            )
            .await
            .unwrap();
        }
        let done = stop(&ev.kind);
        seen.push(ev);
        if done {
            return seen;
        }
    }
}

fn approve_shell_deny_write(r: &radar_sessions::PermissionRequest) -> PermissionDecision {
    if r.kind == PermissionKind::Write {
        PermissionDecision::Deny {
            reason: Some("no writes".into()),
        }
    } else {
        PermissionDecision::ApproveOnce
    }
}

#[tokio::test(start_paused = true)]
async fn a_turn_streams_asks_and_ends_idle() {
    let a = adapter(&["turn1"]);
    let mut updates = open(&a).await;
    a.send(
        &sid(),
        SendInput {
            text: "list the files".into(),
        },
    )
    .await
    .unwrap();
    let evs = until(&a, &mut updates, approve_shell_deny_write, |k| {
        matches!(k, EventKind::Idle)
    })
    .await;

    // Sequences are strictly increasing from 1 and stamped by the injected clock.
    assert_eq!(evs[0].sequence, 1);
    assert!(evs.windows(2).all(|w| w[1].sequence == w[0].sequence + 1));
    assert!(evs
        .iter()
        .all(|e| e.observed_at == 1_700_000_000 && e.provider.as_str() == "copilot"));

    // Deltas are ephemeral, the finished message is persisted.
    assert!(evs
        .iter()
        .any(|e| matches!(e.kind, EventKind::AssistantDelta { .. })
            && e.durability == Durability::Ephemeral));
    assert!(evs
        .iter()
        .any(|e| matches!(e.kind, EventKind::AssistantMessage { .. })
            && e.durability == Durability::Persisted));

    // A shell request carried its exact command; a write request carried the path.
    let reqs: Vec<_> = evs
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::PermissionRequested { request } => Some(request.clone()),
            _ => None,
        })
        .collect();
    assert!(reqs.iter().any(
        |r| matches!(&r.target, PermissionTarget::Command { command, .. } if command == "ls")
    ));
    assert!(reqs.iter().any(
        |r| matches!(&r.target, PermissionTarget::Write { path, .. } if path.ends_with("notes.txt"))
    ));

    // The approval ran the tool; the denial came back to the model as a failed tool.
    assert!(evs
        .iter()
        .any(|e| matches!(&e.kind, EventKind::ToolCompleted { ok: true, .. })));
    assert!(evs.iter().any(|e| matches!(
        &e.kind,
        EventKind::PermissionResolved {
            approved: false,
            ..
        }
    )));

    // The turn took three model calls (`assistant.turn_end` after each): only `session.idle` ends
    // it, so nothing that keeps an eye on the turn (the dock's stop key, a review that saves its
    // note when the turn ends) sees it end after the first call.
    let rounds = evs.iter().filter(|e| e.kind == EventKind::TurnStarted).count();
    assert_eq!(rounds, 3);
    let (last, before) = evs.split_last().unwrap();
    assert_eq!(last.kind, EventKind::Idle);
    assert!(!before.iter().any(|e| matches!(e.kind, EventKind::TurnEnded | EventKind::Idle)), "a round's end is not the turn's");
}

#[tokio::test(start_paused = true)]
async fn nothing_is_answered_until_the_operator_answers() {
    let a = adapter(&["turn1"]);
    let mut updates = open(&a).await;
    a.send(&sid(), SendInput { text: "go".into() })
        .await
        .unwrap();
    // Read up to the first request and do not answer it: nothing after it arrives.
    let mut last = None;
    while let Ok(Some(Update::Event(ev))) =
        tokio::time::timeout(Duration::from_secs(5), updates.recv()).await
    {
        let stop = matches!(ev.kind, EventKind::PermissionRequested { .. });
        last = Some(ev.kind);
        if stop {
            break;
        }
    }
    assert!(matches!(last, Some(EventKind::PermissionRequested { .. })));
    assert!(
        tokio::time::timeout(Duration::from_secs(60), updates.recv())
            .await
            .is_err(),
        "the turn waits for the operator"
    );
}

#[tokio::test(start_paused = true)]
async fn answering_twice_is_a_no_op() {
    let a = adapter(&["turn1"]);
    let mut updates = open(&a).await;
    a.send(&sid(), SendInput { text: "go".into() })
        .await
        .unwrap();
    let mut first = true;
    let evs = until(
        &a,
        &mut updates,
        |r| {
            if first {
                first = false;
                PermissionDecision::ApproveOnce
            } else {
                approve_shell_deny_write(r)
            }
        },
        |k| matches!(k, EventKind::PermissionRequested { .. }),
    )
    .await;
    let EventKind::PermissionRequested { request } = &evs.last().unwrap().kind else {
        unreachable!()
    };
    // A second, different answer to the same request changes nothing and is not an error.
    a.answer_permission(
        &sid(),
        PermissionAnswer {
            request_id: request.request_id.clone(),
            decision: PermissionDecision::Deny { reason: None },
        },
    )
    .await
    .unwrap();
    let rest = until(&a, &mut updates, approve_shell_deny_write, |k| {
        matches!(k, EventKind::PermissionResolved { .. })
    })
    .await;
    assert!(matches!(
        rest.last().unwrap().kind,
        EventKind::PermissionResolved { approved: true, .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn stop_ends_the_turn_and_the_session_stays_usable() {
    let a = adapter(&["turn2", "turn3"]);
    let mut updates = open(&a).await;
    a.send(
        &sid(),
        SendInput {
            text: "sleep".into(),
        },
    )
    .await
    .unwrap();
    let _ = until(&a, &mut updates, approve_shell_deny_write, |k| {
        matches!(k, EventKind::ToolStarted { .. })
    })
    .await;
    a.stop(&sid()).await.unwrap();
    let evs = until(&a, &mut updates, approve_shell_deny_write, |k| {
        matches!(k, EventKind::Idle)
    })
    .await;
    assert!(evs.iter().any(|e| matches!(e.kind, EventKind::Aborted)));
    a.send(
        &sid(),
        SendInput {
            text: "again".into(),
        },
    )
    .await
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn resume_replays_persisted_history_and_continues_the_sequence() {
    let a = adapter(&["turn3"]);
    let mut updates = a.subscribe().await.unwrap();
    a.resume(ResumeSession {
        radar_session_id: sid(),
        provider_session_id: radar_sessions::ProviderSessionId::new(sid().as_str()),
        cwd: PathBuf::from("/work/repo"),
        cursor: None,
        setup: Default::default(),
    })
    .await
    .unwrap();
    let mut history = Vec::new();
    while let Ok(Update::Event(ev)) = updates.try_recv() {
        history.push(ev);
    }
    // After the history, radar's own word that the resumed session runs nothing: the history keeps no
    // `session.idle`, and its last turn may have been cut off when the CLI that ran it stopped.
    let settled = history.last().expect("events").clone();
    assert_eq!((&settled.kind, settled.sequence), (&EventKind::Idle, history.len() as u64));
    let last = history[history.len() - 2].provider_event_id.clone().unwrap();
    assert_eq!(settled.provider_event_id.unwrap().as_str(), format!("{last}#radar-end-of-history"));
    assert!(
        history
            .iter()
            .all(|e| e.durability == Durability::Persisted),
        "ephemeral events are not in the provider's history"
    );
    assert_eq!(history[0].sequence, 1);
    assert!(history
        .iter()
        .any(|e| matches!(e.kind, EventKind::UserMessage { .. })));
    assert!(history.iter().all(|e| e.provider_event_id.is_some()));
}

#[tokio::test(start_paused = true)]
async fn two_restarts_in_a_row_each_get_the_whole_history() {
    let mut seen = Vec::new();
    for _ in 0..2 {
        // A fresh adapter is a restarted radar; the provider keeps the session.
        let a = adapter(&["turn3"]);
        let mut updates = a.subscribe().await.unwrap();
        a.resume(ResumeSession {
            radar_session_id: sid(),
            provider_session_id: radar_sessions::ProviderSessionId::new(sid().as_str()),
            cwd: PathBuf::from("/work/repo"),
            cursor: None,
            setup: Default::default(),
        })
        .await
        .unwrap();
        let mut ids = Vec::new();
        while let Ok(Update::Event(ev)) = updates.try_recv() {
            ids.push(ev.provider_event_id);
        }
        assert!(!ids.is_empty(), "a restart restores the history");
        seen.push(ids);
    }
    assert_eq!(seen[0], seen[1]);
}

#[tokio::test(start_paused = true)]
async fn a_lost_connection_is_reported() {
    let a = adapter(&["turn1"]);
    let mut updates = open(&a).await;
    a.backend().drop_connection(&sid());
    assert!(matches!(
        updates.recv().await,
        Some(Update::Disconnected { .. })
    ));
    // The part resumes from its cursor; the adapter says the connection is back.
    a.resume(ResumeSession {
        radar_session_id: sid(),
        provider_session_id: radar_sessions::ProviderSessionId::new(sid().as_str()),
        cwd: PathBuf::from("/work/repo"),
        cursor: None,
        setup: Default::default(),
    })
    .await
    .unwrap();
    let mut got_reconnected = false;
    while let Ok(u) = updates.try_recv() {
        got_reconnected |= matches!(u, Update::Reconnected);
    }
    assert!(got_reconnected);
}

#[tokio::test(start_paused = true)]
async fn cloud_hand_off_is_refused() {
    let a = adapter(&["turn1"]);
    let _updates = open(&a).await;
    for text in [
        "/delegate fix the bug",
        "  /delegate x",
        "& run this in the cloud",
    ] {
        let err = a
            .send(&sid(), SendInput { text: text.into() })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("locally only"), "{text}: {err}");
    }
    a.send(
        &sid(),
        SendInput {
            text: "a & b are fine".into(),
        },
    )
    .await
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn subscribe_is_once() {
    let a = adapter(&[]);
    let _u = a.subscribe().await.unwrap();
    assert!(a.subscribe().await.is_err());
}

fn status_with(s: Result<BackendStatus, StatusError>) -> CopilotAdapter<ReplayBackend> {
    CopilotAdapter::new(
        ReplayBackend::new(Recording::default(), &[]).with_status(s),
        None,
        clock(),
    )
}

fn ok_status() -> BackendStatus {
    BackendStatus {
        version: "1.0.95".into(),
        protocol: 3,
        authenticated: true,
        login: Some("octo".into()),
        host: Some("https://github.com".into()),
        message: None,
    }
}

#[tokio::test]
async fn status_tells_missing_from_signed_out_from_mismatch() {
    let s = status_with(Ok(ok_status())).status().await;
    assert!(s.installed && s.error.is_none());
    assert_eq!(
        s.auth,
        AuthStatus::SignedIn {
            as_user: "octo@github.com".into()
        }
    );
    assert_eq!(s.version.as_deref(), Some("1.0.95"));
    assert_eq!(s.protocol.as_deref(), Some("3"));

    let s = status_with(Err(StatusError::Missing)).status().await;
    assert!(!s.installed);

    let s = status_with(Ok(BackendStatus {
        authenticated: false,
        login: None,
        ..ok_status()
    }))
    .status()
    .await;
    assert!(s.installed);
    assert_eq!(s.auth, AuthStatus::SignedOut);

    let s = status_with(Ok(BackendStatus {
        authenticated: false,
        message: Some("blocked by organization policy".into()),
        ..ok_status()
    }))
    .status()
    .await;
    assert!(matches!(s.auth, AuthStatus::PolicyBlocked { .. }));

    let s = status_with(Ok(BackendStatus {
        protocol: 4,
        ..ok_status()
    }))
    .status()
    .await;
    assert!(s.error.unwrap().contains("protocol 4"));

    let s = status_with(Err(StatusError::Connection("closed".into())))
        .status()
        .await;
    assert!(s.installed && s.error.is_some());
}

#[tokio::test]
async fn delete_goes_to_the_provider() {
    let a = adapter(&[]);
    a.delete_session(&radar_sessions::ProviderSessionId::new("abc"))
        .await
        .unwrap();
    assert_eq!(a.backend().deleted(), vec!["abc".to_string()]);
}

fn setup() -> SessionSetup {
    SessionSetup {
        extra_dirs: vec![PathBuf::from("/data/shared")],
        agent: Some("scratch-reviewer".into()),
        skill_dirs: vec![PathBuf::from("/skills")],
        skills: vec!["plan".into()],
        instruction_dirs: vec![PathBuf::from("/rules")],
        plugin_dirs: vec![PathBuf::from("/plugins/x")],
        project_config: true,
        local_only: true,
        review: false,
        tool_server: None,
    }
}

#[tokio::test]
async fn a_new_and_a_resumed_session_get_the_setup_whole() {
    let a = adapter(&["turn1"]);
    let _updates = a.subscribe().await.unwrap();
    a.create(CreateSession {
        radar_session_id: sid(),
        cwd: PathBuf::from("/work/repo"),
        model: None,
        setup: setup(),
    })
    .await
    .unwrap();
    a.resume(ResumeSession {
        radar_session_id: sid(),
        provider_session_id: radar_sessions::ProviderSessionId::new(sid().as_str()),
        cwd: PathBuf::from("/work/repo"),
        cursor: None,
        setup: SessionSetup {
            project_config: false,
            ..setup()
        },
    })
    .await
    .unwrap();
    assert_eq!(a.backend().created(), vec![setup()]);
    let resumed = a.backend().resumed();
    assert_eq!(resumed.len(), 1);
    assert!(!resumed[0].project_config);
    assert_eq!(resumed[0].agent.as_deref(), Some("scratch-reviewer"));
}

#[tokio::test]
async fn agents_and_skills_are_offered() {
    let c = adapter(&[]).capabilities();
    assert!(c.agents && c.skills);
}

#[tokio::test]
async fn configure_reaches_the_backend_and_its_note_the_log() {
    let notes = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = notes.clone();
    let rec = Recording::from_jsonl(RECORDING).unwrap();
    let a = CopilotAdapter::new(
        ReplayBackend::new(rec, &[]).with_configure_note("applies when the sessions end"),
        None,
        clock(),
    )
    .with_log(Arc::new(move |m| sink.lock().unwrap().push(m.to_string())));
    let setup = ProviderSetup {
        executable: Some(PathBuf::from("/opt/copilot")),
        config_dir: Some(PathBuf::from("/opt/home")),
    };
    a.configure(setup.clone());
    assert_eq!(a.backend().configured(), vec![setup]);
    assert_eq!(
        *notes.lock().unwrap(),
        vec!["applies when the sessions end".to_string()]
    );
}

#[tokio::test]
async fn the_catalog_is_asked_in_the_directory_with_the_setup() {
    let catalog = Catalog {
        agents: vec![CatalogEntry {
            name: "scratch-reviewer".into(),
            description: Some("Reviews".into()),
            source: Some("project".into()),
        }],
        skills: vec![],
    };
    let rec = Recording::from_jsonl(RECORDING).unwrap();
    let a = CopilotAdapter::new(
        ReplayBackend::new(rec, &[]).with_catalog(catalog.clone()),
        None,
        clock(),
    );
    let got = a
        .catalog(PathBuf::from("/work/repo"), setup())
        .await
        .unwrap();
    assert_eq!(got, catalog);
    assert_eq!(
        a.backend().catalogs(),
        vec![(PathBuf::from("/work/repo"), setup())]
    );
}

#[tokio::test]
async fn local_only_still_refuses_delegation_and_ampersand() {
    let a = adapter(&[]);
    let _updates = a.subscribe().await.unwrap();
    a.create(CreateSession {
        radar_session_id: sid(),
        cwd: PathBuf::from("/work/repo"),
        model: None,
        setup: setup(),
    })
    .await
    .unwrap();
    for text in ["/delegate fix it", "& fix it"] {
        let err = a
            .send(
                &sid(),
                SendInput { text: text.into() },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("locally only"), "{err}");
    }
}
