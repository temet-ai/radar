//! Explicit offline fixtures for cross-part tests. This is not a cache loader:
//! supplied identity is ignored, the account is fixed synthetic demo data, and
//! provider clients, CLI reads and picture fetches stay offline across config and demo resets.
//! The separate scripted CLI fixture uses the real read parser with an in-memory,
//! fail-closed transport. Neither fixture is selected by normal startup.

mod hosted;
pub use hosted::{
    AcceptedPost, ExpectedRequest, HostedFixture, Reply, Request, RequestRecord, hosted_plugin,
};

use crate::{Settings, SlackPlugin, cache::AccountOwner, demo, model::Data};

/// Build synthetic account-local rows for an integration test. Never used by
/// normal startup, cache restore, settings authentication, or the CLI.
pub fn offline_plugin(mut cfg: Settings, data: serde_json::Value) -> anyhow::Result<SlackPlugin> {
    let mut p = SlackPlugin::new();
    p.data = serde_json::from_value::<Data>(data)?;
    p.data.me = demo::sample().data.me;
    p.session_owner = p.data.me.as_ref().and_then(AccountOwner::of);
    cfg.token = "xoxp-offline-synthetic-fixture".into();
    p.cfg = cfg;
    p.offline_fixture = true;
    // The fixture's clock is the demo's, so "5d ago" does not move with the date.
    p.now = radar_core::plugin::DEMO_NOW;
    Ok(p)
}

/// A guard for scripted CLI reads. Check it even when the command is expected to
/// fail: a swallowed optional read must not hide an unexpected provider call.
#[derive(Clone)]
pub struct CliFixture(std::sync::Arc<std::sync::Mutex<CliScript>>);

#[derive(Default)]
struct CliScript {
    replies: std::collections::VecDeque<(String, serde_json::Value)>,
    calls: Vec<String>,
    violations: usize,
}

impl CliFixture {
    pub fn assert_finished(&self) {
        let script = self.0.lock().unwrap();
        assert_eq!(
            script.violations, 0,
            "scripted CLI transport contract violated"
        );
        assert!(
            script.replies.is_empty(),
            "scripted CLI replies left unread"
        );
    }

    pub fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap().calls.clone()
    }
}

impl crate::client::Transport for CliFixture {
    fn send<'a>(
        &'a self,
        request: &'a crate::client::Request,
    ) -> crate::client::Fut<'a, anyhow::Result<crate::client::Reply>> {
        Box::pin(async move {
            let mut script = self.0.lock().unwrap();
            script.calls.push(request.method.into());
            if request.write || request.body.is_some() {
                script.violations += 1;
                anyhow::bail!("scripted CLI fixture refuses writes");
            }
            let expected = script.replies.pop_front();
            let Some((method, body)) = expected else {
                script.violations += 1;
                anyhow::bail!("scripted CLI fixture exhausted");
            };
            if request.method != method {
                script.violations += 1;
                anyhow::bail!("scripted CLI fixture method mismatch");
            }
            Ok(crate::client::Reply {
                status: 200,
                body: body.to_string(),
                ..Default::default()
            })
        })
    }
}

