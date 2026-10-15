//! Local routing only: returned tasks and external opens are never executed.
use super::*;
use crate::{Plugin, demo, model};
use radar_core::Host;
use radar_core::host::{Event, HostEffect, JournalRecord, Return, View};
use radar_core::plugin::{Msg, ScreenSpec};

const PROJECT: &str = "acme/team-a/api";
const WEB: &str = "acme/team-a/web";

fn direct(kind: &str, project: &str, number: u64) -> Subject {
    let (id, field) = match kind {
        MERGE_REQUEST => (format!("{project}!{number}"), Some("iid")),
        JOB | PIPELINE => (format!("{project}#{number}"), Some("id")),
        _ => (project.into(), None),
    };
    let s = Subject::new(kind, id, "requested item").with("project", project);
    match field {
        Some(f) => s.with(f, number.to_string()),
        None => s,
    }
}
fn url(base: &str, kind: &str, project: &str, number: u64) -> String {
    let base = base.trim_end_matches('/');
    match kind {
        MERGE_REQUEST => format!("{base}/{project}/-/merge_requests/{number}"),
        JOB => format!("{base}/{project}/-/jobs/{number}"),
        PIPELINE => format!("{base}/{project}/-/pipelines/{number}"),
        _ => format!("{base}/{project}"),
    }
}
fn held(p: &GitlabPlugin) -> String {
    format!(
        "{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}",
        p.snapshot(),
        p.pages,
        p.mr_pages,
        p.runs,
        p.logs,
        p.jobs,
        p.diffs,
        p.failed,
        p.mr_page_reads,
        p.log_reads,
        p.failed_reads,
        p.next_page_read,
        p.next_log_activity,
    )
}
fn assert_refused(p: &mut GitlabPlugin, subject: &Subject) {
    let before = held(p);
    assert!(matches!(show(p, subject), Showing::Cannot), "{subject:?}");
    assert_eq!(
        held(p),
        before,
        "refusal changed navigation/cache/read ownership"
    );
    assert!(
        p.journal_subject(subject).is_none(),
        "contradictory owner snapshot"
    );
}

#[test]
fn direct_and_no_url_requests_keep_exact_target_and_numeric_compatibility() {
    for base in ["https://gitlab.com/", "http://git.example:8181/install/"] {
        for (kind, project, number, expected) in [
            (REPO, PROJECT, 0, Kind::Repo(PROJECT.into())),
            (
                MERGE_REQUEST,
                PROJECT,
                263,
                Kind::Mr {
                    project: PROJECT.into(),
                    iid: 263,
                },
            ),
            (
                JOB,
                WEB,
                5001,
                Kind::Job {
                    project: WEB.into(),
                    id: 5001,
                },
            ),
        ] {
            for with_url in [false, true] {
                let mut p = demo::sample();
                p.cfg.url = base.into();
                let mut s = direct(kind, project, number);
                if with_url {
                    s.url = Some(url(base, kind, project, number));
                }
                let Showing::Page { effects, .. } = show(&mut p, &s) else {
                    panic!("{s:?}")
                };
                assert_eq!(p.pages.last().unwrap().kind, expected);
                assert!(matches!(effects[0], Effect::Focus(Pane::Middle)));
                assert_eq!(p.journal_subject(&s).is_some(), with_url);
                drop(effects);
            }
        }
    }
    for (id_number, field_number, url_number, number) in [
        ("000263", "0263", "263", 263),
        ("+263", "+0263", "000263", 263),
        ("000", "0", "00", 0),
    ] {
        let mut p = demo::sample();
        let s = Subject::new(MERGE_REQUEST, format!("{PROJECT}!{id_number}"), "numeric")
            .with("project", PROJECT)
            .with("iid", field_number)
            .with_url(format!(
                "https://gitlab.com/{PROJECT}/-/merge_requests/{url_number}"
            ));
        assert!(matches!(show(&mut p, &s), Showing::Page { .. }));
        assert_eq!(
            p.pages.last().unwrap().kind,
            Kind::Mr {
                project: PROJECT.into(),
                iid: number
            }
        );
    }
    // A valid direct subject without a URL retains current-context navigation,
    // even if the configured URL cannot be used to prove a permalink's origin.
    let mut p = demo::sample();
    p.cfg.url = "not configured".into();
    assert!(matches!(
        show(&mut p, &direct(REPO, PROJECT, 0)),
        Showing::Page { .. }
    ));
}

