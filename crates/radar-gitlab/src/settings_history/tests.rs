use super::*;
use crate::client::Client;
use crate::client::recorded::{Replay, block};
use crate::{Check, GitlabPlugin, demo, settings};
use radar_core::Plugin;
use serde_json::{Value, json};

const URL: &str =
    "https://SECRET_USER:SECRET_PASSWORD@git.example/?token=SECRET_TOKEN#SECRET_FRAGMENT";
const PATH: &str = "acme/team";

fn good(kind: Kind) -> Value {
    match kind {
        Kind::Auth => {
            json!({"data":{"currentUser":{"username":"SECRET_USERNAME", "name":"SECRET_NAME", "id":"SECRET_ID", "email":"SECRET_EMAIL"}}})
        }
        Kind::Base | Kind::Source => {
            json!({"data":{"group":{"fullName":"SECRET_DISPLAY", "projects":{"count":3}, "mergeRequests":{"count":7}}, "project":null}})
        }
    }
}

async fn read_async(client: Client, kind: Kind) -> anyhow::Result<CheckRead> {
    match kind {
        Kind::Auth => client.check_user_observed().await,
        Kind::Base | Kind::Source => client.check_path_observed(PATH).await,
    }
}

fn read(kind: Kind, body: Value) -> CheckRead {
    block(read_async(
        Client::over(Replay::new(vec![(200, body)])),
        kind,
    ))
    .unwrap()
}

fn observations(effects: &[Effect]) -> Vec<(&str, Option<u32>, Outcome, Option<Reason>)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Observe { action, outcome } => Some((
                action.kind.as_str(),
                action.metadata.count,
                *outcome,
                action.metadata.reason,
            )),
            _ => None,
        })
        .collect()
}

fn report(effects: &[Effect]) -> Outcome {
    let Effect::Report(outcome) = effects[0] else {
        panic!("report actual result first")
    };
    outcome
}

fn target(key: &str) -> crate::settings_checks::Target {
    use crate::settings_checks::Target;
    match key {
        "auth" => Target::Auth,
        "base" => Target::Base,
        key => Target::Source(key.strip_prefix("group:").unwrap().into()),
    }
}
fn requested(p: &mut GitlabPlugin, key: &str) -> Vec<Effect> {
    settings::on_menu(p, &target(key).menu(), "yes")
}

#[test]
fn scope_is_only_the_original_requested_server_or_source() {
    for kind in [Kind::Auth, Kind::Base, Kind::Source] {
        let scope = action(kind, URL, PATH);
        assert_eq!(scope.kind.as_str(), kind.code());
        assert_eq!(scope.subjects.len(), 1);
        let subject = &scope.subjects[0];
        assert_eq!(subject.key.instance, "https://git.example/");
        assert_eq!(subject.key.provider, "gitlab");
        assert_eq!(
            subject.key.kind,
            if kind == Kind::Auth {
                "server"
            } else {
                "source"
            }
        );
        assert_eq!(
            subject.key.id,
            if kind == Kind::Auth { "server" } else { PATH }
        );
        assert_eq!(subject.role, SubjectRole::Primary);
        assert!(subject.url.is_none() && scope.metadata.url.is_none());
        assert_eq!(scope.metadata.count, None);
        let fx = observed(kind, &scope, &Ok(read(kind, good(kind))));
        assert_eq!(report(&fx), Outcome::Succeeded);
        for effect in &fx {
            if let Effect::Observe { action, .. } = effect {
                assert_eq!(action.subjects, scope.subjects);
                assert!(!format!("{action:?}").contains("SECRET"));
            }
        }
    }
}

