//! Deletes named Copilot sessions with the SDK's own delete (the CLI's `delete_session`).
//!
//! cargo run -p radar-sessions-copilot --example delete -- <provider-session-id>...

use radar_sessions_copilot::backend::Backend;
use radar_sessions_copilot::SdkBackend;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let backend = SdkBackend::new(None);
    for id in std::env::args().skip(1) {
        match backend.delete(&id).await {
            Ok(()) => eprintln!("deleted {id}"),
            Err(e) => eprintln!("could not delete {id}: {e}"),
        }
    }
    Ok(())
}
