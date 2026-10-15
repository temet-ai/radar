//! GitHub Copilot as a radar-sessions adapter, through the official Rust SDK
//! (`github-copilot-sdk`, pinned) driving the user's installed `copilot`.
//!
//! - [`adapter`]: the `Adapter` impl, generic over a [`backend::Backend`].
//! - [`sdk`]: the real backend. [`replay`]: a backend that replays a recorded stream.
//! - [`setup`]: a radar `SessionSetup` as the SDK's session options; agents and skills as a catalog.
//! - [`map`]: Copilot events into the radar envelope.
//! - [`login`]: sign-in through `copilot login --device-code`; no token passes through radar.

pub mod adapter;
pub mod backend;
pub mod commands;
pub mod login;
pub mod map;
pub mod replay;
pub mod sdk;
pub mod setup;
pub mod sim;

pub use adapter::CopilotAdapter;
pub use sdk::SdkBackend;

/// The adapter over the real CLI. `program` is the configured `copilot` executable;
/// `None` finds it on `PATH`. Without one installed the adapter says so in its status.
pub fn copilot(program: Option<std::path::PathBuf>) -> CopilotAdapter<SdkBackend> {
    CopilotAdapter::new(
        SdkBackend::new(program.clone()),
        program,
        adapter::system_clock(),
    )
}
