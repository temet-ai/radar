//! The two legacy demo-backed page-confirm tests, split at their real boundary.
//! Unit cases own only fixed-time models and question/reducer effects. One host
//! checks x-menu replay, direct keys, provider tasks, refreshes and pending text.
//!
//! Assertion map:
//! - the_merge_request_pages_actions_ask_first_and_say_why_not: rebase question,
//!   blocked-merge reason and no-before-yes -> unit plus the hosted x choices;
//!   decline -> unit plus hosted n; yes/spawn/pending -> held hosted rebase.
//! - a_mergeable_merge_request_offers_the_merge_behind_a_confirm: web/!187 route,
//!   x's merge label, irreversible question and yes/spawn -> hosted merge, with
//!   the question itself also checked at unit speed.
//! - choosing_a_row_in_the_keys_menu_presses_its_key stays unchanged in lib.rs.

use crate::model::Mr;
use crate::test_support::MrBuilder;
use crate::{GitlabPlugin, Settings};

const NOW: i64 = 1_791_273_600;
const SERVER: &str = "https://gitlab.example.test";
const GROUP: &str = "ankh-morpork/watch";
const API: &str = "ankh-morpork/watch/api";
const WEB: &str = "ankh-morpork/watch/web";

// Keep the legacy iid/branch question words while shedding the unrelated demo.
fn needs_approval() -> Mr {
    let mut mr = MrBuilder::default()
        .identity(SERVER, API, 263)
        .title("Repair the Watch bell")
        .build();
    mr.source_branch = "feature/ABC-263".into();
    mr.approvals_required = 1;
    mr.approvals_left = 1;
    mr
}

fn approved() -> Mr {
    let mut mr = MrBuilder::default()
        .identity(SERVER, WEB, 187)
        .title("Publish the Watch rota")
        .build();
    mr.approved = true;
    mr
}

mod unit {
    use super::*;
    use radar_core::journal::Outcome;
    use radar_core::{Effect, Menu};

    // Default is intentional: GitlabPlugin::new registers log secrets and reads
    // the wall clock before demo() overwrites it. No host, view or I/O here.
    fn plugin() -> GitlabPlugin {
        GitlabPlugin {
            cfg: Settings {
                url: SERVER.into(),
                token: "env:RADAR_GITLAB_FIXTURE_NEVER_RESOLVE".into(),
                ..Default::default()
            },
            now: NOW,
            env: Some(vec![]),
            scripted_fixture: true,
            ..Default::default()
        }
    }

    fn question(effects: &[Effect]) -> &Menu {
        assert!(
            effects
                .iter()
                .all(|e| matches!(e, Effect::Journal(_) | Effect::Menu(_)))
        );
        effects
            .iter()
            .find_map(|e| match e {
                Effect::Menu(menu) => Some(menu),
                _ => None,
            })
            .expect("a question, with no provider task")
    }

    #[test]
    fn rebase_question_and_decline_do_not_start_provider_work() {
        let mut p = plugin();
        let effects = crate::pages::act::ask_rebase(&p, &needs_approval());
        let menu = question(&effects);
        assert_eq!(menu.title, "rebase feature/ABC-263 onto main?");
        assert_eq!(menu.id, format!("act-rebase:{API}!263"));
        assert!(menu.question);
        assert_eq!(menu.change, Some(radar_core::writes::Change::Edit));
        assert!(
            menu.items
                .iter()
                .any(|i| i.note && i.label == "GitLab rewrites the branch on top of main")
        );
        let no = menu.items.iter().find(|i| i.key == Some('n')).unwrap();
        assert_eq!(no.hint, "no, leave it as it is");
        let effects = crate::pages::act::on_menu(&mut p, &menu.id, &no.value).unwrap();
        assert!(matches!(
            effects.as_slice(),
            [Effect::Report(Outcome::Cancelled)]
        ));
        assert!(p.acting.is_empty());
    }

    #[test]
    fn blocked_merge_explains_the_missing_approval_without_a_question_or_task() {
        let p = plugin();
        let effects = crate::pages::act::ask_merge(&p, &needs_approval());
        assert!(
            matches!(effects.as_slice(), [Effect::Journal(_), Effect::Report(Outcome::Blocked), Effect::Notify(message)]
            if message == "cannot merge !263 yet: needs 1 approval")
        );
        assert!(p.acting.is_empty());
    }

