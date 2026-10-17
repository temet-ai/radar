//! A small process-boundary suite using the actual runner, parser and SQLite
//! writer. This target has no libtest harness so its child processes can pass
//! ordinary `radar slack ...` arguments to `runner::main` unchanged.
//!
//! Run: cargo test -p radar --test slack_cli_history --no-default-features --features slack,test-support
//! Every provider reply is synthetic; the fixture has no HTTP fallback. The
//! same target runs on the PR24 baseline with only the fixture transport seam.

const CHILD: &str = "RADAR_SYNTHETIC_SLACK_CLI_TEST";
const TEST_NAME: &str = "slack_cli_history";

#[cfg(unix)]
mod unix {
    use super::CHILD;
    use radar_core::journal::work_map::{EventFilter, Limits, Selection};
    use radar_core::journal::{ActionEvent, Journal, Origin, Outcome, Query, Reason, WriteMode};
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use std::process::{Command, Output, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    const ALIAS: &str = "private-channel-alias-sentinel";
    const FILTER: &str = "private-filter-sentinel";
    const BODY: &str = "private-message-body-sentinel";
    const DISPLAY: &str = "private-display-name-sentinel";
    const RAW_ERROR: &str = "private-provider-error-sentinel";
    const SENTINELS: &[&str] = &[
        "xoxp-cli-token-sentinel",
        ALIAS,
        FILTER,
        BODY,
        DISPLAY,
        RAW_ERROR,
        "private-channel-name-sentinel",
        "private-channel-purpose-sentinel",
        "private-user-handle-sentinel",
        "private-workspace-name-sentinel",
        "private-profile-bio-sentinel",
        "private-url-query-sentinel",
        "private-recovery-sentinel",
        "UPRIVATEAUTH1",
        "UPRIVATEAUTHOR1",
        "UPRIVATEFALLBACK1",
    ];

    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new(two_channels: bool, token: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "radar-slack-cli-history-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(root.join("config/radar")).unwrap();
            let second = if two_channels {
                "\n[[slack.channel]]\nalias = 'second-private-alias'\nid = 'CCLI00002'\n"
            } else {
                ""
            };
            std::fs::write(
            root.join("config/radar/config.toml"),
            format!(
                "[slack]\ntoken = '{token}'\ndays = 7\n[[slack.channel]]\nalias = '{ALIAS}'\nid = 'CCLI00001'\nname = 'private-channel-name-sentinel'\npurpose = 'private-channel-purpose-sentinel'\n{second}"
            ),
        )
        .unwrap();
            Self(root)
        }

        fn run(&self, scenario: &str, args: &[&str]) -> Output {
            Command::new(std::env::current_exe().unwrap())
                .args(args)
                .env_clear()
                .env("RADAR_SANDBOX", "1")
                .env(CHILD, scenario)
                .env("HOME", self.0.join("home"))
                .env("XDG_CONFIG_HOME", self.0.join("config"))
                .env("XDG_STATE_HOME", self.0.join("state"))
                .env("XDG_CACHE_HOME", self.0.join("cache"))
                .env("RADAR_MODE", "read-only")
                .env("RADAR_NO_WRITES", "1")
                .env("NO_COLOR", "1")
                .stdin(Stdio::null())
                .output()
                .unwrap()
        }

        fn journal_path(&self) -> PathBuf {
            self.0.join("state/radar/journal/actions.sqlite3")
        }

        fn events(&self) -> Vec<ActionEvent> {
            assert!(
                self.journal_path().is_file(),
                "runner did not create its journal"
            );
            Journal::open(&self.journal_path())
                .unwrap()
                .query(&Query {
                    limit: 1000,
                    ..Query::default()
                })
                .unwrap()
        }

        fn block_journal(&self) {
            std::fs::create_dir_all(self.0.join("state/radar")).unwrap();
            std::fs::write(self.0.join("state/radar/journal"), b"not a directory").unwrap();
        }

        fn assert_private_storage_and_exports(&self, events: &[ActionEvent]) {
            let journal = Journal::open(&self.journal_path()).unwrap();
            assert_eq!(journal.work_map_snapshot().unwrap().map_sequence(), 0);
            let page = journal
                .query_events(&EventFilter::default(), Limits::default(), None)
                .unwrap();
            assert!(
                !page.truncated,
                "fixture export must include every observation"
            );
            let exported = serde_json::to_vec(&page).unwrap();
            assert_private(&exported);
            let source: Value = serde_json::from_slice(&exported).unwrap();
            assert!(
                source["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|e| e["source"]["session_origin"] == "cli")
            );
            let mut actions = Vec::new();
            for event in events {
                if !actions.contains(&event.action_id) {
                    actions.push(event.action_id.clone());
                }
            }
            let handoff = journal
                .handoff(
                    &Selection {
                        items: vec![],
                        actions,
                    },
                    Limits::default(),
                    None,
                )
                .unwrap();
            let handoff = serde_json::to_vec(&handoff).unwrap();
            assert_private(&handoff);
            let handoff: Value = serde_json::from_slice(&handoff).unwrap();
            assert_eq!(handoff["truncated"], false);
            assert_eq!(handoff["authority"], "observations_only");
            assert_eq!(handoff["execution_authorization"], "unspecified");
            drop(journal);
            // Scan SQLite itself and any live WAL/SHM, not just a sanitized JSON
            // view. Config and CLI output deliberately retain these sentinels.
            for entry in std::fs::read_dir(self.journal_path().parent().unwrap()).unwrap() {
                let path = entry.unwrap().path();
                if path.is_file() {
                    assert_private(&std::fs::read(path).unwrap());
                }
            }
            assert!(
                !self.0.join("state/radar/slack.json").exists(),
                "read-only CLI must not persist provider cache or caught-up marks"
            );
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn assert_private(bytes: &[u8]) {
        for sentinel in SENTINELS {
            assert!(
                !bytes
                    .windows(sentinel.len())
                    .any(|window| window == sentinel.as_bytes()),
                "private sentinel leaked into journal or export: {sentinel}"
            );
        }
    }

    fn success(output: &Output) {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn terminal<'a>(events: &'a [ActionEvent], kind: &str) -> &'a ActionEvent {
        let matches: Vec<_> = events
            .iter()
            .filter(|event| event.kind.as_str() == kind && event.outcome.is_terminal())
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "required Slack CLI provider evidence missing or duplicated: {kind}"
        );
        matches[0]
    }

    fn fact(events: &[ActionEvent], kind: &str, outcome: Outcome, count: u32) {
        let event = terminal(events, kind);
        assert_eq!(event.outcome, outcome, "{kind}");
        assert_eq!(event.metadata.count, Some(count), "{kind}");
    }

    fn correlation(events: &[ActionEvent], command: &str, outcome: Outcome) {
        let terminal_command = terminal(events, "cli.command");
        let started = events
            .iter()
            .find(|event| event.kind.as_str() == "cli.command" && event.outcome == Outcome::Started)
            .unwrap();
        assert_eq!(terminal_command.outcome, outcome);
        let requested = events
            .iter()
            .find(|event| {
                event.kind.as_str() == "cli.command" && event.outcome == Outcome::Requested
            })
            .unwrap();
        assert_eq!(
            requested.metadata.command.as_ref().unwrap().as_str(),
            command
        );
        assert_eq!(terminal(events, "session.cli").outcome, outcome);
        for event in events
            .iter()
            .filter(|event| event.kind.as_str().starts_with("slack.cli."))
        {
            assert_eq!(
                event.parent_action_id.as_ref(),
                Some(&terminal_command.action_id)
            );
            assert_eq!(event.root_action_id, terminal_command.root_action_id);
            assert_eq!(event.correlation_id, terminal_command.correlation_id);
            assert_eq!(event.session_id, terminal_command.session_id);
            assert_eq!(event.origin, Origin::Plugin("slack".into()));
            assert_eq!(event.metadata.mode, Some(WriteMode::ReadOnly));
            assert!(
                event.sequence > started.sequence && event.sequence < terminal_command.sequence
            );
            assert_ne!(
                event.outcome,
                Outcome::Started,
                "a completed fact must not invent a provider start timestamp"
            );
        }
    }

    fn list_required_reads_and_optional_names_are_separate() {
        let sandbox = Sandbox::new(false, "xoxp-cli-token-sentinel");
        let args = [
            "slack", "list", "--view", ALIAS, "--filter", FILTER, "--json",
        ];
        let output = sandbox.run("list", &args);
        success(&output);
        let rows: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["author"]["name"], DISPLAY);
        assert!(rows[0]["text"].as_str().unwrap().contains(BODY));
        let events = sandbox.events();
        println!(
            "list fixture: exit=0, returned_rows=1, recorded_events={}, provider_history_events={}",
            events.len(),
            events
                .iter()
                .filter(|event| event.kind.as_str() == "slack.cli.history.messages")
                .count()
        );
        // This is deliberately first: on the unmodified PR24 production code,
        // output succeeds but this assertion fails because provider evidence is absent.
        fact(&events, "slack.cli.history.messages", Outcome::Succeeded, 3);
        fact(&events, "slack.cli.auth", Outcome::Succeeded, 1);
        fact(&events, "slack.cli.identity", Outcome::Succeeded, 1);
        fact(&events, "slack.cli.names.attempted", Outcome::Partial, 2);
        fact(&events, "slack.cli.names.returned", Outcome::Succeeded, 1);
        fact(&events, "slack.cli.names.fallback", Outcome::Partial, 1);
        let history = terminal(&events, "slack.cli.history.messages");
        assert!(!history.metadata.truncated);
        assert_eq!(history.subjects.len(), 1);
        assert_eq!(history.subjects[0].key.provider, "slack");
        assert_eq!(history.subjects[0].key.instance, "TCLI1");
        assert_eq!(history.subjects[0].key.kind, "channel");
        assert_eq!(history.subjects[0].key.id, "CCLI00001");
        assert!(terminal(&events, "slack.cli.auth").subjects.is_empty());
        correlation(&events, "slack.list", Outcome::Succeeded);
        sandbox.assert_private_storage_and_exports(&events);

        let unavailable = Sandbox::new(false, "xoxp-cli-token-sentinel");
        unavailable.block_journal();
        let without = unavailable.run("list", &args);
        success(&without);
        assert_eq!(
            without.stdout, output.stdout,
            "unavailable history must not change command output"
        );
        assert!(String::from_utf8_lossy(&without.stderr).contains("action recording unavailable"));
        assert!(!unavailable.journal_path().exists());
    }

