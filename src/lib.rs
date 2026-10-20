//! The parts compiled into this build of radar, each behind its cargo feature.
//! Adding a part is one feature in `Cargo.toml` and one line in each list
//! here.

// One `push` per part, each behind its feature, so a part is one line.
#![allow(unused_mut, clippy::vec_init_then_push)]

/// Every part this build has, in the order the host lists them (screens,
/// settings, and contributions to each other's screens).
pub fn parts() -> Vec<Box<dyn radar_core::Plugin>> {
    let mut parts: Vec<Box<dyn radar_core::Plugin>> = Vec::new();
    #[cfg(feature = "gitlab")]
    parts.push(Box::new(radar_gitlab::GitlabPlugin::new()));
    #[cfg(feature = "github")]
    parts.push(Box::new(radar_github::GithubPlugin::default()));
    #[cfg(feature = "jira")]
    parts.push(Box::new(radar_jira::JiraPlugin::new()));
    #[cfg(feature = "slack")]
    parts.push(Box::new(radar_slack::SlackPlugin::new()));
    // Last: what it adds to repos and merge requests (c e t s) gives way to
    // the parts before it where they use a key.
    #[cfg(feature = "git")]
    parts.push(Box::new(radar_git::GitPlugin::new()));
    // After git: its actions on tickets give way to the parts before it.
    #[cfg(feature = "workspace")]
    parts.push(Box::new(radar_workspace::WorkspacePlugin::new()));
    #[cfg(feature = "workbench")]
    parts.push(Box::new(radar_workbench::WorkbenchPlugin::default()));
    #[cfg(feature = "sessions")]
    {
        // `RADAR_FAKE_AGENT=1` adds the deterministic fake provider (no CLI needed).
        let sessions = radar_sessions::SessionsPlugin::new().with_fake_from_env();
        // A CLI configured in `[[sessions.pty]]` gets a terminal tab of its own.
        #[cfg(feature = "sessions-pty")]
        let sessions = sessions.with_terminal_factory(|cli| Box::new(radar_sessions_pty::for_cli(cli)));
        let mut sessions = sessions;
        // Copilot through the official SDK, driving the installed `copilot` (found on PATH).
        #[cfg(feature = "sessions-copilot")]
        sessions.add_adapter(Box::new(radar_sessions_copilot::copilot(None)));
        // Claude Code through the Agent SDK sidecar, driving the installed `claude`.
        #[cfg(feature = "sessions-claude")]
        sessions.add_adapter(Box::new(radar_sessions_claude::ClaudeAdapter::from_env(radar_sessions_claude::ClaudeConfig::default())));
        // Codex through `codex app-server`, driving the installed `codex`.
        #[cfg(feature = "sessions-codex")]
        sessions.add_adapter(Box::new(radar_sessions_codex::CodexAdapter::new()));
        // Last, so a native provider stays the default; a terminal tab runs any CLI's own screen.
        #[cfg(feature = "sessions-pty")]
        for adapter in radar_sessions_pty::standard_adapters() {
            sessions.add_adapter(Box::new(adapter));
        }
        parts.push(Box::new(sessions));
    }
    parts
}

/// The features of this build that are parts, by name, for tests per mix.
pub fn mix() -> Vec<&'static str> {
    let mut mix = Vec::new();
    #[cfg(feature = "gitlab")]
    mix.push("gitlab");
    #[cfg(feature = "github")]
    mix.push("github");
    #[cfg(feature = "jira")]
    mix.push("jira");
    #[cfg(feature = "slack")]
    mix.push("slack");
    #[cfg(feature = "git")]
    mix.push("git");
    #[cfg(feature = "workspace")]
    mix.push("workspace");
    #[cfg(feature = "workbench")]
    mix.push("workbench");
    #[cfg(feature = "sessions")]
    mix.push("sessions");
    mix
}