    #[test]
    fn approved_merge_question_warns_that_it_cannot_be_undone() {
        let p = plugin();
        let effects = crate::pages::act::ask_merge(&p, &approved());
        let menu = question(&effects);
        assert_eq!(menu.title, "merge !187 into main? It cannot be undone.");
        assert_eq!(menu.id, format!("act-merge:{WEB}!187"));
        assert!(menu.question);
        assert_eq!(menu.change, Some(radar_core::writes::Change::Edit));
        assert!(
            menu.items
                .iter()
                .any(|i| i.key == Some('y') && i.hint == "yes, merge it")
        );
        assert!(menu.items.iter().any(|i| i.note && i.label == "GitLab checks it once more, then merges that commit"));
        assert!(p.acting.is_empty());
    }
}

mod integration {
    use super::*;
    use crate::test_support::{
        ExpectedRequest, HostedFixture, Method, Req, Request, Resp, hosted_plugin,
        initial_group_requests,
    };
    use radar_core::host::Event;
    use radar_core::view::{Chord, Key};
    use radar_core::scenario::Scenario;
    use radar_core::view::{Intent, Node, PaneId, ScreenView, Target, TreeRow};
    use radar_core::writes::Mode;
    use radar_core::{Host, Pane};
    use serde_json::{Value, json};

    const BUDGET: usize = 128;

    // Deliberately exact wire expectation: no wildcard/substring provider
    // matching and no private result-message injection.
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
        state mergedAt closedAt
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

    fn graphql(query: &str, variables: Value) -> Request {
        Request::new(
            SERVER,
            Req::new(
                Method::Post,
                "/api/graphql",
                Some(json!({"query": query, "variables": variables})),
            ),
        )
    }

    fn detail_request(mr: &Mr) -> Request {
        graphql(
            DETAIL_QUERY,
            json!({"project": mr.project_path, "iid": mr.iid.to_string()}),
        )
    }

    fn row(mr: &Mr) -> Value {
        json!({
            "iid": mr.iid.to_string(), "title": mr.title, "state": "opened",
            "draft": mr.draft, "webUrl": mr.web_url, "updatedAt": mr.updated_at,
            "sourceBranch": mr.source_branch, "targetBranch": mr.target_branch,
            "diffHeadSha": mr.head_sha, "detailedMergeStatus": mr.merge_status,
            "conflicts": mr.conflicts, "approved": mr.approved,
            "approvalsRequired": mr.approvals_required, "approvalsLeft": mr.approvals_left,
            "project": {"fullPath": mr.project_path, "name": mr.project_name},
            "author": {"username": mr.author.username, "name": mr.author.name}
        })
    }

    fn detail(mut row: Value, description: &str) -> Value {
        row["description"] = json!(description);
        row["diffStats"] = json!([]);
        row["commits"] = json!({"nodes": []});
        row["discussions"] = json!({"nodes": []});
        row["squashOnMerge"] = json!(true);
        row["shouldRemoveSourceBranch"] = json!(false);
        json!({"data": {"project": {"mergeRequest": row}}})
    }

    fn refresh(expected: &mut Vec<ExpectedRequest>, label: &str, rows: Vec<Value>, first: bool) {
        let responses = [
            json!({"data":{"group":{"id":"gid://gitlab/Group/12"},"project":null}}),
            json!({"data":{"currentUser":{"username":"vimes"},"group":{"mergeRequests":{
                "nodes":rows,"pageInfo":{"hasNextPage":false}}}}}),
            json!({"data":{"group":{"projects":{"nodes":[
                {"fullPath": API, "name":"api", "archived":false, "repository":{"rootRef":"main"}},
                {"fullPath": WEB, "name":"web", "archived":false, "repository":{"rootRef":"main"}}
            ],"pageInfo":{"hasNextPage":false}}}}}),
        ];
        expected.extend(
            initial_group_requests(SERVER, GROUP)
                .into_iter()
                .zip(responses)
                .enumerate()
                .skip(usize::from(!first))
                .map(|(i, (request, response))| {
                    ExpectedRequest::json(format!("{label}-{i}"), request, response)
                }),
        );
    }