    fn catchup_incomplete_collection_is_partial_without_inventing_a_cutoff() {
        let sandbox = Sandbox::new(false, "xoxp-cli-token-sentinel");
        let output = sandbox.run("catchup", &["slack", "catchup", ALIAS, "--json"]);
        success(&output);
        let rows: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["author"]["name"], "UPRIVATEAUTHOR1");
        let events = sandbox.events();
        fact(&events, "slack.cli.history.messages", Outcome::Partial, 1);
        fact(
            &events,
            "slack.cli.history.pages.incomplete",
            Outcome::Partial,
            1,
        );
        fact(&events, "slack.cli.names.fallback", Outcome::Partial, 1);
        assert!(
            !terminal(&events, "slack.cli.history.messages")
                .metadata
                .truncated
        );
        assert!(
            !events
                .iter()
                .any(|event| event.kind.as_str() == "slack.cli.history.lists.truncated")
        );
        correlation(&events, "slack.catchup", Outcome::Succeeded);
        sandbox.assert_private_storage_and_exports(&events);
    }

    fn later_required_failure_keeps_prior_channel_evidence_and_cli_failure() {
        let sandbox = Sandbox::new(true, "xoxp-cli-token-sentinel");
        let output = sandbox.run("required-failure", &["slack", "list", "--json"]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let events = sandbox.events();
        fact(&events, "slack.cli.history.messages", Outcome::Succeeded, 0);
        fact(&events, "slack.cli.history.errors", Outcome::Failed, 1);
        assert_eq!(
            terminal(&events, "slack.cli.history.messages").subjects[0]
                .key
                .id,
            "CCLI00001"
        );
        assert_eq!(
            terminal(&events, "slack.cli.history.errors").subjects[0]
                .key
                .id,
            "CCLI00002"
        );
        correlation(&events, "slack.list", Outcome::Failed);
        sandbox.assert_private_storage_and_exports(&events);

        let unavailable = Sandbox::new(true, "xoxp-cli-token-sentinel");
        unavailable.block_journal();
        let without = unavailable.run("required-failure", &["slack", "list", "--json"]);
        assert!(!without.status.success());
        assert_eq!(without.stdout, output.stdout);
        let warning = "radar: action recording unavailable: could not open private journal storage\n";
        assert_eq!(
            String::from_utf8(without.stderr)
                .unwrap()
                .replace(warning, ""),
            String::from_utf8(output.stderr).unwrap()
        );
    }

    fn local_refusals_do_not_claim_a_provider_request() {
        for (token, args, reason) in [
            (
                "xoxp-cli-token-sentinel",
                vec![
                    "slack",
                    "list",
                    "--view",
                    "private-filter-sentinel",
                    "--json",
                ],
                Reason::InvalidInput,
            ),
            ("", vec!["slack", "catchup", "--json"], Reason::Unavailable),
        ] {
            let sandbox = Sandbox::new(false, token);
            let output = sandbox.run("no-calls", &args);
            assert!(!output.status.success());
            let events = sandbox.events();
            fact(&events, "slack.cli.refused", Outcome::Blocked, 1);
            let refusal = terminal(&events, "slack.cli.refused");
            assert_eq!(refusal.metadata.reason, Some(reason));
            assert!(refusal.subjects.is_empty());
            assert!(
                !events
                    .iter()
                    .any(|event| event.kind.as_str() == "slack.cli.auth")
            );
            sandbox.assert_private_storage_and_exports(&events);
        }
    }

    fn cache_recovery_remains_local_without_provider_facts() {
        let sandbox = Sandbox::new(false, "");
        let cache = sandbox.0.join("state/radar/slack.json");
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        let original = json!({"legacy":"private-recovery-sentinel"});
        std::fs::write(&cache, original.to_string()).unwrap();
        let before = std::fs::read(&cache).unwrap();
        let output = sandbox.run("no-calls", &["slack", "cache-recovery"]);
        success(&output);
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            json!([original])
        );
        // The host imported the old file into its cache database once and
        // kept its bytes under a new name; the next start reads the database.
        assert!(!cache.exists());
        assert_eq!(
            std::fs::read(sandbox.0.join("state/radar/slack.json.imported")).unwrap(),
            before
        );
        let events = sandbox.events();
        assert!(
            !events
                .iter()
                .any(|event| event.kind.as_str().starts_with("slack.cli."))
        );
        correlation(&events, "slack.cache-recovery", Outcome::Succeeded);
        // The original recovery file is intentionally retained and never a journal
        // payload; only the SQLite store is scanned here.
        assert_private(&std::fs::read(sandbox.journal_path()).unwrap());
        // The next start restores the same cache from the database.
        let again = sandbox.run("no-calls", &["slack", "cache-recovery"]);
        success(&again);
        assert_eq!(
            serde_json::from_slice::<Value>(&again.stdout).unwrap(),
            json!([original])
        );
    }

    fn auth() -> (String, Value) {
        (
            "auth.test".into(),
            json!({"ok":true,"team_id":"TCLI1","user_id":"UPRIVATEAUTH1","user":"private-user-handle-sentinel","team":"private-workspace-name-sentinel","url":"https://fixture.slack.com/"}),
        )
    }

    fn scripted(scenario: &str) -> Vec<(String, Value)> {
        match scenario {
            "list" => vec![
                auth(),
                (
                    "conversations.history".into(),
                    json!({"ok":true,"messages":[
                {"ts":"1780000000.000001","user":"UPRIVATEAUTHOR1","text":format!("{BODY} {FILTER} <https://example.test/?private-url-query-sentinel|private link>"),"reply_count":2},
                {"ts":"1780000000.000002","user":"UPRIVATEFALLBACK1","text":"filtered-out private body"},
                {"ts":"1780000000.000003","text":"filtered-out bot body","bot_profile":{"name":"private bot"}}
            ],"has_more":false,"response_metadata":{"next_cursor":""}}),
                ),
                (
                    "users.info".into(),
                    json!({"ok":true,"user":{"id":"UPRIVATEAUTHOR1","name":"private-user-handle-sentinel","profile":{"display_name":DISPLAY,"bio":"private-profile-bio-sentinel"}}}),
                ),
                ("users.info".into(), json!({"ok":false,"error":RAW_ERROR})),
            ],
            "catchup" => vec![
                auth(),
                (
                    "conversations.history".into(),
                    json!({"ok":true,"messages":[{"ts":"1780000000.000001","user":"UPRIVATEAUTHOR1","text":BODY}],"has_more":true}),
                ),
                ("users.info".into(), json!({"ok":false,"error":RAW_ERROR})),
            ],
            "required-failure" => vec![
                auth(),
                (
                    "conversations.history".into(),
                    json!({"ok":true,"messages":[],"has_more":false}),
                ),
                (
                    "conversations.history".into(),
                    json!({"ok":false,"error":RAW_ERROR}),
                ),
            ],
            "no-calls" => vec![],
            _ => panic!("unknown synthetic fixture scenario"),
        }
    }

    pub fn run() -> anyhow::Result<()> {
        if let Ok(scenario) = std::env::var(CHILD) {
            let (plugin, fixture) = radar_slack::test_support::cli_plugin(scripted(&scenario));
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let result = runtime.block_on(radar_core::runner::main(
                vec![Box::new(plugin)],
                radar_core::runner::Renderer::Ratatui,
            ));
            fixture.assert_finished();
            return result;
        }
        list_required_reads_and_optional_names_are_separate();
        catchup_incomplete_collection_is_partial_without_inventing_a_cutoff();
        later_required_failure_keeps_prior_channel_evidence_and_cli_failure();
        local_refusals_do_not_claim_a_provider_request();
        cache_recovery_remains_local_without_provider_facts();
        println!("5 Slack CLI runner integration checks passed (synthetic transport only)");
        Ok(())
    }
}

