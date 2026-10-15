//! The skill catalog through the real adapter and the real `codex app-server`,
//! with no thread and no prompt.
//!
//!   cargo run -p radar-sessions-codex --example catalog -- <codex-home> <work-dir> [skill-dir]
//!
//! `<codex-home>` becomes CODEX_HOME for the app-server (a scratch folder keeps
//! the operator's own ~/.codex out of it).

use radar_sessions::adapter::{Adapter, ProviderSetup, SessionSetup};
use radar_sessions_codex::CodexAdapter;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let home = args.next().expect("codex home");
    let work = args.next().expect("work dir");
    let skills: Vec<_> = args.map(std::path::PathBuf::from).collect();
    let a = CodexAdapter::new();
    a.configure(ProviderSetup {
        executable: None,
        config_dir: Some(home.into()),
    });
    let cat = a
        .catalog(
            work.into(),
            SessionSetup {
                skill_dirs: skills,
                ..Default::default()
            },
        )
        .await?;
    println!("agents: {}", cat.agents.len());
    for s in cat.skills {
        println!(
            "skill {:<14} source={:?} description={:?}",
            s.name,
            s.source,
            s.description.map(|d| d.chars().take(48).collect::<String>())
        );
    }
    Ok(())
}
