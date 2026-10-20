//! radar: the host plus the parts compiled into this build (see `lib.rs`).

fn main() -> anyhow::Result<()> {
    let tokio = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    // `radar mcp --socket <path> --ticket <t>`: the stdio relay an agent CLI starts as radar's MCP
    // server (radar-sessions `tools`). No config, log, journal or host: it only copies bytes.
    #[cfg(feature = "sessions")]
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        std::process::exit(tokio.block_on(radar_sessions::tools::relay_main(std::env::args().skip(2))));
    }
    let parts = radar::parts();
    let ids: Vec<&str> = parts.iter().map(|p| p.id()).collect();
    for (old, moved) in radar_core::runner::move_folders_from_gg(&radar_core::runner::folder_bases(), &ids) {
        if let Err(e) = moved {
            eprintln!("radar: could not move {} to its new name radar: {e}", old.display());
        }
    }
    unpack_runtime();
    tokio.block_on(radar_core::runner::main(parts))
}

/// A release build carries radar's runtime (the Ink renderer and the Claude Code
/// sidecar; build.rs): written to the cache folder on the first start of this
/// build and named by `RADAR_RUNTIME` for radar and what it starts. A `RADAR_RUNTIME`
/// already set wins.
#[cfg(radar_runtime)]
fn unpack_runtime() {
    use radar_core::runtime;
    if std::env::var_os(runtime::VAR).is_some() {
        return;
    }
    static BYTES: &[u8] = include_bytes!(env!("RADAR_RUNTIME_BLOB"));
    match runtime::unpack_into_cache(BYTES, env!("RADAR_RUNTIME_ID")) {
        // SAFETY: called first in main, before the async runtime or any other thread starts.
        Ok(path) => unsafe { std::env::set_var(runtime::VAR, path) },
        Err(e) => eprintln!("radar: cannot write its runtime into the cache folder: {e}"),
    }
}

#[cfg(not(radar_runtime))]
fn unpack_runtime() {}
