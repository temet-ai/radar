//! The sample data the GitLab part's tests use, and `radar shot` draws with
//! (`Plugin::demo`): two tenancies, merge requests in every role, a red
//! pipeline with failed jobs and their logs, a local checkout. Neutral names;
//! no live service. Compiled for tests and under the `demo` feature.

use crate::config::Source;
use crate::model::fixtures::mr;
use crate::{GitlabPlugin, GroupData, joblog, model, pages};

pub fn sample() -> GitlabPlugin {
    let mut p = GitlabPlugin::new();
    // Never the environment this runs in.
    p.env = Some(vec![]);
    p.now = 1_791_271_000;
    p.me = "me".into();
    p.cfg.base = "acme".into();
    p.cfg.sources = vec![
        Source {
            group: "team-b".into(),
            code: "b".into(),
            color: "cyan".into(),
            hide: vec![],
        },
        Source {
            group: "team-a".into(),
            code: "a".into(),
            color: "green".into(),
            hide: vec!["tools/renovate".into()],
        },
    ];
    let a = "acme/team-a";
    let mut review = mr(263, "alice", &format!("{a}/api"));
    review.title = "ABC-12: Add a stats endpoint for the dashboard".into();
    let mut ready = mr(187, "me", &format!("{a}/web"));
    ready.approved = true;
    ready.approved_by = vec!["bob".into()];
    ready.approvals_left = 0;
    ready.merge_status = "MERGEABLE".into();
    review.labels = vec!["backend".into()];
    let bot = mr(198, "group_1_bot_abc", &format!("{a}/web"));
    let hidden = mr(5, "me", &format!("{a}/tools/renovate"));
    // Merged yesterday: shown only by a view that asks for merged ones.
    let mut merged = mr(150, "bob", &format!("{a}/api"));
    merged.author.name = "Bob Example".into();
    merged.title = "ABC-9: Cache the dashboard".into();
    merged.state = model::MrState::Merged;
    merged.ended_at = "2026-10-05T16:00:00Z".into();
    merged.updated_at = "2026-10-05T16:00:00Z".into();
    merged.labels = vec!["backend".into(), "performance".into()];
    let projects = ["api", "web", "tools/renovate", "docs"]
        .iter()
        .map(|p| model::Project {
            name: p.rsplit('/').next().unwrap().into(),
            full_path: format!("{a}/{p}"),
            default_branch: "main".into(),
            main_pipeline: Some(if *p == "web" {
                model::Pipeline::Failed
            } else {
                model::Pipeline::Passed
            }),
            main_pipeline_url: (*p == "web")
                .then(|| "https://gitlab.com/acme/team-a/web/-/pipelines/8001".into()),
            main_pipeline_id: (*p == "web").then_some(8001),
            main_pipeline_at: if *p == "web" { "2026-10-01T09:00:00Z".into() } else { String::new() },
            last_activity: "2026-10-01T09:00:00Z".into(),
            main_commit: (*p == "api").then(|| model::MainCommit {
                sha: "a22afd6f1c0de5b7e9c4a3d2f1b0a9c8d7e6f5a4".into(),
                short_id: "a22afd6f".into(),
                author: "Alice Example".into(),
                at: "2026-10-06T05:00:00Z".into(),
                title: "Merge branch 'ABC-9-cache' into 'main'".into(),
            }),
        })
        .collect();
    p.data.insert(
        a.into(),
        GroupData {
            mrs: vec![bot, ready, review, hidden, merged],
            projects,
            ended_since: Some("2026-09-22T00:00:00Z".into()),
            ..Default::default()
        },
    );
    let b = "acme/team-b";
    let mut draft = mr(342, "me", &format!("{b}/worker"));
    draft.draft = true;
    // As GitLab says it: the draft first.
    draft.merge_status = "DRAFT_STATUS".into();
    // Its pipeline is red: two jobs failed, one allowed to.
    draft.pipeline = Some(model::Pipeline::Failed);
    draft.pipeline_url = Some(format!("https://gitlab.com/{b}/worker/-/pipelines/9001"));
    {
        use joblog::fixtures::{failed_test, job, timed_out};
        let worker = format!("{b}/worker");
        let test = job(5101, &worker, "unit-test", "test", "script_failure", 9001);
        let mut lint = job(5102, &worker, "lint", "test", "script_failure", 9001);
        lint.allow_failure = true;
        let web = job(
            5001,
            "acme/team-a/web",
            "unit-test",
            "test",
            "job_execution_timeout",
            8001,
        );
        for (project, pipeline, jobs) in [
            (&*worker, 9001, vec![lint, test]),
            ("acme/team-a/web", 8001, vec![web]),
        ] {
            let key = crate::why::key(project, pipeline);
            for j in &jobs {
                p.jobs.insert(crate::log_ownership::JobKey::new(&j.project, j.id), j.clone());
            }
            p.failed.insert(
                key,
                pages::Fetched {
                    value: Some(jobs),
                    at: p.now,
                    ..Default::default()
                },
            );
        }
        for (project, id, part) in [(&*worker, 5101, failed_test()), ("acme/team-a/web", 5001, timed_out())] {
            p.logs.insert(
                crate::log_ownership::JobKey::new(project, id),
                pages::Fetched {
                    value: Some(joblog::JobLog::new(part)),
                    at: p.now,
                    ..Default::default()
                },
            );
        }
    }
    p.data.insert(
        b.into(),
        GroupData {
            mrs: vec![draft],
            ..Default::default()
        },
    );
    // The jobs above were made from the list as it is.
    crate::why::made_from_list(&mut p, &format!("{b}/worker"), 9001);
    crate::why::made_from_list(&mut p, "acme/team-a/web", 8001);
    p
}

/// The sample with !263's page data and its files' diffs already held, as
/// after they arrived (`diff_editor_views` draws the Changes tab from it).
pub fn sample_reviewed(side_by_side: bool) -> GitlabPlugin {
    let mut p = sample();
    p.cfg.side_by_side = side_by_side;
    let mr = p
        .data
        .values()
        .flat_map(|d| d.mrs.iter())
        .find(|m| m.iid == 263)
        .expect("the sample's !263")
        .clone();
    let key = pages::mr_key("acme/team-a/api", 263);
    let detail = model::fixtures::detail();
    let head = detail.refs.as_ref().expect("the fixture's refs").head.clone();
    p.mr_pages.insert(key.clone(), pages::Fetched { value: Some((mr, detail)), at: p.now, ..Default::default() });
    p.diffs.insert(
        key,
        pages::changes::Diffs {
            head,
            files: model::fixtures::diffs().into_iter().map(|f| (f.new_path.clone(), f)).collect(),
            asked: [1].into(),
            ended: Some(1),
            ..Default::default()
        },
    );
    p
}
