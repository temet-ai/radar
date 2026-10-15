use super::*;
use crate::{demo, detail_key, join, page};
use radar_core::host::{Event, HostEffect, JournalRecord, View};
use radar_core::journal::{ActionDescriptor, Outcome};
use radar_core::plugin::{Effect, Msg, Pane};
use radar_core::{Host, Plugin, Showing};

const KEY: &str = "ABC-12";
const PRIVATE: &str = "PRIVATE_ROUTING_CONTEXT_547";

fn ticket() -> Subject {
    Subject::new(join::TICKET, KEY, PRIVATE)
}

fn refuse(p: &mut JiraPlugin, subject: Subject) {
    let state = format!("{p:?}");
    let snapshot = p.snapshot();
    assert!(
        matches!(p.show(&subject), Showing::Cannot),
        "must refuse local routing"
    );
    assert_eq!(
        format!("{p:?}"),
        state,
        "no pages/cache/owners/loading/counters change"
    );
    assert_eq!(p.snapshot(), snapshot);
    assert!(
        p.journal_subject(&subject).is_none(),
        "refused explicit scope has no owner snapshot"
    );
}

fn open(p: &mut JiraPlugin, subject: Subject, alias: &str) -> Vec<Effect> {
    let before = format!("{subject:?}");
    let depth = p.pages.len();
    let Showing::Page { screen, effects } = p.show(&subject) else {
        panic!("consistent subject opens")
    };
    assert_eq!(screen, crate::SCREEN.id);
    assert_eq!(p.pages.len(), depth + 1);
    let page = p.pages.last().unwrap();
    assert_eq!((&*page.site, &*page.key), (alias, &*subject.id));
    assert_eq!(
        format!("{subject:?}"),
        before,
        "original fallback context is unchanged"
    );
    let opens: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Observe { action, outcome } if action.kind.as_str() == "jira.page.open" => {
                assert_eq!(*outcome, Outcome::Succeeded);
                Some(action)
            }
            _ => None,
        })
        .collect();
    assert_eq!(opens.len(), 1);
    effects
}

#[test]
fn explicit_known_alias_needs_no_watched_project_and_accepts_matching_url() {
    for alias in ["acme", "beta"] {
        for supplied_url in [false, true] {
            let mut p = demo::sample();
            let mut subject = ticket().with("site", alias);
            if supplied_url {
                subject = subject.with_url(p.cfg.site(alias).unwrap().browse(KEY).unwrap());
            }
            let fx = open(&mut p, subject, alias);
            assert!(fx.iter().any(|e| matches!(e, Effect::SpawnAction { .. })));
            assert!(p.details[&detail_key(alias, KEY)].request.is_some());
        }
    }
}

#[test]
fn unknown_empty_and_removed_aliases_never_become_bare_fallbacks() {
    let mut p = demo::sample();
    page::open(&mut p, "beta", "XYZ-7");
    for alias in ["unknown", "", " acme", "ACME"] {
        for subject in [
            ticket(),
            ticket().with_url("https://acme.atlassian.net/browse/ABC-12"),
        ] {
            refuse(&mut p, subject.with("site", alias));
        }
    }
    let subject = join::ticket(&p, "acme", KEY);
    p.cfg.sites.remove(0);
    refuse(&mut p, subject);
    // Even a malformed imported configuration must not turn an empty field into
    // a usable explicit scope.
    p.cfg.sites[0].name.clear();
    refuse(&mut p, ticket().with("site", ""));
}

#[test]
fn unresolved_configured_address_cannot_match_a_url_but_no_url_behavior_stays() {
    let mut p = demo::sample();
    let missing = "RADAR_JIRA_SUBJECT_SCOPE_TEST_UNSET_9F34B1";
    assert!(std::env::var_os(missing).is_none());
    p.cfg.sites[0].url = format!("env:{missing}");
    let explicit = ticket().with("site", "acme");
    assert!(p.journal_subject(&explicit).is_none());
    open(&mut p, explicit.clone(), "acme");
    refuse(
        &mut p,
        explicit.with_url("https://acme.atlassian.net/browse/ABC-12"),
    );
    refuse(
        &mut p,
        ticket().with_url("https://acme.atlassian.net/browse/ABC-12"),
    );
    open(
        &mut p,
        ticket().with_url("https://beta.atlassian.net/browse/ABC-12"),
        "beta",
    );
    p.cfg.sites.clear();
    refuse(&mut p, ticket());
}

