//! One real GitLab + Sessions review request, stopped by the host's missing
//! worktree provider. Only GitLab transport and the agent adapter are scripted.
//! No Git plugin, CLI, process, MCP server, notification or successful review is
//! simulated. Settings may inspect PATH executable metadata and current_dir;
//! explicit scratch paths win. This fixture makes no zero-ambient-read claim.
#![cfg(all(feature = "gitlab", feature = "sessions"))]

#[path = "support/probes.rs"]
mod probes;
use probes::{Probe, require, verify};

use std::path::PathBuf;

use radar_core::agent_context::REVIEW_LOCALLY_ACTION;
use radar_core::dock::DockView;
use radar_core::host::Event;
use radar_core::journal::{ActionEvent, ActionId, Journal, Origin, Outcome, Query};
use radar_core::scenario::{JournalRecord, Scenario};
use radar_core::view::{Intent, Node, PaneId, ScreenView, Target, TreeRow};
use radar_core::writes::Mode;
use radar_core::{Host, Pane, Point};
use radar_gitlab::test_support::{
    ExpectedRequest, HostedFixture, Method, MrBuilder, Req, Request, RequestRecord, hosted_plugin,
    initial_group_requests,
};
use radar_sessions::SessionsPlugin;
use radar_sessions::fake::{FakeCall, FakeProvider};
use serde_json::{Value, json};

const NOW: i64 = 1_791_271_000;
const BUDGET: usize = 128;
const SERVER: &str = "https://gitlab.example.test";
const PROJECT: &str = "ankh-morpork/watch";
const TITLE: &str = "Repair the Watch bell";
const DESCRIPTION: &str = "The Watch bell needs a new clapper.";
const HEAD: &str = "1234567890abcdef1234567890abcdef12345678";
const REFUSAL: &str = "review of Repair the Watch bell failed: could not make the worktree: no part here makes worktrees";

// Exact production MR-detail wire query, including its shared MR fields. This
// deliberate expectation catches request drift; it does not normalize queries
// or auto-answer arbitrary detail requests.
const DETAIL_QUERY: &str = r#"
query($project: ID!, $iid: String!) {
  project(fullPath: $project) {
    mergeRequest(iid: $iid) {
        iid title draft webUrl updatedAt sourceBranch targetBranch diffHeadSha userNotesCount
        conflicts detailedMergeStatus approved approvalsRequired approvalsLeft
        shouldBeRebased mergeabilityChecks { identifier status }
        author { username name }
        reviewers { nodes { username } }
        approvedBy { nodes { username } }
        headPipeline { status path duration stages { nodes { name status } } }
        resolvableDiscussionsCount resolvedDiscussionsCount
        notes(filter: ONLY_COMMENTS, last: 1) { nodes { url body createdAt author { username name } } }
        diffStatsSummary { additions deletions fileCount }
        project { name fullPath }
        state mergedAt closedAt mergeError mergeOngoing
        labels { nodes { title } }

      description
      diffStats { path additions deletions }
      diffRefs { baseSha headSha startSha }
      squashOnMerge shouldRemoveSourceBranch forceRemoveSourceBranch
      commits(first: 100) { nodes { shortId title authorName authoredDate webUrl } }
      jobs: headPipeline {
        iid
        stages { nodes { name status jobs(first: 100) { nodes { name status duration webPath allowFailure } } } }
      }
      discussions(first: 100) {
        nodes {
          id resolved resolvable
          notes(first: 100) {
            nodes { body system createdAt author { username name } position { filePath newLine oldLine } }
          }
        }
      }
    }
  }
}"#;

