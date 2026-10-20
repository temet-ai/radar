//! Recorded completions only: refresh tasks are created but never polled.

use super::*;
use crate::client::{FailedJobsEvidence, FailedJobsRead};
use crate::joblog::JobLog;
use crate::log_ownership::{self, JobKey, LogRead, PipelineRead};
use crate::model::TracePart;
use crate::why::DiagnosisRead;
use radar_core::input::TextInput;
use radar_core::ui::log::Row;

const JOB: u64 = 6001;
const OLD: &str = "section_start:1:build\rBuild\nneedle old\nold detail\n\
section_end:2:build\r\n\
section_start:3:test\rTest\nneedle second\nold tail\n\
section_end:4:test\r\n";

fn part(text: &str) -> TracePart {
    TracePart {
        text: text.into(),
        start: 0,
        total: text.len() as u64,
    }
}

fn direct_request(p: &mut GitlabPlugin) -> LogRead {
    drop(why::open_log(p, WORKER, JOB, true));
    LogRead {
        key: JobKey::new(WORKER, JOB),
        owner: p.log_reads[&JobKey::new(WORKER, JOB)].direct.unwrap(),
        earlier: None,
        action: log_ownership::action(&p.cfg.url, &JobKey::new(WORKER, JOB)),
    }
}

fn deliver(p: &mut GitlabPlugin, bundled: bool, result: Result<TracePart, String>) {
    let message = if bundled {
        drop(why::diagnose(p, WORKER, 9001, true, true));
        let request = PipelineRead {
            project: WORKER.into(),
            pipeline: 9001,
            owner: p.failed_reads[&why::key(WORKER, 9001)],
            activity: p.next_log_activity,
        };
        GlMsg::Failed {
            request,
            action: crate::failure_history::action("https://gitlab.com", WORKER, 9001),
            read: DiagnosisRead {
                result: Ok(FailedJobsRead {
                    jobs: vec![],
                    evidence: FailedJobsEvidence::default(),
                }),
                logs: Some(vec![(JOB, result)]),
            },
        }
    } else {
        GlMsg::Log {
            request: direct_request(p),
            info: None,
            result,
        }
    };
    p.on_msg(Box::new(message));
}

fn page() -> GitlabPlugin {
    let mut p = sample();
    p.pages.clear();
    p.pages.push(Page::new(Kind::Job {
        project: WORKER.into(),
        id: JOB,
    }));
    deliver(&mut p, false, Ok(part(OLD)));
    p
}

fn text(p: &GitlabPlugin) -> &radar_core::ui::log::LogText {
    &p.logs[&JobKey::new(WORKER, JOB)]
        .value
        .as_ref()
        .unwrap()
        .text
}

fn fold(p: &mut GitlabPlugin) {
    p.pages.last().unwrap().log.goto(text(p), 2);
    press(p, Key::Char(' '));
    assert!(text(p).sections.len() > 1);
    assert!(
        !p.pages
            .last()
            .unwrap()
            .log
            .rows(text(p))
            .contains(&(Row::Line(2), 1)),
        "space folds the second section and leaves the cursor on its title"
    );
}

fn refresh(p: &mut GitlabPlugin) {
    let effects = press(p, Key::Char('r'));
    assert!(matches!(&effects[..], [Effect::SpawnAction { action, .. }]
        if action.kind.as_str() == "gitlab.job.log"));
    assert!(p.logs[&JobKey::new(WORKER, JOB)].loading);
    // Dropped without polling: no authentication, network or remote write.
}

#[test]
fn folded_title_refresh_to_plain_text_renders_and_navigates() {
    let mut p = page();
    fold(&mut p);
    refresh(&mut p);
    deliver(&mut p, false, Ok(part("ordinary output\nlast line\n")));

    // This real page render panics without replacement reconciliation:
    // the former Title(1) cannot index a log with no sections.
    let rendered = draw(&p, 80, 24);
    assert!(rendered.contains("ordinary output"), "{rendered}");
    assert!(text(&p).sections.is_empty());
    press(&mut p, Key::Home);
    assert_eq!(p.pages[0].log.line(text(&p)), Some(0));
    press(&mut p, Key::Down);
    assert_eq!(p.pages[0].log.line(text(&p)), Some(1));
}