#[test]
fn legacy_id_url_journal_snapshots_need_no_fields_but_show_does() {
    let mut p = demo::sample();
    p.cfg.url = "http://git.example:8181/Install/".into();
    for kind in [REPO, MERGE_REQUEST, JOB, PIPELINE] {
        let mut s = direct(kind, PROJECT, 263).with_url(url(&p.cfg.url, kind, PROJECT, 263));
        s.fields.clear();
        let snapshot = p.journal_subject(&s).expect("legacy ID+URL snapshot");
        assert_eq!(snapshot.key.instance, "http://git.example:8181/Install/");
        if kind != PIPELINE {
            assert!(matches!(show(&mut p, &s), Showing::Cannot));
            s = s.with("project", PROJECT);
            if kind != REPO {
                assert!(matches!(show(&mut p, &s), Showing::Cannot));
            }
        }
        s = s.with("project", "other/project");
        assert!(p.journal_subject(&s).is_none());
    }
}

#[test]
fn supplied_contradictions_refuse_before_pages_cache_owners_or_counters_change() {
    let mut p = demo::sample();
    // Keep real pending owners and pages as sentinels; closures are dropped unpolled.
    drop(pages::push(
        &mut p,
        Kind::Mr {
            project: "acme/pending".into(),
            iid: 44,
        },
    ));
    drop(pages::push(
        &mut p,
        Kind::Job {
            project: "acme/pending".into(),
            id: 45,
        },
    ));
    crate::log_ownership::pipeline(&mut p, "acme/pending", 46);
    let good =
        direct(MERGE_REQUEST, PROJECT, 263).with_url(url(&p.cfg.url, MERGE_REQUEST, PROJECT, 263));
    let mut bad_id = good.clone();
    bad_id.id = format!("{PROJECT}!264");
    let mut bad_kind = good.clone();
    bad_kind.kind = JOB.into();
    for bad in [
        bad_id,
        bad_kind,
        good.clone().with("project", WEB),
        good.clone().with("iid", "264"),
        good.clone().with("iid", "NaN"),
        good.clone().with("id", "999"),
        good.clone()
            .with_url(url(&p.cfg.url, MERGE_REQUEST, PROJECT, 264)),
        good.clone().with_url(url(&p.cfg.url, JOB, PROJECT, 263)),
        good.clone()
            .with_url(url(&p.cfg.url, MERGE_REQUEST, WEB, 263)),
        good.clone()
            .with_url(url("https://foreign.example", MERGE_REQUEST, PROJECT, 263)),
        good.clone()
            .with_url(url("http://gitlab.com", MERGE_REQUEST, PROJECT, 263)),
        good.with_url(url("https://gitlab.com:8443", MERGE_REQUEST, PROJECT, 263)),
        direct(REPO, PROJECT, 0).with("project", WEB),
        direct(JOB, WEB, 5001).with("id", "5002"),
    ] {
        assert_refused(&mut p, &bad);
    }
    for project in [
        "", "a/../b", "a/./b", "a//b", "/a", "a/", "a%2Fb", "a\\b", "bad path",
    ] {
        for kind in [REPO, MERGE_REQUEST, JOB] {
            assert_refused(&mut p, &direct(kind, project, 1));
        }
    }
}

#[test]
fn origin_effective_port_and_installation_boundary_are_independent_of_cache() {
    let mut p = demo::sample();
    p.cfg.url = "https://Git.Example:443/git/".into();
    for prefix in ["https://git.example/git", "https://git.example:443/git"] {
        let s = direct(MERGE_REQUEST, "Acme/Team.Api", 12).with_url(url(
            prefix,
            MERGE_REQUEST,
            "Acme/Team.Api",
            12,
        ));
        assert!(matches!(show(&mut p, &s), Showing::Page { .. }));
        assert_eq!(
            p.pages.last().unwrap().kind,
            Kind::Mr {
                project: "Acme/Team.Api".into(),
                iid: 12
            }
        );
    }
    for prefix in [
        "https://git.example/gitlab",
        "https://git.example/Git",
        "https://git.example",
        "https://git.example:444/git",
        "http://git.example/git",
        "https://git.example.evil/git",
    ] {
        assert_refused(
            &mut p,
            &direct(MERGE_REQUEST, PROJECT, 12).with_url(url(prefix, MERGE_REQUEST, PROJECT, 12)),
        );
    }
    p.cfg.url = "http://git.example:8181/OtherInstall/".into();
    let original = url(&p.cfg.url, MERGE_REQUEST, PROJECT, 263) + "/diffs?view=parallel#note_3";
    let found = recognise(&p, &original);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, 0..original.len());
    assert_eq!(found[0].1.url.as_deref(), Some(original.as_str()));
    assert!(matches!(show(&mut p, &found[0].1), Showing::Page { .. }));
    assert!(
        p.journal_subject(&found[0].1).is_none(),
        "subroutes do not relax journal URL equality"
    );
}