#[test]
fn foreign_wrong_key_and_alias_url_disagreement_refuse_before_page_work() {
    let mut p = demo::sample();
    page::open(&mut p, "beta", "XYZ-7");
    crate::tests::complete_detail(&mut p, "acme", KEY, Ok(crate::tests::detail()));
    page::fetch(&mut p, "acme", KEY, true);
    for url in [
        "https://foreign.example/browse/ABC-12?token=PRIVATE_ROUTING_CONTEXT_547#secret",
        "https://beta.atlassian.net/browse/ABC-12",
        "https://acme.atlassian.net/browse/ABC-13",
        "https://acme.atlassian.net/browse/abc-12",
        "https://acme.atlassian.net/browse/ABC-012",
        "https://acme.atlassian.net/browse/ABC-12/",
        "https://acme.atlassian.net/browse/ABC-12/comments",
        "https://acme.atlassian.net/jira/browse/ABC-12",
        "https://acme.atlassian.net/?ticket=ABC-12#/browse/ABC-12",
        "",
    ] {
        refuse(&mut p, ticket().with("site", "acme").with_url(url));
    }
}

#[test]
fn url_without_alias_selects_unique_origin_even_when_another_site_watches_key() {
    let mut p = demo::sample();
    let subject = ticket().with_url("https://beta.atlassian.net/browse/ABC-12?x=1#comment");
    assert!(
        p.journal_subject(&subject).is_none(),
        "journal never infers an alias"
    );
    open(&mut p, subject, "beta");
    refuse(
        &mut p,
        ticket().with_url("https://foreign.example/browse/ABC-12"),
    );
}

#[test]
fn duplicate_origin_aliases_require_explicit_disambiguation() {
    let mut p = demo::sample();
    let mut duplicate = p.cfg.sites[0].clone();
    duplicate.name = "second-account".into();
    duplicate.url = "https://ACME.atlassian.net:443/jira/software?config=discarded".into();
    duplicate.projects.clear();
    p.cfg.sites.push(duplicate);
    let subject = ticket().with_url("https://acme.atlassian.net/browse/ABC-12");
    refuse(&mut p, subject.clone());
    open(
        &mut p,
        subject.with("site", "second-account"),
        "second-account",
    );
}

#[test]
fn bare_no_url_preserves_watched_then_held_then_first_precedence() {
    let mut p = demo::sample();
    open(&mut p, Subject::new(join::TICKET, "XYZ-7", "XYZ-7"), "beta");
    // Watchers win over a different site's held copy.
    p.cfg.sites[1].projects.push(crate::config::Project {
        key: "ABC".into(),
        ac_field: String::new(),
    });
    p.cfg.sites[0].projects.clear();
    open(&mut p, ticket(), "beta");
    // Once nobody watches, the held copy wins over configuration order.
    p.cfg.sites[1].projects.clear();
    p.cfg.sites.swap(0, 1);
    open(&mut p, ticket(), "acme");
    open(
        &mut p,
        Subject::new(join::TICKET, "NOPE-7", "NOPE-7"),
        "beta",
    );
    assert!(p.journal_subject(&ticket()).is_none());
}

#[test]
fn origin_normalization_keeps_case_default_ports_and_cloud_config_path_behavior() {
    let mut p = demo::sample();
    for configured in [
        "https://ACME.atlassian.net:443/",
        " acme.atlassian.net/jira/software/projects/ABC?token=PRIVATE_ROUTING_CONTEXT_547#fragment ",
    ] {
        p.cfg.sites[0].url = configured.into();
        for given in [
            "https://acme.atlassian.net/browse/ABC-12",
            "HTTPS://ACME.ATLASSIAN.NET:443/browse/ABC-12?token=PRIVATE_ROUTING_CONTEXT_547#fragment",
        ] {
            let subject = ticket().with("site", "acme").with_url(given);
            let snapshot = p.journal_subject(&subject).unwrap();
            assert!(!format!("{snapshot:?}").contains(PRIVATE));
            open(&mut p, subject, "acme");
        }
    }
}