fn detail_request() -> Request {
    Request::new(
        SERVER,
        Req::new(
            Method::Post,
            "/api/graphql",
            Some(json!({
                "query": DETAIL_QUERY, "variables": {"project": PROJECT, "iid": "12"}
            })),
        ),
    )
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "radar-review-scenario-{}",
            ActionId::new().as_str()
        ));
        std::fs::create_dir(&path).unwrap();
        let scratch = Self(path.canonicalize().unwrap());
        for name in ["work", "recipes", "reviews"] {
            std::fs::create_dir(scratch.0.join(name)).unwrap();
        }
        std::fs::write(scratch.0.join("work/patrol.txt"), "Watch patrol notes\n").unwrap();
        scratch
    }

    fn journal(&self) -> PathBuf {
        self.0.join("actions.sqlite3")
    }

    fn config(&self) -> String {
        let path = |name| serde_json::to_string(self.0.join(name).to_str().unwrap()).unwrap();
        format!(
            "[theme]\nicons = 'off'\n\
             [gitlab]\nurl = '{SERVER}'\ntoken = 'glab'\n\
             [[gitlab.source]]\ngroup = '{PROJECT}'\ncode = 'WATCH'\n\
             [sessions]\nprovider = 'fake'\nnotify = false\n\
             work_dir = {}\nrecipes_dir = {}\nreviews_dir = {}\n\
             project_config = false\nlocal_only = true\n\
             permitted_roots = []\nskill_dirs = []\nallow = []\ndeny = []\npty = []\n\
             [sessions.providers.fake]\nskill_dirs = []\ninstruction_dirs = []\nplugin_dirs = []\n",
            path("work"),
            path("recipes"),
            path("reviews"),
        )
    }

    fn unchanged(&self) -> bool {
        std::fs::read(self.0.join("work/patrol.txt")).unwrap() == b"Watch patrol notes\n"
            && std::fs::read_dir(self.0.join("work")).unwrap().count() == 1
            && std::fs::read_dir(self.0.join("recipes")).unwrap().count() == 0
            && std::fs::read_dir(self.0.join("reviews")).unwrap().count() == 0
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn view(s: &Scenario) -> ScreenView {
    s.inspect(|host| {
        host.screen_view_in(0, "repos", 120, 40)
            .expect("real GitLab view")
    })
}

fn mr_target(rows: &[TreeRow], prefix: &[String]) -> Option<Target> {
    for row in rows {
        let mut path = prefix.to_vec();
        path.push(row.id.clone());
        if serde_json::to_string(&row.name).unwrap().contains(TITLE) {
            return Some(TreeRow::target_of(&path));
        }
        if let Some(target) = mr_target(&row.children, &path) {
            return Some(target);
        }
    }
    None
}

/// Named starting state: one refreshed, selected MR; the only other part is
/// Sessions with its fake adapter. No cache restore or private result message.
async fn at_selected_mr_without_worktree_provider(
    scratch: &Scratch,
    fake: &FakeProvider,
) -> (Scenario, HostedFixture) {
    let mr = MrBuilder::default().build();
    assert_eq!(mr.head_sha, HEAD);
    assert_eq!(HEAD.len(), 40, "the provider supplies a full head SHA");
    let row = json!({
        "iid": mr.iid.to_string(), "title": mr.title, "state": "opened",
        "draft": mr.draft, "webUrl": mr.web_url, "updatedAt": mr.updated_at,
        "sourceBranch": mr.source_branch, "targetBranch": mr.target_branch,
        "diffHeadSha": mr.head_sha,
        "project": {"fullPath": mr.project_path, "name": mr.project_name},
        "author": {"username": mr.author.username, "name": mr.author.name}
    });
    let replies = [
        json!({"data":{"group":{"id":"gid://gitlab/Group/12"},"project":null}}),
        json!({"data":{"currentUser":{"username":"vimes"},"group":{"mergeRequests":{
            "nodes":[row.clone()],"pageInfo":{"hasNextPage":false}}}}}),
        json!({"data":{"group":{"projects":{"nodes":[{
            "fullPath": PROJECT, "name": "Watch", "archived": false,
            "webUrl": format!("{SERVER}/{PROJECT}"), "repository":{"rootRef":"main"}
        }],"pageInfo":{"hasNextPage":false}}}}}),
    ];
    let mut expected: Vec<_> = initial_group_requests(SERVER, PROJECT)
        .into_iter()
        .zip(replies)
        .enumerate()
        .map(|(i, (request, reply))| ExpectedRequest::json(format!("refresh-{i}"), request, reply))
        .collect();
    let mut detail = row;
    detail["description"] = json!(DESCRIPTION);
    detail["diffRefs"] =
        json!({"baseSha":"b".repeat(40), "headSha":HEAD, "startSha":"b".repeat(40)});
    detail["diffStats"] = json!([]);
    detail["commits"] = json!({"nodes":[]});
    detail["discussions"] = json!({"nodes":[]});
    expected.push(
        ExpectedRequest::json(
            "detail",
            detail_request(),
            json!({"data":{"project":{"mergeRequest":detail}}}),
        )
        .held(),
    );
    let (gitlab, fixture) = hosted_plugin(expected).unwrap();
    // Set review storage before configure, which otherwise may load an ambient
    // reviews directory. No native adapters or tool-server factory are installed.
    let mut sessions = SessionsPlugin::new().with_review_dir(scratch.0.join("reviews"));
    sessions.add_adapter(Box::new(fake.adapter()));
    let mut host = Host::new(vec![Box::new(gitlab), Box::new(sessions)], NOW).unwrap();
    host.load_config(Some(&scratch.config())).unwrap();
    host.set_env_mode(Some((Mode::Write, "scenario".into())));
    host.focus = Pane::Middle;
    let mut s = Scenario::with_journal(host, &scratch.journal()).unwrap();
    s.advance_time(NOW).unwrap();
    s.settle(BUDGET).await.unwrap();
    let screen = view(&s);
    let Node::Tree(tree) = &screen.pane(PaneId::Middle).expect("middle pane").body else {
        panic!("expected the real GitLab repository tree");
    };
    let target = mr_target(&tree.rows, &[]).expect("refreshed MR row");
    s.dispatch(Event::Intent(Intent::Select { target }))
        .unwrap();
    s.settle(BUDGET).await.unwrap();
    assert!(
        view(&s)
            .actions
            .iter()
            .any(|a| a.id == REVIEW_LOCALLY_ACTION)
    );
    let selected = s.host().plugins[0]
        .subject("repos", Pane::Middle)
        .expect("selected MR");
    assert_eq!(selected.kind, "merge-request");
    assert_eq!(selected.id, format!("{PROJECT}!12"));
    assert_eq!(selected.field("head_sha"), Some(HEAD));
    assert_eq!(
        fixture.ledger().len(),
        3,
        "selection does not fetch MR detail"
    );
    assert!(
        fake.has_live_subscriber(),
        "real Sessions subscription started"
    );
    (s, fixture)
}

fn begin<'a>(records: &'a [JournalRecord], kind: &str) -> &'a ActionId {
    let ids: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. } if descriptor.kind.as_str() == kind => {
                Some(id)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        ids.len(),
        1,
        "one {kind} action is a checkpoint prerequisite"
    );
    ids[0]
}