#[test]
fn unsafe_oversized_and_known_secret_scope_is_omitted_without_rewriting() {
    for kind in [Kind::Auth, Kind::Base, Kind::Source] {
        for url in [
            "bad URL".to_owned(),
            "file:///private".into(),
            "https://git.example/token=SECRET".into(),
            format!("https://git.example/{}", "x".repeat(8192)),
        ] {
            let scope = action(kind, &url, PATH);
            assert!(scope.subjects.is_empty() && scope.metadata.redacted);
            assert_eq!(scope.metadata.omitted_subjects, 1);
        }
    }
    for kind in [Kind::Base, Kind::Source] {
        for path in [
            "",
            "  ",
            "acme/../team",
            "acme/./team",
            "acme//team",
            "acme/private?token=SECRET",
            "acme/#SECRET",
            "acme/team\n",
            "acme/team with spaces",
            "acme/https://elsewhere.example",
        ] {
            let scope = action(kind, URL, path);
            assert!(
                scope.subjects.is_empty() && scope.metadata.redacted,
                "{path}"
            );
            assert_eq!(scope.metadata.omitted_subjects, 1);
        }
        assert!(action(kind, URL, &"a".repeat(2049)).subjects.is_empty());
        assert_eq!(
            action(kind, URL, &"a".repeat(2048)).subjects[0]
                .key
                .id
                .len(),
            2048
        );
    }
    let secret = "checkscope-private-unique-742899";
    radar_core::log::secret(secret);
    for kind in [Kind::Auth, Kind::Base, Kind::Source] {
        assert!(
            action(kind, &format!("https://{secret}.example"), PATH)
                .subjects
                .is_empty()
        );
        assert!(
            action(kind, &format!("https://git.example/{secret}"), PATH)
                .subjects
                .is_empty()
        );
    }
    for kind in [Kind::Base, Kind::Source] {
        assert!(
            action(kind, URL, &format!("acme/{secret}"))
                .subjects
                .is_empty()
        );
    }
    // Removed URL credentials, query, fragment and unused auth path are never subjects.
    assert_eq!(action(Kind::Auth, URL, secret).subjects.len(), 1);
}

#[test]
fn final_safe_url_normalization_is_checked_for_registered_secrets() {
    let secret = "lowercase-settings-server-927416";
    radar_core::log::secret(secret);
    let scope = action(
        Kind::Auth,
        &format!("https://{}.example", secret.to_ascii_uppercase()),
        "",
    );
    assert!(scope.subjects.is_empty() && scope.metadata.redacted);
}