#[test]
fn origins_compare_scheme_host_and_effective_port_exactly() {
    let mut p = demo::sample();
    for (configured, good, bad) in [
        (
            "https://acme.atlassian.net",
            "https://ACME.atlassian.net:443",
            vec![
                "http://acme.atlassian.net",
                "https://acme.atlassian.net:8443",
                "https://acme.atlassian.net.evil.example",
            ],
        ),
        (
            "http://jira.example:8181",
            "http://JIRA.example:8181",
            vec![
                "https://jira.example:8181",
                "http://jira.example",
                "http://jira.example:8182",
            ],
        ),
        (
            "http://jira.example",
            "http://jira.example:80",
            vec!["https://jira.example", "http://jira.example:81"],
        ),
        (
            "https://[::1]:8443",
            "https://[::1]:8443",
            vec!["https://[::1]", "https://[::2]:8443"],
        ),
    ] {
        p.cfg.sites[0].url = configured.into();
        open(
            &mut p,
            ticket()
                .with("site", "acme")
                .with_url(format!("{good}/browse/{KEY}")),
            "acme",
        );
        for origin in bad {
            refuse(
                &mut p,
                ticket()
                    .with("site", "acme")
                    .with_url(format!("{origin}/browse/{KEY}")),
            );
        }
    }
}

#[test]
fn credentials_malformed_urls_and_parser_repairs_cannot_manufacture_scope() {
    let mut p = demo::sample();
    for url in [
        "https://alice:PRIVATE_ROUTING_CONTEXT_547@acme.atlassian.net/browse/ABC-12",
        "https://@acme.atlassian.net/browse/ABC-12",
        "https://acme.atlassian.net@foreign.example/browse/ABC-12",
        "https://acme.atlassian.net:/browse/ABC-12",
        "https://%61cme.atlassian.net/browse/ABC-12",
        " https://acme.atlassian.net/browse/ABC-12",
        "https://acme.atlassian.net/browse/ABC-12\n",
        "https://acme.atlas\tsian.net/browse/ABC-12",
        "https:acme.atlassian.net/browse/ABC-12",
        "https:///acme.atlassian.net/browse/ABC-12",
        "//acme.atlassian.net/browse/ABC-12",
        "https://[invalid/browse/ABC-12",
        "https://acme.atlassian.net\\browse\\ABC-12",
        "https://acme.atlassian.net/other/../browse/ABC-12",
        "https://acme.atlassian.net/./browse/ABC-12",
        "https://acme.atlassian.net/%2e/browse/ABC-12",
        "https://acme.atlassian.net/other/%2E%2e/browse/ABC-12",
        "https://acme.atlassian.net/browse%2fABC-12",
        "https://acme.atlassian.net/browse%5CABC-12",
        "https://acme.atlassian.net/browse/%41BC-12",
        "https://acme.atlassian.net//browse/ABC-12",
        "ftp://acme.atlassian.net/browse/ABC-12",
    ] {
        refuse(&mut p, ticket().with("site", "acme").with_url(url));
        refuse(&mut p, ticket().with_url(url));
    }
}

#[test]
fn configured_userinfo_does_not_authorize_supplied_userinfo() {
    let mut p = demo::sample();
    p.cfg.sites[0].url =
        "https://alice:PRIVATE_ROUTING_CONTEXT_547@acme.atlassian.net/config".into();
    open(
        &mut p,
        ticket()
            .with("site", "acme")
            .with_url("https://acme.atlassian.net/browse/ABC-12"),
        "acme",
    );
    let generated = join::ticket(&p, "acme", KEY);
    refuse(&mut p, generated);
}

#[test]
fn same_key_on_two_sites_keeps_actual_page_read_and_snapshot_scopes_distinct() {
    let mut p = demo::sample();
    let mut snapshots = vec![];
    for alias in ["acme", "beta"] {
        let subject = ticket().with("site", alias);
        snapshots.push(p.journal_subject(&subject).unwrap());
        let fx = open(&mut p, subject, alias);
        let expected = format!("https://{alias}.atlassian.net/");
        for effect in fx {
            if let Effect::Observe { action, .. } | Effect::SpawnAction { action, .. } = effect {
                assert_eq!(action.subjects[0].key.instance, expected);
                assert_eq!(action.subjects[0].key.id, KEY);
            }
        }
    }
    assert_ne!(snapshots[0].key, snapshots[1].key);
    assert_eq!(p.details.len(), 2);
}