fn terminal(records: &[JournalRecord], id: &ActionId) -> Vec<Outcome> {
    records
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Outcome {
                id: actual,
                outcome,
                ..
            } if actual == id && outcome.is_terminal() => Some(*outcome),
            _ => None,
        })
        .collect()
}

/// Captured values only. Associated probes cannot drive a second host or
/// dispatch, release, read files, or invoke providers from inside a check.
struct AwaitingDetail {
    requests: Vec<RequestRecord>,
    records: Vec<JournalRecord>,
    request: ActionId,
    detail: ActionId,
    calls: Vec<FakeCall>,
    dock: DockView,
    notice: Option<String>,
    desktop_empty: bool,
}

fn awaiting_detail(s: &Scenario, fixture: &HostedFixture, fake: &FakeProvider) {
    let c = AwaitingDetail {
        requests: fixture.ledger(),
        records: s.records().to_vec(),
        request: begin(s.records(), "agent.review.request").clone(),
        detail: begin(s.records(), "gitlab.page.detail").clone(),
        calls: fake.observations(),
        dock: s.inspect(|h| *h.dock_view().unwrap()),
        notice: s.host().message.as_ref().map(ToString::to_string),
        desktop_empty: s.desktop_effects().is_empty(),
    };
    verify("review awaiting real MR detail", &c, &[
        Probe { name: "exact detail request remains held", check: |c| require(
            c.requests.len() == 4 && c.requests[3].key.as_deref() == Some("detail")
                && c.requests[3].request == detail_request() && !c.requests[3].completed,
            "the fourth exact request must be detail and remain unfinished") },
        Probe { name: "accepted request is not completed review", check: |c| require(
            terminal(&c.records, &c.request) == [Outcome::Succeeded]
                && terminal(&c.records, &c.detail).is_empty(),
            "request dispatch must be accepted while the actual detail read has no terminal result") },
        Probe { name: "no premature agent turn", check: |c| require(
            c.calls == [FakeCall::Subscribe] && c.dock.tabs.is_empty() && c.dock.transcript.is_empty(),
            "resolving an MR must not create or send to an agent") },
        Probe { name: "no premature worktree result", check: |c| require(
            !c.notice.as_deref().unwrap_or("").contains("worktree") && c.desktop_empty,
            "a held resolution cannot announce a worktree outcome or request desktop I/O") },
    ]).unwrap_or_else(|error| panic!("{error}"));
}

