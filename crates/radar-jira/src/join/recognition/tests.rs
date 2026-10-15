use super::*;
use crate::{demo, join};
use radar_core::Plugin;

fn one(p: &JiraPlugin, url: &str, alias: &str, key: &str) -> Subject {
    let state = format!("{p:?}");
    let got = recognise(p, url);
    assert_eq!(got.len(), 1, "{url}: {got:?}");
    let (range, subject) = &got[0];
    assert_eq!(*range, 0..url.len());
    assert_eq!(&url[range.clone()], url);
    assert_eq!(subject.kind, join::TICKET);
    assert_eq!(subject.id, key);
    assert_eq!(subject.field("site"), Some(alias));
    assert_eq!(subject.url.as_deref(), Some(url));
    assert_eq!(subject_scope::resolve(p, subject).unwrap().name, alias);
    let snapshot = p.journal_subject(subject).expect("truthful explicit owner");
    let expected = p.cfg.site(alias).unwrap();
    assert_eq!(
        snapshot.key.instance,
        radar_core::journal::SafeUrl::parse(&expected.base_url().unwrap())
            .unwrap()
            .as_str()
    );
    assert_eq!(snapshot.key.id, key);
    assert_eq!(
        snapshot.url.unwrap().as_str(),
        expected.browse(key).unwrap()
    );
    assert_eq!(format!("{p:?}"), state, "recognition does not mutate state");
    subject.clone()
}

#[test]
fn configured_browse_urls_have_full_ranges_exact_original_and_correct_site() {
    let p = demo::sample();
    one(
        &p,
        "https://acme.atlassian.net/browse/ABC-12",
        "acme",
        "ABC-12",
    );
    let subject = one(
        &p,
        "https://beta.atlassian.net/browse/ABC-12?other=XYZ-7#ABC-13",
        "beta",
        "ABC-12",
    );
    assert!(
        subject.field("title").is_none(),
        "must not borrow acme's held fields"
    );
    one(
        &p,
        "https://beta.atlassian.net/browse/NOPE-7",
        "beta",
        "NOPE-7",
    );
    assert!(
        recognise(&p, "NOPE-7").is_empty(),
        "bare unknown stays unknown"
    );
}

#[test]
fn case_default_ports_and_configured_cloud_paths_share_the_scope_parser() {
    let mut p = demo::sample();
    p.cfg.sites[0].url = "https://ACME.atlassian.net:443/jira/software?discard=1#config".into();
    // Journal snapshots canonicalize the configured origin too.
    let url = "HTTPS://ACME.ATLASSIAN.NET:443/browse/ABC-12?other=XYZ-7#note";
    let got = recognise(&p, url);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, 0..url.len());
    assert_eq!(got[0].1.url.as_deref(), Some(url));
    assert_eq!(subject_scope::resolve(&p, &got[0].1).unwrap().name, "acme");
    let snapshot = p.journal_subject(&got[0].1).unwrap();
    assert_eq!(snapshot.key.instance, "https://acme.atlassian.net/");
    assert_eq!(
        snapshot.url.unwrap().as_str(),
        "https://acme.atlassian.net/browse/ABC-12"
    );
}

#[test]
fn duplicate_normalized_origins_cannot_choose_by_project_or_held_key() {
    let mut p = demo::sample();
    let mut other = p.cfg.sites[0].clone();
    other.name = "second-account".into();
    other.url = "https://ACME.ATLASSIAN.NET:443/other".into();
    other.projects.clear();
    p.cfg.sites.push(other);
    assert!(recognise(&p, "https://acme.atlassian.net/browse/ABC-12").is_empty());
    assert_eq!(recognise(&p, "ABC-12")[0].1.field("site"), Some("acme"));
    one(
        &p,
        "https://beta.atlassian.net/browse/ABC-12",
        "beta",
        "ABC-12",
    );
}

#[test]
fn unresolved_missing_empty_and_shadowed_site_aliases_never_gain_scope() {
    let mut p = demo::sample();
    let missing = "RADAR_JIRA_URL_RECOGNITION_UNSET_C3D66";
    assert!(std::env::var_os(missing).is_none());
    p.cfg.sites[0].url = format!("env:{missing}");
    assert!(recognise(&p, "https://acme.atlassian.net/browse/ABC-12").is_empty());
    p.cfg.sites[1].name.clear();
    assert!(recognise(&p, "https://beta.atlassian.net/browse/ABC-12").is_empty());
    p.cfg.sites[1].name = "acme".into();
    assert!(recognise(&p, "https://beta.atlassian.net/browse/ABC-12").is_empty());
    p.cfg.sites.clear();
    assert!(recognise(&p, "https://beta.atlassian.net/browse/ABC-12 ABC-12").is_empty());
}

