use super::*;

fn descriptor(effect: Effect) -> ActionDescriptor {
    let Effect::Observe { action, outcome } = effect else {
        panic!("independent open observation")
    };
    assert_eq!(outcome, Outcome::Succeeded);
    assert_eq!(action.kind.as_str(), "jira.page.open");
    action
}

fn opens(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| {
            matches!(effect, Effect::Observe { action, outcome: Outcome::Succeeded }
            if action.kind.as_str() == "jira.page.open")
        })
        .count()
}

fn reads(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, Effect::SpawnAction { .. }))
        .count()
}

#[test]
fn open_scope_is_configured_server_and_key_with_explicit_omissions() {
    let one = site();
    let mut two = one.clone();
    two.name = "SECRET_ALIAS".into();
    two.url = "https://other.example".into();
    let a = descriptor(opened(Some(&one), "ABC-12"));
    let b = descriptor(opened(Some(&two), "ABC-12"));
    assert_eq!(a.subjects[0].key.id, b.subjects[0].key.id);
    assert_ne!(a.subjects[0].key.instance, b.subjects[0].key.instance);
    assert_eq!(b.subjects[0].key.instance, "https://other.example/");
    assert_eq!(a.subjects[0].role, SubjectRole::Primary);
    assert!(a.subjects[0].url.is_none());
    assert!(!format!("{b:?}").contains("SECRET_ALIAS"));

    let mut bad = one.clone();
    bad.url = "https://[SECRET".into();
    let long_key = format!("ABC-{}", "1".repeat(2048));
    for (site, key) in [
        (None, "ABC-12"),
        (Some(&bad), "ABC-12"),
        (Some(&one), "SECRET invalid key"),
        (Some(&one), long_key.as_str()),
    ] {
        let action = descriptor(opened(site, key));
        assert!(action.subjects.is_empty());
        assert!(action.metadata.redacted);
        assert_eq!(action.metadata.omitted_subjects, 1);
    }
}

#[test]
fn open_scope_omits_registered_secrets_without_changing_existing_read_scope() {
    let mut configured = site();
    radar_core::log::secret("SECRETOPEN-791");
    radar_core::log::secret("secret-page-open");
    for (url, key) in [
        ("https://jira.example", "SECRETOPEN-791"),
        ("https://secret-page-open.example", "ABC-12"),
    ] {
        configured.url = url.into();
        let open = descriptor(opened(Some(&configured), key));
        assert!(open.subjects.is_empty());
        assert!(open.metadata.redacted);
        assert_eq!(open.metadata.omitted_subjects, 1);
        assert_eq!(
            action(&configured, key).subjects.len(),
            1,
            "the existing read-history scope is unchanged"
        );
    }
}

#[test]
fn actual_pushes_record_once_even_when_fetch_is_suppressed_or_scope_is_missing() {
    let mut p = demo::sample();
    let first = page::open(&mut p, "acme", "ABC-12");
    assert_eq!((p.pages.len(), opens(&first), reads(&first)), (1, 1, 1));
    assert!(matches!(first.first(), Some(Effect::Observe { .. })));
    assert!(matches!(
        first.get(1),
        Some(Effect::Focus(radar_core::Pane::Middle))
    ));
    let owner = p.details[&detail_key("acme", "ABC-12")].request;

    let pending = page::open(&mut p, "acme", "ABC-12");
    assert_eq!((p.pages.len(), opens(&pending), reads(&pending)), (2, 1, 0));
    assert_eq!(p.details[&detail_key("acme", "ABC-12")].request, owner);
    crate::tests::complete_detail(&mut p, "acme", "ABC-12", Ok(crate::tests::detail()));
    let before = p.snapshot();
    let fresh = page::open(&mut p, "acme", "ABC-12");
    assert_eq!((p.pages.len(), opens(&fresh), reads(&fresh)), (3, 1, 0));
    assert_eq!(p.snapshot(), before);

    let missing = page::open(&mut p, "SECRET_MISSING_ALIAS", "ABC-12");
    assert_eq!((p.pages.len(), opens(&missing), reads(&missing)), (4, 1, 0));
    let action = descriptor(missing.into_iter().next().unwrap());
    assert!(action.subjects.is_empty());
    assert_eq!(action.metadata.omitted_subjects, 1);
    assert_eq!(p.pages.last().unwrap().site, "SECRET_MISSING_ALIAS");
    assert!(
        !p.details
            .contains_key(&detail_key("SECRET_MISSING_ALIAS", "ABC-12"))
    );
}

#[test]
fn non_open_navigation_and_rejected_show_record_no_transition() {
    let mut p = demo::sample();
    p.tree.borrow_mut().select(vec![]);
    assert_eq!(opens(&page::open_from_tree(&mut p)), 0);
    p.tree
        .borrow_mut()
        .select(vec![crate::screen::site_id("acme")]);
    assert_eq!(opens(&page::open_from_tree(&mut p)), 0);
    assert!(p.pages.is_empty());
    assert!(matches!(
        crate::join::show(
            &mut p,
            &radar_core::Subject::new("ticket", "SECRET_INVALID", "SECRET_LABEL")
        ),
        radar_core::Showing::Cannot
    ));
    assert!(p.pages.is_empty());

    page::open(&mut p, "acme", "ABC-12");
    for code in [KeyCode::Enter, KeyCode::Char('r'), KeyCode::Esc] {
        let radar_core::plugin::Outcome::Handled(fx) =
            page::key(&mut p, KeyEvent::new(code, KeyModifiers::NONE))
        else {
            panic!("page key handled")
        };
        assert_eq!(opens(&fx), 0);
    }
    assert!(p.pages.is_empty());
}
