//! One session's MCP listener, alone, for a live run of a CLI against radar's tool server
//! (a live check of radar's MCP server).
//!
//!   cargo run -p radar-sessions --example mcp_listen -- <socket> <radar binary> <server.json>
//!
//! Binds the listener at `<socket>`, writes the session's `ToolServer` (the `radar mcp` command
//! with its socket and ticket) to `<server.json>` for the recording script to set up the
//! session with, and prints one line for each `tools/call` that reaches it. Runs until killed.

use std::path::Path;

use radar_sessions::RadarSessionId;
use radar_sessions::tools::McpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, socket, radar, out] = &args[..] else {
        anyhow::bail!("usage: mcp_listen <socket> <radar binary> <server.json>");
    };
    let (tx, mut seen) = tokio::sync::mpsc::unbounded_channel();
    let listener = McpListener::bind(socket, RadarSessionId::new("spike"), tx)?;
    std::fs::write(out, serde_json::to_string(&listener.tool_server(Path::new(radar)))?)?;
    eprintln!("listening on {}", listener.path().display());
    while let Some(call) = seen.recv().await {
        println!(
            "{}",
            serde_json::json!({ "seen": call.tool, "session": call.session.as_str(), "arguments": call.arguments })
        );
    }
    Ok(())
}