#[test]
fn foreign_and_wrong_origin_links_mask_path_query_and_fragment_keys() {
    let p = demo::sample();
    for origin in [
        "https://foreign.example",
        "http://acme.atlassian.net",
        "https://acme.atlassian.net:8443",
        "https://acme.atlassian.net.evil.example",
        "ftp://acme.atlassian.net",
        "custom+v1.2://acme.atlassian.net",
        "//acme.atlassian.net",
        "www.acme.atlassian.net",
    ] {
        let text = format!("{origin}/browse/ABC-12?other=XYZ-7#ABC-13");
        assert!(recognise(&p, &text).is_empty(), "{text}");
    }
}

#[test]
fn invalid_paths_and_encoded_keys_never_fall_back_to_substring_keys() {
    let p = demo::sample();
    for path in [
        "/browse/abc-12",
        "/browse/ABC-012",
        "/browse/ABC-0",
        "/browse/ABC-12x",
        "/browse/ABC-12/",
        "/browse/ABC-12/comments",
        "/jira/browse/ABC-12",
        "/other/../browse/ABC-12",
        "/./browse/ABC-12",
        "/%2e/browse/ABC-12",
        "/other/%2E%2e/browse/ABC-12",
        "/browse%2fABC-12",
        "/browse/%41BC-12",
        "/browse%5CABC-12",
        "//browse/ABC-12",
        "/?ticket=ABC-12#/browse/XYZ-7",
    ] {
        let text = format!("https://acme.atlassian.net{path}");
        assert!(recognise(&p, &text).is_empty(), "{text}");
    }
}

#[test]
fn malformed_schemes_authorities_and_ipv6_still_mask_the_entire_token() {
    let p = demo::sample();
    for url in [
        "https://alice:secret@acme.atlassian.net/browse/ABC-12",
        "https://@acme.atlassian.net/browse/ABC-12",
        "https://acme.atlassian.net:/browse/ABC-12",
        "https://%61cme.atlassian.net/browse/ABC-12",
        "https:///acme.atlassian.net/browse/ABC-12",
        "https:acme.atlassian.net/browse/ABC-12",
        "HtTpS:/acme.atlassian.net/browse/ABC-12",
        "HTTP:/acme.atlassian.net/browse/ABC-12",
        "https://[invalid/browse/ABC-12",
        "https://[::1/browse/ABC-12",
        "https://acme.atlassian.net\\browse\\ABC-12",
        "https://:bad/browse/ABC-12",
    ] {
        let text = format!("{url}?other=XYZ-7#ABC-13");
        assert!(recognise(&p, &text).is_empty(), "{text}");
    }
}

#[test]
fn nested_urls_and_punctuation_never_restart_the_candidate() {
    let p = demo::sample();
    for url in [
        "https://foreign.example/?next=https://beta.atlassian.net/browse/ABC-12",
        "https:/foreign.example/?next=HTTPS://beta.atlassian.net/browse/ABC-12",
        "https://[broken/?next=https://beta.atlassian.net/browse/ABC-12",
        "https://foreign.example/(ABC-12)/[XYZ-7]#ABC-13",
        "https://foreign.example/'ABC-12'\"XYZ-7\"<https://beta.atlassian.net/browse/ABC-13>",
        "https://foreign.example/ABC-12,https://beta.atlassian.net/browse/XYZ-7",
        "//foreign.example/ABC-12?next=https://beta.atlassian.net/browse/XYZ-7",
        "www.foreign.example/ABC-12?next=https://beta.atlassian.net/browse/XYZ-7",
    ] {
        assert!(recognise(&p, url).is_empty(), "{url}");
    }
}

#[test]
fn configured_query_and_fragment_keys_and_urls_only_name_the_path_ticket() {
    let p = demo::sample();
    one(
        &p,
        "https://beta.atlassian.net/browse/ABC-12?next=https://acme.atlassian.net/browse/XYZ-7#ABC-99",
        "beta",
        "ABC-12",
    );
}

#[test]
fn unicode_punctuation_multiple_urls_and_bare_keys_keep_exact_byte_order() {
    let p = demo::sample();
    let first = "https://beta.atlassian.net/browse/ABC-12";
    let second = "https://acme.atlassian.net/browse/ABC-13";
    let text = format!(
        "🔎 XYZ-7 ({first}), 你好 «{second}>!\u{2003}ABC-14 https://foreign.example/ABC-99 XYZ-8"
    );
    let got = recognise(&p, &text);
    let expected = [
        ("XYZ-7", "beta"),
        (first, "beta"),
        (second, "acme"),
        ("ABC-14", "acme"),
        ("XYZ-8", "beta"),
    ];
    assert_eq!(got.len(), expected.len(), "{got:?}");
    let mut previous = 0;
    for ((range, subject), (exact, site)) in got.iter().zip(expected) {
        assert_eq!(&text[range.clone()], exact);
        assert_eq!(range.start, text.find(exact).unwrap());
        assert!(range.start >= previous);
        previous = range.end;
        assert_eq!(subject.field("site"), Some(site));
        assert_eq!(subject_scope::resolve(&p, subject).unwrap().name, site);
        assert!(p.journal_subject(subject).is_some());
    }
}

