//! The push is evidence of local navigation, independently of provider reads.
use super::*;
use crate::log_ownership::JobKey;
use crate::pages::{self, Fetched, Kind};
use crate::{GitlabPlugin, Plugin, SCREEN, demo, join, model, screen};
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionEvent, ActionId, Journal, Metadata, Query};
use radar_core::plugin::{Outcome as KeyOutcome, Pane};
use radar_core::view::{Chord, Key};
use radar_core::{Host, Showing, Subject};

const OPEN: &str = "gitlab.page.open";
const PROJECT: &str = "acme/team-a/api";
const WEB: &str = "acme/team-a/web";
const PRIVATE: &str = "PRIVATE_PAGE_OPEN_PAYLOAD_841";
const URL: &str = "https://PRIVATE_USER:PRIVATE_PASSWORD@git.example/install/?token=PRIVATE_TOKEN#PRIVATE_FRAGMENT";

fn mr() -> Kind {
    Kind::Mr {
        project: PROJECT.into(),
        iid: 263,
    }
}
fn job() -> Kind {
    Kind::Job {
        project: WEB.into(),
        id: 5001,
    }
}
fn sample() -> GitlabPlugin {
    let mut p = demo::sample();
    p.cfg.url = URL.into();
    let m = pages::find_mr(&p, PROJECT, 263).unwrap().clone();
    let mut detail = model::fixtures::detail();
    detail.description = PRIVATE.into();
    p.mr_pages.insert(
        pages::mr_key(PROJECT, 263),
        Fetched {
            value: Some((m, detail)),
            at: p.now,
            ..Default::default()
        },
    );
    crate::runs_ownership::begin(&mut p, PROJECT, "main");
    p.runs.insert(
        PROJECT.into(),
        Fetched {
            value: Some(vec![]),
            at: p.now,
            ..Default::default()
        },
    );
    p
}
fn open(effects: &[Effect]) -> &ActionDescriptor {
    let found: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Observe { action, outcome } if action.kind.as_str() == OPEN => {
                assert_eq!(*outcome, Outcome::Succeeded);
                Some(action)
            }
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "one successful open: {effects:?}");
    found[0]
}
fn no_open(effects: &[Effect]) {
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Observe { action, .. } if action.kind.as_str() == OPEN))
    );
}
fn scope(a: &ActionDescriptor, instance: &str, kind: &str, id: &str) {
    assert_eq!(a.kind.as_str(), OPEN);
    assert_eq!(a.origin, Origin::Plugin("gitlab".into()));
    assert_eq!(
        a.metadata,
        Metadata {
            mode: a.metadata.mode,
            ..Default::default()
        }
    );
    assert_eq!(a.subjects.len(), 1);
    let s = &a.subjects[0];
    assert_eq!(
        (&*s.key.provider, &*s.key.instance, &*s.key.kind, &*s.key.id),
        ("gitlab", instance, kind, id)
    );
    assert_eq!(s.role, SubjectRole::Primary);
    assert!(s.url.is_none());
}
fn omitted(a: &ActionDescriptor) {
    assert!(a.subjects.is_empty());
    assert_eq!(
        a.metadata,
        Metadata {
            redacted: true,
            omitted_subjects: 1,
            ..Default::default()
        }
    );
}
fn handled(outcome: KeyOutcome) -> Vec<Effect> {
    match outcome {
        KeyOutcome::Handled(fx) => fx,
        KeyOutcome::Ignored => vec![],
    }
}
fn key(p: &mut GitlabPlugin, pane: Pane, code: Key) -> Vec<Effect> {
    handled(p.key(SCREEN.id, pane, Chord::new(code)))
}
fn select(p: &mut GitlabPlugin, id: &str) {
    let target = screen::targets(p)
        .into_iter()
        .find(|t| t.path.last().is_some_and(|s| s == id))
        .expect("fixture target");
    p.tree.borrow_mut().select(target.path);
}
fn pending(effects: &[Effect]) -> Vec<&str> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SpawnAction { action, .. } => Some(action.kind.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn four_requested_kinds_keep_exact_scope_and_distinct_sanitized_installations() {
    let mut p = sample();
    for (kind, name, id) in [
        (Kind::Tenancy("team-a".into()), "source", "acme/team-a"),
        (Kind::Repo(PROJECT.into()), join::REPO, PROJECT),
        (mr(), join::MERGE_REQUEST, "acme/team-a/api!263"),
        (job(), join::JOB, "acme/team-a/web/5001"),
    ] {
        p.cfg.url = URL.into();
        let first = [opened(&p, &kind)];
        scope(open(&first), "https://git.example/install/", name, id);
        assert!(open(&first).metadata.mode.is_none());
        p.cfg.url = "http://other.example:8181/OtherInstall".into();
        let second = [opened(&p, &kind)];
        scope(
            open(&second),
            "http://other.example:8181/OtherInstall",
            name,
            id,
        );
        assert_ne!(open(&first).subjects, open(&second).subjects);
    }
    let kind = Kind::Repo("Acme/Team.Api".into());
    scope(
        open(&[opened(&p, &kind)]),
        "http://other.example:8181/OtherInstall",
        join::REPO,
        "Acme/Team.Api",
    );
}

#[test]
fn tenancy_uses_exact_configured_source_then_current_base_never_alias_or_cache() {
    let mut p = sample();
    p.cfg.sources[1].code = PRIVATE.into();
    let kind = Kind::Tenancy("team-a".into());
    p.cfg.base = "other/base".into();
    scope(
        open(&[opened(&p, &kind)]),
        "https://git.example/install/",
        "source",
        "other/base/team-a",
    );
    p.cfg.base.clear();
    p.cfg.sources[1].group = PROJECT.into();
    scope(
        open(&[opened(&p, &Kind::Tenancy(PROJECT.into()))]),
        "https://git.example/install/",
        "source",
        PROJECT,
    );
    for missing in ["team-a", "acme/team-a", PRIVATE] {
        omitted(open(&[opened(&p, &Kind::Tenancy(missing.into()))]));
    }
    for group in ["bad group", "a//b", "a/../b", "a/./b", "a%2Fb"] {
        p.cfg.sources[1].group = group.into();
        omitted(open(&[opened(&p, &Kind::Tenancy(group.into()))]));
    }
    p.cfg.sources[1].group = "team-a".into();
    for base in ["a/..", "a/.", "bad base"] {
        p.cfg.base = base.into();
        omitted(open(&[opened(&p, &kind)]));
    }
}

#[test]
fn invalid_zero_oversized_and_composed_secret_identities_omit_only_new_scope() {
    let mut p = sample();
    for url in [
        "",
        "bad URL",
        "file:///private",
        "https://git.example/glpat-SECRET",
    ] {
        p.cfg.url = url.into();
        for kind in [
            Kind::Tenancy("team-a".into()),
            Kind::Repo(PROJECT.into()),
            mr(),
            job(),
        ] {
            omitted(open(&[opened(&p, &kind)]));
        }
    }
    p.cfg.url = URL.into();
    for path in [
        "",
        "a/../b",
        "a/./b",
        "a//b",
        "/a",
        "a/",
        "a%2Fb",
        "a?token=x",
        "a\\b",
        &"a".repeat(2049),
    ] {
        for kind in [
            Kind::Repo(path.into()),
            Kind::Mr {
                project: path.into(),
                iid: 1,
            },
            Kind::Job {
                project: path.into(),
                id: 1,
            },
        ] {
            omitted(open(&[opened(&p, &kind)]));
        }
    }
    for kind in [
        Kind::Mr {
            project: PROJECT.into(),
            iid: 0,
        },
        Kind::Job {
            project: PROJECT.into(),
            id: 0,
        },
        Kind::Mr {
            project: "a".repeat(2048),
            iid: 1,
        },
        Kind::Job {
            project: "a".repeat(2048),
            id: 1,
        },
    ] {
        omitted(open(&[opened(&p, &kind)]));
    }
    // Final composed IDs, rather than only project components, own the budget.
    for kind in [
        Kind::Repo("a".repeat(2048)),
        Kind::Mr {
            project: "a".repeat(2046),
            iid: 1,
        },
        Kind::Job {
            project: "a".repeat(2046),
            id: 1,
        },
    ] {
        let fx = [opened(&p, &kind)];
        assert_eq!(open(&fx).subjects[0].key.id.len(), 2048);
    }
    p.cfg.base.clear();
    p.cfg.sources[1].group = "a".repeat(2049);
    omitted(open(&[opened(&p, &Kind::Tenancy("a".repeat(2049)))]));
    p.cfg.base = "acme".into();
    p.cfg.url = format!("https://git.example/{}", "a".repeat(2048));
    omitted(open(&[opened(&p, &mr())]));
    for (url, kind, secret) in [
        (
            "https://git.example/open-secret-instance-841",
            mr(),
            "open-secret-instance-841",
        ),
        (
            URL,
            Kind::Mr {
                project: "acme/open-secret-mr-842".into(),
                iid: 842,
            },
            "acme/open-secret-mr-842!842",
        ),
        (
            URL,
            Kind::Job {
                project: "acme/open-secret-job-843".into(),
                id: 843,
            },
            "acme/open-secret-job-843/843",
        ),
        (
            URL,
            Kind::Repo("acme/open-secret-repo-844".into()),
            "open-secret-repo-844",
        ),
        (
            URL,
            Kind::Tenancy("open-secret-group-845".into()),
            "acme/open-secret-group-845",
        ),
    ] {
        p.cfg.url = url.into();
        p.cfg.sources[1].group = "open-secret-group-845".into();
        radar_core::log::secret(secret);
        let fx = [opened(&p, &kind)];
        omitted(open(&fx));
        assert!(!format!("{fx:?}").contains(secret));
    }
}

#[test]
fn poisoned_returned_mr_and_job_metadata_cannot_replace_requested_kind() {
    let mut p = sample();
    p.data.clear();
    let held = &mut p
        .mr_pages
        .get_mut(&pages::mr_key(PROJECT, 263))
        .unwrap()
        .value
        .as_mut()
        .unwrap()
        .0;
    held.project_path = PRIVATE.into();
    held.iid = 999;
    held.web_url = format!("https://private.example/{PRIVATE}");
    held.title = PRIVATE.into();
    let info = p.jobs.get_mut(&JobKey::new(WEB, 5001)).unwrap();
    info.project = PRIVATE.into();
    info.id = 999;
    info.name = PRIVATE.into();
    info.url = format!("https://private.example/{PRIVATE}");
    for (kind, name, id) in [
        (mr(), join::MERGE_REQUEST, "acme/team-a/api!263"),
        (job(), join::JOB, "acme/team-a/web/5001"),
    ] {
        let fx = pages::push(&mut p, kind);
        scope(open(&fx), "https://git.example/install/", name, id);
        assert!(pending(&fx).is_empty());
        assert!(!format!("{:?}", open(&fx)).contains(PRIVATE));
    }
}

#[test]
fn cached_repeated_push_preserves_stack_tab_log_position_cache_and_owners() {
    let mut p = sample();
    let before = p.snapshot();
    let held = format!(
        "{:?}{:?}{:?}{:?}{:?}",
        p.mr_pages, p.runs, p.logs, p.jobs, p.failed
    );
    let owners = format!("{:?}{:?}{:?}", p.mr_page_reads, p.log_reads, p.failed_reads);
    for kind in [
        Kind::Tenancy("team-a".into()),
        Kind::Repo(PROJECT.into()),
        mr(),
        job(),
        job(),
    ] {
        let depth = p.pages.len();
        let prior = format!("{:?}", p.pages);
        let fx = pages::push(&mut p, kind.clone());
        open(&fx);
        assert_eq!(fx.len(), 1);
        assert_eq!(p.pages.len(), depth + 1);
        assert_eq!(p.pages.last().unwrap().kind, kind);
        assert_eq!(p.pages.last().unwrap().tab, 0);
        assert_eq!(format!("{:?}", &p.pages[..depth]), prior);
        if let Kind::Job { project, id } = kind {
            let log = p.logs[&JobKey::new(&project, id)].value.as_ref().unwrap();
            let expected = radar_core::ui::log::LogState::new();
            if let Some(line) = log.excerpt.as_ref().and_then(|e| e.error) {
                expected.goto(&log.text, line);
            }
            assert_eq!(
                format!("{:?}", p.pages.last().unwrap().log),
                format!("{expected:?}")
            );
        }
    }
    assert_eq!(p.snapshot(), before);
    assert_eq!(
        format!(
            "{:?}{:?}{:?}{:?}{:?}",
            p.mr_pages, p.runs, p.logs, p.jobs, p.failed
        ),
        held
    );
    assert_eq!(
        format!("{:?}{:?}{:?}", p.mr_page_reads, p.log_reads, p.failed_reads),
        owners
    );
    let fx = pages::push(&mut p, Kind::Repo(WEB.into()));
    open(&fx);
    assert_eq!(
        p.pages.last().unwrap().tab,
        1,
        "red repo starts on Pipelines"
    );
    assert_eq!(pending(&fx), ["gitlab.page.runs"]);
}

#[test]
fn cold_busy_and_error_pushes_keep_existing_read_suppression_and_one_open_each() {
    for kind in [Kind::Repo(PROJECT.into()), mr(), job()] {
        let mut p = sample();
        p.runs.clear();
        p.mr_pages.clear();
        p.logs.clear();
        let fx = pages::push(&mut p, kind.clone());
        open(&fx);
        assert_eq!(pending(&fx).len(), 1);
        let owners = format!("{:?}{:?}", p.mr_page_reads, p.log_reads);
        let fx = pages::push(&mut p, kind.clone());
        open(&fx);
        assert!(pending(&fx).is_empty());
        assert_eq!(format!("{:?}{:?}", p.mr_page_reads, p.log_reads), owners);
        match &kind {
            Kind::Repo(path) => {
                let f = p.runs.get_mut(path).unwrap();
                f.loading = false;
                f.error = Some(PRIVATE.into());
            }
            Kind::Mr { project, iid } => {
                let f = p.mr_pages.get_mut(&pages::mr_key(project, *iid)).unwrap();
                f.loading = false;
                f.error = Some(PRIVATE.into());
            }
            Kind::Job { project, id } => {
                let f = p.logs.get_mut(&JobKey::new(project, *id)).unwrap();
                f.loading = false;
                f.error = Some(PRIVATE.into());
            }
            _ => unreachable!(),
        }
        let fx = pages::push(&mut p, kind.clone());
        open(&fx);
        assert_eq!(
            pending(&fx).len(),
            usize::from(!matches!(kind, Kind::Job { .. }))
        );
        assert_eq!(p.pages.len(), 3);
    }
}

#[test]
fn tree_and_preview_enter_capture_each_selected_kind_with_existing_focus_order() {
    for pane in [Pane::Middle, Pane::Detail] {
        for (id, expected) in [
            ("t:team-a", Kind::Tenancy("team-a".into())),
            ("r:acme/team-a/api", Kind::Repo(PROJECT.into())),
            ("m:acme/team-a/api!263", mr()),
        ] {
            let mut p = sample();
            select(&mut p, id);
            let selected = p.tree.borrow().selected().to_vec();
            let fx = key(&mut p, pane, Key::Enter);
            open(&fx);
            assert!(matches!(fx.first(), Some(Effect::Focus(Pane::Middle))));
            assert_eq!(p.pages.len(), 1);
            assert_eq!(p.pages[0].kind, expected);
            assert_eq!(p.tree.borrow().selected(), selected);
        }
    }
}

#[test]
fn nested_enters_capture_actual_repo_mr_and_failed_job_pushes() {
    let mut p = sample();
    pages::push(&mut p, Kind::Tenancy("team-a".into()));
    // Choose the api row through the page's unchanged repository ordering.
    for row in 0..4 {
        p.pages.last().unwrap().row.set(row);
        let fx = key(&mut p, Pane::Middle, Key::Enter);
        open(&fx);
        let target = p.pages.last().unwrap().kind.clone();
        if target == Kind::Repo(PROJECT.into()) {
            break;
        }
        no_open(&key(&mut p, Pane::Middle, Key::Esc));
    }
    assert_eq!(p.pages.last().unwrap().kind, Kind::Repo(PROJECT.into()));
    let fx = key(&mut p, Pane::Middle, Key::Enter);
    scope(
        open(&fx),
        "https://git.example/install/",
        join::MERGE_REQUEST,
        "acme/team-a/api!263",
    );
    assert_eq!(p.pages.len(), 3);
    pages::push(&mut p, Kind::Repo(WEB.into()));
    let fx = key(&mut p, Pane::Middle, Key::Enter);
    scope(
        open(&fx),
        "https://git.example/install/",
        join::JOB,
        "acme/team-a/web/5001",
    );
    assert_eq!(p.pages.last().unwrap().kind, job());
    pages::push(
        &mut p,
        Kind::Mr {
            project: "acme/team-b/worker".into(),
            iid: 342,
        },
    );
    no_open(&p.show_tab(SCREEN.id, 3));
    let fx = key(&mut p, Pane::Middle, Key::Enter);
    open(&fx);
    assert!(matches!(p.pages.last().unwrap().kind, Kind::Job { .. }));
}

#[test]
fn show_rejects_contradictions_and_records_consistent_current_context_navigation() {
    let mut p = sample();
    for subject in [
        Subject::new(join::REPO, "disagrees", PRIVATE).with("project", PROJECT),
        Subject::new(join::MERGE_REQUEST, "other/project!999", PRIVATE)
            .with("project", PROJECT)
            .with("iid", "263"),
        Subject::new(join::JOB, "other/project#999", PRIVATE)
            .with("project", WEB)
            .with("id", "5001"),
        Subject::new(join::MERGE_REQUEST, "invalid", PRIVATE)
            .with("project", "malformed project")
            .with("iid", "0"),
    ] {
        assert!(matches!(p.show(&subject), Showing::Cannot));
        assert!(p.pages.is_empty());
    }
    for (subject, expected) in [
        (
            Subject::new(join::REPO, PROJECT, PRIVATE).with("project", PROJECT),
            Kind::Repo(PROJECT.into()),
        ),
        (
            Subject::new(join::MERGE_REQUEST, format!("{PROJECT}!263"), PRIVATE)
                .with("project", PROJECT)
                .with("iid", "263"),
            mr(),
        ),
        (
            Subject::new(join::JOB, format!("{WEB}#5001"), PRIVATE)
                .with("project", WEB)
                .with("id", "5001"),
            job(),
        ),
        (
            Subject::new(join::MERGE_REQUEST, format!("{PROJECT}!0"), PRIVATE)
                .with("project", PROJECT)
                .with("iid", "0"),
            Kind::Mr {
                project: PROJECT.into(),
                iid: 0,
            },
        ),
    ] {
        let Showing::Page { effects, .. } = p.show(&subject) else {
            panic!("consistent current-context fields")
        };
        let a = open(&effects);
        assert_eq!(p.pages.last().unwrap().kind, expected);
        if matches!(expected, Kind::Mr { iid: 0, .. }) {
            omitted(a);
        }
        assert!(matches!(effects[0], Effect::Focus(Pane::Middle)));
    }
    let depth = p.pages.len();
    for subject in [
        Subject::new(join::MERGE_REQUEST, "x", PRIVATE),
        Subject::new(join::JOB, "x", PRIVATE),
        Subject::new("other", "x", PRIVATE),
        Subject::new(join::PIPELINE, "x", PRIVATE),
    ] {
        assert!(matches!(p.show(&subject), Showing::Cannot));
    }
    let Showing::Done(effects) = p.show(&pipeline_subject()) else {
        panic!("external pipeline")
    };
    no_open(&effects);
    assert_eq!(p.pages.len(), depth);
}

#[test]
fn selection_folds_filters_goto_tabs_scroll_refresh_callbacks_back_and_external_are_not_opens() {
    let mut p = sample();
    select(&mut p, "m:acme/team-a/api!263");
    for code in [
        Key::Down,
        Key::Char(' '),
        Key::Char('/'),
        Key::Char('a'),
        Key::Enter,
        Key::Esc,
        Key::Char(':'),
        Key::Char('a'),
        Key::Enter,
        Key::Char('o'),
        Key::Char('p'),
    ] {
        no_open(&key(&mut p, Pane::Middle, code));
    }
    for id in ["tree", "view:0"] {
        no_open(&handled(p.click(SCREEN.id, id)));
    }
    pages::push(&mut p, mr());
    for code in [
        Key::Down,
        Key::PageDown,
        Key::Char(']'),
        Key::Char('['),
        Key::Char('o'),
        Key::Char('p'),
        Key::Char('r'),
    ] {
        no_open(&key(&mut p, Pane::Middle, code));
    }
    for id in ["tab:1", "row:0", "file:0", "thread:0"] {
        no_open(&handled(p.click(SCREEN.id, id)));
    }
    let now = p.now;
    no_open(&p.tick(now));
    no_open(&pages::on_mr_page(
        &mut p,
        pages::mr_key(PROJECT, 263),
        Err(PRIVATE.into()),
    ));
    no_open(&key(&mut p, Pane::Middle, Key::Esc));
    pages::push(&mut p, job());
    for code in [
        Key::Char('/'),
        Key::Char('x'),
        Key::Enter,
        Key::Char('n'),
        Key::Char(' '),
        Key::Down,
        Key::Esc,
    ] {
        no_open(&key(&mut p, Pane::Middle, code));
    }
}

fn pipeline_subject() -> Subject {
    let mut s = Subject::new(join::PIPELINE, "acme/team-a/api#901", PRIVATE);
    s.url = Some("https://git.example/acme/team-a/api/-/pipelines/901".into());
    s
}

#[test]
fn within_page_thread_diff_and_pipeline_link_enters_and_write_callbacks_do_not_push() {
    let mut p = crate::tests::sample_reviewed();
    pages::push(&mut p, mr());
    p.pages.last_mut().unwrap().tab = 2;
    p.pages.last().unwrap().row.set(1);
    p.pages.last().unwrap().detail_painted.set(true);
    no_open(&key(&mut p, Pane::Middle, Key::Enter));
    assert_eq!(p.pages.len(), 1);
    assert_eq!(
        p.pages.last().unwrap().tab,
        1,
        "thread navigates within the MR"
    );
    for code in [Key::Enter, Key::Down, Key::PageDown] {
        no_open(&key(&mut p, Pane::Middle, code));
        assert_eq!(p.pages.len(), 1);
    }
    let connection = p.connection_stamp();
    no_open(&pages::act::on_msg(
        &mut p,
        Box::new(crate::mr_actions::ActMsg {
            connection,
            key: pages::mr_key(PROJECT, 263),
            what: "rebase",
            result: Ok(PRIVATE.into()),
            outcome: Outcome::Succeeded,
        }),
    ));
    assert_eq!(p.pages.len(), 1);
    pages::push(&mut p, Kind::Repo(PROJECT.into()));
    p.runs.get_mut(PROJECT).unwrap().value = Some(vec![model::PipelineRun {
        iid: "901".into(),
        status: Some(model::Pipeline::Passed),
        created_at: String::new(),
        duration_secs: None,
        url: Some("https://git.example/pipeline/901".into()),
        source: PRIVATE.into(),
        user: PRIVATE.into(),
        commit_sha: PRIVATE.into(),
        commit_title: PRIVATE.into(),
    }]);
    no_open(&p.show_tab(SCREEN.id, 1));
    let activate = radar_core::view::Intent::Activate { target: radar_core::view::Target::new("row:0") };
    let fx = handled(Plugin::intent(&mut p, SCREEN.id, Pane::Middle, &activate));
    no_open(&fx);
    assert!(matches!(&fx[..], [Effect::Open(url)] if url == "https://git.example/pipeline/901"), "{fx:?}");
    assert_eq!(p.pages.len(), 2);
}

fn host(p: GitlabPlugin) -> Host {
    let mut h = Host::new(vec![Box::new(demo::sample()), Box::new(Sender)], p.now).unwrap();
    h.load_config(Some("")).unwrap();
    h.plugins[0] = Box::new(p);
    h.focus = Pane::Middle;
    h.enable_journal();
    h
}
fn plugin(h: &mut Host) -> &mut GitlabPlugin {
    assert_eq!(h.plugins[0].id(), "gitlab");
    // SAFETY: host() installs this exact type at slot zero, never replaced.
    unsafe { &mut *(h.plugins[0].as_mut() as *mut dyn Plugin as *mut GitlabPlugin) }
}
struct Sender;
impl Plugin for Sender {
    fn id(&self) -> &'static str {
        "sender"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        vec![Effect::Show(*msg.downcast::<Subject>().unwrap())]
    }
}
fn press(h: &mut Host, code: Key) -> Vec<HostEffect> {
    h.update(crate::key_event(Chord::new(code)))
}
fn records(fx: &[HostEffect]) -> Vec<JournalRecord> {
    fx.iter()
        .filter_map(|e| match e {
            HostEffect::Journal(r) => Some(r.clone()),
            _ => None,
        })
        .collect()
}
fn begin(records: &[JournalRecord], kind: &str) -> (ActionId, ActionDescriptor, Option<ActionId>) {
    let got: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } if descriptor.kind.as_str() == kind => {
                Some((id.clone(), descriptor.clone(), parent.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(got.len(), 1, "one {kind}: {records:?}");
    got.into_iter().next().unwrap()
}
fn terminal(records: &[JournalRecord], id: &ActionId) -> Vec<Outcome> {
    records
        .iter()
        .filter_map(|r| match r {
            JournalRecord::Outcome {
                id: got, outcome, ..
            } if got == id && outcome.is_terminal() => Some(*outcome),
            _ => None,
        })
        .collect()
}
fn no_host_open(records: &[JournalRecord]) {
    assert!(!records.iter().any(
        |r| matches!(r, JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == OPEN)
    ));
}
fn tasks(fx: &[HostEffect]) -> Vec<(ActionId, ActionId)> {
    fx.iter()
        .filter_map(|e| match e {
            HostEffect::Tracked {
                action,
                semantic: Some(semantic),
                effect,
            } if matches!(&**effect, HostEffect::Spawn { .. }) => {
                Some((action.clone(), semantic.clone()))
            }
            _ => None,
        })
        .collect()
}
fn export(records: &[JournalRecord]) -> Vec<ActionEvent> {
    let dir =
        std::env::temp_dir().join(format!("radar-gitlab-page-open-{}", ActionId::new().as_str()));
    let mut db = Journal::open(&dir.join("actions.sqlite3")).unwrap();
    let session = db.start_session().unwrap();
    for r in records {
        match r {
            JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } => db
                .begin_with_id(&session, id, descriptor.clone(), parent.as_ref())
                .unwrap(),
            JournalRecord::Outcome {
                id,
                outcome,
                metadata,
            } => {
                db.record(id, *outcome, metadata.clone()).unwrap();
            }
            JournalRecord::Gap { .. } => panic!("unexpected gap"),
        }
    }
    db.finish_session(&session).unwrap();
    assert_eq!(
        db.work_map_snapshot().unwrap().map_sequence(),
        0,
        "opens create no work-map facts"
    );
    let events = db
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    let json = serde_json::to_string(&events).unwrap();
    for forbidden in [
        PRIVATE,
        "PRIVATE_USER",
        "PRIVATE_PASSWORD",
        "PRIVATE_TOKEN",
        "PRIVATE_FRAGMENT",
        "private.example",
        "PRIVATE_ERROR",
        "PRIVATE_REF",
        "PRIVATE_QUERY",
    ] {
        assert!(!json.contains(forbidden), "{forbidden} in export");
        for file in std::fs::read_dir(&dir).unwrap() {
            let bytes = std::fs::read(file.unwrap().path()).unwrap();
            assert!(
                !bytes
                    .windows(forbidden.len())
                    .any(|w| w == forbidden.as_bytes()),
                "{forbidden} in SQLite/sidecars"
            );
        }
    }
    drop(db);
    std::fs::remove_dir_all(dir).unwrap();
    events
}

#[test]
fn host_cached_repeated_enter_back_keeps_distinct_success_and_no_reads_or_payload() {
    let mut p = sample();
    select(&mut p, "m:acme/team-a/api!263");
    p.cfg.token = "env:PRIVATE_REF".into();
    p.cfg.sources[1].code = PRIVATE.into();
    let snapshot = p.snapshot();
    let mut h = host(p);
    let mut all = vec![];
    let mut ids = vec![];
    for _ in 0..2 {
        let fx = press(&mut h, Key::Enter);
        assert!(tasks(&fx).is_empty());
        let got = records(&fx);
        let (id, a, parent) = begin(&got, OPEN);
        let (input, _, _) = begin(&got, "plugin.input");
        assert_eq!(parent, Some(input));
        scope(
            &a,
            "https://git.example/install/",
            join::MERGE_REQUEST,
            "acme/team-a/api!263",
        );
        assert_eq!(terminal(&got, &id), [Outcome::Succeeded]);
        ids.push(id);
        all.extend(got);
        assert_eq!(h.focus, Pane::Middle);
        let back = records(&press(&mut h, Key::Esc));
        no_host_open(&back);
        all.extend(back);
    }
    assert_ne!(ids[0], ids[1]);
    assert!(plugin(&mut h).pages.is_empty());
    assert_eq!(plugin(&mut h).snapshot(), snapshot);
    let events = export(&all);
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind.as_str() == OPEN && e.outcome == Outcome::Succeeded)
            .count(),
        2
    );
    assert!(
        !events
            .iter()
            .any(|e| e.kind.as_str() == "gitlab.page.detail")
    );
}

