//! Lists what the installed `copilot` offers a session: agents and skills.
//!
//!   cargo run -p radar-sessions-copilot --example catalog -- <cwd> [skill-dir] [project-config: true|false] [copilot-home]
//!
//! With project config on and no plugin folders the CLI's own discovery answers and no
//! session is created. With it off the adapter starts one probe session, lists, and
//! deletes it. No prompt is ever sent. A `copilot-home` runs the CLI with that
//! `COPILOT_HOME` (a scratch one is signed out).

use std::path::PathBuf;

use radar_sessions::adapter::{ProviderSetup, SessionSetup};
use radar_sessions::Adapter;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let cwd = PathBuf::from(args.next().ok_or_else(|| anyhow::anyhow!("a directory"))?);
    let skill_dirs: Vec<PathBuf> = args.next().map(PathBuf::from).into_iter().collect();
    let project_config = args.next().map(|a| a != "false").unwrap_or(true);
    let adapter = radar_sessions_copilot::copilot(None);
    if let Some(home) = args.next() {
        adapter.configure(ProviderSetup {
            executable: None,
            config_dir: Some(PathBuf::from(home)),
        });
    }
    let catalog = adapter
        .catalog(
            cwd,
            SessionSetup {
                skill_dirs,
                project_config,
                local_only: true,
                ..Default::default()
            },
        )
        .await?;
    for (kind, entries) in [("agent", &catalog.agents), ("skill", &catalog.skills)] {
        for e in entries {
            println!(
                "{kind}\t{}\t{}\t{}",
                e.name,
                e.source.as_deref().unwrap_or("-"),
                e.description.as_deref().unwrap_or("-")
            );
        }
    }
    Ok(())
}