async fn pump_until_refusal(s: &mut Scenario, fake: &FakeProvider) {
    // Scenario's idle state covers host effects only; Sessions also has detached
    // jobs. Real wakes and ticks, bounded by a domain predicate, drive both.
    for _ in 0..BUDGET {
        tokio::task::yield_now().await;
        s.dispatch(Event::Wake(NOW)).unwrap();
        s.dispatch(Event::Tick(NOW)).unwrap();
        s.settle(BUDGET).await.unwrap();
        assert_eq!(fake.script_failure(), None);
        if s.host()
            .message
            .as_ref()
            .is_some_and(|message| message.text == REFUSAL)
        {
            return;
        }
    }
    panic!(
        "missing worktree refusal did not arrive: {:?}; calls={:?}",
        s.host().message,
        fake.observations()
    );
}

struct RefusedReview {
    notice: Option<String>,
    dock: DockView,
    calls: Vec<FakeCall>,
    creates: usize,
    sessions: usize,
    running: usize,
    scripts: (usize, usize),
    script_failure: Option<String>,
    reviews: Value,
    chip_absent: bool,
    review_actions_absent: bool,
    files_unchanged: bool,
    work_dir: String,
    expected_work_dir: String,
    desktop_empty: bool,
    requests: Vec<RequestRecord>,
    transport_result: Result<(), String>,
    records: Vec<JournalRecord>,
    events: Vec<ActionEvent>,
    request: ActionId,
    idle: bool,
}

