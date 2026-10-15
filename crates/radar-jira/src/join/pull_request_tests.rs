use super::*;

const URL: &str = "https://code.example/acme/api/pull/7";

fn pull(title: &str, branch: &str) -> Subject {
    Subject::new(PULL_REQUEST, URL, "api #7")
        .with_url(URL)
        .with("title", title)
        .with("branch", branch)
}

fn code(p: &mut JiraPlugin, site: &str, key: &str, url: &str) {
    p.data.get_mut(site).unwrap().code.insert(
        key.into(),
        crate::CodeEntry {
            at: p.now,
            updated: String::new(),
            links: vec![crate::model::CodeLink {
                id: "#7".into(),
                title: "A change".into(),
                url: url.into(),
                status: "open".into(),
                repo: "api".into(),
                branch: "tidy".into(),
            }],
        },
    );
}

fn actions(p: &JiraPlugin, s: &Subject) -> Vec<Action> {
    let Some(Contribution::Actions(actions)) = contribute(p, Point::Actions, s) else {
        return vec![];
    };
    actions
}

#[test]
fn pull_request_tickets_fill_read_only_places_and_open_exact_site() {
    let mut p = crate::tests::sample();
    let s = pull("ABC-12: Add stats", "feature/def-3-rename");
    assert_eq!(
        related(&p, &s)
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["ABC-12", "DEF-3"]
    );
    let Some(Contribution::Cell(cell)) = contribute(&p, Point::Column, &s) else {
        panic!()
    };
    assert_eq!(cell.text, "ABC-12 · In Progress +1");
    assert_eq!(cell.show.unwrap().field("site"), Some("acme"));
    assert!(matches!(
        contribute(&p, Point::Section, &s),
        Some(Contribution::Section { .. })
    ));
    let Some(Contribution::Tab { rows, .. }) = contribute(&p, Point::Tab, &s) else {
        panic!()
    };
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|r| r.subject.as_ref().unwrap().field("site") == Some("acme"))
    );
    let actions = actions(&p, &s);
    assert_eq!(
        actions.iter().map(|a| a.what.as_str()).collect::<Vec<_>>(),
        ["open ABC-12", "open DEF-3"]
    );
    assert_eq!(
        actions.iter().map(|a| a.key).collect::<Vec<_>>(),
        [Some('v'), None]
    );
    assert!(actions.iter().all(|a| a.change.is_none()));
    assert!(
        matches!(&act(&mut p, &s, &actions[0].id)[..], [Effect::Show(ticket)]
        if ticket.id == "ABC-12" && ticket.field("site") == Some("acme"))
    );
    for action in ["move\0ABC-12", "open\0ABC-12", "unknown"] {
        assert!(act(&mut p, &s, action).is_empty());
    }
}

#[test]
fn pull_request_development_links_match_exact_urls_without_path_aliases() {
    let mut p = crate::tests::sample();
    let s = pull("Tidy", "tidy");
    assert!(related(&p, &s).is_empty());
    for url in [
        "https://other.example/acme/api/pull/7",
        "https://code.example/acme/api/pull/70",
        "https://code.example/acme/api/pull/7/",
        "https://code.example/acme/api/pull/7?context=1",
    ] {
        code(&mut p, "acme", "ABC-12", url);
        assert!(related(&p, &s).is_empty(), "{url}");
    }
    code(&mut p, "acme", "ABC-12", URL);
    assert_eq!(related(&p, &s)[0].id, "ABC-12");
    assert!(actions(&p, &s).iter().all(|a| a.what == "open ABC-12"));
}

#[test]
fn pull_request_page_and_background_evidence_deduplicate_by_site_and_key() {
    let mut p = crate::tests::sample();
    let mut detail = crate::tests::detail();
    detail.code[0].url = URL.into();
    crate::tests::complete_detail(&mut p, "acme", "ABC-12", Ok(detail));
    let s = pull("ABC-12 ABC-12", "abc-12-stats");
    assert_eq!(related(&p, &s).len(), 1);
    assert_eq!(actions(&p, &s).len(), 1);
}

#[test]
fn pull_request_ambiguous_keys_are_omitted_without_changing_legacy_precedence() {
    let mut p = crate::tests::sample();
    let project = p.cfg.sites[0].projects[0].clone();
    p.cfg.sites[1].projects.push(project);
    let s = pull("ABC-12", "abc-12");
    assert!(related(&p, &s).is_empty());
    assert!(contribute(&p, Point::Section, &s).is_none());
    assert!(actions(&p, &s).is_empty());
    let mut legacy = s;
    legacy.kind = MERGE_REQUEST.into();
    assert_eq!(related(&p, &legacy)[0].field("site"), Some("acme"));
    assert!(
        actions(&p, &legacy)
            .iter()
            .any(|a| a.what.starts_with("move "))
    );
}

#[test]
fn pull_request_explicit_link_does_not_override_another_sites_held_same_key() {
    let mut p = crate::tests::sample();
    let ticket = p.data["acme"].tickets["ABC-12"].clone();
    p.data
        .get_mut("beta")
        .unwrap()
        .tickets
        .insert(ticket.key.clone(), ticket);
    code(&mut p, "acme", "ABC-12", URL);
    assert!(related(&p, &pull("Tidy", "tidy")).is_empty());
    code(&mut p, "beta", "ABC-12", URL);
    assert!(related(&p, &pull("Tidy", "tidy")).is_empty());
}

#[test]
fn pull_request_open_actions_cannot_retarget_after_site_or_relationship_changes() {
    let s = pull("ABC-12", "tidy");
    for change in ["url", "alias", "ambiguous", "removed"] {
        let mut p = crate::tests::sample();
        let action = actions(&p, &s).remove(0);
        match change {
            "url" => p.cfg.sites[0].url = "https://replacement.example".into(),
            "alias" => p.cfg.sites[0].name = "replacement".into(),
            "ambiguous" => {
                let project = p.cfg.sites[0].projects[0].clone();
                p.cfg.sites[1].projects.push(project);
            }
            "removed" => {
                p.cfg.sites.remove(0);
            }
            _ => unreachable!(),
        }
        assert!(act(&mut p, &s, &action.id).is_empty(), "{change}");
    }
    let mut p = crate::tests::sample();
    let action = actions(&p, &s).remove(0);
    assert!(act(&mut p, &pull("No key", "tidy"), &action.id).is_empty());
}

#[test]
fn pull_request_missing_matches_and_other_kinds_add_nothing() {
    let p = crate::tests::sample();
    for s in [
        pull("NOPE-12 and aABC-12 and ABC-12x", "tidy"),
        Subject::new("repository", "https://code.example/acme/api", "api").with("title", "ABC-12"),
    ] {
        assert!(related(&p, &s).is_empty());
        assert!(contribute(&p, Point::Actions, &s).is_none());
    }
}