#[test]
fn replacement_rebuilds_search_matches_and_drops_old_folds() {
    let mut p = page();
    p.pages[0].log.search(text(&p), "NEEDLE");
    fold(&mut p);
    refresh(&mut p);
    deliver(
        &mut p,
        false,
        Ok(part(
            "section_start:1:build\rBuild\nnew output\nneedle moved\nneedle added\nneedle last\nsection_end:2:build\r\n",
        )),
    );
    let rendered = draw(&p, 160, 40);
    assert!(rendered.contains("log · /NEEDLE · 3 of 3"), "{rendered}");
    assert_eq!(p.pages[0].log.query(), "NEEDLE");
    assert_eq!(p.pages[0].log.found(text(&p)), (3, 3));
    assert!(p.pages[0].log.rows(text(&p)).contains(&(Row::Line(1), 1)));
    press(&mut p, Key::Char('n'));
    assert_eq!(p.pages[0].log.line(text(&p)), Some(1));
    press(&mut p, Key::Char('N'));
    assert_eq!(p.pages[0].log.line(text(&p)), Some(3));

    deliver(&mut p, false, Ok(part("nothing matching\n")));
    assert_eq!(p.pages[0].log.query(), "NEEDLE");
    assert_eq!(p.pages[0].log.found(text(&p)), (0, 0));
    assert!(draw(&p, 80, 24).contains("log · /NEEDLE · no match"));
    press(&mut p, Key::Char('n'));
    assert_eq!(p.pages[0].log.line(text(&p)), Some(0));
}

#[test]
fn direct_and_bundled_replacements_reconcile_hidden_duplicate_job_pages_only() {
    for bundled in [false, true] {
        let mut p = page();
        p.pages[0].log.search(text(&p), "needle");
        fold(&mut p);
        p.pages.push(Page::new(Kind::Job {
            project: WORKER.into(),
            id: JOB,
        }));
        p.pages[1].log.search(text(&p), "old");
        fold(&mut p);
        p.pages.push(Page::new(Kind::Job {
            project: WORKER.into(),
            id: JOB + 1,
        }));
        let other = JobLog::new(part(OLD));
        p.pages[2].log.search(&other.text, "second");
        p.pages[2].log.fold(&other.text);
        p.logs
            .entry(JobKey::new(WORKER, JOB + 1))
            .or_default()
            .value = Some(other);
        let untouched = format!("{:?}", p.pages[2].log);
        deliver(&mut p, bundled, Ok(part("needle changed\nnew tail\n")));

        assert_eq!(format!("{:?}", p.pages[2].log), untouched);
        assert_eq!(p.pages[0].log.query(), "needle");
        assert_eq!(p.pages[0].log.found(text(&p)), (1, 1));
        assert_eq!(p.pages[1].log.query(), "old");
        assert_eq!(p.pages[1].log.found(text(&p)), (0, 0));
        // Reveal each hidden page without opening a new one or resetting state.
        p.pages.pop();
        assert!(draw(&p, 80, 24).contains("no match"));
        press(&mut p, Key::Home);
        p.pages.pop();
        assert!(draw(&p, 80, 24).contains("1 of 1"));
        press(&mut p, Key::Down);
    }
}

