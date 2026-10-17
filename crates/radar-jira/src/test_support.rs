//! Minimal owner-local models, direct starting states and strict hosted scripts.
//! [`TicketSelected`] skips unrelated setup for downstream selection tests;
//! [`hosted_plugin`] starts empty for tests of the refresh journey itself.
//! Only the transport changes; production requests, parsers and reducers still execute.
//! CLI, pictures and environment-backed site URLs are deliberately unsupported
//! and fail with sticky violations. Email/token references are never resolved.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub use crate::client::{Method, Req, Resp};
pub use crate::config::{Project, Site, View};
pub use crate::model::{Category, Ticket};
use crate::{
    JiraPlugin, Settings,
    client::{Sent, Transport},
};

/// A pure, owned ticket with stable identities and timestamps, not a demo world.
#[derive(Clone, Debug)]
pub struct TicketBuilder(Ticket);

impl Default for TicketBuilder {
    fn default() -> Self {
        Self(Ticket {
            key: "WATCH-12".into(),
            id: "10012".into(),
            summary: "Patrol the Shades".into(),
            kind: "Task".into(),
            status: "To Do".into(),
            category: Category::Todo,
            project: "WATCH".into(),
            project_name: "City Watch".into(),
            created: "2026-01-01T09:00:00.000+0000".into(),
            updated: "2026-01-02T09:00:00.000+0000".into(),
            ..Default::default()
        })
    }
}

impl TicketBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.0.key = key.into();
        self.0.project = self.0.key.split('-').next().unwrap_or_default().into();
        self
    }
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.0.id = id.into();
        self
    }
    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.0.summary = summary.into();
        self
    }
    pub fn status(mut self, status: impl Into<String>, category: Category) -> Self {
        self.0.status = status.into();
        self.0.category = category;
        self
    }
    pub fn assignee(mut self, id: impl Into<String>, name: impl Into<String>) -> Self {
        self.0.assignee_id = id.into();
        self.0.assignee = name.into();
        self
    }
    pub fn build(self) -> Ticket {
        self.0
    }
}

/// Direct fixture initialization at a selected ticket in the list, not a replay
/// or a claim that refresh/authentication succeeded. Only the supplied tickets
/// and synthetic account (`vimes`, Sam Vimes) are folded into the one `watch`
/// site. The real owner view supplies the path; real fold/selection intents open
/// its ancestors and select it, including a ticket under a folded Done group.
///
/// Sync timestamps, development/page reads, actions and journal history stay
/// empty. Tests of those journeys must execute their effects instead. The strict
/// transport remains installed for subsequent actions: an empty script permits
/// no provider requests, and unexpected requests remain sticky failures.
///
/// ```
/// use radar_jira::test_support::{TicketBuilder, TicketSelected};
/// let (plugin, fixture) = TicketSelected::new(
///     TicketBuilder::new().summary("Find the missing dragon"),
/// ).build(vec![]).unwrap();
/// fixture.assert_finished();
/// ```
#[derive(Clone, Debug)]
pub struct TicketSelected {
    tickets: Vec<Ticket>,
    site: String,
    key: String,
}

impl TicketSelected {
    /// Start with exactly this owned ticket selected on the synthetic site.
    pub fn new(ticket: TicketBuilder) -> Self {
        let ticket = ticket.build();
        Self {
            key: ticket.key.clone(),
            site: "watch".into(),
            tickets: vec![ticket],
        }
    }

    /// Add a sibling, or replace the existing ticket with this stable key in
    /// place. Adding an override never creates duplicate list identities or
    /// silently changes which ticket is selected.
    pub fn with_ticket(mut self, ticket: TicketBuilder) -> Self {
        let ticket = ticket.build();
        if let Some(held) = self.tickets.iter_mut().find(|held| held.key == ticket.key) {
            *held = ticket;
        } else {
            self.tickets.push(ticket);
        }
        self
    }

