//! Three renderer-free journeys through owner-local transports and the real host.
//! Provider replies are scripted; parsing, reduction, views and journal I/O are real.
#[cfg(not(any(feature = "github", feature = "gitlab", feature = "jira")))]
compile_error!("provider_scenarios requires at least one provider feature: github, gitlab or jira");

use radar_core::journal::{ActionEvent, Journal, Outcome, Query};
use radar_core::scenario::{JournalRecord, Scenario};
use radar_core::view::ScreenView;
use radar_core::writes::Mode;
use radar_core::{Host, Pane, Plugin};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const NOW: i64 = 1_791_271_000;
const BUDGET: usize = 256;
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "radar-provider-scenario-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn journal(&self) -> PathBuf {
        self.0.join("actions.sqlite3")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scenario(plugin: impl Plugin + 'static, config: &str, scratch: &Scratch) -> Scenario {
    let mut host = Host::new(vec![Box::new(plugin)], NOW).unwrap();
    host.load_config(Some(config)).unwrap();
    host.set_env_mode(Some((Mode::ReadOnly, "RADAR_MODE".into())));
    host.focus = Pane::Middle;
    Scenario::with_journal(host, &scratch.journal()).unwrap()
}

fn view(s: &Scenario, screen: &str) -> ScreenView {
    s.inspect(|host| {
        host.screen_view_in(0, screen, 120, 40)
            .expect("provider view")
    })
}

fn pending(s: &Scenario, kind: &str) {
    let ids: Vec<_> = s
        .records()
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. } if descriptor.kind.as_str() == kind => {
                Some(id)
            }
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 1, "one semantic {kind} action requested");
    // Begin records Requested for the semantic action. Started belongs to the
    // runtime task beneath it, whose typed reply supplies the eventual result.
    let task = s
        .records()
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } if descriptor.kind.as_str() == "runtime.task" && parent.as_ref() == Some(ids[0]) => {
                Some(id)
            }
            _ => None,
        })
        .expect("semantic read owns its real runtime task");
    assert!(s.records().iter().any(|record| matches!(record,
        JournalRecord::Outcome { id, outcome: Outcome::Started, .. } if id == task)));
    assert!(
        !s.records().iter().any(|record| matches!(record,
        JournalRecord::Outcome { id, outcome, .. }
            if (id == ids[0] || id == task) && outcome.is_terminal())),
        "{kind}: a held provider response cannot imply completion"
    );
}

fn persisted(
    s: &mut Scenario,
    scratch: &Scratch,
    content: &str,
) -> Vec<ActionEvent> {
    assert!(s.desktop_effects().is_empty(), "no desktop I/O requested");
    s.finish_journal().unwrap();
    let events = Journal::open(&scratch.journal())
        .unwrap()
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    assert!(
        !serde_json::to_string(&events).unwrap().contains(content),
        "journal keeps safe evidence rather than provider content"
    );
    events
}

fn terminal(events: &[ActionEvent], kind: &str, outcome: Outcome) {
    let actions: Vec<_> = events.iter().filter(|e| e.kind.as_str() == kind).collect();
    assert!(!actions.is_empty(), "{kind} persisted");
    assert!(actions.iter().all(|e| e.action_id == actions[0].action_id));
    assert_eq!(
        actions
            .iter()
            .filter(|e| e.outcome.is_terminal())
            .map(|e| e.outcome)
            .collect::<Vec<_>>(),
        vec![outcome],
        "{kind}: exactly one explicit terminal result persisted"
    );
}

fn count(events: &[ActionEvent], kind: &str, number: u32, outcome: Outcome) {
    terminal(events, kind, outcome);
    // Descriptor metadata is stored on Requested, while the terminal event
    // records the outcome separately. Tie both facts to the same observation.
    let evidence: Vec<_> = events
        .iter()
        .filter(|e| e.kind.as_str() == kind && e.outcome == Outcome::Requested)
        .collect();
    assert_eq!(evidence.len(), 1, "one {kind} observation persisted");
    assert_eq!(evidence[0].metadata.count, Some(number));
    assert!(
        events.iter().any(|event| {
            event.action_id == evidence[0].action_id
                && event.outcome == outcome
                && event.sequence > evidence[0].sequence
        }),
        "{kind}: the counted observation has its matching terminal result"
    );
}

#[cfg(feature = "github")]
mod github {
    use super::*;
    use radar_core::host::Event;
    use radar_core::view::{Chord, Key};
    use radar_core::view::{Intent, Node, PaneId};
    use radar_github::test_support::{ExpectedRequest, Reply, Request, hosted_plugin};