#[test]
fn equal_parsed_text_and_failed_reads_preserve_all_derived_state() {
    for bundled in [false, true] {
        let mut p = page();
        let original = OLD.replace("old detail", "error: old detail");
        deliver(&mut p, false, Ok(part(&original)));
        p.pages[0].log.search(text(&p), "needle");
        fold(&mut p);
        draw(&p, 80, 8);
        let state = format!("{:?}", p.pages[0].log);
        // Transport and byte metadata differ, but the parsed coordinate space
        // is identical. An existing first error must not steal the cursor.
        let same = TracePart {
            text: format!("discarded partial line\n{original}"),
            start: 100,
            total: 2000,
        };
        assert_eq!(&JobLog::new(same.clone()).text, text(&p));
        let base = p.log_reads[&JobKey::new(WORKER, JOB)].base;
        p.now += 10;
        deliver(&mut p, bundled, Ok(same));
        assert!(
            p.log_reads[&JobKey::new(WORKER, JOB)].base > base,
            "accepted equal parsed text still installs a new base"
        );
        assert_eq!(format!("{:?}", p.pages[0].log), state);
        assert_eq!(
            p.logs[&JobKey::new(WORKER, JOB)]
                .value
                .as_ref()
                .unwrap()
                .start,
            100
        );
        assert_eq!(p.logs[&JobKey::new(WORKER, JOB)].at, p.now);
        let held = p.logs[&JobKey::new(WORKER, JOB)].value.clone();
        let at = p.logs[&JobKey::new(WORKER, JOB)].at;
        let base = p.log_reads[&JobKey::new(WORKER, JOB)].base;
        p.now += 10;
        deliver(&mut p, bundled, Err("recorded read failure".into()));
        assert_eq!(format!("{:?}", p.pages[0].log), state);
        assert_eq!(p.logs[&JobKey::new(WORKER, JOB)].value, held);
        assert_eq!(p.logs[&JobKey::new(WORKER, JOB)].at, at);
        assert_eq!(p.log_reads[&JobKey::new(WORKER, JOB)].base, base);
        assert!(!p.logs[&JobKey::new(WORKER, JOB)].loading);
        assert_eq!(
            p.logs[&JobKey::new(WORKER, JOB)].error.as_deref(),
            Some("recorded read failure")
        );
        assert!(draw(&p, 80, 24).contains("recorded read failure"));
    }
}

#[test]
fn changed_direct_text_keeps_first_error_policy_but_bundles_do_not_add_it() {
    for bundled in [false, true] {
        let mut p = page();
        p.pages[0].log.search(text(&p), "needle");
        fold(&mut p);
        deliver(
            &mut p,
            bundled,
            Ok(part(
                "ordinary output\nerror: changed failure\nneedle new\n",
            )),
        );
        assert_eq!(p.pages[0].log.query(), "needle");
        assert_eq!(p.pages[0].log.found(text(&p)).1, 1);
        assert_eq!(
            p.pages[0].log.line(text(&p)),
            Some(if bundled { 2 } else { 1 })
        );
    }
}

#[test]
fn replacement_does_not_recreate_the_active_search_editor() {
    for bundled in [false, true] {
        let mut p = page();
        p.pages[0].log.search(text(&p), "needle");
        fold(&mut p);
        press(&mut p, Key::Char('/'));
        press(&mut p, Key::Char('!'));
        let mut expected = TextInput::new("needle");
        expected.key(Chord::new(Key::Char('!')));
        let left = Chord::new(Key::Left);
        p.log_search.as_mut().unwrap().key(left);
        expected.key(left);
        deliver(&mut p, bundled, Ok(part("needle replacement\n")));
        assert!(p.capturing());
        assert_eq!(p.log_search.as_ref().unwrap().value(), expected.value());
        assert_eq!(p.log_search.as_ref().unwrap().cursor(), expected.cursor());
        assert_eq!(p.pages[0].log.query(), "needle");
        let undo = Chord::ctrl('z');
        p.log_search.as_mut().unwrap().key(undo);
        expected.key(undo);
        assert_eq!(p.log_search.as_ref().unwrap().value(), expected.value());
        assert_eq!(p.log_search.as_ref().unwrap().cursor(), expected.cursor());
        assert_eq!(
            expected.value(),
            "needle",
            "the edit really had undo history"
        );
    }
}

