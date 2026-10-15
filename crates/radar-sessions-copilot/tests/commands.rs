//! Slash commands through the Copilot adapter, over a backend that answers like
//! `session.commands.list` / `.invoke` did on the live CLI (shapes in src/commands.rs tests).
//! The CLI does not read a slash in a message, so the adapter must route a command to
//! `invoke` and send everything else as written.

use std::path::Path;
use std::sync::{Arc, Mutex};

use radar_sessions::adapter::Updates;
use radar_sessions::{
    Adapter, BoxFuture, CreateSession, EventKind, RadarSessionId, ModelInfo, ProviderSessionInfo, SendInput,
    SlashCommand, Update,
};
use radar_sessions_copilot::adapter::CopilotAdapter;
use radar_sessions_copilot::backend::{Backend, BackendStatus, Sink, StatusError};
use radar_sessions_copilot::commands::{CommandInfo, Invoked};

/// What the adapter asked of the backend, in order.
type Log = Arc<Mutex<Vec<String>>>;

struct Stub {
    log: Log,
}

fn info(name: &str, aliases: &[&str], description: &str, hint: Option<&str>) -> CommandInfo {
    CommandInfo {
        name: name.into(),
        aliases: aliases.iter().map(|a| a.to_string()).collect(),
        description: description.into(),
        hint: hint.map(str::to_string),
    }
}

impl Backend for Stub {
    fn status(&self) -> BoxFuture<'_, Result<BackendStatus, StatusError>> {
        Box::pin(async { Err(StatusError::Missing) })
    }
    fn models(&self) -> BoxFuture<'_, anyhow::Result<Vec<ModelInfo>>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn create<'a>(&'a self, _: &'a RadarSessionId, _: &'a Path, _: Option<&'a str>, _: &'a radar_sessions::SessionSetup, _: Arc<dyn Sink>) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn resume<'a>(&'a self, _: &'a RadarSessionId, _: &'a Path, _: &'a radar_sessions::SessionSetup, _: Arc<dyn Sink>) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn send<'a>(&'a self, _: &'a RadarSessionId, text: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        self.log.lock().unwrap().push(format!("send {text}"));
        Box::pin(async { Ok(()) })
    }
    fn abort<'a>(&'a self, _: &'a RadarSessionId) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn delete<'a>(&'a self, _: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn list(&self) -> BoxFuture<'_, anyhow::Result<Vec<ProviderSessionInfo>>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn commands<'a>(&'a self, _: &'a RadarSessionId) -> BoxFuture<'a, anyhow::Result<Vec<CommandInfo>>> {
        Box::pin(async {
            Ok(vec![
                info("allow-all", &["yolo"], "Enable all permissions", Some("[on|off|show]")),
                info("compact", &[], "Summarize conversation history to reduce context window usage. Optionally provide focus instructions.", Some("focus instructions")),
                info("env", &[], "Show loaded environment details", None),
                info("model", &["models"], "Select the AI model for this session", Some("model")),
                info("plan", &[], "Create an implementation plan before coding", Some("prompt")),
                info("mcp", &[], "Manage MCP server configuration", None),
            ])
        })
    }
    fn invoke<'a>(&'a self, _: &'a RadarSessionId, name: &'a str, input: Option<&'a str>) -> BoxFuture<'a, anyhow::Result<Invoked>> {
        self.log.lock().unwrap().push(format!("invoke {name} {}", input.unwrap_or("-")));
        let answer = match name {
            "env" => Ok(Invoked::Text("**MCP Servers**\n\n- mike (connected)".into())),
            "model" => Ok(Invoked::Done(None)),
            "plan" => Ok(Invoked::Prompt { prompt: "[[PLAN]] Create a plan. My request: x".into(), display: input.unwrap_or_default().into(), plan: true }),
            "compact" => Err(anyhow::anyhow!("RPC error -32603: Request session.commands.invoke failed with message: Nothing to compact.")),
            other => panic!("the adapter invoked /{other}, which radar does not run"),
        };
        Box::pin(async move { answer })
    }
    fn send_prompt<'a>(&'a self, _: &'a RadarSessionId, prompt: &'a str, display: &'a str, plan: bool) -> BoxFuture<'a, anyhow::Result<()>> {
        self.log.lock().unwrap().push(format!("prompt plan={plan} shown={display} :: {prompt}"));
        Box::pin(async { Ok(()) })
    }
}