#[test]
fn unsafe_url_repairs_do_not_manufacture_an_owned_target() {
    let mut p = demo::sample();
    for bad in [
        format!("https://user:pass@gitlab.com/{PROJECT}/-/merge_requests/263"),
        format!("https://@gitlab.com/{PROJECT}/-/merge_requests/263"),
        format!("https://gitlab.com\\{PROJECT}/-/merge_requests/263"),
        format!("https://gitlab.com/a/../{PROJECT}/-/merge_requests/263"),
        format!("https://gitlab.com/a/%2E%2e/{PROJECT}/-/merge_requests/263"),
        format!("https://gitlab.com/./{PROJECT}/-/merge_requests/263"),
        format!("https://gitlab.com/{PROJECT}/-/merge_requests/263/%2e"),
        format!("https://gitlab.com/{PROJECT}/-/merge_requests/263/a%2fb"),
        format!("https://gitlab.com/{PROJECT}/-/merge_requests/263/a%5Cb"),
        format!("https://gitlab.com/{PROJECT}/-/merge_requests/2\n63"),
        format!(" https://gitlab.com/{PROJECT}/-/merge_requests/263"),
        format!("https:///gitlab.com/{PROJECT}/-/merge_requests/263"),
        format!("https://gitlab.com/{PROJECT}/-/merge_requests/263\u{0000}"),
    ] {
        assert_refused(&mut p, &direct(MERGE_REQUEST, PROJECT, 263).with_url(&bad));
    }
    for bad in [
        format!("https://gitlab.com/a/%2e%2e/{PROJECT}/-/merge_requests/263"),
        format!("https://u@gitlab.com/{PROJECT}/-/merge_requests/263"),
        "https://gitlab.com/acme%2Fteam-a/api/-/merge_requests/263".to_string(),
    ] {
        assert!(recognise(&p, &bad).is_empty(), "{bad}");
    }
}

#[test]
fn full_candidates_keep_subroutes_query_fragments_and_ignore_foreign_embeddings() {
    let p = demo::sample();
    for (kind, tail) in [
        (MERGE_REQUEST, "/diffs"),
        (MERGE_REQUEST, "/commits"),
        (JOB, "/artifacts/browse"),
        (PIPELINE, "/test_report"),
    ] {
        let original = url(&p.cfg.url, kind, PROJECT, 263) + tail + "?view=one#note_3";
        let text = format!("See ({original}), then continue.");
        let got = recognise(&p, &text);
        assert_eq!(got.len(), 1);
        assert_eq!(&text[got[0].0.clone()], original);
        assert_eq!(got[0].1.url.as_deref(), Some(original.as_str()));
        assert_eq!(got[0].1.kind, kind);
    }
    let own = url(&p.cfg.url, MERGE_REQUEST, PROJECT, 263);
    for foreign in [
        format!("https://foreign.example/?next={own}"),
        format!("https://foreign.example/#{own}"),
        format!("https://foreign.example/{own}"),
        format!("https://foreign.example/?next=({own})"),
        format!("prefix{own}"),
        format!("text={own}"),
    ] {
        assert!(recognise(&p, &foreign).is_empty(), "{foreign}");
    }
}