    /// Select an exact site/key. `build` rejects an unknown site, missing ticket
    /// or ticket absent from the real tree instead of falling back to a row.
    pub fn select(mut self, site: impl Into<String>, key: impl Into<String>) -> Self {
        self.site = site.into();
        self.key = key.into();
        self
    }

    pub fn build(
        self,
        expected: Vec<ExpectedRequest>,
    ) -> anyhow::Result<(JiraPlugin, HostedFixture)> {
        use radar_core::view::{Intent, TreeRow};
        use radar_core::{Outcome, Pane, Plugin};

        let (mut plugin, fixture) = hosted_plugin(expected)?;
        anyhow::ensure!(
            self.site == "watch",
            "selected Jira fixture site does not exist: {}",
            self.site
        );
        anyhow::ensure!(
            self.tickets.iter().any(|ticket| ticket.key == self.key),
            "selected Jira fixture ticket does not exist: {}/{}",
            self.site,
            self.key
        );
        let jql = plugin
            .jql_for(0, &plugin.cfg.views[0])
            .expect("synthetic view asks watch");
        // This is data initialization, deliberately not JiraMsg::Loaded: no
        // network evidence, successful sync marker or action history is made.
        plugin.fold(
            self.site.clone(),
            jql,
            Some(crate::model::Me {
                account_id: "vimes".into(),
                name: "Sam Vimes".into(),
            }),
            self.tickets,
            false,
            None,
        );
        let tree = checkpoint_tree(&plugin)?;
        let path = row_path(&tree.rows, &crate::screen::ticket_id(&self.site, &self.key))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "selected Jira fixture ticket is absent from the tree: {}/{}",
                    self.site,
                    self.key
                )
            })?;
        let mut intents = Vec::new();
        for depth in 1..path.len() {
            if !tree.open.iter().any(|open| open == &path[..depth]) {
                intents.push(Intent::Fold {
                    target: TreeRow::target_of(&path[..depth]),
                    open: true,
                });
            }
        }
        intents.push(Intent::Select {
            target: TreeRow::target_of(&path),
        });
        for intent in intents {
            anyhow::ensure!(
                matches!(plugin.intent(crate::SCREEN.id, Pane::Middle, &intent), Outcome::Handled(effects) if effects.is_empty()),
                "selected Jira fixture setup must handle selection without effects"
            );
        }
        anyhow::ensure!(
            matches!(crate::screen::selected(&plugin), Some(crate::screen::Selected::Ticket(item)) if item.site.name == self.site && item.t.key == self.key),
            "selected Jira fixture did not select {}/{}",
            self.site,
            self.key
        );
        Ok((plugin, fixture))
    }
}