#[test]
fn host_show_and_failed_state_click_keep_their_real_causes_and_accepted_scope() {
    let mut p = sample();
    let m = p
        .data
        .values_mut()
        .flat_map(|d| &mut d.mrs)
        .find(|m| m.project_path == PROJECT && m.iid == 263)
        .unwrap();
    m.merge_status = "CI_MUST_PASS".into();
    m.pipeline = Some(model::Pipeline::Failed);
    m.pipeline_url = Some(format!("https://git.example/{PROJECT}/-/pipelines/901"));
    assert_eq!(
        crate::list_actions::state_click(m),
        Some(crate::list_actions::StateClick::Why)
    );
    select(&mut p, "m:acme/team-a/api!263");
    let mut h = host(p);
    let shown = h.update(Event::Msg {
        plugin: 1,
        msg: Box::new(
            Subject::new(join::MERGE_REQUEST, format!("{PROJECT}!263"), PRIVATE)
                .with("project", PROJECT)
                .with("iid", "263"),
        ),
    });
    let mut all = records(&shown);
    let (id, a, parent) = begin(&all, OPEN);
    assert_eq!(parent, Some(begin(&all, "ui.show").0));
    scope(
        &a,
        "https://git.example/install/",
        join::MERGE_REQUEST,
        "acme/team-a/api!263",
    );
    assert_eq!(terminal(&all, &id), [Outcome::Succeeded]);
    let back = records(&press(&mut h, Key::Esc));
    no_host_open(&back);
    all.extend(back);
    // Use the failed-state target the view shows, not an artificial routing call.
    let _ = crate::tests::shown(&h, 180, 45);
    let target = crate::tests::state_targets(plugin(&mut h))
        .into_iter()
        .next()
        .expect("a failed-state target in the view");
    let clicked = h.update(Event::Intent(radar_core::view::Intent::Select {
        target: radar_core::view::Target::new(target),
    }));
    let got = records(&clicked);
    let (clicked_id, _, parent) = begin(&got, OPEN);
    let (pointer, _, pointer_parent) = begin(&got, "plugin.input");
    assert_eq!(parent, Some(pointer));
    assert_eq!(pointer_parent, Some(begin(&got, "ui.action").0));
    assert_eq!(terminal(&got, &clicked_id), [Outcome::Succeeded]);
    assert_ne!(clicked_id, id);
    assert_eq!(plugin(&mut h).pages.last().unwrap().tab, 3);
    all.extend(got);
    for subject in [
        Subject::new(join::MERGE_REQUEST, "missing", PRIVATE),
        pipeline_subject(),
    ] {
        let fx = h.update(Event::Msg {
            plugin: 1,
            msg: Box::new(subject),
        });
        let got = records(&fx);
        no_host_open(&got);
        all.extend(got);
    }
    export(&all);
}