#[test]
fn poisoned_cached_mr_cannot_replace_recognised_project_iid_url_or_label() {
    let mut p = demo::sample();
    let mut m = pages::find_mr(&p, PROJECT, 263).unwrap().clone();
    p.data.clear();
    m.project_path = "poisoned/project".into();
    m.iid = 999;
    m.web_url = "https://foreign.example/poisoned".into();
    m.title = "cached metadata is allowed".into();
    p.mr_pages.insert(
        pages::mr_key(PROJECT, 263),
        pages::Fetched {
            value: Some((m, model::fixtures::detail())),
            ..Default::default()
        },
    );
    let original = url(&p.cfg.url, MERGE_REQUEST, PROJECT, 263) + "/diffs?view=one#note_3";
    let got = recognise(&p, &original);
    let s = &got[0].1;
    assert_eq!(s.id, format!("{PROJECT}!263"));
    assert_eq!(s.field("project"), Some(PROJECT));
    assert_eq!(s.field("iid"), Some("263"));
    assert_eq!(s.field("title"), Some("cached metadata is allowed"));
    assert_eq!(s.label, "api !263");
    assert_eq!(s.url.as_deref(), Some(original.as_str()));
    assert!(matches!(show(&mut p, s), Showing::Page { .. }));
    assert_eq!(
        p.pages.last().unwrap().kind,
        Kind::Mr {
            project: PROJECT.into(),
            iid: 263
        }
    );
}

#[test]
fn written_and_plain_links_preserve_original_external_fallback_and_written_label() {
    let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(demo::sample())];
    let parts = Parts::new(&plugins, None);
    let own = url("https://gitlab.com", MERGE_REQUEST, PROJECT, 263);
    let external = format!("https://foreign.example/?next={own}#keep");
    let written = [radar_core::ui::rich::Link {
        text: "read this".into(),
        url: external.clone(),
    }];
    let got = links_in(&parts, &written, &[]);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].kind, "link");
    assert_eq!(got[0].url.as_deref(), Some(external.as_str()));
    assert_eq!(got[0].label, "read this");
    let got = links_in(&parts, &[], std::slice::from_ref(&external));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].kind, "link");
    assert_eq!(got[0].url.as_deref(), Some(external.as_str()));
    let written = [radar_core::ui::rich::Link {
        text: "review changes".into(),
        url: own.clone(),
    }];
    let got = links_in(&parts, &written, &[]);
    assert_eq!(got[0].kind, MERGE_REQUEST);
    assert_eq!(got[0].label, "api !263");
    assert_eq!(got[0].url.as_deref(), Some(own.as_str()));
}

struct Sender;
impl Plugin for Sender {
    fn id(&self) -> &'static str {
        "routing-sender"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "source",
            name: "source",
            key: 'Z',
        }]
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        vec![Effect::Show(*msg.downcast::<Subject>().unwrap())]
    }
}
fn host() -> Host {
    let p = demo::sample();
    let mut h = Host::new(vec![Box::new(p), Box::new(Sender)], 0).unwrap();
    h.enable_journal();
    h.view = View::Screen(1);
    h.focus = Pane::Detail;
    h.back.push(Return {
        view: View::Screen(0),
        focus: Pane::Nav,
        to: 1,
        depth: 1,
    });
    h
}
fn external_opens(fx: &[HostEffect]) -> Vec<&str> {
    fx.iter()
        .flat_map(|e| match e {
            HostEffect::Open(url) => vec![url.as_str()],
            HostEffect::Tracked { effect, .. } => external_opens(std::slice::from_ref(&**effect)),
            HostEffect::Spawn { .. } => panic!("refusal must not dispatch a provider read"),
            _ => vec![],
        })
        .collect()
}
fn assert_host_fallback(h: &mut Host, s: Subject, expected: &str) {
    let before = (
        h.view,
        h.focus,
        format!("{:?}", h.back),
        h.plugins[0].snapshot(),
        h.plugins[0].depth(crate::SCREEN.id),
    );
    let fx = h.update(Event::Msg {
        plugin: 1,
        msg: Box::new(s),
    });
    assert_eq!(external_opens(&fx), [expected]);
    assert_eq!(
        (
            h.view,
            h.focus,
            format!("{:?}", h.back),
            h.plugins[0].snapshot(),
            h.plugins[0].depth(crate::SCREEN.id)
        ),
        before
    );
    for effect in &fx {
        if let HostEffect::Journal(JournalRecord::Begin { descriptor, .. }) = effect {
            assert!(!descriptor.kind.as_str().starts_with("gitlab.page."));
            assert!(
                descriptor
                    .subjects
                    .iter()
                    .all(|s| s.key.provider != "gitlab"),
                "{descriptor:?}"
            );
        }
    }
}