    fn script() -> Vec<ExpectedRequest> {
        let rebase = needs_approval();
        let merge = approved();
        let mut expected = vec![];
        refresh(
            &mut expected,
            "initial",
            vec![row(&rebase), row(&merge)],
            true,
        );
        expected.push(ExpectedRequest::json(
            "rebase-page",
            detail_request(&rebase),
            detail(row(&rebase), "Before the rebase."),
        ));
        expected.push(
            ExpectedRequest::reply(
                "rebase",
                Request::new(
                    SERVER,
                    Req::new(
                        Method::Put,
                        "/api/v4/projects/ankh-morpork%2Fwatch%2Fapi/merge_requests/263/rebase",
                        None,
                    ),
                ),
                Resp {
                    status: 202,
                    body: br#"{"rebase_in_progress":true}"#.to_vec(),
                    ..Default::default()
                },
            )
            .held(),
        );
        expected.push(ExpectedRequest::json(
            "rebase-page-refreshed",
            detail_request(&rebase),
            detail(row(&rebase), "The rebase request was accepted."),
        ));
        refresh(
            &mut expected,
            "after-rebase",
            vec![row(&rebase), row(&merge)],
            false,
        );
        expected.push(ExpectedRequest::json(
            "merge-page",
            detail_request(&merge),
            detail(row(&merge), "Before the merge."),
        ));
        expected.push(ExpectedRequest::json("fresh-before-merge", graphql(crate::mr_actions::FRESH_QUERY,
            json!({"project":WEB,"iid":"187"})),
            json!({"data":{"project":{"mergeRequest":{
                "diffHeadSha": merge.head_sha, "detailedMergeStatus":"MERGEABLE",
                "draft":false,"conflicts":false,"approved":true,"approvalsLeft":0,"state":"opened"
            }}}})).held());
        expected.push(ExpectedRequest::json("merge", graphql(crate::mr_actions::ACCEPT_MUTATION,
            json!({"project":WEB,"iid":"187","sha":merge.head_sha,"squash":true,"remove":false})),
            json!({"data":{"mergeRequestAccept":{"errors":[],"mergeRequest":{"state":"merged","detailedMergeStatus":"MERGEABLE"}}}})).held());
        let mut merged = row(&merge);
        merged["state"] = json!("merged");
        expected.push(ExpectedRequest::json(
            "merge-page-refreshed",
            detail_request(&merge),
            detail(merged, "The rota was merged."),
        ));
        refresh(&mut expected, "after-merge", vec![row(&rebase)], false);
        expected
    }

    fn view(s: &Scenario) -> ScreenView {
        s.inspect(|host| host.screen_view_in(0, "repos", 160, 40).unwrap())
    }

    fn target(rows: &[TreeRow], prefix: &[String], title: &str) -> Option<Target> {
        for row in rows {
            let mut path = prefix.to_vec();
            path.push(row.id.clone());
            if row.name.words().contains(title) {
                return Some(TreeRow::target_of(&path));
            }
            if let Some(found) = target(&row.children, &path, title) {
                return Some(found);
            }
        }
        None
    }

    fn press(s: &mut Scenario, code: Key) {
        s.dispatch(crate::key_event(Chord::new(code)))
            .unwrap();
    }

    async fn open(s: &mut Scenario, mr: &Mr) {
        let screen = view(s);
        let Node::Tree(tree) = &screen.pane(PaneId::Middle).unwrap().body else {
            panic!("expected the refreshed repository tree");
        };
        let target = target(&tree.rows, &[], &mr.title).expect("the refreshed MR row");
        s.dispatch(Event::Intent(Intent::Select { target }))
            .unwrap();
        press(s, Key::Enter);
        s.settle(BUDGET).await.unwrap();
        let screen = view(s);
        let crumbs: Vec<_> = screen
            .page
            .as_ref()
            .unwrap()
            .crumbs
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(
            &crumbs[crumbs.len() - 2..],
            &[mr.project_name.as_str(), &format!("!{}", mr.iid)]
        );
        let subject = s.host().plugins[0].subject("repos", Pane::Middle).unwrap();
        assert_eq!(subject.id, format!("{}!{}", mr.project_path, mr.iid));
    }

    async fn held(s: &mut Scenario, fixture: &HostedFixture, key: &str) {
        s.settle_until(BUDGET, |_| {
            fixture
                .ledger()
                .iter()
                .any(|r| r.key.as_deref() == Some(key) && !r.completed)
        })
        .await
        .unwrap();
        assert_eq!(
            s.pending_tasks(),
            1,
            "the actual provider task remains held"
        );
    }