#[test]
fn earlier_prepend_keeps_cursor_folds_and_researches_the_query() {
    let mut p = page();
    deliver(
        &mut p,
        false,
        Ok(TracePart {
            text: format!("discarded\n{OLD}"),
            start: 100,
            total: 1000,
        }),
    );
    p.pages[0].log.search(text(&p), "needle");
    p.pages[0].log.goto(text(&p), 0);
    p.pages[0].log.fold(text(&p));
    p.pages[0].log.goto(text(&p), 2);
    let before = p.pages[0].log.line(text(&p)).unwrap();
    let fx = why::earlier(&mut p, WORKER, JOB);
    let [Effect::SpawnAction { action, .. }] = &fx[..] else { panic!("named earlier read") };
    let action = action.clone();
    drop(fx); // Never poll a production credential/transport future.
    let earlier = p.log_reads[&JobKey::new(WORKER, JOB)].earlier.unwrap();
    let request = LogRead {
        key: JobKey::new(WORKER, JOB),
        owner: earlier.owner,
        earlier: Some(earlier),
        action,
    };
    let result = crate::client::recorded::block(
        crate::client::Client::over(crate::client::recorded::Replay::raw(vec![crate::client::Resp {
            status: 206,
            body: b"needle before\njoined ".to_vec(),
            content_range: Some("bytes 0-20/1000".into()),
            ..Default::default()
        }])).trace_observed(WORKER, JOB, Some(earlier.before)),
    ).map_err(|e| format!("{e:#}"));
    let fx = p.on_msg(Box::new(GlMsg::EarlierLog { request, result }));
    assert!(matches!(fx.first(), Some(Effect::Report(radar_core::journal::Outcome::Succeeded))));
    assert!(!log_ownership::earlier_loading(
        &p,
        &JobKey::new(WORKER, JOB)
    ));
    assert_eq!(p.pages[0].log.line(text(&p)), Some(before + 2));
    assert_eq!(p.pages[0].log.query(), "needle");
    assert_eq!(p.pages[0].log.found(text(&p)), (3, 3));
    let rows = p.pages[0].log.rows(text(&p));
    assert!(rows.contains(&(Row::Title(0), 0)));
    assert!(
        !rows.contains(&(Row::Line(2), 1)),
        "the first section stays folded"
    );
    assert!(
        rows.contains(&(Row::Line(4), 1)),
        "the cursor's section stays open"
    );
}

#[test]
fn empty_replacement_clears_old_coordinates_and_keeps_the_query() {
    let mut p = page();
    p.pages[0].log.search(text(&p), "needle");
    fold(&mut p);
    deliver(&mut p, false, Ok(part("")));
    assert_eq!(p.pages[0].log.query(), "needle");
    assert_eq!(p.pages[0].log.found(text(&p)), (0, 0));
    assert_eq!(p.pages[0].log.line(text(&p)), None);
    assert!(draw(&p, 80, 24).contains("This job wrote nothing to its log."));
    press(&mut p, Key::Down);
    deliver(&mut p, false, Ok(part("needle returned\n")));
    assert_eq!(p.pages[0].log.found(text(&p)), (1, 1));
    assert!(draw(&p, 80, 24).contains("needle returned"));
}

#[test]
fn same_numeric_job_in_another_project_keeps_its_log_coordinates_metadata_and_links() {
    let mut p = page();
    let other = "acme/other/repo";
    let other_key = JobKey::new(other, JOB);
    let other_log = JobLog::new(part("other needle\nerror: other failure\n"));
    p.logs.entry(other_key.clone()).or_default().value = Some(other_log);
    let info = crate::joblog::fixtures::job(
        JOB,
        other,
        "other job",
        "other stage",
        "script_failure",
        4321,
    );
    p.jobs.insert(other_key.clone(), info.clone());
    p.pages.push(Page::new(Kind::Job {
        project: other.into(),
        id: JOB,
    }));
    p.pages[1]
        .log
        .search(&p.logs[&other_key].value.as_ref().unwrap().text, "other");
    draw(&p, 80, 24);
    let unchanged = format!("{:?}", p.pages[1]);
    deliver(&mut p, false, Ok(part("needle new\n")));
    assert_eq!(format!("{:?}", p.pages[1]), unchanged);
    assert_eq!(p.jobs[&other_key], info);
    assert!(draw(&p, 80, 24).contains("other job"));
    assert_eq!(
        crate::join::job(&p, other, JOB).field("pipeline"),
        Some("4321")
    );
    assert_eq!(job::info(&p, WORKER, JOB), None);
    assert_eq!(
        link(&p, &p.pages[1], true).unwrap(),
        format!("https://gitlab.com/{other}/-/pipelines/4321")
    );
    let copied = press(&mut p, Key::Char('c'));
    assert!(copied.iter().any(|fx| matches!(fx, Effect::Copy(text) if text.contains("other failure") && !text.contains("needle new"))));
    assert_eq!(
        why::excerpt_text(&p, WORKER, JOB).as_deref(),
        Some("needle new")
    );
}