#[test]
fn journal_keeps_no_inference_and_existing_snapshot_limits() {
    let mut p = demo::sample();
    let subject = ticket().with("site", "acme").with_url("https://acme.atlassian.net/browse/ABC-12?next=%2fother%2fABC-99#PRIVATE_ROUTING_CONTEXT_547");
    let snapshot = p.journal_subject(&subject).unwrap();
    assert_eq!(
        snapshot.url.unwrap().as_str(),
        "https://acme.atlassian.net/browse/ABC-12"
    );
    open(&mut p, subject, "acme");
    let long = ticket().with("site", "acme").with_url(format!(
        "https://acme.atlassian.net/browse/ABC-12?context={}",
        "x".repeat(8192)
    ));
    assert!(
        p.journal_subject(&long).is_none(),
        "canonical snapshot limit stays strict"
    );
    open(&mut p, long, "acme");
    for subject in [
        ticket(),
        ticket().with_url("https://acme.atlassian.net/browse/ABC-12"),
    ] {
        assert!(p.journal_subject(&subject).is_none());
    }
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

fn host() -> Host {
    let mut host = Host::new(vec![Box::new(demo::sample()), Box::new(Sender)], demo::NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(demo::sample());
    host.view = View::Settings;
    host.focus = Pane::Detail;
    host.enable_journal();
    host
}

fn send(host: &mut Host, subject: Subject) -> Vec<HostEffect> {
    host.update(Event::Msg {
        plugin: 1,
        msg: Box::new(subject),
    })
}

fn actions(effects: &[HostEffect]) -> Vec<&ActionDescriptor> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Journal(JournalRecord::Begin { descriptor, .. }) => Some(descriptor),
            _ => None,
        })
        .collect()
}

fn untracked(effect: &HostEffect) -> &HostEffect {
    match effect {
        HostEffect::Tracked { effect, .. } => untracked(effect),
        _ => effect,
    }
}

#[test]
fn host_refusal_keeps_exact_original_fallback_view_stack_and_journal_omission() {
    let mut h = host();
    // First establish a genuine cross-view return frame and a pending read.
    drop(send(&mut h, ticket().with("site", "beta")));
    assert_eq!(h.back.len(), 1);
    h.focus = Pane::Detail;
    h.tabs = true;
    for subject in [
        ticket().with("site", "acme").with_url(
            "https://foreign.example/browse/ABC-12?token=PRIVATE_ROUTING_CONTEXT_547#secret",
        ),
        ticket()
            .with("site", "removed")
            .with_url("https://acme.atlassian.net/browse/ABC-12?token=PRIVATE_ROUTING_CONTEXT_547"),
        ticket()
            .with("site", "acme")
            .with_url("https://alice:PRIVATE_ROUTING_CONTEXT_547@acme.atlassian.net/browse/ABC-12"),
        ticket().with("site", "acme").with_url(
            "https://acme.atlassian.net/secret/../browse/ABC-12?token=PRIVATE_ROUTING_CONTEXT_547",
        ),
        ticket().with("site", "removed"),
    ] {
        let state = format!("{:?}", crate::tests::jira_mut(&mut h));
        let navigation = format!("{:?}{:?}{:?}{:?}", h.view, h.focus, h.tabs, h.back);
        let fx = send(&mut h, subject.clone());
        let opens: Vec<_> = fx
            .iter()
            .filter_map(|e| match untracked(e) {
                HostEffect::Open(url) => Some(url.as_str()),
                HostEffect::Spawn { .. } => panic!("refusal cannot launch a read"),
                _ => None,
            })
            .collect();
        assert_eq!(
            opens,
            subject.url.as_deref().into_iter().collect::<Vec<_>>()
        );
        assert_eq!(format!("{:?}", crate::tests::jira_mut(&mut h)), state);
        assert_eq!(
            format!("{:?}{:?}{:?}{:?}", h.view, h.focus, h.tabs, h.back),
            navigation
        );
        let records = actions(&fx);
        assert!(
            !records
                .iter()
                .any(|a| a.kind.as_str().starts_with("jira.page."))
        );
        let show = records
            .iter()
            .find(|a| a.kind.as_str() == "ui.show")
            .unwrap();
        assert!(show.subjects.is_empty());
        assert!(show.metadata.redacted);
        assert_eq!(show.metadata.omitted_subjects, 1);
        assert!(!format!("{records:?}").contains(PRIVATE));
        // Host effects are only inspected/dropped; no opener or provider runs.
    }
}