/// Build a strict synthetic sequence of JSON replies for the real CLI runner.
/// Config and cache still load normally. A test must provide only synthetic
/// inputs and call the returned guard's `assert_finished` after the runner exits.
/// There is no network fallback, including when the script is exhausted.
pub fn cli_plugin(replies: Vec<(String, serde_json::Value)>) -> (SlackPlugin, CliFixture) {
    let fixture = CliFixture(std::sync::Arc::new(std::sync::Mutex::new(CliScript {
        replies: replies.into(),
        ..Default::default()
    })));
    let mut plugin = SlackPlugin::new();
    plugin.scripted_fixture = true;
    plugin.via = crate::Via(Some(std::sync::Arc::new(fixture.clone())));
    (plugin, fixture)
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_core::Plugin;
    use serde_json::json;

    #[tokio::test]
    async fn scripted_cli_transport_survives_config_and_demo_without_network_fallback() {
        let (mut plugin, fixture) = cli_plugin(vec![(
            "auth.test".into(),
            json!({"ok":true,"team_id":"TCLI1","user_id":"UCLI1"}),
        )]);
        plugin.configure(None).unwrap();
        plugin.replace_token("env:NEVER_RESOLVE_SCRIPTED_FIXTURE".into());
        plugin.demo();
        let client = plugin.client().unwrap();
        assert_eq!(client.auth_test().await.unwrap().team_id, "TCLI1");
        fixture.assert_finished();
        assert_eq!(fixture.calls(), vec!["auth.test"]);
        assert!(
            client
                .auth_test()
                .await
                .unwrap_err()
                .to_string()
                .contains("fixture exhausted")
        );
        assert_eq!(fixture.0.lock().unwrap().violations, 1);
    }

    #[tokio::test]
    async fn scripted_cli_runner_never_resolves_configured_credentials() {
        let (mut plugin, fixture) = cli_plugin(vec![(
            "auth.test".into(), json!({"ok":false,"error":"invalid_auth"}),
        )]);
        plugin.configure(Some(&toml::Value::try_from(Settings {
            token: "env:NEVER_RESOLVE_CLI_TOKEN".into(),
            api: "env:NEVER_RESOLVE_CLI_API".into(),
            channels: vec![toml::from_str("alias = 'watch'\nid = 'CWATCH001'\n").unwrap()],
            ..Default::default()
        }).unwrap())).unwrap();
        let command = plugin.cli().unwrap().try_get_matches_from(["slack", "list"]).unwrap();
        let error = plugin.run_cli(&command).await.unwrap_err();
        fixture.assert_finished();
        assert_eq!(fixture.calls(), vec!["auth.test"], "{error:#}");
        assert_eq!(error.to_string(), "the token was refused (invalid_auth)", "{error:#}");
    }

    #[tokio::test]
    async fn cli_fixture_still_refuses_writes_without_consuming_a_read_reply() {
        let (plugin, fixture) = cli_plugin(vec![("auth.test".into(), json!({"ok":true,"team_id":"TWATCH","user_id":"UVIMES"}))]);
        let client = plugin.client().unwrap();
        assert!(client.react("CWATCH", "1791200000.000100", "eyes").await.unwrap_err().to_string().contains("refuses writes"));
        assert_eq!(fixture.0.lock().unwrap().replies.len(), 1);
        assert_eq!(fixture.0.lock().unwrap().violations, 1);
        assert!(client.auth_test().await.is_ok());
        assert_eq!(fixture.0.lock().unwrap().violations, 1);
    }

    #[test]
    fn fixture_ignores_supplied_identity_and_never_uses_a_configured_credential() {
        let cfg = Settings {
            token: "env:NEVER_RESOLVE_THIS".into(),
            ..Default::default()
        };
        let mut p = offline_plugin(
            cfg,
            json!({"me":{"team_id":"TREAL","user_id":"UREAL"},"users":{"UROW":"synthetic-only"}}),
        )
        .unwrap();
        assert_eq!(p.data.me.as_ref().unwrap().team_id, "T000ACME1");
        assert_eq!(p.cfg.token, "xoxp-offline-synthetic-fixture");
        let mut normal = SlackPlugin::new();
        normal.restore(p.snapshot().unwrap());
        normal.install_me(demo::sample().data.me.unwrap());
        assert!(
            normal.data.users.is_empty(),
            "fixture snapshots are unverified imports"
        );
        assert!(normal.has_quarantined_cache());
        p.replace_token("env:DIFFERENT_REAL_REFERENCE".into());
        assert!(p.client().err().unwrap().contains("offline test fixture"));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let command = p
            .cli()
            .unwrap()
            .try_get_matches_from(["slack", "list"])
            .unwrap();
        assert!(
            rt.block_on(p.run_cli(&command))
                .unwrap_err()
                .to_string()
                .contains("offline test fixture")
        );
        assert!(
            rt.block_on(p.pictures()("https://files.slack.com/test.png".into()))
                .is_err()
        );
        p.configure(Some(
            &toml::Value::try_from(Settings {
                token: "env:NEVER_RESOLVE_AFTER_CONFIG".into(),
                ..Default::default()
            })
            .unwrap(),
        ))
        .unwrap();
        assert!(p.client().is_err());
        p.demo();
        assert!(p.client().is_err());
        assert!(rt.block_on(p.run_cli(&command)).is_err());
        assert!(
            rt.block_on(p.pictures()("https://files.slack.com/test.png".into()))
                .is_err()
        );
        p.restore(json!({"data":{"me":{"team_id":"TREAL","user_id":"UREAL"}}}));
        assert!(!p.authenticated());
        assert!(p.data.me.is_none());
    }
}