#[test]
fn host_failed_partial_and_interrupted_read_and_diagnosis_siblings_never_relabel_open() {
    for expected in [Outcome::Failed, Outcome::Partial, Outcome::Unknown] {
        let mut p = sample();
        p.mr_pages.clear();
        p.failed.clear();
        let m = p
            .data
            .values_mut()
            .flat_map(|d| &mut d.mrs)
            .find(|m| m.project_path == PROJECT && m.iid == 263)
            .unwrap();
        m.merge_status = "CI_MUST_PASS".into();
        m.pipeline = Some(model::Pipeline::Failed);
        m.pipeline_url = Some(format!("https://git.example/{PROJECT}/-/pipelines/901"));
        select(&mut p, "m:acme/team-a/api!263");
        let mut h = host(p);
        let started = press(&mut h, Key::Enter);
        let pending = tasks(&started);
        assert_eq!(pending.len(), 2);
        let mut all = records(&started);
        let (open_id, _, open_parent) = begin(&all, OPEN);
        let (detail_id, detail_scope, detail_parent) = begin(&all, "gitlab.page.detail");
        let (diagnosis_id, diagnosis_scope, diagnosis_parent) =
            begin(&all, "gitlab.pipeline.diagnosis");
        assert_eq!(open_parent, detail_parent);
        assert_eq!(open_parent, diagnosis_parent);
        assert_ne!(open_id, detail_id);
        assert_ne!(open_id, diagnosis_id);
        assert_eq!(terminal(&all, &open_id), [Outcome::Succeeded]);
        // Drop real provider closures unpolled. Completion fixtures below preserve
        // the actual request owner and runner context, without credentials/I/O.
        drop(started);
        let got = records(&press(&mut h, Key::Esc));
        no_host_open(&got);
        all.extend(got);
        let again = press(&mut h, Key::Enter);
        assert!(tasks(&again).is_empty(), "busy siblings remain suppressed");
        let got = records(&again);
        let reopen = begin(&got, OPEN).0;
        all.extend(got);
        if expected != Outcome::Unknown {
            let detail_owner = plugin(&mut h).mr_page_reads[&pages::mr_key(PROJECT, 263)];
            let diagnosis_owner = plugin(&mut h).failed_reads[&crate::why::key(PROJECT, 901)];
            let result = if expected == Outcome::Failed {
                Err("PRIVATE_ERROR".into())
            } else {
                use crate::client::recorded::{Replay, block};
                let replay = Replay::new(vec![(
                    200,
                    serde_json::json!({"data": {"project": {"mergeRequest": {"iid": "263", "project": {"fullPath": PROJECT}, "description": PRIVATE, "diffStats": null}}}}),
                )]);
                let got = block(
                    crate::client::Client::over(replay.clone()).mr_detail_observed(PROJECT, 263),
                )
                .unwrap();
                assert_eq!(replay.asked().len(), 1);
                Ok(Box::new(got))
            };
            let diagnosis = if expected == Outcome::Failed {
                Err("PRIVATE_ERROR".into())
            } else {
                Ok(crate::client::FailedJobsRead {
                    jobs: vec![],
                    evidence: crate::client::FailedJobsEvidence {
                        incomplete_response: true,
                        ..Default::default()
                    },
                })
            };
            for (semantic, msg) in [
                (
                    diagnosis_id.clone(),
                    Box::new(GlMsg::Failed {
                        request: crate::log_ownership::PipelineRead {
                            project: PROJECT.into(),
                            pipeline: 901,
                            owner: diagnosis_owner,
                            activity: 0,
                        },
                        action: diagnosis_scope,
                        read: crate::why::DiagnosisRead {
                            result: diagnosis,
                            logs: Some(vec![]),
                        },
                    }) as Msg,
                ),
                (
                    detail_id.clone(),
                    Box::new(GlMsg::MrPage {
                        key: pages::mr_key(PROJECT, 263),
                        read: detail_owner,
                        requested: crate::page_identity::Requested::new(PROJECT, 263),
                        action: detail_scope,
                        result,
                    }) as Msg,
                ),
            ] {
                let runtime = pending
                    .iter()
                    .find(|(_, id)| *id == semantic)
                    .unwrap()
                    .0
                    .clone();
                let done = h.update(Event::Tracked {
                    action: runtime,
                    semantic: Some(semantic.clone()),
                    event: Box::new(Event::Msg { plugin: 0, msg }),
                });
                let got = records(&done);
                no_host_open(&got);
                assert_eq!(terminal(&got, &semantic), [expected]);
                assert!(terminal(&got, &open_id).is_empty() && terminal(&got, &reopen).is_empty());
                all.extend(got);
            }
        }
        let events = export(&all);
        for id in [&open_id, &reopen] {
            assert_eq!(
                events
                    .iter()
                    .filter(|e| &e.action_id == id && e.outcome.is_terminal())
                    .map(|e| e.outcome)
                    .collect::<Vec<_>>(),
                [Outcome::Succeeded]
            );
        }
        for id in [&detail_id, &diagnosis_id] {
            assert_eq!(
                events
                    .iter()
                    .filter(|e| &e.action_id == id && e.outcome.is_terminal())
                    .map(|e| e.outcome)
                    .collect::<Vec<_>>(),
                [expected]
            );
        }
    }
}