    async fn no_work(s: &mut Scenario, fixture: &HostedFixture, calls: usize) {
        assert_eq!(s.pending_tasks(), 0, "no task is spawned before yes");
        s.settle(BUDGET).await.unwrap();
        assert!(s.is_idle());
        assert_eq!(
            fixture.ledger().len(),
            calls,
            "no request before confirmation"
        );
        assert!(
            s.desktop_effects().is_empty(),
            "no browser, clipboard or process action"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn page_keys_confirm_rebase_and_merge_then_ingest_real_refreshes() {
        let (p, fixture) = hosted_plugin(script()).unwrap();
        let mut host = Host::new(vec![Box::new(p)], NOW).unwrap();
        host.load_config(Some(&format!("[theme]\nicons = 'off'\n[gitlab]\nurl = '{SERVER}'\ntoken = 'glab'\n[[gitlab.source]]\ngroup = '{GROUP}'\ncode = 'WATCH'\n"))).unwrap();
        host.set_env_mode(Some((Mode::Edit, "fixture".into())));
        host.focus = Pane::Middle;
        host.set_view_room(160, 40);
        let mut s = Scenario::new(host);
        s.advance_time(NOW).unwrap();
        s.settle(BUDGET).await.unwrap();
        assert_eq!(fixture.ledger().len(), 3);
        open(&mut s, &needs_approval()).await;
        let before = fixture.ledger().len();

        // x really replays the selected page's key, and no means no.
        press(&mut s, Key::Char('x'));
        press(&mut s, Key::Char('u'));
        assert_eq!(
            s.host().menu.as_ref().unwrap().menu.title,
            "rebase feature/ABC-263 onto main?"
        );
        no_work(&mut s, &fixture, before).await;
        press(&mut s, Key::Char('n'));
        assert!(s.host().menu.is_none());
        no_work(&mut s, &fixture, before).await;
        press(&mut s, Key::Char('x'));
        press(&mut s, Key::Char('m'));
        assert!(s.host().menu.is_none());
        assert_eq!(
            s.host().message.as_deref(),
            Some("cannot merge !263 yet: needs 1 approval")
        );
        no_work(&mut s, &fixture, before).await;

        // Direct key, affirmative confirmation, real task, held wire request.
        press(&mut s, Key::Char('u'));
        no_work(&mut s, &fixture, before).await;
        press(&mut s, Key::Char('y'));
        assert!(s.host().menu.is_none());
        assert_eq!(s.pending_tasks(), 1, "yes spawned the real rebase task");
        held(&mut s, &fixture, "rebase").await;
        assert_eq!(fixture.ledger().len(), before + 1);
        // The sole text checkpoint preserves the legacy pending wording.
        let text = s.inspect(|host| crate::tests::shown(host, 160, 40));
        assert!(text.contains(":run_running: rebase asked of GitLab…"), "{text}");
        fixture.release("rebase").unwrap();
        s.settle(BUDGET).await.unwrap();
        let refreshed = serde_json::to_string(&view(&s)).unwrap();
        assert!(refreshed.contains("The rebase request was accepted."));
        assert!(refreshed.contains("rebase requested: GitLab is rebasing the branch"));
        assert!(!refreshed.contains("rebase asked of GitLab…"));
        assert_eq!(fixture.ledger().len(), before + 4);

        press(&mut s, Key::Esc);
        open(&mut s, &approved()).await;
        let before = fixture.ledger().len();
        press(&mut s, Key::Char('x'));
        assert!(
            s.host()
                .menu
                .as_ref()
                .unwrap()
                .menu
                .items
                .iter()
                .any(|i| i.label == "merge into main…")
        );
        press(&mut s, Key::Esc);
        press(&mut s, Key::Char('m'));
        assert_eq!(
            s.host().menu.as_ref().unwrap().menu.title,
            "merge !187 into main? It cannot be undone."
        );
        no_work(&mut s, &fixture, before).await;
        press(&mut s, Key::Char('y'));
        assert_eq!(s.pending_tasks(), 1, "yes spawned the real merge task");
        held(&mut s, &fixture, "fresh-before-merge").await;
        assert_eq!(
            fixture.ledger().len(),
            before + 1,
            "only the fresh prerequisite has started"
        );
        fixture.release("fresh-before-merge").unwrap();
        held(&mut s, &fixture, "merge").await;
        assert_eq!(fixture.ledger().len(), before + 2);
        assert!(
            serde_json::to_string(&view(&s))
                .unwrap()
                .contains("merge asked of GitLab…")
        );
        fixture.release("merge").unwrap();
        s.settle(BUDGET).await.unwrap();
        let refreshed = serde_json::to_string(&view(&s)).unwrap();
        assert!(refreshed.contains("The rota was merged."));
        assert!(!refreshed.contains("merge asked of GitLab…"));
        assert_eq!(fixture.ledger().len(), before + 5);
        assert!(s.is_idle());
        assert!(
            !s.cache_writes().is_empty(),
            "real refresh reducers persisted their parsed state"
        );
        let saved = s.host().plugins[0].snapshot().unwrap();
        let remaining: Vec<_> = saved["data"][GROUP]["mrs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|mr| mr["iid"].as_u64().unwrap())
            .collect();
        assert_eq!(
            remaining,
            [263],
            "the full refresh removed the merged MR from the open list"
        );
        assert!(s.desktop_effects().is_empty());
        fixture.assert_finished();
    }
}
