use super::*;

#[test]
fn missing_facts_are_omitted_but_zero_and_unicode_survive() {
    let mut header = DetailHeader::new("watch / stores", "Inventory");
    header.author = Some(Text::plain(" \t "));
    let content = DetailContent::new(header, vec![
        DetailFact::new("missing", " \n"), DetailFact::new("count", "0"),
        DetailFact::new("場所", "橋"), DetailFact::new("accent", "e\u{301}"),
    ], vec![]);
    assert_eq!(content.facts.len(), 3);
    assert_eq!(content.facts[0].value.words(), "0");
    assert_eq!(content.header.author, None);
}

#[test]
fn compact_caption_recomputes_metadata_without_changing_authority() {
    let original = ActionSpec::named("watch.edit", &crate::Binding::new("E", "Edit inventory"));
    let action = FootAction::new(original.clone()).caption("edit").disabled("waiting");
    assert_eq!(action.action.id, original.id);
    assert_eq!(action.action.key, original.key);
    assert_eq!(action.action.change, original.change);
    assert_eq!(action.action.mnemonic_at, Some(0));
    assert!(action.action.mnemonic_shift);
    assert_eq!(action.target, None);
    let fallback = FootAction::new(original).caption("write");
    assert_eq!(fallback.action.mnemonic_at, Some(4));
}

#[test]
fn complete_fallback_is_readable_and_has_no_action_targets() {
    let mut header = DetailHeader::new("watch / stores", "Recount supplies");
    header.branch = Some("stock".into());
    header.author = Some("Sam Vimes".into());
    header.age = Some("2d".into());
    let section = Block::section(None, "description", None, vec![Block::line("Keep every detail")]);
    let mut content = DetailContent::new(header, vec![DetailFact::new("count", "0")], vec![section.clone()]);
    content.pictures = Some("watch-images".into());
    let foot = FootActions::new(vec![FootAction::new(ActionSpec::named("watch.edit", &crate::Binding::new("e", "edit"))).disabled("waiting")]);
    let node = content.node(&foot);
    assert_eq!(DetailContent::of(&node), Some(content.clone()));
    let Node::Extension { fallback, .. } = &node else { panic!("typed detail") };
    let Node::Rich { blocks, pictures } = fallback.as_ref() else { panic!("ordinary rich fallback") };
    assert_eq!(pictures, &content.pictures);
    assert!(blocks.contains(&section));
    for words in ["watch / stores", "Recount supplies", "stock", "Sam Vimes", "2d", "count: 0"] {
        assert!(blocks.iter().any(|block| matches!(block, Block::Line { text, .. } if text.words() == words)), "{words}");
    }
    let Some(Block::Section { blocks: actions, .. }) = blocks.last() else { panic!("actions fallback") };
    assert!(matches!(&actions[0], Block::Line { text, target: None, key: Some(key), .. } if text.words() == "edit (waiting)" && key == "e"));
    assert!(matches!(&actions[1], Block::Line { target: None, key: Some(key), .. } if key == "x"));
    let pane = pane("detail", content, foot);
    assert!(pane.foot_actions.as_ref().unwrap().actions.iter().all(|action| action.target.is_none()));
}

#[test]
fn unknown_and_malformed_extensions_are_not_decoded_as_detail() {
    let node = DetailContent::new(DetailHeader::new("watch", "Inventory"), vec![], vec![]).node(&FootActions::new(vec![]));
    for (version, data) in [(2, serde_json::json!({"header": {"path": [], "title": []}})), (1, serde_json::json!({"header": null}))] {
        let Node::Extension { fallback, .. } = node.clone() else { unreachable!() };
        let unknown = Node::Extension { capability: CAPABILITY.into(), version, data, fallback };
        assert_eq!(DetailContent::of(&unknown), None);
    }
}

#[test]
fn footer_omits_heading_and_empty_authority_and_owns_its_menu() {
    let heading = ActionSpec::named("ignored", &crate::Binding::heading("Read"));
    let empty = ActionSpec::named("", &crate::Binding::new("e", "edit"));
    let foot = FootActions::new(vec![FootAction::new(heading), FootAction::new(empty)]);
    assert!(foot.actions.is_empty());
    assert_eq!(foot.menu.action.id, MENU_ID);
    assert_eq!(foot.menu.action.key.unwrap().name(), "x");
    assert_eq!(foot.menu.action.mnemonic_at, Some(1));
}