#[test]
fn connection_change_retires_job_remote_buffers_but_keeps_query_editor_and_navigation() {
    let mut p = page();
    p.pages[0].log.search(text(&p), "needle");
    fold(&mut p);
    draw(&p, 80, 24);
    let key = JobKey::new(WORKER, JOB);
    p.jobs.insert(
        key,
        crate::joblog::fixtures::job(JOB, WORKER, "old remote name", "test", "", 345),
    );
    p.runs.insert(WORKER.into(), Default::default());
    p.recovery.test_keep("authored kept text");
    let snapshot = p.snapshot();
    let kept = format!("{:?}", p.recovery);
    let page = &mut p.pages[0];
    page.extra.get_mut().push(("old extra".into(), vec![]));
    page.links
        .get_mut()
        .push(radar_core::Subject::link("https://old.example/link"));
    page.under.get_mut().push((
        "local",
        radar_core::Subject::new("job", "old", "old"),
        "old-action".into(),
    ));
    page.tab = 1;
    page.row.set(8);
    p.log_search = Some(TextInput::new("unfinished query"));
    p.log_search
        .as_mut()
        .unwrap()
        .key(Chord::new(Key::Left));
    let editor = (
        p.log_search.as_ref().unwrap().value(),
        p.log_search.as_ref().unwrap().cursor(),
    );
    let sequence = (p.next_page_read, p.next_log_activity);
    p.replace_connection("https://new.example".into(), "env:OTHER".into());
    assert!(
        p.failed.is_empty()
            && p.failed_reads.is_empty()
            && p.jobs.is_empty()
            && p.logs.is_empty()
            && p.log_reads.is_empty()
    );
    assert_eq!((p.next_page_read, p.next_log_activity), sequence);
    assert_eq!(p.snapshot(), snapshot);
    assert!(p.runs.is_empty() && p.runs_scopes.is_empty());
    assert_eq!(format!("{:?}", p.recovery), kept);
    assert_eq!(
        (
            p.log_search.as_ref().unwrap().value(),
            p.log_search.as_ref().unwrap().cursor()
        ),
        editor
    );
    let page = &p.pages[0];
    assert_eq!(page.log.query(), "needle");
    assert_eq!(page.log.found(&Default::default()), (0, 0));
    assert!(!page.log_painted.get());
    assert!(
        page.extra.borrow().is_empty()
            && page.links.borrow().is_empty()
            && page.under.borrow().is_empty()
    );
    assert_eq!((page.tab, page.row.get()), (0, 0));
    assert_eq!(job::name(&p, WORKER, JOB), format!("job #{JOB}"));
    assert!(link(&p, page, true).is_none());
    assert_eq!(
        link(&p, page, false).unwrap(),
        format!("https://new.example/{WORKER}/-/jobs/{JOB}")
    );
    assert!(matches!(click(&mut p, "under:0"), Outcome::Handled(fx) if fx.is_empty()));
    p.log_search = None;
    deliver(&mut p, false, Ok(part("first\nneedle current\nlast\n")));
    let before = p.pages[0].log.line(text(&p));
    assert_eq!(before, Some(1));
    click(&mut p, "log:0");
    assert_eq!(
        p.pages[0].log.line(text(&p)),
        before,
        "old coordinates remain inert until render"
    );
    draw(&p, 80, 24);
    click(&mut p, "log:0");
    assert_eq!(p.pages[0].log.line(text(&p)), Some(0));
    press(&mut p, Key::Esc);
    assert_eq!(p.pages[0].log.query(), "");
    press(&mut p, Key::Esc);
    assert!(p.pages.is_empty());
}