#[test]
fn clear_outer_wrappers_trim_only_positive_ranges_and_balanced_punctuation_stays() {
    let p = demo::sample();
    for suffix in [
        ".", ",", ";", "!", ")", "]", "}", ">", "\"", "'", "`", ").", "]>;!",
    ] {
        let url = "https://beta.atlassian.net/browse/ABC-12";
        let text = format!("({url}{suffix}");
        let got = recognise(&p, &text);
        assert_eq!(got.len(), 1, "{text}");
        assert_eq!(&text[got[0].0.clone()], url);
        assert_eq!(got[0].1.url.as_deref(), Some(url));
    }
    for tail in [
        "?x=(ABC-13)",
        "?x=[XYZ-7]",
        "#(ABC-13)",
        "?x='ABC-13'",
        "?x=\"ABC-13\"",
        "?x={ABC-13}",
    ] {
        let url = format!("https://beta.atlassian.net/browse/ABC-12{tail}");
        one(&p, &url, "beta", "ABC-12");
    }
}

#[test]
fn configured_ipv6_brackets_and_nondefault_ports_remain_intact() {
    let mut p = demo::sample();
    p.cfg.sites[0].url = "https://[::1]:8443".into();
    one(
        &p,
        "https://[::1]:8443/browse/ABC-12?x=[XYZ-7]",
        "acme",
        "ABC-12",
    );
    assert!(recognise(&p, "https://[::2]:8443/browse/ABC-12").is_empty());
    assert!(recognise(&p, "https://[::1]/browse/ABC-12").is_empty());
}

#[test]
fn masking_does_not_change_bare_key_case_boundaries_or_project_precedence() {
    let p = demo::sample();
    let text = "abc-12 aABC-1 ABC-123x ABC-0 NOPE-7 ABC-12,https://foreign.example/ABC-13 XYZ-7";
    let got = recognise(&p, text);
    assert_eq!(got.len(), 2);
    for ((range, subject), (key, site)) in got.iter().zip([("ABC-12", "acme"), ("XYZ-7", "beta")]) {
        assert_eq!(&text[range.clone()], key);
        assert_eq!(subject.field("site"), Some(site));
    }
}

#[test]
fn long_original_urls_can_navigate_without_weakening_journal_snapshot_limits() {
    let p = demo::sample();
    let url = format!(
        "https://beta.atlassian.net/browse/ABC-12?context={}",
        "x".repeat(8192)
    );
    let got = recognise(&p, &url);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].1.url.as_deref(), Some(url.as_str()));
    assert_eq!(subject_scope::resolve(&p, &got[0].1).unwrap().name, "beta");
    assert!(
        p.journal_subject(&got[0].1).is_none(),
        "existing strict bounded omission"
    );
}

#[test]
fn malformed_prefixes_are_masked_but_only_clear_opening_prose_allows_recognition() {
    let p = demo::sample();
    for prefix in ["123", "prefix", "prefix_", "x=", "ABC-13,"] {
        let text = format!("{prefix}https://beta.atlassian.net/browse/ABC-12");
        let got = recognise(&p, &text);
        assert!(!got.iter().any(|(_, s)| s.id == "ABC-12"), "{text}");
        if prefix == "ABC-13," {
            assert_eq!(got.len(), 1);
            assert_eq!(&text[got[0].0.clone()], "ABC-13");
        } else {
            assert!(got.is_empty(), "{text}");
        }
    }
    for prefix in ["(", "[", "<", "\"", "'", "`", "«", "“", "see:("] {
        let url = "https://beta.atlassian.net/browse/ABC-12";
        let text = format!("{prefix}{url}");
        let got = recognise(&p, &text);
        assert_eq!(got.len(), 1, "{text}");
        assert_eq!(got[0].0, prefix.len()..text.len());
    }
}

#[test]
fn large_tokens_and_wrapper_tails_have_bounded_single_scan_behavior() {
    let p = demo::sample();
    let plain = "a".repeat(100_000);
    assert!(candidates(&plain).is_empty());
    let disguised = format!("{plain}https://beta.atlassian.net/browse/ABC-12");
    let masked = candidates(&disguised);
    assert_eq!(masked.len(), 1);
    assert_eq!(masked[0], 0..disguised.len());
    assert!(recognise(&p, &disguised).is_empty());
    let url = "https://beta.atlassian.net/browse/ABC-12";
    let wrapped = format!("({url}{}", ")".repeat(100_000));
    let got = recognise(&p, &wrapped);
    assert_eq!(got.len(), 1);
    assert_eq!(&wrapped[got[0].0.clone()], url);
    let many = "ABC-12 https://foreign.example/XYZ-7 ".repeat(2_000);
    let got = recognise(&p, &many);
    assert_eq!(got.len(), 2_000);
    assert!(
        got.iter()
            .all(|(range, subject)| &many[range.clone()] == "ABC-12" && subject.id == "ABC-12")
    );
}