/// Read the actual owner view, without a host or a rendered frame.
fn checkpoint_tree(plugin: &JiraPlugin) -> anyhow::Result<radar_core::view::Tree> {
    use radar_core::{
        Plugin,
        view::{Node, PaneId, ViewCx},
    };
    let view = plugin
        .view(
            crate::SCREEN.id,
            &ViewCx {
                now: plugin.now,
                focus: PaneId::Middle,
                width: 160,
                height: 40,
                parts: radar_core::Parts::none(),
                icon_lead: 0,
                setup: false,
            },
        )
        .ok_or_else(|| anyhow::anyhow!("selected Jira fixture has no screen view"))?;
    view.panes
        .into_iter()
        .find_map(|pane| match pane.body {
            Node::Tree(tree) if pane.pane == PaneId::Middle => Some(tree),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("selected Jira fixture has no ticket tree"))
}

/// Traverse the production tree's identities, never reconstruct its grouping.
fn row_path(rows: &[radar_core::view::TreeRow], id: &str) -> Option<Vec<String>> {
    for row in rows {
        if row.id == id {
            return Some(vec![row.id.clone()]);
        }
        if let Some(mut path) = row_path(&row.children, id) {
            path.insert(0, row.id.clone());
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
#[path = "test_support/selected_ticket_tests.rs"]
mod selected_ticket_tests;

/// Synthetic configuration deliberately uses unresolved credential references.
/// No environment variable needs to be set, changed, or read by these fixtures.
pub fn fixture_settings() -> Settings {
    Settings {
        sites: vec![Site {
            name: "watch".into(),
            url: "https://watch.invalid".into(),
            email: "env:RADAR_JIRA_FIXTURE_EMAIL_NEVER_RESOLVE".into(),
            token: "env:RADAR_JIRA_FIXTURE_TOKEN_NEVER_RESOLVE".into(),
            color: "blue".into(),
            projects: vec![Project {
                key: "WATCH".into(),
                ac_field: String::new(),
            }],
            from: String::new(),
        }],
        views: vec![View {
            name: "Watch patrol".into(),
            when: vec![],
            jql: "project = WATCH".into(),
            site: "watch".into(),
        }],
        ..Default::default()
    }
}

#[derive(Debug)]
pub struct ExpectedRequest {
    key: String,
    site: String,
    origin: String,
    request: Req,
    reply: Resp,
    held: bool,
}

impl ExpectedRequest {
    pub fn json(
        key: impl Into<String>,
        site: impl Into<String>,
        origin: impl Into<String>,
        request: Req,
        body: serde_json::Value,
    ) -> Self {
        Self::reply(
            key,
            site,
            origin,
            request,
            Resp {
                status: 200,
                retry_after: None,
                rate: Default::default(),
                body: body.to_string(),
            },
        )
    }
    pub fn reply(
        key: impl Into<String>,
        site: impl Into<String>,
        origin: impl Into<String>,
        request: Req,
        reply: Resp,
    ) -> Self {
        Self {
            key: key.into(),
            site: site.into(),
            origin: origin.into(),
            request,
            reply,
            held: false,
        }
    }
    pub fn held(mut self) -> Self {
        self.held = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestRecord {
    /// An unexpected call has no key and remains a violation if swallowed.
    pub key: Option<String>,
    pub site: String,
    /// Normalized literal provider origin, without credentials.
    pub origin: String,
    pub request: Req,
    pub completed: bool,
}

#[derive(Debug, Default)]
struct Script {
    expected: Vec<ExpectedRequest>,
    ledger: Vec<RequestRecord>,
    held: BTreeMap<String, tokio::sync::oneshot::Sender<()>>,
    violations: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct HostedFixture(Arc<Mutex<Script>>);

impl HostedFixture {
    pub fn ledger(&self) -> Vec<RequestRecord> {
        self.0.lock().unwrap().ledger.clone()
    }
    pub(crate) fn blocked(&self, operation: &str) {
        self.0
            .lock()
            .unwrap()
            .violations
            .push(format!("unsupported fixture I/O: {operation}"));
    }
    pub(crate) fn for_site(&self, site: &Site) -> anyhow::Result<SiteTransport> {
        let origin = match literal_origin(&site.url) {
            Ok(origin) => origin,
            Err(error) => {
                self.blocked("nonliteral or invalid site origin");
                return Err(error);
            }
        };
        Ok(SiteTransport {
            site: site.name.clone(),
            origin,
            fixture: self.clone(),
        })
    }
    pub fn release(&self, key: &str) -> anyhow::Result<()> {
        let mut s = self.0.lock().unwrap();
        let Some(sender) = s.held.remove(key) else {
            s.violations
                .push(format!("release without a held request: {key}"));
            anyhow::bail!("release without a held request: {key}");
        };
        if sender.send(()).is_err() {
            s.violations
                .push(format!("held request was cancelled: {key}"));
            anyhow::bail!("held request was cancelled: {key}");
        }
        Ok(())
    }
    pub fn check_finished(&self) -> anyhow::Result<()> {
        let s = self.0.lock().unwrap();
        anyhow::ensure!(
            s.violations.is_empty(),
            "Jira fixture violations: {:?}",
            s.violations
        );
        let unused: Vec<_> = s.expected.iter().map(|e| &e.key).collect();
        anyhow::ensure!(unused.is_empty(), "unused Jira requests: {unused:?}");
        anyhow::ensure!(
            s.ledger.iter().all(|r| r.completed),
            "Jira requests remain in flight"
        );
        anyhow::ensure!(s.held.is_empty(), "Jira responses remain held");
        Ok(())
    }
    pub fn assert_finished(&self) {
        self.check_finished().unwrap();
    }
}

pub(crate) struct SiteTransport {
    site: String,
    origin: String,
    fixture: HostedFixture,
}
impl Transport for SiteTransport {
    fn send(&self, request: &Req) -> Sent<'_> {
        let request = request.clone();
        Box::pin(async move {
            let (reply, gate, index) = {
                let mut s = self.fixture.0.lock().unwrap();
                let found = s.expected.iter().position(|e| {
                    e.site == self.site && e.origin == self.origin && e.request == request
                });
                let index = s.ledger.len();
                let Some(found) = found else {
                    s.ledger.push(RequestRecord {
                        key: None,
                        site: self.site.clone(),
                        origin: self.origin.clone(),
                        request: request.clone(),
                        completed: true,
                    });
                    s.violations.push(format!(
                        "unexpected request on {} at {}: {:?}",
                        self.site, self.origin, request
                    ));
                    return Err(format!(
                        "unexpected Jira fixture request on {} at {}: {:?}",
                        self.site, self.origin, request
                    ));
                };
                let e = s.expected.remove(found);
                s.ledger.push(RequestRecord {
                    key: Some(e.key.clone()),
                    site: self.site.clone(),
                    origin: self.origin.clone(),
                    request,
                    completed: false,
                });
                let gate = if e.held {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    s.held.insert(e.key, tx);
                    Some(rx)
                } else {
                    None
                };
                (e.reply, gate, index)
            };
            if let Some(gate) = gate {
                gate.await
                    .map_err(|_| "Jira response gate dropped".to_string())?;
            }
            self.fixture.0.lock().unwrap().ledger[index].completed = true;
            Ok(reply)
        })
    }
}

/// Mirror production site normalization without calling `Site::base_url`,
/// which may resolve environment references. Authentication is never inspected.
fn literal_origin(value: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        !value.trim().starts_with("env:"),
        "Jira fixture requires a literal origin"
    );
    let url = reqwest::Url::parse(&crate::config::site_url(value))?;
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "Jira fixture requires an HTTP(S) origin"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Jira fixture origins cannot contain credentials"
    );
    Ok(url.origin().ascii_serialization())
}

/// Empty data with fixed time and explicit provider injection before auth.
/// Configuration/cache restoration cannot remove this provider. The CLI and
/// picture loaders fail closed rather than implementing alternate offline paths.
pub fn hosted_plugin(
    mut expected: Vec<ExpectedRequest>,
) -> anyhow::Result<(JiraPlugin, HostedFixture)> {
    let mut keys = std::collections::BTreeSet::new();
    for e in &mut expected {
        e.origin = literal_origin(&e.origin)?;
        anyhow::ensure!(
            !e.key.is_empty() && keys.insert(e.key.clone()),
            "Jira request keys must be nonempty and unique"
        );
        anyhow::ensure!(!e.site.is_empty(), "Jira request sites must be nonempty");
    }
    let fixture = HostedFixture(Arc::new(Mutex::new(Script {
        expected,
        ..Default::default()
    })));
    let plugin = JiraPlugin {
        cfg: fixture_settings(),
        provider: crate::client::Provider {
            fixture: Some(fixture.clone()),
        },
        now: radar_core::plugin::DEMO_NOW,
        section_rows: [0, 1, 0, 1],
        env: Some(Vec::new()),
        mcp: std::sync::OnceLock::from(Vec::new()),
        ..Default::default()
    };
    Ok((plugin, fixture))
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_core::{Effect, Plugin};
    use serde_json::json;
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };

    fn get(path: &str) -> Req {
        Req {
            method: Method::Get,
            path: path.into(),
            body: None,
        }
    }
    fn me() -> serde_json::Value {
        json!({"accountId":"vimes", "displayName":"Sam Vimes"})
    }

    #[test]
    fn builder_is_minimal_owned_and_deterministic() {
        let original = TicketBuilder::new().build();
        let changed = TicketBuilder::new()
            .key("WITCHES-7")
            .id("10007")
            .summary("Check the chalk")
            .assignee("weatherwax", "Granny Weatherwax")
            .status("In Progress", Category::Doing)
            .build();
        assert_eq!(original, TicketBuilder::new().build());
        assert_eq!(original.key, "WATCH-12");
        assert_eq!(changed.project, "WITCHES");
        assert!(original.description.is_empty());
        assert!(original.assignee.is_empty());
        assert_eq!(changed.assignee_id, "weatherwax");
    }

    #[tokio::test]
    async fn gated_exact_request_preserves_production_parser() {
        let (p, f) = hosted_plugin(vec![
            ExpectedRequest::json(
                "identity",
                "watch",
                "https://watch.invalid",
                get("/rest/api/3/myself"),
                me(),
            )
            .held(),
        ])
        .unwrap();
        let mut site = p.cfg.sites[0].clone();
        site.url = "https://WATCH.invalid:443/jira/software/projects/WATCH".into();
        let client = p.provider.client(&site).unwrap();
        let mut call = Box::pin(client.myself());
        assert!(matches!(
            call.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        assert_eq!(f.ledger()[0].key.as_deref(), Some("identity"));
        assert!(!f.ledger()[0].completed);
        assert!(f.check_finished().is_err());
        f.release("identity").unwrap();
        assert_eq!(call.await.unwrap().account_id, "vimes");
        f.assert_finished();
    }

    #[tokio::test]
    async fn unexpected_calls_are_sticky_and_do_not_consume_other_sites() {
        let (p, f) = hosted_plugin(vec![ExpectedRequest::json(
            "identity",
            "watch",
            "https://watch.invalid",
            get("/rest/api/3/myself"),
            me(),
        )])
        .unwrap();
        let mut other = p.cfg.sites[0].clone();
        other.name = "witches".into();
        assert!(p.provider.client(&other).unwrap().myself().await.is_err());
        assert!(
            p.provider
                .client(&p.cfg.sites[0])
                .unwrap()
                .myself()
                .await
                .is_ok()
        );
        assert_eq!(f.ledger().len(), 2);
        assert!(
            f.check_finished()
                .unwrap_err()
                .to_string()
                .contains("violations")
        );
    }

    #[tokio::test]
    async fn same_alias_on_changed_origin_cannot_consume_an_old_expectation() {
        let (mut p, f) = hosted_plugin(vec![ExpectedRequest::json(
            "identity",
            "watch",
            "https://watch.invalid",
            get("/rest/api/3/myself"),
            me(),
        )])
        .unwrap();
        let mut cfg = fixture_settings();
        cfg.sites[0].url = "https://witches.invalid".into();
        p.configure(Some(&toml::Value::try_from(cfg).unwrap()))
            .unwrap();
        assert!(
            p.provider
                .client(&p.cfg.sites[0])
                .unwrap()
                .myself()
                .await
                .is_err()
        );
        assert_eq!(f.ledger()[0].origin, "https://witches.invalid");
        assert!(f.ledger()[0].key.is_none());
        p.configure(Some(&toml::Value::try_from(fixture_settings()).unwrap()))
            .unwrap();
        p.provider
            .client(&p.cfg.sites[0])
            .unwrap()
            .myself()
            .await
            .unwrap();
        assert_eq!(f.ledger()[1].origin, "https://watch.invalid");
        assert_eq!(f.ledger()[1].key.as_deref(), Some("identity"));
        assert!(
            f.check_finished()
                .unwrap_err()
                .to_string()
                .contains("violations")
        );
    }

    #[tokio::test]
    async fn configuration_restore_and_reset_keep_injection_before_auth() {
        let (mut p, f) = hosted_plugin(
            (0..3)
                .map(|i| {
                    ExpectedRequest::json(
                        format!("identity-{i}"),
                        "watch",
                        "https://watch.invalid",
                        get("/rest/api/3/myself"),
                        me(),
                    )
                })
                .collect(),
        )
        .unwrap();
        for _ in 0..3 {
            p.configure(None).unwrap();
            p.restore(json!({"data":{}}));
            p.configure(Some(&toml::Value::try_from(fixture_settings()).unwrap()))
                .unwrap();
            assert!(p.vars().is_empty());
            assert!(p.offers().is_empty());
            assert_eq!(
                p.provider
                    .client(&p.cfg.sites[0])
                    .unwrap()
                    .myself()
                    .await
                    .unwrap()
                    .name,
                "Sam Vimes"
            );
        }
        f.assert_finished();
    }

    #[cfg(feature = "demo")]
    #[tokio::test]
    async fn demo_cannot_remove_provider_or_enable_discovery() {
        let (mut p, f) = hosted_plugin(vec![ExpectedRequest::json(
            "identity",
            "watch",
            "https://watch.invalid",
            get("/rest/api/3/myself"),
            me(),
        )])
        .unwrap();
        p.demo();
        assert!(p.vars().is_empty());
        assert!(p.offers().is_empty());
        p.configure(Some(&toml::Value::try_from(fixture_settings()).unwrap()))
            .unwrap();
        p.provider
            .client(&p.cfg.sites[0])
            .unwrap()
            .myself()
            .await
            .unwrap();
        f.assert_finished();
    }

    #[tokio::test]
    async fn unsupported_cli_and_pictures_fail_closed_and_sticky() {
        let (p, f) = hosted_plugin(vec![]).unwrap();
        let command = p
            .cli()
            .unwrap()
            .try_get_matches_from(["jira", "list"])
            .unwrap();
        assert!(p.run_cli(&command).await.is_err());
        assert!(
            p.pictures("watch").unwrap()("https://watch.invalid/picture.png".into())
                .await
                .is_err()
        );
        let error = f.check_finished().unwrap_err().to_string();
        assert!(error.contains("CLI") && error.contains("pictures"));
        assert!(f.ledger().is_empty());
    }

    #[test]
    fn environment_backed_site_origins_fail_before_configuration_is_installed() {
        let (mut p, f) = hosted_plugin(vec![]).unwrap();
        let mut cfg = fixture_settings();
        cfg.sites[0].url = "env:RADAR_JIRA_FIXTURE_URL_NEVER_RESOLVE".into();
        assert!(
            p.configure(Some(&toml::Value::try_from(cfg).unwrap()))
                .is_err()
        );
        assert_eq!(p.cfg.sites[0].url, "https://watch.invalid");
        assert!(
            f.check_finished()
                .unwrap_err()
                .to_string()
                .contains("site URL")
        );
    }

    #[test]
    fn invalid_scripts_unused_expectations_and_bad_releases_fail() {
        assert!(
            hosted_plugin(vec![ExpectedRequest::json(
                "",
                "watch",
                "https://watch.invalid",
                get("/rest/api/3/myself"),
                me()
            )])
            .is_err()
        );
        assert!(
            hosted_plugin(vec![
                ExpectedRequest::json(
                    "same",
                    "watch",
                    "https://watch.invalid",
                    get("/rest/api/3/myself"),
                    me()
                ),
                ExpectedRequest::json(
                    "same",
                    "watch",
                    "https://watch.invalid",
                    get("/rest/api/3/myself"),
                    me()
                ),
            ])
            .is_err()
        );
        let (_, f) = hosted_plugin(vec![ExpectedRequest::json(
            "identity",
            "watch",
            "https://watch.invalid",
            get("/rest/api/3/myself"),
            me(),
        )])
        .unwrap();
        assert!(
            f.check_finished()
                .unwrap_err()
                .to_string()
                .contains("unused")
        );
        assert!(f.release("identity").is_err());
        assert!(
            f.check_finished()
                .unwrap_err()
                .to_string()
                .contains("violations")
        );
    }

    #[tokio::test]
    async fn refresh_executes_real_requests_and_reducer_without_credentials() {
        let search = "/rest/api/3/search/jql?jql=project%20%3D%20WATCH%20ORDER%20BY%20updated%20DESC&fields=summary%2Cissuetype%2Cstatus%2Cpriority%2Cassignee%2Creporter%2Ccreated%2Cupdated%2Cparent%2Clabels%2Ccomponents%2CfixVersions%2Cproject%2Cdescription&maxResults=100";
        let (mut p, f) = hosted_plugin(vec![
            ExpectedRequest::json(
                "identity",
                "watch",
                "https://watch.invalid",
                get("/rest/api/3/myself"),
                me(),
            ),
            ExpectedRequest::json(
                "tickets",
                "watch",
                "https://watch.invalid",
                get(search),
                json!({"issues":[],"isLast":true}),
            ),
        ])
        .unwrap();
        let effects = p.refresh_now();
        let mut ran = 0;
        for effect in effects {
            if let Effect::SpawnAction { task, .. } = effect {
                let effects = p.on_msg(task.await);
                assert!(effects.iter().any(|e| matches!(e, Effect::PersistCache)));
                ran += 1;
            }
        }
        assert_eq!(ran, 1);
        assert_eq!(p.data["watch"].me.as_ref().unwrap().account_id, "vimes");
        assert!(p.sync.errors.is_empty());
        assert!(!p.data["watch"].synced.is_empty());
        f.assert_finished();
    }

    #[tokio::test]
    async fn write_permission_is_checked_before_spawn_and_reply_uses_real_reducer() {
        use crate::write::{self, Change};
        use radar_core::writes;
        let expected = Req {
            method: Method::Post,
            path: "/rest/api/3/issue/WATCH-12/comment".into(),
            body: Some(
                json!({"body":{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"Patrol complete"}]}]}}),
            ),
        };
        let (mut p, f) = hosted_plugin(vec![ExpectedRequest::json(
            "comment",
            "watch",
            "https://watch.invalid",
            expected,
            json!({"id":"70012"}),
        )])
        .unwrap();
        let change = Change::Comment {
            site: "watch".into(),
            key: "WATCH-12".into(),
            text: "Patrol complete".into(),
        };
        let previous = writes::mode();
        writes::set_mode(writes::Mode::ReadOnly, None);
        let blocked = write::send(&mut p, change.clone());
        writes::set_mode(previous.0, previous.1.clone());
        assert!(!blocked.iter().any(|e| matches!(e, Effect::Spawn(_))));
        assert!(f.ledger().is_empty());
        writes::set_mode(writes::Mode::Write, None);
        let sent = write::send(&mut p, change);
        writes::set_mode(writes::Mode::ReadOnly, None);
        let task = sent.into_iter().find_map(|e| {
            if let Effect::Spawn(task) = e {
                Some(task)
            } else {
                None
            }
        });
        let Some(task) = task else {
            writes::set_mode(previous.0, previous.1);
            panic!("approved write is spawned");
        };
        let message = task.await;
        writes::set_mode(previous.0, previous.1);
        let effects = p.on_msg(message);
        assert!(!effects.is_empty());
        assert!(p.sent["watch\0WATCH-12"].result.as_ref().unwrap().is_ok());
        f.assert_finished();
    }
}