#[test]
fn changed_tail_and_earlier_prepend_invalidate_hits_until_their_render() {
    let mut p = page();
    draw(&p, 80, 24);
    assert!(p.pages[0].log_painted.get());
    deliver(&mut p, false, Ok(part("first\nsecond\nthird\n")));
    assert!(!p.pages[0].log_painted.get());
    let at = p.pages[0].log.line(text(&p));
    click(&mut p, "log:0");
    assert_eq!(p.pages[0].log.line(text(&p)), at);
    draw(&p, 80, 24);
    click(&mut p, "log:0");
    assert_eq!(p.pages[0].log.line(text(&p)), Some(0));
    deliver(
        &mut p,
        false,
        Ok(TracePart {
            text: "discarded\nfirst\nsecond\n".into(),
            start: 100,
            total: 1000,
        }),
    );
    draw(&p, 80, 24);
    let request = log_ownership::earlier(&mut p, JobKey::new(WORKER, JOB)).unwrap();
    why::on_log(&mut p, request, None, Ok(part("earlier\n")));
    assert!(!p.pages[0].log_painted.get());
}

#[test]
fn source_job_seed_is_insert_only_and_only_real_insertions_advance_activity() {
    let mut p = mr_page();
    let project = "acme/team-a/api";
    let key = JobKey::new(project, 1002);
    let kind = Kind::Job {
        project: project.into(),
        id: 1002,
    };
    let before = p.next_log_activity;
    seed_job(&mut p, &kind);
    assert_eq!(p.jobs[&key].name, "unit-test");
    assert!(p.next_log_activity > before);
    p.jobs.get_mut(&key).unwrap().name = "current remote name".into();
    let activity = p.next_log_activity;
    seed_job(&mut p, &kind);
    assert_eq!(p.jobs[&key].name, "current remote name");
    assert_eq!(p.next_log_activity, activity);
}

#[test]
fn hidden_requests_remain_valid_after_navigation_and_the_mr_pipeline_changes() {
    let mut p = page();
    let direct = direct_request(&mut p);
    let earlier_key = JobKey::new(WORKER, JOB + 2);
    p.logs.entry(earlier_key.clone()).or_default().value = Some(JobLog::new(TracePart {
        text: "fragment\nheld earlier base\n".into(),
        start: 100,
        total: 1000,
    }));
    let earlier = log_ownership::earlier(&mut p, earlier_key.clone()).unwrap();
    drop(why::diagnose(&mut p, WORKER, 9001, true, true));
    let diagnosis = PipelineRead {
        project: WORKER.into(),
        pipeline: 9001,
        owner: p.failed_reads[&why::key(WORKER, 9001)],
        activity: p.next_log_activity,
    };
    for data in p.data.values_mut() {
        for mr in &mut data.mrs {
            mr.pipeline_url = Some(format!(
                "https://gitlab.com/{}/-/pipelines/9999",
                mr.project_path
            ));
        }
    }
    p.pages.clear();
    p.pages.push(Page::new(Kind::Repo("different/repo".into())));
    assert!(why::on_log(&mut p, direct, None, Ok(part("hidden direct"))).is_empty());
    assert!(why::on_log(&mut p, earlier, None, Ok(part("hidden prefix"))).is_empty());
    let effects = p.on_msg(Box::new(GlMsg::Failed {
        request: diagnosis,
        action: crate::failure_history::action(&p.cfg.url, WORKER, 9001),
        read: DiagnosisRead {
            result: Ok(FailedJobsRead {
                jobs: vec![crate::joblog::fixtures::job(
                    JOB + 1,
                    WORKER,
                    "hidden job",
                    "test",
                    "",
                    9001,
                )],
                evidence: FailedJobsEvidence::default(),
            }),
            logs: Some(vec![(JOB + 1, Ok(part("hidden diagnosis")))]),
        },
    }));
    assert!(!format!("{effects:?}").contains("skipped"));
    assert_eq!(why::failed(&p, WORKER, 9001).unwrap()[0].name, "hidden job");
    assert!(
        p.logs[&earlier_key]
            .value
            .as_ref()
            .unwrap()
            .raw
            .starts_with("hidden prefix")
    );
    assert_eq!(
        p.logs[&JobKey::new(WORKER, JOB)]
            .value
            .as_ref()
            .unwrap()
            .raw,
        "hidden direct"
    );
    assert_eq!(p.pages.len(), 1);
    assert_eq!(p.pages[0].kind, Kind::Repo("different/repo".into()));
}