/// The single aggregate case follows nextest's custom-harness discovery and
/// exact-invocation protocol. Parsing discovery must never start the runner.
fn selected() -> anyhow::Result<bool> {
    let mut args = std::env::args().skip(1);
    let mut list = false;
    let mut ignored = false;
    let mut exact = false;
    let mut filters = Vec::new();
    let mut skip = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--list" => list = true,
            "--ignored" | "--bench" => ignored = true,
            "--exact" => exact = true,
            "--nocapture" | "--show-output" | "--include-ignored" | "--test" | "--quiet" | "-q" => {
            }
            "--format" | "--color" | "--test-threads" => {
                anyhow::ensure!(args.next().is_some(), "missing harness option value");
            }
            "--skip" => skip.push(
                args.next()
                    .ok_or_else(|| anyhow::anyhow!("missing skip filter"))?,
            ),
            "--help" | "-h" => {
                println!(
                    "{TEST_NAME}: synthetic Slack CLI runner checks\n  --list --format terse    list without running\n  --ignored               select ignored tests (none)\n  --exact                 match the full test name\n  --skip FILTER           exclude matching tests"
                );
                return Ok(false);
            }
            _ if ["--format=", "--color=", "--test-threads="]
                .iter()
                .any(|prefix| arg.starts_with(prefix)) => {}
            _ if arg.starts_with('-') => anyhow::bail!("unknown test harness option"),
            _ => filters.push(arg),
        }
    }
    let matched = !ignored
        && (filters.is_empty()
            || filters.iter().any(|filter| {
                if exact {
                    filter == TEST_NAME
                } else {
                    TEST_NAME.contains(filter.as_str())
                }
            }))
        && !skip
            .iter()
            .any(|filter| TEST_NAME.contains(filter.as_str()));
    if list {
        if matched {
            println!("{TEST_NAME}: test");
        }
        return Ok(false);
    }
    Ok(matched)
}