#[test]
fn host_contradictory_subjects_keep_exact_external_url_view_and_back_stack() {
    let mut h = host();
    let original = url("https://gitlab.com", MERGE_REQUEST, PROJECT, 263) + "?view=one#note_3";
    let s = direct(MERGE_REQUEST, PROJECT, 263)
        .with("iid", "264")
        .with_url(&original);
    assert_host_fallback(&mut h, s, &original);
    let original = url("https://foreign.example", JOB, WEB, 5001) + "?download=1#keep";
    let s = direct(JOB, WEB, 5001).with_url(&original);
    assert_host_fallback(&mut h, s, &original);
}

#[test]
fn host_external_embedded_own_link_never_becomes_local_page() {
    let mut h = host();
    let original = format!(
        "https://foreign.example/?next={}#keep",
        url("https://gitlab.com", MERGE_REQUEST, PROJECT, 263)
    );
    assert_host_fallback(&mut h, Subject::link(&original), &original);
    let parts = Parts::new(&h.plugins, None);
    let subjects = links_in(
        &parts,
        &[radar_core::ui::rich::Link {
            text: "external".into(),
            url: original.clone(),
        }],
        &[],
    );
    assert_host_fallback(&mut h, subjects[0].clone(), &original);
}

#[test]
fn ipv6_and_case_insensitive_http_schemes_keep_original_candidate_spelling() {
    let mut p = demo::sample();
    p.cfg.url = "http://[::1]:8181/git/".into();
    let original =
        format!("HTTP://[::1]:8181/git/{PROJECT}/-/merge_requests/263?view=(one)#note_[3]");
    for text in [
        original.clone(),
        format!("[{original}]"),
        format!("[review]({original})"),
    ] {
        let got = recognise(&p, &text);
        assert_eq!(got.len(), 1, "{text}");
        assert_eq!(&text[got[0].0.clone()], original);
        assert_eq!(got[0].1.url.as_deref(), Some(original.as_str()));
        assert!(matches!(show(&mut p, &got[0].1), Showing::Page { .. }));
    }
}

struct PartialRecogniser;
impl Plugin for PartialRecogniser {
    fn id(&self) -> &'static str {
        "partial-recogniser"
    }
    fn owns(&self) -> Vec<Owned> {
        vec![Owned::page("partial", "partial references")]
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn recognise(&self, text: &str) -> Vec<(Range<usize>, Subject)> {
        text.find("inside")
            .map(|at| (at..at + 6, Subject::new("partial", "inside", "partial")))
            .into_iter()
            .collect()
    }
}
#[test]
fn partial_recognition_by_any_part_cannot_suppress_a_complete_external_link() {
    let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(PartialRecogniser)];
    let parts = Parts::new(&plugins, None);
    let original = "https://external.example/inside?keep=1#original";
    let recognised = parts.recognise(original);
    assert_eq!(recognised.len(), 1, "exercise actual partial ownership");
    assert_ne!(recognised[0].0, 0..original.len());
    let written = [radar_core::ui::rich::Link {
        text: "original label".into(),
        url: original.into(),
    }];
    let got = links_in(&parts, &written, &[]);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].kind, "link");
    assert_eq!(got[0].label, "original label");
    assert_eq!(got[0].url.as_deref(), Some(original));
    let got = links_in(&parts, &[], &[original.into()]);
    assert!(
        got.iter()
            .any(|s| s.kind == "link" && s.url.as_deref() == Some(original))
    );
}

#[test]
fn configured_credentials_are_not_permalink_authority_and_generated_urls_refuse_locally() {
    let mut p = demo::sample();
    p.cfg.url = "https://ROUTING_USER:ROUTING_PASSWORD@git.example/install/".into();
    let clean = direct(MERGE_REQUEST, PROJECT, 263).with_url(url(
        "https://git.example/install",
        MERGE_REQUEST,
        PROJECT,
        263,
    ));
    assert!(matches!(show(&mut p, &clean), Showing::Page { .. }));
    let snapshot = p
        .journal_subject(&clean)
        .expect("existing configured-base sanitization");
    assert_eq!(snapshot.key.instance, "https://git.example/install/");
    assert!(!format!("{snapshot:?}").contains("ROUTING_"));
    let supplied = clean.with_url(url(&p.cfg.url, MERGE_REQUEST, PROJECT, 263));
    assert_refused(&mut p, &supplied);
    let generated = repo(&p, PROJECT);
    let original = generated.url.clone().unwrap();
    assert_refused(&mut p, &generated);
    let mut h = host();
    h.plugins[0] = Box::new(p);
    assert_host_fallback(&mut h, generated, &original);
}