fn refused_review(
    s: &mut Scenario,
    fixture: &HostedFixture,
    fake: &FakeProvider,
    scratch: &Scratch,
) {
    let request = begin(s.records(), "agent.review.request").clone();
    s.finish_journal().unwrap();
    let events = Journal::open(&scratch.journal())
        .unwrap()
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    assert!(events.len() < 1000, "bounded journal query is complete");
    let selected = s.host().plugins[0]
        .subject("repos", Pane::Middle)
        .expect("selected MR retained");
    let sessions = &s.host().plugins[1];
    let snapshot = sessions.snapshot();
    let c = RefusedReview {
        notice: s.host().message.as_ref().map(ToString::to_string),
        dock: s.inspect(|h| *h.dock_view().unwrap()),
        calls: fake.observations(),
        creates: fake.calls("create"),
        sessions: fake.session_count(),
        running: fake.running(),
        scripts: (fake.consumed_scripts(), fake.remaining_scripts()),
        script_failure: fake.script_failure(),
        reviews: snapshot
            .as_ref()
            .map(|v| v["reviews"]["records"].clone())
            .unwrap_or(json!([])),
        chip_absent: sessions.contribute(Point::Chips, &selected).is_none(),
        review_actions_absent: sessions.contribute(Point::Actions, &selected).is_none(),
        files_unchanged: scratch.unchanged(),
        work_dir: sessions.config().unwrap()["work_dir"]
            .as_str()
            .unwrap()
            .to_owned(),
        expected_work_dir: scratch.0.join("work").to_str().unwrap().to_owned(),
        desktop_empty: s.desktop_effects().is_empty(),
        requests: fixture.ledger(),
        transport_result: fixture.check_finished().map_err(|e| e.to_string()),
        records: s.records().to_vec(),
        events,
        request,
        idle: s.is_idle(),
    };
    verify("review refused without a worktree provider", &c, &[
        Probe { name: "actual host refusal", check: |c| require(
            c.notice.as_deref() == Some(REFUSAL) && c.idle,
            "the real Host worktree refusal must end the request") },
        Probe { name: "no agent session or turn", check: |c| require(
            c.calls == [FakeCall::Subscribe] && c.creates == 0 && c.sessions == 0
                && c.running == 0 && c.scripts == (0, 0) && c.script_failure.is_none()
                && c.dock.tabs.is_empty() && c.dock.transcript.is_empty(),
            "the refusal must not create, send, run, or synthesize a fake turn") },
        Probe { name: "no saved review or contribution", check: |c| require(
            c.reviews == json!([]) && c.chip_absent && c.review_actions_absent,
            "failure must not create review notes, a reviewed chip, or saved-review actions") },
        Probe { name: "explicit paths stay unchanged", check: |c| require(
            c.files_unchanged && c.work_dir == c.expected_work_dir,
            "work/recipe/review files or the explicit work directory changed") },
        Probe { name: "no desktop side effects", check: |c| require(c.desktop_empty,
            "desktop effect requested") },
        Probe { name: "strict GitLab script consumed once", check: |c| {
            c.transport_result.clone()?;
            require(c.requests.len() == 4 && c.requests.iter().all(|r| r.completed)
                && c.requests[3].request == detail_request(), "expected exactly the refresh and detail reads")
        } },
        Probe { name: "persisted request and dispatch truth", check: |c| {
            let requested: Vec<_> = c.events.iter().filter(|e|
                e.kind.as_str() == "agent.review.request" && e.outcome == Outcome::Requested).collect();
            require(requested.len() == 1, "exactly one review request must persist")?;
            let request = requested[0];
            require(request.action_id == c.request && request.origin == Origin::Plugin("gitlab".into()),
                "persisted request must match the actual GitLab action")?;
            let dispatch = request.parent_action_id.as_ref().ok_or("request has no dispatch parent")?;
            require(c.events.iter().any(|e| &e.action_id == dispatch && e.kind.as_str() == "plugin.input"
                && e.origin == Origin::Plugin("gitlab".into()) && e.outcome == Outcome::Requested),
                "request must retain its real GitLab input dispatch parent")?;
            for id in [&c.request, dispatch] {
                require(c.events.iter().filter(|e| &e.action_id == id && e.outcome.is_terminal())
                    .map(|e| e.outcome).collect::<Vec<_>>() == [Outcome::Succeeded],
                    "request and dispatch success mean acceptance, not review completion")?;
            }
            require(terminal(&c.records, &c.request) == [Outcome::Succeeded],
                "persisted acceptance must agree with the actual captured request")
        } },
        Probe { name: "actual detail read succeeded", check: |c| {
            let events: Vec<_> = c.events.iter().filter(|e| e.kind.as_str() == "gitlab.page.detail").collect();
            require(events.iter().filter(|e| e.outcome == Outcome::Requested).count() == 1
                && events.iter().filter(|e| e.outcome.is_terminal()).map(|e| e.outcome)
                    .collect::<Vec<_>>() == [Outcome::Succeeded],
                "the real detail parser/reducer must succeed before worktree refusal")
        } },
        Probe { name: "no fabricated review-success evidence", check: |c| require(
            !c.events.iter().any(|e| e.kind.as_str().starts_with("sessions.")
                || (e.kind.as_str().starts_with("agent.review.") && e.kind.as_str() != "agent.review.request"))
                && !serde_json::to_string(&c.events).unwrap().contains(DESCRIPTION),
            "journal must not invent a semantic review outcome or retain the description") },
    ]).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn selected_gitlab_mr_review_refuses_missing_worktree_provider() {
    let scratch = Scratch::new();
    let fake = FakeProvider::new();
    fake.strict_scripts(); // No queued turn: an accidental send fails closed.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (mut s, fixture) = at_selected_mr_without_worktree_provider(&scratch, &fake).await;
        s.dispatch(Event::Intent(Intent::Invoke {
            action: REVIEW_LOCALLY_ACTION.into(),
        }))
        .unwrap();
        s.dispatch(Event::Tick(NOW)).unwrap();
        s.settle_until(BUDGET, |_| fixture.ledger().len() == 4)
            .await
            .unwrap();
        awaiting_detail(&s, &fixture, &fake);
        fixture.release("detail").unwrap();
        pump_until_refusal(&mut s, &fake).await;
        refused_review(&mut s, &fixture, &fake, &scratch);
        drop(s);
    });
    // The detached Sessions subscriber belongs to this dedicated runtime.
    // Teardown closes it rather than leaving a task on a shared test runtime.
    drop(runtime);
    let teardown = (
        !fake.has_live_subscriber(),
        fake.running(),
        scratch.unchanged(),
    );
    verify(
        "review runtime torn down",
        &teardown,
        &[
            Probe {
                name: "subscriber closed",
                check: |c| require(c.0, "detached subscriber remains live"),
            },
            Probe {
                name: "no leftover work",
                check: |c| {
                    require(
                        c.1 == 0 && c.2,
                        "running agent work or changed scratch files remain",
                    )
                },
            },
        ],
    )
    .unwrap_or_else(|error| panic!("{error}"));
}