#[test]
fn gaps_ambiguity_and_count_loss_are_partial_without_changing_passed_ui() {
    for (kind, body, suffix) in [
        (
            Kind::Auth,
            json!({"data":{"currentUser":false}}),
            "objects.incomplete",
        ),
        (
            Kind::Auth,
            json!({"data":{"currentUser":{"username":"u"}}}),
            "fields.defaulted",
        ),
        (
            Kind::Base,
            json!({"data":{"group":{"fullName":"G","projects":{"count":1},"mergeRequests":{"count":2}},"project":false}}),
            "paths.ambiguous",
        ),
        (
            Kind::Source,
            json!({"data":{"project":{"nameWithNamespace":"P","mergeRequests":{"count":4294967296_u64}}}}),
            "numbers.lossy",
        ),
    ] {
        let mut p = demo::sample();
        let got = read(kind, body);
        let expected = got.value.clone();
        let key = if kind == Kind::Auth {
            "auth"
        } else if kind == Kind::Base {
            "base"
        } else {
            "group:acme/team"
        };
        let target = target(key);
        let request = crate::settings_checks::begin(&mut p, &target);
        let effects = p.on_msg(Box::new(GlMsg::Checked {
            target,
            request,
            action: action(kind, URL, PATH),
            result: Ok(got),
        }));
        assert_eq!(report(&effects), Outcome::Partial);
        assert!(
            observations(&effects)
                .iter()
                .any(|row| row.0 == format!("{}.{suffix}", kind.code()))
        );
        assert_eq!(
            p.checks.get(key).unwrap().0,
            Check::Passed(expected.clone())
        );
        assert!(
            matches!(effects.last(), Some(Effect::Notify(message)) if message == &radar_core::writes::done_line(&expected))
        );
        if suffix == "numbers.lossy" {
            assert!(
                !observations(&effects)
                    .iter()
                    .any(|row| row.0.ends_with("merge_requests.open.reported"))
            );
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Scenario {
    initial_error: bool,
    renewal_error: bool,
    second_error: bool,
    renewable: bool,
}
struct Tokens {
    scenario: Scenario,
    reads: std::sync::atomic::AtomicUsize,
    renewals: std::sync::atomic::AtomicUsize,
}
impl Tokens {
    fn new(scenario: Scenario) -> Self {
        Self {
            scenario,
            reads: Default::default(),
            renewals: Default::default(),
        }
    }
}
impl crate::auth::Tokens for Tokens {
    fn token(&self) -> anyhow::Result<String> {
        let n = self.reads.fetch_add(1, Ordering::Relaxed);
        if (n == 0 && self.scenario.initial_error) || (n > 0 && self.scenario.second_error) {
            anyhow::bail!("SECRET_TOKEN_RESOLUTION")
        }
        Ok(format!("fake-token-{n}"))
    }
    fn renew(&self) -> anyhow::Result<bool> {
        self.renewals.fetch_add(1, Ordering::Relaxed);
        if self.scenario.renewal_error {
            anyhow::bail!("SECRET_RENEWAL")
        }
        Ok(self.scenario.renewable)
    }
}

#[test]
fn callback_entry_classifies_initial_setup_and_every_existing_retry_failure() {
    for kind in [Kind::Auth, Kind::Base, Kind::Source] {
        for (scenario, statuses, outcome, reads, renewals) in [
            (
                Scenario {
                    initial_error: true,
                    ..Default::default()
                },
                vec![200],
                Outcome::Blocked,
                1,
                0,
            ),
            (Scenario::default(), vec![200], Outcome::Succeeded, 1, 0),
            (
                Scenario {
                    renewable: true,
                    ..Default::default()
                },
                vec![401, 200],
                Outcome::Succeeded,
                2,
                1,
            ),
            (Scenario::default(), vec![401], Outcome::Failed, 1, 1),
            (
                Scenario {
                    renewal_error: true,
                    ..Default::default()
                },
                vec![401],
                Outcome::Failed,
                1,
                1,
            ),
            (
                Scenario {
                    renewable: true,
                    second_error: true,
                    ..Default::default()
                },
                vec![401, 200],
                Outcome::Failed,
                2,
                1,
            ),
            (
                Scenario {
                    renewable: true,
                    ..Default::default()
                },
                vec![401, 401],
                Outcome::Failed,
                2,
                1,
            ),
            // Permissive non-401 GraphQL status handling stays unchanged.
            (
                Scenario {
                    renewable: true,
                    ..Default::default()
                },
                vec![500],
                Outcome::Succeeded,
                1,
                0,
            ),
        ] {
            let replies = statuses
                .iter()
                .map(|status| (*status, good(kind)))
                .collect::<Vec<_>>();
            let plain_replay = Replay::new(replies.clone());
            let plain_tokens = Tokens::new(scenario);
            let expected = block(crate::auth::with_token(&plain_tokens, |_| {
                let client = Client::over(plain_replay.clone());
                async move {
                    match kind {
                        Kind::Auth => client.check_user().await,
                        _ => client.check_path(PATH).await,
                    }
                }
            }))
            .map_err(|e| format!("{e:#}"));
            let replay = Replay::new(replies);
            let tokens = Tokens::new(scenario);
            let attempt = Attempt::default();
            let actual = block(crate::auth::with_token(&tokens, |_| {
                attempt.enter();
                read_async(Client::over(replay.clone()), kind)
            }));
            let result = attempt.finish(actual);
            assert_eq!(
                result
                    .as_ref()
                    .map(|got| got.value.clone())
                    .map_err(|error| error.message.clone()),
                expected
            );
            assert_eq!(replay.asked(), plain_replay.asked());
            assert_eq!(tokens.reads.load(Ordering::Relaxed), reads);
            assert_eq!(tokens.renewals.load(Ordering::Relaxed), renewals);
            let effects = observed(kind, &action(kind, URL, PATH), &result);
            assert_eq!(report(&effects), outcome);
            assert!(
                observations(&effects)
                    .iter()
                    .all(|row| !row.0.contains("attempt"))
            );
            if result.is_err() {
                assert_eq!(
                    observations(&effects),
                    [(
                        format!(
                            "{}.{}",
                            kind.code(),
                            if outcome == Outcome::Blocked {
                                "requests.skipped"
                            } else {
                                "errors"
                            }
                        )
                        .as_str(),
                        Some(1),
                        outcome,
                        Some(if outcome == Outcome::Blocked {
                            Reason::Unavailable
                        } else {
                            Reason::Unspecified
                        })
                    )]
                );
            } else {
                assert_eq!(observations(&effects)[0].1, Some(1));
            }
            for effect in &effects {
                if let Effect::Observe { action, .. } = effect {
                    assert!(!format!("{action:?}").contains("SECRET"));
                }
            }
        }
    }
}

#[test]
fn read_errors_do_not_depend_on_error_text_or_invent_response_evidence() {
    for message in ["", "no GitLab token configured", "401", "SECRET_ERROR"] {
        let attempt = Attempt::default();
        attempt.enter();
        let result = attempt.finish(Err(anyhow::anyhow!("{message}")));
        let effects = observed(Kind::Auth, &action(Kind::Auth, URL, ""), &result);
        assert_eq!(report(&effects), Outcome::Failed);
        assert_eq!(
            observations(&effects),
            [(
                "gitlab.settings.check.auth.errors",
                Some(1),
                Outcome::Failed,
                Some(Reason::Unspecified)
            )]
        );
    }
}

#[test]
fn retry_reports_only_the_final_selected_reply_evidence() {
    let replay = Replay::new(vec![
        (401, good(Kind::Auth)),
        (200, json!({"data":{"currentUser":false}})),
    ]);
    let tokens = Tokens::new(Scenario {
        renewable: true,
        ..Default::default()
    });
    let attempt = Attempt::default();
    let result = attempt.finish(block(crate::auth::with_token(&tokens, |_| {
        attempt.enter();
        read_async(Client::over(replay.clone()), Kind::Auth)
    })));
    let effects = observed(Kind::Auth, &action(Kind::Auth, URL, ""), &result);
    assert_eq!(report(&effects), Outcome::Partial);
    assert_eq!(
        observations(&effects),
        [
            (
                "gitlab.settings.check.auth.replies",
                Some(1),
                Outcome::Partial,
                None
            ),
            (
                "gitlab.settings.check.auth.objects.incomplete",
                Some(1),
                Outcome::Partial,
                None
            ),
            (
                "gitlab.settings.check.auth.fields.incomplete",
                Some(2),
                Outcome::Partial,
                None
            ),
            (
                "gitlab.settings.check.auth.fields.defaulted",
                Some(2),
                Outcome::Partial,
                None
            ),
        ]
    );
    assert_eq!(replay.asked().len(), 2);
    assert_eq!(replay.asked()[0], replay.asked()[1]);
}

#[test]
fn production_dispatch_captures_scope_and_initial_setup_failure_before_reconfiguration() {
    let mut p = demo::sample();
    p.cfg.url = URL.into();
    p.cfg.base = PATH.into();
    p.cfg.token.clear(); // Deterministic pre-callback error: no environment, glab or HTTP.
    for (key, kind, path) in [
        ("auth", Kind::Auth, "server"),
        ("base", Kind::Base, PATH),
        ("group:group:acme/team", Kind::Source, PATH),
    ] {
        let mut effects = requested(&mut p, key);
        let Effect::SpawnAction { action, task } = effects.remove(0) else {
            panic!("actual check has independent scope")
        };
        assert_eq!(action.kind.as_str(), kind.code());
        assert_eq!(action.subjects[0].key.id, path);
        let mut cfg = p.cfg.clone();
        cfg.url = "https://changed.example".into();
        cfg.base = "changed/base".into();
        p.configure(Some(&toml::Value::try_from(cfg).unwrap()))
            .unwrap();
        let message = block(task);
        let msg = message.downcast::<GlMsg>().unwrap();
        let GlMsg::Checked {
            target: got_target,
            request,
            action: captured,
            result,
        } = *msg
        else {
            panic!()
        };
        assert_eq!(got_target, target(key));
        assert_eq!(got_target.kind(), kind);
        assert_eq!(captured.kind, action.kind);
        assert_eq!(captured.subjects, action.subjects);
        assert_eq!(captured.metadata, action.metadata);
        assert_eq!(
            result.as_ref().unwrap_err().message,
            "no GitLab token configured"
        );
        assert_eq!(
            report(&observed(kind, &captured, &result)),
            Outcome::Blocked
        );
        let fx = p.on_msg(Box::new(GlMsg::Checked {
            target: got_target,
            request,
            action: captured,
            result,
        }));
        assert!(!p.checks.contains_key(key));
        assert!(
            matches!(fx.last(), Some(Effect::Observe { action, outcome: Outcome::Blocked })
            if action.kind.as_str().ends_with(".application.skipped"))
        );
        assert!(!fx.iter().any(|effect| matches!(effect, Effect::Notify(_) | Effect::NotifyWith(_, _) | Effect::Notice(_))));
        p.cfg.url = URL.into();
        p.cfg.base = PATH.into();
    }
}

#[test]
fn empty_and_whitespace_base_dispatch_differences_remain_unchanged() {
    for base in ["", " ", "acme"] {
        let mut p = demo::sample();
        p.cfg.base = base.into();
        p.cfg.sources.clear();
        let bulk = settings::on_bulk(&mut p, "check-all");
        assert_eq!(bulk.len(), if base.trim().is_empty() { 1 } else { 2 });
        p.section_rows[0] = settings::Field::ALL
            .iter()
            .position(|f| *f == settings::Field::Check)
            .unwrap();
        let radar_core::plugin::Outcome::Handled(connection) = settings::key(
            &mut p,
            "connection",
            radar_core::ratatui::crossterm::event::KeyEvent::from(
                radar_core::ratatui::crossterm::event::KeyCode::Enter,
            ),
        ) else {
            panic!()
        };
        assert_eq!(connection.len(), if base.is_empty() { 1 } else { 2 });
        let direct = requested(&mut p, "base");
        assert_eq!(direct.len(), 1);
        let Effect::SpawnAction { action, .. } = &direct[0] else {
            panic!()
        };
        assert_eq!(action.subjects.is_empty(), base.trim().is_empty());
    }
}

mod host_tests;