#[cfg(unix)]
fn harness_protocol_is_clean() {
    use std::process::Command;
    let exe = std::env::current_exe().unwrap();
    let listed = format!("{TEST_NAME}: test\n");
    for (args, expected) in [
        (vec!["--list", "--format", "terse"], listed.as_str()),
        (vec!["--ignored", "--list", "--format", "terse"], ""),
        (
            vec![TEST_NAME, "--exact", "--list", "--format", "terse"],
            listed.as_str(),
        ),
        (vec!["absent_case", "--nocapture", "--exact"], ""),
        (vec!["--skip", TEST_NAME], ""),
    ] {
        let output = Command::new(&exe)
            .env_remove(CHILD)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "harness discovery or filtering failed"
        );
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        assert!(
            output.stderr.is_empty(),
            "discovery must not start the CLI runner"
        );
    }
}

fn main() -> anyhow::Result<()> {
    // Actual CLI arguments belong to the runner, including --help or strings
    // matching harness flags. The fixture child takes priority over discovery.
    if std::env::var_os(CHILD).is_some() {
        #[cfg(unix)]
        {
            return unix::run();
        }
        #[cfg(not(unix))]
        {
            anyhow::bail!("synthetic journal fixture requires Unix");
        }
    }
    if !selected()? {
        return Ok(());
    }
    #[cfg(unix)]
    {
        harness_protocol_is_clean();
        unix::run()
    }
    #[cfg(not(unix))]
    {
        println!(
            "Slack CLI persisted-history integration skipped: private journal storage is Unix-only"
        );
        Ok(())
    }
}