#[test]
fn host_accepted_scope_matches_actual_page_and_url_only_show_still_omits_snapshot() {
    for explicit in [false, true] {
        let mut h = host();
        let mut subject = ticket().with_url(
            "https://beta.atlassian.net/browse/ABC-12?token=PRIVATE_ROUTING_CONTEXT_547#secret",
        );
        if explicit {
            subject = subject.with("site", "beta");
        }
        let fx = send(&mut h, subject);
        assert!(
            !fx.iter()
                .any(|e| matches!(untracked(e), HostEffect::Open(_)))
        );
        assert_eq!(h.back.len(), 1);
        let p = crate::tests::jira_mut(&mut h);
        assert_eq!(p.pages.last().unwrap().site, "beta");
        assert!(p.details.contains_key(&detail_key("beta", KEY)));
        assert!(!p.details.contains_key(&detail_key("acme", KEY)));
        let records = actions(&fx);
        let show = records
            .iter()
            .find(|a| a.kind.as_str() == "ui.show")
            .unwrap();
        assert_eq!(show.subjects.len(), usize::from(explicit));
        assert_eq!(show.metadata.omitted_subjects, u32::from(!explicit));
        for kind in ["jira.page.open", "jira.page.detail"] {
            let action = records.iter().find(|a| a.kind.as_str() == kind).unwrap();
            assert_eq!(
                action.subjects[0].key.instance,
                "https://beta.atlassian.net/"
            );
            assert_eq!(action.subjects[0].key.id, KEY);
        }
        assert!(!format!("{records:?}").contains(PRIVATE));
    }
}

#[test]
fn host_full_browse_link_recognition_opens_the_unique_site_with_truthful_page_history() {
    let mut h = host();
    let url = "https://beta.atlassian.net/browse/ABC-12?other=XYZ-7#PRIVATE_ROUTING_CONTEXT_547";
    let fx = send(&mut h, Subject::link(url));
    assert!(
        !fx.iter()
            .any(|e| matches!(untracked(e), HostEffect::Open(_)))
    );
    assert_eq!(h.back.len(), 1);
    let p = crate::tests::jira_mut(&mut h);
    assert_eq!(p.pages.last().unwrap().site, "beta");
    assert!(p.details.contains_key(&detail_key("beta", KEY)));
    assert!(!p.details.contains_key(&detail_key("acme", KEY)));
    let records = actions(&fx);
    for kind in ["jira.page.open", "jira.page.detail"] {
        let action = records.iter().find(|a| a.kind.as_str() == kind).unwrap();
        assert_eq!(
            action.subjects[0].key.instance,
            "https://beta.atlassian.net/"
        );
        assert_eq!(action.subjects[0].key.id, KEY);
    }
    assert!(!format!("{records:?}").contains(PRIVATE));
    // The initiating ui.show records the supplied raw link; only the actual
    // page/read actions claim Jira scope. No production task/opener is polled.
}

#[test]
fn host_rejected_raw_links_keep_exact_fallback_and_emit_no_jira_page_or_read_history() {
    for duplicate in [false, true] {
        let mut h = host();
        if duplicate {
            let p = crate::tests::jira_mut(&mut h);
            let mut alias = p.cfg.sites[0].clone();
            alias.name = "second-account".into();
            alias.url = "https://ACME.ATLASSIAN.NET:443/other".into();
            p.cfg.sites.push(alias);
        }
        let urls = if duplicate {
            vec!["https://acme.atlassian.net/browse/ABC-12?other=XYZ-7#PRIVATE_ROUTING_CONTEXT_547"]
        } else {
            vec![
                "https://foreign.example/browse/ABC-12?other=XYZ-7#PRIVATE_ROUTING_CONTEXT_547",
                "https:/acme.atlassian.net/browse/ABC-12",
                "https://[broken/browse/ABC-12",
                "https://foreign.example/?next=https://beta.atlassian.net/browse/ABC-12",
                "https://acme.atlassian.net/other/../browse/ABC-12",
            ]
        };
        for url in urls {
            let state = format!("{:?}", crate::tests::jira_mut(&mut h));
            let navigation = format!("{:?}{:?}{:?}{:?}", h.view, h.focus, h.tabs, h.back);
            let fx = send(&mut h, Subject::link(url));
            let opens: Vec<_> = fx
                .iter()
                .filter_map(|e| match untracked(e) {
                    HostEffect::Open(url) => Some(url.as_str()),
                    HostEffect::Spawn { .. } => panic!("external link cannot dispatch a Jira read"),
                    _ => None,
                })
                .collect();
            assert_eq!(opens, [url]);
            assert_eq!(format!("{:?}", crate::tests::jira_mut(&mut h)), state);
            assert_eq!(
                format!("{:?}{:?}{:?}{:?}", h.view, h.focus, h.tabs, h.back),
                navigation
            );
            let records = actions(&fx);
            assert!(
                !records
                    .iter()
                    .any(|a| a.kind.as_str().starts_with("jira.page."))
            );
            assert!(!format!("{records:?}").contains(PRIVATE));
        }
    }
}
