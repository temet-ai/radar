//! One live Codex session through the real adapter, for the record.
//!
//!   cargo run -p radar-sessions-codex --example live -- <scratch-git-dir>
//!
//! Prints status, models, then runs one tiny turn that asks to run `ls`. The one
//! command approval is answered ApproveOnce (a scratch directory); the session is
//! deleted with Codex's own delete. Prints event kinds, never tokens.

use std::time::Duration;

use radar_sessions::{
    Adapter, CreateSession, EventKind, RadarSessionId, PermissionAnswer, PermissionDecision,
    SendInput, Update,
};
use radar_sessions_codex::CodexAdapter;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let dir = std::env::args()
        .nth(1)
        .expect("scratch dir, or `delete <thread id>`");
    let a = CodexAdapter::new();
    if dir == "delete" {
        let id = std::env::args().nth(2).expect("thread id");
        a.delete_session(&radar_sessions::ProviderSessionId::new(id))
            .await?;
        println!("deleted");
        return Ok(());
    }
    let st = a.status().await;
    println!(
        "status: installed={} version={:?} auth={:?} error={:?}",
        st.installed, st.version, st.auth, st.error
    );
    println!("models: {:?}", a.list_models().await.map(|m| m.len()));
    let mut ups = a.subscribe().await?;
    let radar = RadarSessionId::new("live-1");
    let pid = a
        .create(CreateSession {
            radar_session_id: radar.clone(),
            cwd: dir.into(),
            model: None,
            setup: Default::default(),
        })
        .await?;
    println!("created thread (opaque id, {} chars)", pid.as_str().len());
    a.send(
        &radar,
        SendInput {
            text: "Run `ls` in the shell, then reply with one short sentence.".into(),
        },
    )
    .await?;
    let mut ended = false;
    let deadline = tokio::time::sleep(Duration::from_secs(120));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => { println!("timeout"); break; }
            u = ups.recv() => match u {
                Some(Update::Event(e)) => {
                    println!("seq {} {:?} {}", e.sequence, e.durability, serde_json::to_string(&e.kind)?.chars().take(160).collect::<String>());
                    match e.kind {
                        EventKind::PermissionRequested { request } => {
                            a.answer_permission(&radar, PermissionAnswer { request_id: request.request_id, decision: PermissionDecision::ApproveOnce }).await?;
                        }
                        EventKind::TurnEnded => ended = true,
                        EventKind::Idle if ended => break,
                        _ => {}
                    }
                }
                Some(o) => println!("update {o:?}"),
                None => break,
            }
        }
    }
    a.delete_session(&pid).await?;
    println!("deleted");
    Ok(())
}