struct Run {
    adapter: CopilotAdapter<Stub>,
    updates: Updates,
    log: Log,
    radar: RadarSessionId,
}

async fn started() -> Run {
    let log = Log::default();
    let adapter = CopilotAdapter::new(Stub { log: log.clone() }, None, Arc::new(|| 1_700_000_000));
    let updates = adapter.subscribe().await.unwrap();
    let radar = RadarSessionId::new("g1");
    adapter.create(CreateSession { radar_session_id: radar.clone(), cwd: std::env::temp_dir(), model: None, setup: Default::default() }).await.unwrap();
    Run { adapter, updates, log, radar }
}

impl Run {
    async fn say(&self, text: &str) {
        self.adapter.send(&self.radar, SendInput { text: text.into() }).await.unwrap();
    }

    fn notices(&mut self) -> Vec<String> {
        let mut out = vec![];
        while let Ok(u) = self.updates.try_recv() {
            if let Update::Event(e) = u
                && let EventKind::Notice { text } = e.kind
            {
                out.push(text);
            }
        }
        out
    }
}

#[tokio::test]
async fn the_menu_has_the_commands_radar_runs_and_none_that_hand_the_agent_more_freedom() {
    let mut run = started().await;
    let Some(Update::Commands { session, commands }) = run.updates.recv().await else { panic!("the session reports its commands") };
    assert_eq!(session, run.radar);
    assert_eq!(
        commands,
        vec![
            SlashCommand::new("compact", "Summarize conversation history to reduce context window usage.").with_hint("focus instructions"),
            SlashCommand::new("env", "Show loaded environment details"),
            SlashCommand::new("model", "Select the AI model for this session").with_hint("model"),
            SlashCommand::new("plan", "Create an implementation plan before coding").with_hint("prompt"),
        ]
    );
}

#[tokio::test]
async fn a_command_runs_through_invoke_and_its_text_lands_in_the_transcript() {
    let mut run = started().await;
    run.say("/env").await;
    run.say("/MODELS auto").await;
    // Neither reached the model as a sentence.
    assert_eq!(*run.log.lock().unwrap(), vec!["invoke env -", "invoke model auto"]);
    let got = run.notices();
    assert_eq!(got, vec!["/env\n**MCP Servers**\n\n- mike (connected)", "/MODELS auto\ndone"]);
}

#[tokio::test]
async fn a_command_that_answers_with_a_prompt_submits_it_shown_as_the_cli_says() {
    let run = started().await;
    run.say("/plan add a readme").await;
    assert_eq!(*run.log.lock().unwrap(), vec!["invoke plan add a readme", "prompt plan=true shown=add a readme :: [[PLAN]] Create a plan. My request: x"]);
}

#[tokio::test]
async fn a_failing_command_says_why_in_the_transcript_and_the_session_stays_well() {
    let mut run = started().await;
    run.say("/compact").await;
    assert_eq!(run.notices(), vec!["/compact\nNothing to compact."]);
}

#[tokio::test]
async fn a_command_radar_does_not_run_is_explained_not_sent_to_the_model_and_not_invoked() {
    let mut run = started().await;
    run.say("/yolo on").await;
    run.say("/mcp").await;
    assert!(run.log.lock().unwrap().is_empty(), "nothing was invoked or sent: {:?}", run.log.lock().unwrap());
    let got = run.notices();
    assert!(got[0].starts_with("/yolo on\nradar does not run /allow-all:") && got[0].contains("operator answers"), "{got:?}");
    assert!(got[1].contains("terminal tab"), "{got:?}");
}

#[tokio::test]
async fn text_that_is_not_one_of_the_sessions_commands_is_sent_as_written() {
    let run = started().await;
    run.say("/etc/hosts has a typo").await;
    run.say("/nonesuch now").await;
    run.say("explain /env").await;
    assert_eq!(*run.log.lock().unwrap(), vec!["send /etc/hosts has a typo", "send /nonesuch now", "send explain /env"]);
}