    const REPO: &str = "ankh-morpork/watch";
    const TITLE: &str = "Patrol the Brass Bridge";
    const BASE: &str = "https://api.github.com/repos/ankh-morpork/watch";
    const CONFIG: &str = "[theme]\nicons = 'off'\n[github]\ntoken = ''\n[[github.groups]]\nname = 'The Watch'\nrepositories = ['ankh-morpork/watch']\n";

    /// One real checkpoint from the hosted refresh, never a demo world: the
    /// view data a renderer is sent. Default runs verify the checked-in JSON;
    /// regeneration is explicit. Beside it, `github_refresh_120x40.txt` is
    /// Ink's golden of this checkpoint (the text as it was when this test
    /// moved to the view data's text form); this test no longer writes it.
    fn renderer_checkpoint(s: &Scenario) {
        let (snapshot, text) = s.inspect(|host| {
            host.set_view_room(120, 40);
            let text = radar_core::headless::host_text_of(host, 120, 40);
            let snapshot = json!({
                "width": 120,
                "height": 40,
                "chrome": host.chrome_view(120, 40),
                // The shell reserves one row each for its top and status bars.
                "screen": host.screen_view_in(0, "github", 120, 38),
                "section": null,
                "focus": PaneId::from(host.focus),
            });
            (
                serde_json::to_string_pretty(&snapshot).unwrap() + "\n",
                text,
            )
        });
        assert!(
            text.contains(TITLE),
            "the view data's text form contains the refreshed PR:\n{text}"
        );
        let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ink/test/fixtures/scenarios/github_refresh_120x40.txt");
        assert!(
            std::fs::read_to_string(&golden).is_ok_and(|t| t.contains(TITLE)),
            "Ink's golden of this checkpoint: {}",
            golden.display()
        );
        let files = [("github_refresh_120x40.json", snapshot)];
        if let Some(dir) = std::env::var_os("RADAR_SCENARIO_FIXTURES_DIR") {
            let dir = PathBuf::from(dir);
            std::fs::create_dir(&dir)
                .expect("scenario fixture destination must be a new directory");
            for (name, contents) in files {
                let mut file = std::fs::File::create_new(dir.join(name)).unwrap();
                use std::io::Write;
                file.write_all(contents.as_bytes()).unwrap();
            }
        } else {
            let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ink/test/fixtures/scenarios");
            for (name, contents) in files {
                assert_eq!(
                    std::fs::read_to_string(dir.join(name))
                        .expect("committed scenario renderer fixture"),
                    contents,
                    "scenario fixture drift: {name}; inspect before explicit regeneration"
                );
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn held_refresh_then_denied_detail_preserves_rows_and_persists_failure() {
        let scratch = Scratch::new();
        let repository = json!({
            "id":52,"full_name":REPO,"name":"watch","owner":{"login":"ankh-morpork"},
            "html_url":"https://github.com/ankh-morpork/watch","description":"City Watch patrols",
            "default_branch":"main","private":false,"archived":false
        });
        let pull = json!({
            "number":12,"html_url":"https://github.com/ankh-morpork/watch/pull/12",
            "title":TITLE,"body":"Vimes needs the bridge report","user":{"login":"vimes"},
            "state":"open","draft":false,"merged":false,
            "head":{"sha":"a".repeat(40),"ref":"patrol","repo":repository.clone()},
            "base":{"sha":"b".repeat(40),"ref":"main","repo":repository.clone()},
            "updated_at":"2026-10-05T12:00:00Z"
        });
        let (plugin, fixture) = hosted_plugin(vec![
            ExpectedRequest::json("repository", Request::get(BASE), repository),
            ExpectedRequest::json(
                "pulls",
                Request::get(format!(
                    "{BASE}/pulls?state=open&sort=updated&direction=desc&per_page=100&page=1"
                )),
                json!([pull]),
            )
            .held(),
            ExpectedRequest::reply(
                "detail",
                Request::get(format!("{BASE}/pulls/12")),
                Reply {
                    status: 403,
                    headers: Default::default(),
                    body: Ok(br#"{"message":"Forbidden"}"#.to_vec()),
                },
            )
            .held(),
        ])
        .unwrap();
        let mut s = scenario(plugin, CONFIG, &scratch);
        s.advance_time(NOW).unwrap();
        s.settle_until(BUDGET, |_| fixture.ledger().len() == 2)
            .await
            .unwrap();
        assert!(!fixture.ledger()[1].completed);
        assert!(
            !serde_json::to_string(&view(&s, "github"))
                .unwrap()
                .contains(TITLE)
        );
        pending(&s, "github.repository.read");
        fixture.release("pulls").unwrap();
        s.settle(BUDGET).await.unwrap();
        assert!(
            !s.cache_writes().is_empty(),
            "refresh reached the real reducer"
        );

        renderer_checkpoint(&s);
        let screen = view(&s, "github");
        let Node::Tree(tree) = &screen.pane(PaneId::Middle).unwrap().body else {
            panic!("GitHub repository tree: {screen:?}")
        };
        let (path, row) = tree
            .visible()
            .into_iter()
            .find(|(_, row)| row.id == "pull:12")
            .expect("visible refreshed pull request row");
        assert_eq!(path.get(1), Some(&format!("repo:{REPO}")));
        assert!(serde_json::to_string(&row.name).unwrap().contains(TITLE));
        let target = radar_core::view::TreeRow::target_of(&path);
        s.dispatch(Event::Intent(Intent::Activate { target }))
            .unwrap();
        s.settle_until(BUDGET, |_| fixture.ledger().len() == 3)
            .await
            .unwrap();
        pending(&s, "github.pull-request.read");
        fixture.release("detail").unwrap();
        s.settle(BUDGET).await.unwrap();
        let detail = serde_json::to_string(&view(&s, "github")).unwrap();
        assert!(
            detail.contains("access denied"),
            "real provider denial reaches the page"
        );
        assert!(
            detail.contains("details have not been loaded"),
            "denial cannot fabricate detail"
        );
        s.advance_time(NOW + 1).unwrap();
        s.settle(BUDGET).await.unwrap();
        s.dispatch(Event::Intent(radar_core::view::Intent::Key { chord: Chord::new(Key::Esc) }))
            .unwrap();
        assert!(
            serde_json::to_string(&view(&s, "github"))
                .unwrap()
                .contains(TITLE),
            "the successful list remains available after leaving failed detail"
        );
        fixture.assert_finished();
        assert_eq!(
            fixture.ledger().len(),
            3,
            "failed detail does not silently retry"
        );
        // The checkpoint reads view data and paints no frame.
        let events = persisted(&mut s, &scratch, TITLE);
        terminal(&events, "github.repository.read", Outcome::Succeeded);
        terminal(&events, "github.pull-request.read", Outcome::Failed);
        count(&events, "github.read.items", 1, Outcome::Succeeded);
    }
}

#[cfg(feature = "gitlab")]
mod gitlab {
    use super::*;
    use radar_gitlab::test_support::{ExpectedRequest, hosted_plugin, initial_group_requests};

    const TITLE: &str = "Repair the Watch bell";
    const SERVER: &str = "https://gitlab.example.test";
    const CONFIG: &str = "[gitlab]\nurl = 'https://gitlab.example.test'\ntoken = 'glab'\n[[gitlab.source]]\ngroup = 'ankh-morpork/watch'\ncode = 'WATCH'\n";

    #[tokio::test(flavor = "current_thread")]
    async fn held_full_refresh_populates_real_view_and_persists_counts() {
        let scratch = Scratch::new();
        let replies = [
            json!({"data":{"group":{"id":"gid://gitlab/Group/12"},"project":null}}),
            json!({"data":{"currentUser":{"username":"vimes"},"group":{"mergeRequests":{
                "nodes":[{"iid":"12","title":TITLE,"state":"opened",
                    "webUrl":"https://gitlab.example.test/ankh-morpork/watch/-/merge_requests/12",
                    "updatedAt":"2026-10-05T12:00:00Z","sourceBranch":"watch-bell","targetBranch":"main",
                    "project":{"fullPath":"ankh-morpork/watch","name":"Watch"},
                    "author":{"username":"vimes","name":"Sam Vimes"}}],
                "pageInfo":{"hasNextPage":false}}}}}),
            json!({"data":{"group":{"projects":{"nodes":[{
                "fullPath":"ankh-morpork/watch","name":"Watch","archived":false,
                "webUrl":"https://gitlab.example.test/ankh-morpork/watch","repository":{"rootRef":"main"}
            }],"pageInfo":{"hasNextPage":false}}}}}),
        ];
        let expected = initial_group_requests(SERVER, "ankh-morpork/watch")
            .into_iter()
            .zip(replies)
            .enumerate()
            .map(|(index, (request, reply))| {
                let request = ExpectedRequest::json(format!("refresh-{index}"), request, reply);
                if index == 2 { request.held() } else { request }
            })
            .collect();
        let (plugin, fixture) = hosted_plugin(expected).unwrap();
        let mut s = scenario(plugin, CONFIG, &scratch);
        s.advance_time(NOW).unwrap();
        s.settle_until(BUDGET, |_| fixture.ledger().len() == 3)
            .await
            .unwrap();
        assert!(!fixture.ledger()[2].completed);
        pending(&s, "gitlab.sync.full");
        assert!(
            !serde_json::to_string(&view(&s, "repos"))
                .unwrap()
                .contains(TITLE)
        );
        fixture.release("refresh-2").unwrap();
        s.settle(BUDGET).await.unwrap();
        fixture.assert_finished();
        assert!(
            !s.cache_writes().is_empty(),
            "full refresh reached the real reducer"
        );
        assert!(
            serde_json::to_string(&view(&s, "repos"))
                .unwrap()
                .contains(TITLE)
        );
        let events = persisted(&mut s, &scratch, TITLE);
        terminal(&events, "gitlab.sync.full", Outcome::Succeeded);
        count(&events, "gitlab.sync.merge_requests", 1, Outcome::Succeeded);
        count(&events, "gitlab.sync.projects", 1, Outcome::Succeeded);
    }
}

#[cfg(feature = "jira")]
mod jira {
    use super::*;
    use radar_jira::test_support::{ExpectedRequest, Method, Req, hosted_plugin};

    const TITLE: &str = "Patrol the Shades";
    const CONFIG: &str = "[jira]\n[[jira.site]]\nname = 'watch'\nurl = 'https://watch.invalid'\nemail = 'env:RADAR_JIRA_FIXTURE_EMAIL_NEVER_RESOLVE'\ntoken = 'env:RADAR_JIRA_FIXTURE_TOKEN_NEVER_RESOLVE'\n[[jira.site.project]]\nkey = 'WATCH'\n[[jira.view]]\nname = 'Watch patrol'\njql = 'project = WATCH'\nsite = 'watch'\n";

    fn get(path: &str) -> Req {
        Req {
            method: Method::Get,
            path: path.into(),
            body: None,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn held_partial_refresh_shows_valid_ticket_and_persists_omission() {
        let scratch = Scratch::new();
        let search = "/rest/api/3/search/jql?jql=project%20%3D%20WATCH%20ORDER%20BY%20updated%20DESC&fields=summary%2Cissuetype%2Cstatus%2Cpriority%2Cassignee%2Creporter%2Ccreated%2Cupdated%2Cparent%2Clabels%2Ccomponents%2CfixVersions%2Cproject%2Cdescription&maxResults=100";
        let (plugin, fixture) = hosted_plugin(vec![
            ExpectedRequest::json(
                "identity",
                "watch",
                "https://watch.invalid",
                get("/rest/api/3/myself"),
                json!({"accountId":"vimes","displayName":"Sam Vimes"}),
            ),
            ExpectedRequest::json(
                "tickets",
                "watch",
                "https://watch.invalid",
                get(search),
                json!({
                    "issues":[{"id":"10012","key":"WATCH-12","fields":{
                        "summary":TITLE,"issuetype":{"name":"Task"},
                        "status":{"name":"To Do","statusCategory":{"key":"new"}},
                        "project":{"key":"WATCH","name":"City Watch"},
                        "updated":"2026-10-05T12:00:00.000+0000"}},
                        {"id":"10013","fields":{"summary":"Missing the ticket key"}}],
                    "isLast":true
                }),
            )
            .held(),
            ExpectedRequest::json(
                "code",
                "watch",
                "https://watch.invalid",
                get("/rest/dev-status/latest/issue/summary?issueId=10012"),
                json!({"summary":{"pullrequest":{"byInstanceType":{}}}}),
            ),
        ])
        .unwrap();
        let mut s = scenario(plugin, CONFIG, &scratch);
        s.advance_time(NOW).unwrap();
        s.settle_until(BUDGET, |_| fixture.ledger().len() == 2)
            .await
            .unwrap();
        assert!(!fixture.ledger()[1].completed);
        pending(&s, "jira.sync.view");
        assert!(
            !serde_json::to_string(&view(&s, "tickets"))
                .unwrap()
                .contains(TITLE)
        );
        fixture.release("tickets").unwrap();
        s.settle(BUDGET).await.unwrap();
        fixture.assert_finished();
        assert_eq!(
            fixture.ledger().len(),
            3,
            "only the valid ticket has a code read"
        );
        assert!(
            !s.cache_writes().is_empty(),
            "partial refresh reached the real reducer"
        );
        let screen = serde_json::to_string(&view(&s, "tickets")).unwrap();
        assert!(screen.contains(TITLE));
        assert!(!screen.contains("Missing the ticket key"));
        let events = persisted(&mut s, &scratch, TITLE);
        terminal(&events, "jira.sync.view", Outcome::Partial);
        count(&events, "jira.sync.tickets", 1, Outcome::Partial);
        count(&events, "jira.sync.issues.omitted", 1, Outcome::Partial);
    }
}
