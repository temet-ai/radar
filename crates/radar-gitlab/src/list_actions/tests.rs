//! A merge request row's actions from the tree. Nothing here reaches GitLab:
//! a question is answered with the menu's value, and the task a yes returns
//! is dropped without being run.

use radar_core::Plugin;
use radar_core::host::Event;
use radar_core::plugin::{Effect, Menu, Outcome, Pane};
use radar_core::view::{Chord, Key};
use radar_core::Host;

use super::*;
use crate::model::fixtures::mr;
use crate::GroupData;
use crate::model::Pipeline;
use crate::tests::sample;

const API: &str = "acme/team-a/api";
const WEB: &str = "acme/team-a/web";

/// The sample, with !410 behind its target (GitLab says it needs a rebase,
/// the rest is in order) and !411 with a red pipeline.
fn plugin() -> GitlabPlugin {
    let mut p = sample();
    let mut behind = mr(410, "me", API);
    behind.title = "ABC-410: Page the stats".into();
    behind.merge_status = "NEED_REBASE".into();
    behind.rebase = true;
    behind.approved = true;
    behind.approvals_left = 0;
    behind.updated_at = "2026-10-06T09:00:00Z".into();
    let mut red = mr(411, "me", WEB);
    red.title = "ABC-411: Retry the sync".into();
    red.approved = true;
    red.approvals_left = 0;
    red.merge_status = "CI_MUST_PASS".into();
    red.pipeline = Some(Pipeline::Failed);
    red.pipeline_url = Some("https://gitlab.com/acme/team-a/web/-/pipelines/7001".into());
    red.updated_at = "2026-10-06T08:00:00Z".into();
    let data: &mut GroupData = p.data.get_mut("acme/team-a").unwrap();
    data.mrs.push(behind);
    data.mrs.push(red);
    p
}

#[test]
fn repo_rows_show_the_default_branch_pipeline_without_repeating_its_name() {
    let p = plugin();
    let text = draw(&p, 160, 40);
    let row = |name: &str| {
        text.lines()
            .find(|line| line.contains(&format!(":repo: {name} ")))
            .unwrap_or_else(|| panic!("missing {name} in\n{text}"))
            .to_string()
    };
    assert!(row("api").contains(":run_passed: passed"), "{text}");
    assert!(row("web").contains(":run_failed: failed"), "{text}");
    assert!(!row("worker").contains("pipeline"), "{text}");
    assert!(!text.contains("main :run_passed: passed"), "{text}");

    let mut p = plugin();
    p.data
        .get_mut("acme/team-a")
        .unwrap()
        .projects
        .iter_mut()
        .find(|project| project.name == "docs")
        .unwrap()
        .main_pipeline = None;
    let text = draw(&p, 160, 40);
    let docs = text.lines().find(|line| line.contains(":repo: docs ")).unwrap();
    assert!(docs.contains("no recent run"), "{text}");
}

fn key_of(code: Key) -> Chord {
    Chord::new(code)
}

fn press_in(p: &mut GitlabPlugin, pane: Pane, code: Key) -> Vec<Effect> {
    match p.key("repos", pane, key_of(code)) {
        Outcome::Handled(fx) => fx,
        Outcome::Ignored => vec![],
    }
}

fn press(p: &mut GitlabPlugin, c: char) -> Vec<Effect> {
    press_in(p, Pane::Middle, Key::Char(c))
}

/// The screen body, as view text.
fn draw(p: &GitlabPlugin, w: u16, h: u16) -> String {
    crate::tests::view_text(p, "repos", w, h)
}

/// The path of the tree row `id`, among `rows` and their children.
fn path_to(rows: &[radar_core::view::TreeRow], id: &str) -> Option<Vec<String>> {
    rows.iter().find_map(|row| {
        if row.id == id {
            return Some(vec![row.id.clone()]);
        }
        let mut path = path_to(&row.children, id)?;
        path.insert(0, row.id.clone());
        Some(path)
    })
}

/// Select the merge request in the tree, as a click on its row does.
fn select(p: &mut GitlabPlugin, iid: u64) {
    p.view = 0;
    let vcx = radar_core::view::ViewCx {
        now: p.now,
        focus: radar_core::view::PaneId::Middle,
        width: 160,
        height: 40,
        parts: radar_core::Parts::none(),
        icon_lead: 0,
        setup: false,
    };
    let view = Plugin::view(p, "repos", &vcx).expect("the screen has a view");
    let pane = view.pane(radar_core::view::PaneId::Middle).expect("the middle pane");
    let radar_core::view::Node::Tree(tree) = &pane.body else { panic!("the tree") };
    let id = format!("m:{}!{iid}", p.data.values().flat_map(|d| d.mrs.iter()).find(|m| m.iid == iid).unwrap().project_path);
    let path = path_to(&tree.rows, &id).unwrap_or_else(|| panic!("never got to !{iid}"));
    let target = radar_core::view::TreeRow::target_of(&path);
    Plugin::intent(p, "repos", Pane::Middle, &radar_core::view::Intent::Select { target });
    assert!(selected(p).is_some_and(|m| m.iid == iid), "never got to !{iid}");
}

fn menu_of(fx: &[Effect]) -> &Menu {
    let mut visible = fx.iter().filter(|e| !matches!(e, Effect::Journal(_) | Effect::Report(_)));
    match (visible.next(), visible.next()) {
        (Some(Effect::Menu(m)), None) => m,
        other => panic!("not one menu: {other:?}"),
    }
}

fn notice(fx: &[Effect]) -> &str {
    let mut visible = fx.iter().filter(|e| !matches!(e, Effect::Journal(_) | Effect::Report(_)));
    match (visible.next(), visible.next()) {
        (Some(Effect::Notify(m)), None) => m,
        other => panic!("not one notice: {other:?}"),
    }
}

fn error_notice(fx: &[Effect]) -> &str {
    let mut visible = fx.iter().filter(|e| !matches!(e, Effect::Journal(_) | Effect::Report(_)));
    match (visible.next(), visible.next()) {
        (Some(Effect::Notice(message)), None)
            if message.tone == radar_core::view::chrome::NoticeTone::Error => &message.text,
        other => panic!("not one error notice: {other:?}"),
    }
}

fn sends(fx: &[Effect]) -> bool {
    fx.iter().any(|e| matches!(e, Effect::Spawn(_)))
}

#[test]
fn u_asks_to_rebase_a_merge_request_gitlab_says_is_behind_and_sends_it_once_on_a_yes() {
    let mut p = plugin();
    select(&mut p, 410);
    let fx = press(&mut p, 'u');
    let m = menu_of(&fx).clone();
    assert_eq!(m.id, "act-rebase:acme/team-a/api!410");
    assert_eq!(m.title, "rebase feature/ABC-410 onto main?");
    assert!(!p.acting.contains_key("acme/team-a/api!410"), "nothing asked of GitLab yet");
    // No: nothing happens.
    assert!(matches!(&p.on_menu(&m.id, "no")[..], [Effect::Report(radar_core::journal::Outcome::Cancelled)]));
    assert!(p.acting.is_empty());
    // Yes: one task, "asked of GitLab…" under way, and a second ask refuses.
    let fx = p.on_menu(&m.id, "yes");
    assert!(matches!(&fx[..], [Effect::Report(_), Effect::Report(_), Effect::Spawn(_), Effect::Notify(m)] if m == "asking GitLab to rebase !410…"), "{fx:?}");
    assert!(mr_actions::running(&p));
    let again = p.on_menu(&m.id, "yes");
    assert_eq!(notice(&again), "already asked GitLab for a rebase");
    let row = rows(&p, &selected(&p).unwrap()).remove(0);
    assert_eq!(row.words(), "update by rebasing · asked, waiting for GitLab");
    // The pinned caption stays compact; the original action and reason survive.
    let cx = radar_core::view::ViewCx {
        now: p.now,
        focus: radar_core::view::PaneId::Detail,
        width: 160,
        height: 40,
        parts: radar_core::Parts::none(),
        icon_lead: 0,
        setup: false,
    };
    let view = p.view("repos", &cx).unwrap();
    let detail = view.pane(radar_core::view::PaneId::Detail).unwrap();
    let update = detail.foot_actions.as_ref().unwrap().actions.iter()
        .find(|control| control.action.id == "gitlab.repos.rebase").unwrap();
    let original = crate::actions(&p.rows(Pane::Detail)).into_iter()
        .find(|action| action.id == "gitlab.repos.rebase").unwrap();
    assert_eq!(update.action.id, original.id);
    assert_eq!(update.action.key, original.key);
    assert_eq!(update.action.key.unwrap().name(), "u");
    assert_eq!(update.action.change, original.change);
    assert_eq!(update.action.change, Some(radar_core::view::Change::Edit));
    assert_eq!(update.action.label, "update");
    assert_eq!(update.disabled.as_deref(), Some("asked, waiting for GitLab"));
    assert!(update.target.is_none(), "an in-flight control has no hit target");
    let radar_core::view::Node::Extension { fallback, .. } = &detail.body else {
        panic!("the shared detail has a readable fallback")
    };
    let radar_core::view::Node::Rich { blocks, .. } = fallback.as_ref() else {
        panic!("the shared detail fallback is rich text")
    };
    let actions = blocks.iter().find_map(|block| match block {
        radar_core::view::Block::Section { title, blocks, .. } if title == "ACTIONS" => Some(blocks),
        _ => None,
    }).unwrap();
    assert!(actions.iter().any(|block| matches!(block,
        radar_core::view::Block::Line { text, key, target, .. }
            if text.words() == "update (asked, waiting for GitLab)"
                && key.as_deref() == Some("u") && target.is_none()
    )), "fallback retains the disabled reason without an activation target");
    assert!(p.keys("repos", Pane::Detail).iter().any(|binding|
        binding.keys == "u" && binding.what == "update by rebasing · asked, waiting for GitLab"
    ));
    // GitLab answers: the answer is shown and the tree is refreshed.
    let done = mr_actions::ActMsg { connection: p.connection_stamp(), key: "acme/team-a/api!410".into(), what: "rebase", result: Ok("rebase requested".into()), outcome: radar_core::journal::Outcome::Succeeded };
    let fx = p.on_msg(Box::new(done));
    assert!(fx.iter().any(|e| matches!(e, Effect::Notify(m) if m == "✓ rebase requested")), "{fx:?}");
    assert!(
        fx.iter().any(|e| matches!(e, Effect::SpawnAction { action, .. }
            if action.kind.as_str().starts_with("gitlab.sync."))),
        "the tree is fetched again: {fx:?}"
    );
    assert!(
        fx.iter().any(|e| matches!(e, Effect::SpawnAction { action, .. }
            if action.kind.as_str() == "gitlab.page.detail")),
        "the detail is refreshed independently: {fx:?}"
    );
}

#[test]
fn read_only_or_write_mode_a_yes_sends_nothing_and_says_so() {
    for mode in [radar_core::writes::Mode::ReadOnly, radar_core::writes::Mode::Write] {
        radar_core::writes::set_mode(mode, None);
        let why = match mode {
            radar_core::writes::Mode::ReadOnly => "not sent: radar is read only",
            _ => "not sent: radar is in write mode; this needs edit mode",
        };
        let mut p = plugin();
        select(&mut p, 410);
        let fx = press(&mut p, 'u');
        // The kit's question, as on the page; it edits (the host adds the
        // mode's note before y).
        let m = menu_of(&fx).clone();
        assert_eq!(m.change, Some(radar_core::writes::Change::Edit));
        assert_eq!(
            m.items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            ["GitLab rewrites the branch on top of main", "yes", "no"]
        );
        let fx = p.on_menu(&m.id, "yes");
        assert_eq!(error_notice(&fx), format!("✗ GitLab did not rebase !410: {why}"));
        // The merge.
        select(&mut p, 187);
        let fx = press(&mut p, 'm');
        let m = menu_of(&fx).clone();
        assert_eq!(m.change, Some(radar_core::writes::Change::Edit));
        let fx = p.on_menu(&m.id, "yes:1:1");
        assert!(!sends(&fx), "{fx:?}");
        // And approving.
        select(&mut p, 263);
        let fx = press(&mut p, 'e');
        let m = menu_of(&fx).clone();
        assert_eq!(m.change, Some(radar_core::writes::Change::Edit));
        let fx = p.on_menu(&m.id, "yes");
        assert!(!sends(&fx), "{fx:?}");
        assert_eq!(error_notice(&fx), format!("✗ GitLab did not approve !263: {why}"));
        // The row menu lists them, marked as writes that edit.
        let b = crate::bindings_of(bindings(&p, &crate::pages::find_mr(&p, "acme/team-a/api", 263).unwrap().clone()));
        assert!(b.iter().filter(|r| r.keys == "e").all(|r| r.change == Some(radar_core::writes::Change::Edit)));
        assert!(b.iter().filter(|r| r.keys == "c").all(|r| r.change.is_none()), "a checkout is this machine's");
    }
}

#[test]
fn u_says_why_when_gitlab_does_not_see_the_merge_request_behind() {
    let mut p = plugin();
    select(&mut p, 263);
    assert_eq!(notice(&press(&mut p, 'u')), "cannot rebase !263: not behind main");
    select(&mut p, 342);
    assert_eq!(notice(&press(&mut p, 'u')), "cannot rebase !342: not behind main");
    // A conflict is not for a rebase to fix.
    let mut conflicted = mr(412, "me", API);
    conflicted.merge_status = "CONFLICT".into();
    conflicted.conflicts = true;
    assert_eq!(
        mr_actions::rebase_blocker(&conflicted).as_deref(),
        Some("it has conflicts, which a rebase cannot fix")
    );
    // Nothing is asked of GitLab on any of these.
    assert!(p.acting.is_empty());
}

#[test]
fn m_asks_to_merge_only_what_gitlab_would_take_and_says_why_not_for_the_rest() {
    let mut p = plugin();
    select(&mut p, 187);
    let fx = press(&mut p, 'm');
    let m = menu_of(&fx).clone();
    assert_eq!(m.id, "act-merge:acme/team-a/web!187");
    assert_eq!(m.title, "merge !187 into main? It cannot be undone.");
    assert!(m.items.iter().any(|i| i.label == "yes"));
    assert!(matches!(&p.on_menu(&m.id, "no")[..], [Effect::Report(radar_core::journal::Outcome::Cancelled)]));
    let fx = p.on_menu(&m.id, "yes");
    assert!(matches!(&fx[..], [Effect::Report(_), Effect::Report(_), Effect::Spawn(_), Effect::Notify(m)] if m == "asking GitLab to merge !187…"), "{fx:?}");
    for (iid, why) in [
        (263, "needs 1 approval"),
        (342, "draft"),
        (410, "needs rebase"),
        (411, "pipeline failed"),
    ] {
        select(&mut p, iid);
        assert_eq!(notice(&press(&mut p, 'm')), format!("cannot merge !{iid} yet: {why}"));
    }
}

#[test]
fn e_approves_or_takes_the_approval_back_asking_first() {
    let mut p = plugin();
    select(&mut p, 263);
    let fx = press(&mut p, 'e');
    let m = menu_of(&fx).clone();
    assert_eq!(m.id, "gl-write");
    assert_eq!(m.title, "approve !263?");
    assert!(!sends(&p.on_menu(&m.id, "no")));
    assert!(p.acting.is_empty(), "nothing asked of GitLab");
    let fx = press(&mut p, 'e');
    let fx = p.on_menu(&menu_of(&fx).id.clone(), "yes");
    assert!(matches!(&fx[..], [Effect::Report(_), Effect::Report(_), Effect::Notify(m), Effect::Spawn(_)] if m == "asking GitLab to approve !263…"), "{fx:?}");
    // Yours already: the question takes it back.
    let mut p = plugin();
    p.data.get_mut("acme/team-a").unwrap().mrs.iter_mut().find(|m| m.iid == 263).unwrap().approved_by = vec!["me".into()];
    select(&mut p, 263);
    let fx = press(&mut p, 'e');
    assert_eq!(menu_of(&fx).title, "take back your approval of !263?");
    // One that ended cannot be approved.
    let mut p = plugin();
    p.view = 0;
    p.filter = "state:merged".into();
    select(&mut p, 150);
    assert_eq!(notice(&press(&mut p, 'e')), "!150 is merged already");
}


#[test]
fn the_keys_belong_to_a_merge_request_row_in_the_tree_or_its_preview_and_nowhere_else() {
    let mut p = plugin();
    // On a repo and on a tenancy: not theirs.
    p.view = 0;
    let _ = draw(&p, 160, 40);
    for c in ['u', 'm', 'e', 'c'] {
        assert!(press(&mut p, c).is_empty(), "{c} on a tenancy");
    }
    select(&mut p, 410);
    // In the detail pane too, but not on the nav pane.
    assert!(matches!(press_in(&mut p, Pane::Detail, Key::Char('u'))[..], [Effect::Journal(_), Effect::Menu(_)]));
    assert!(press_in(&mut p, Pane::Nav, Key::Char('u')).is_empty());
    // With a modifier it is not the key.
    let ctrl = Chord::ctrl('u');
    assert!(matches!(p.key("repos", Pane::Middle, ctrl), Outcome::Ignored));
    // `a` is still the authors.
    assert!(matches!(press(&mut p, 'a')[..], [Effect::Menu(_)]));
    assert_eq!(press(&mut p, 'a').len(), 1);
}

#[test]
fn the_actions_menu_of_a_row_lists_them_with_why_for_the_ones_not_offered() {
    let mut p = plugin();
    select(&mut p, 410);
    let b = p.keys("repos", Pane::Middle);
    let what = |k: &str| b.iter().find(|b| b.keys == k).map(|b| b.what.clone());
    assert_eq!(what("u").as_deref(), Some("update by rebasing onto main…"));
    assert_eq!(what("m").as_deref(), Some("merge · needs rebase"));
    assert_eq!(what("e").as_deref(), Some("approve !410…"));
    assert_eq!(
        what("a").as_deref(),
        Some("show one author's merge requests…"),
        "`a` stays the tree's authors, not approve"
    );
    let heads: Vec<&str> = b.iter().filter(|b| b.is_heading()).map(|b| b.what.as_str()).collect();
    assert_eq!(heads, ["api !410", "on GitLab", "with an agent"]);
    // Pressing a row presses its key.
    for r in &b {
        if matches!(r.keys.as_str(), "u" | "m" | "e") {
            assert!(radar_core::plugin::parse_key(&r.keys).is_some(), "{}", r.keys);
        }
    }
    select(&mut p, 263);
    let b = p.keys("repos", Pane::Middle);
    let what = |k: &str| b.iter().find(|b| b.keys == k).map(|b| b.what.clone());
    assert_eq!(what("u").as_deref(), Some("update by rebasing · not behind main"));
    assert_eq!(what("m").as_deref(), Some("merge · needs 1 approval"));
    // The detail pane's menu has them too.
    let b = p.keys("repos", Pane::Detail);
    assert!(b.iter().any(|b| b.keys == "u"));
    // A repo has none.
    p.tree.borrow_mut().select(vec!["t:team-a".into()]);
    assert!(!p.keys("repos", Pane::Middle).iter().any(|b| b.keys == "u"));
}

#[test]
fn a_merge_request_that_ended_offers_none_of_the_writes_and_says_so() {
    let mut p = plugin();
    p.filter = "state:merged".into();
    select(&mut p, 150);
    let m = selected(&p).unwrap();
    let rows = rows(&p, &m);
    for r in &rows[..3] {
        assert_eq!(r.why.as_deref(), Some("it is merged already"), "{r:?}");
    }
    assert_eq!(rows[3].why, None, "its branch can still be checked out");
    assert!(hints(&p, Pane::Middle).is_empty());
}

#[test]
fn the_status_line_names_only_the_main_actions_that_apply() {
    let mut p = plugin();
    let hint = |p: &mut GitlabPlugin, iid| {
        select(p, iid);
        hints(p, Pane::Middle).iter().map(|h| {
            assert!(radar_core::mnemonic::at(h.keys, h.action).is_some());
            format!("{} {}", h.keys, h.action)
        }).collect::<Vec<_>>()
    };
    assert_eq!(hint(&mut p, 410), ["u update by rebasing"]);
    assert_eq!(hint(&mut p, 187), ["m merge"]);
    assert_eq!(hint(&mut p, 263), ["e approve"]);
    assert!(hint(&mut p, 342).is_empty(), "a draft: nothing to offer");
    assert!(hint(&mut p, 411).is_empty(), "red: nothing in the way for these keys");
    // On the nav pane or a tenancy: none. Approved by you: not offered again.
    assert!(hints(&p, Pane::Nav).is_empty());
    p.tree.borrow_mut().select(vec!["t:team-a".into()]);
    assert!(hints(&p, Pane::Middle).is_empty());
    p.data.get_mut("acme/team-a").unwrap().mrs.iter_mut().find(|m| m.iid == 263).unwrap().approved_by = vec!["me".into()];
    assert!(hint(&mut p, 263).is_empty());
}

fn host_of(p: GitlabPlugin, config: &str) -> Host {
    let mut host = Host::new(vec![Box::new(sample())], p.now).unwrap();
    host.load_config(Some(config)).unwrap();
    host.plugins[0] = Box::new(p);
    host
}

fn shown(p: GitlabPlugin, w: u16, h: u16) -> String {
    crate::tests::shown(&host_of(p, ""), w, h)
}

#[test]
fn preview_actions_use_the_same_surface_as_the_other_details() {
    let mut p = plugin();
    select(&mut p, 410);
    let mut host = host_of(p, "");
    host.focus = Pane::Detail;
    // The pinned footer is the kit's: the part's actions, then the shared
    // `extra actions`; the state context shows above it. How they are
    // surfaced (colours, the underlined x) is the renderer's.
    let text = crate::tests::shown(&host, 160, 40);
    let (_, detail) = text.split_once("── detail").expect("the detail pane");
    let foot = detail.lines().find(|line| line.contains("[extra actions]")).expect("pinned footer");
    assert!(foot.starts_with("[update]"), "{foot}");
    let context = detail.lines().position(|line| line.contains(":rebase: needs rebase")).expect("detail state context");
    let at = detail.lines().position(|line| line.contains("[extra actions]")).unwrap();
    assert!(context < at, "{text}");
}

#[test]
fn the_status_line_of_a_row_that_needs_a_rebase_shows_its_key() {
    let mut p = plugin();
    select(&mut p, 410);
    let text = shown(p, 160, 40);
    let (_, bar) = text.split_once("── status").expect("the status line");
    assert!(bar.contains("u update by rebasing"), "{bar}");
    insta::assert_snapshot!(text);
}

#[test]
fn the_preview_lists_a_merge_request_that_needs_a_rebase() {
    let mut p = plugin();
    select(&mut p, 410);
    insta::assert_snapshot!(shown(p, 160, 40));
}

#[test]
fn the_preview_lists_a_merge_request_ready_to_merge() {
    let mut p = plugin();
    select(&mut p, 187);
    insta::assert_snapshot!(shown(p, 160, 40));
}

#[test]
fn the_preview_lists_a_merge_request_that_needs_approval_with_why_the_others_are_muted() {
    let mut p = plugin();
    select(&mut p, 263);
    insta::assert_snapshot!(shown(p, 160, 40));
}

#[test]
fn the_actions_menu_of_a_row() {
    let mut p = plugin();
    select(&mut p, 410);
    let mut host = host_of(p, "");
    let _ = crate::tests::shown(&host, 160, 40);
    host.update(crate::key_event(key_of(Key::Char('x'))));
    insta::assert_snapshot!(crate::tests::shown(&host, 160, 40));
}

/// The preview's pinned footer as the host displays it: each control with
/// the target a press on it sends, none for one that is disabled.
fn foot_controls(host: &Host) -> Vec<radar_core::view::detail::FootAction> {
    use radar_core::view::PaneId;
    let view = host.displayed_screen(160, 40).expect("the screen");
    let detail = view.pane(PaneId::Detail).expect("the detail pane");
    let foot = detail.foot_actions.as_ref().expect("the pinned footer");
    foot.actions.iter().cloned().chain([foot.menu.clone()]).collect()
}

fn press_foot(host: &mut Host, label: &str) -> Vec<radar_core::host::HostEffect> {
    let control = foot_controls(host)
        .into_iter()
        .find(|c| c.action.label == label)
        .unwrap_or_else(|| panic!("no {label:?} in the footer"));
    let Some(target) = control.target else { return vec![] };
    host.update(Event::Intent(radar_core::view::Intent::Activate { target }))
}

#[test]
fn a_press_on_a_line_of_the_previews_actions_does_what_its_key_does() {
    // The detail pane is not the pane in focus, yet one press acts.
    for (line, id) in [
        ("update", "act-rebase:acme/team-a/api!410"),
        ("approve", "gl-write"),
    ] {
        let mut p = plugin();
        select(&mut p, 410);
        let mut host = host_of(p, "");
        let text = crate::tests::shown(&host, 160, 40);
        assert!(text.contains(&format!("[{line}]")), "no {line:?} in:\n{text}");
        press_foot(&mut host, line);
        let open = host.menu.as_ref().unwrap_or_else(|| panic!("{line}: no question\n{text}"));
        assert_eq!(open.menu.id, id, "{line}");
    }
}

#[test]
fn a_disabled_foot_has_no_target_and_open_still_uses_its_original_action() {
    let mut p = plugin();
    select(&mut p, 410);
    let mut host = host_of(p, "");
    let _ = crate::tests::shown(&host, 160, 40);
    let merge = foot_controls(&host).into_iter().find(|c| c.action.label == "merge").expect("the muted merge");
    assert!(merge.disabled.is_some() && merge.target.is_none(), "{merge:?}");
    press_foot(&mut host, "merge");
    assert!(host.menu.is_none());
    assert!(host.message.is_none(), "disabled controls perform no action");
    let reasons = gitlab(&host).rows(Pane::Detail);
    assert!(reasons.iter().any(|(id, row)| id == "gitlab.repos.merge" && row.what.contains("needs rebase")));
    let fx = press_foot(&mut host, "open");
    assert!(
        fx.iter().any(|e| format!("{e:?}").contains("merge_requests/410")),
        "opens its link: {fx:?}"
    );
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// The target of the state cell of the merge request `iid`.
fn state_target(p: &GitlabPlugin, iid: u64) -> String {
    let m = p.data.values().flat_map(|d| d.mrs.iter()).find(|m| m.iid == iid).unwrap();
    format!("state:m:{}!{}", m.project_path, m.iid)
}

#[test]
fn a_click_on_needs_rebase_asks_to_rebase_ready_to_merge_asks_to_merge() {
    let mut p = plugin();
    // Another row is selected, then the click selects its row and asks.
    select(&mut p, 187);
    let target = state_target(&p, 410);
    let out = p.click("repos", &target);
    let Outcome::Handled(fx) = out else { panic!("ignored") };
    let menu = fx.iter().find_map(|e| if let Effect::Menu(m) = e { Some(m) } else { None }).expect("a question");
    assert_eq!(menu.id, "act-rebase:acme/team-a/api!410");
    assert_eq!(selected(&p).unwrap().iid, 410, "the row clicked is selected");

    select(&mut p, 263);
    let target = state_target(&p, 187);
    let Outcome::Handled(fx) = p.click("repos", &target) else { panic!("ignored") };
    let menu = fx.iter().find_map(|e| if let Effect::Menu(m) = e { Some(m) } else { None }).expect("a question");
    assert_eq!(menu.id, "act-merge:acme/team-a/web!187");
    assert_eq!(selected(&p).unwrap().iid, 187);
}

#[test]
fn state_context_selection_requires_an_actual_row_and_never_runs_its_action() {
    use radar_core::view::{Intent, Target};
    let mut p = plugin();
    select(&mut p, 187);
    let target = Target::new(state_target(&p, 410));
    for _ in 0..2 {
        let intent = Intent::Context { target: target.clone() };
        let Outcome::Handled(fx) = Plugin::intent(&mut p, "repos", Pane::Middle, &intent) else {
            panic!("real state row ignored")
        };
        assert!(
            fx.iter()
                .all(|effect| matches!(effect, Effect::Emit { .. })),
            "selection only: {fx:?}"
        );
        assert_eq!(selected(&p).unwrap().iid, 410);
    }
    let missing = Intent::Context { target: Target::new("state:m:acme/team-a/api!99999") };
    assert!(matches!(
        Plugin::intent(&mut p, "repos", Pane::Middle, &missing),
        Outcome::Ignored
    ));
    assert_eq!(selected(&p).unwrap().iid, 410);
}

#[test]
fn a_click_on_the_state_of_a_red_pipeline_opens_why_it_is_red() {
    let mut p = plugin();
    select(&mut p, 187);
    let target = state_target(&p, 411);
    let Outcome::Handled(fx) = p.click("repos", &target) else { panic!("ignored") };
    assert!(fx.iter().any(|e| matches!(e, Effect::Focus(Pane::Middle))), "{fx:?}");
    let text = draw(&p, 160, 40);
    assert!(text.contains("Repositories / a / web / !411"), "{text}");
    assert!(text.contains("<:pipeline: Pipeline"), "on the page's Pipeline tab:\n{text}");
}

#[test]
fn only_the_states_a_click_does_something_on_are_clickable() {
    let mut p = plugin();
    select(&mut p, 187);
    let rows = crate::tests::state_targets(&p);
    // !187 ready, !410 behind, !411 red; not !263 (an approval), !342 (a draft), !198.
    let mut expected: Vec<String> = [187, 410, 411].iter().map(|iid| state_target(&p, *iid)).collect();
    expected.sort();
    assert_eq!(sorted(rows), expected);
    for iid in [263, 342, 198] {
        let m = p.data.values().flat_map(|d| d.mrs.iter()).find(|m| m.iid == iid).unwrap();
        assert_eq!(state_click(m), None, "!{iid}");
    }
    let find = |iid| p.data.values().flat_map(|d| d.mrs.iter()).find(|m| m.iid == iid).unwrap();
    assert_eq!(state_click(find(410)), Some(StateClick::Rebase));
    assert_eq!(state_click(find(187)), Some(StateClick::Merge));
    assert_eq!(state_click(find(411)), Some(StateClick::Why));
    assert_eq!(state_click(find(150)), None, "a merged one");
}

/// Every `x` row of a tree row where a rebase, merge or approve is offered,
/// and of its detail pane, is the part's to take and it takes it.
#[test]
fn every_row_is_taken_where_a_rebase_is_offered() {
    use radar_core::check::{Check, Place};
    let mut places = Vec::new();
    for down in 0..8 {
        places.push(Place::screen("repos", Pane::Middle).down(down));
        places.push(Place::screen("repos", Pane::Middle).down(down).focus(Pane::Detail));
    }
    let make = || Box::new(plugin()) as Box<dyn Plugin>;
    let mut check = Check::new(&make);
    let gaps = check.places(&places);
    assert!(gaps.is_empty(), "{} gaps:\n{}", gaps.len(), gaps.join("\n"));
    assert!(check.checked > 50, "only {} checked", check.checked);
}

fn gitlab(host: &Host) -> &GitlabPlugin {
    assert_eq!(host.plugins[0].id(), "gitlab");
    // SAFETY: the host in these tests holds exactly this plugin type.
    unsafe { &*(host.plugins[0].as_ref() as *const dyn Plugin as *const GitlabPlugin) }
}

#[test]
fn an_action_is_invoked_by_its_stable_id_through_headless() {
    use radar_core::view::{Intent, PaneId};
    let mut headless = radar_core::headless::Headless::new(host_of(plugin(), ""), 160, 40);
    headless.focus(PaneId::Middle);
    let ids: Vec<String> = headless.screen().unwrap().actions.iter().map(|a| a.id.clone()).collect();
    assert!(ids.contains(&"gitlab.repos.filter".to_string()), "{ids:?}");
    assert!(ids.iter().all(|id| id.is_empty() || id.starts_with("gitlab.repos.")), "{ids:?}");
    let named: Vec<&String> = ids.iter().filter(|id| !id.is_empty()).collect();
    let unique: std::collections::BTreeSet<&&String> = named.iter().collect();
    assert_eq!(named.len(), unique.len(), "one id per action: {ids:?}");
    assert!(gitlab(headless.host()).filter_input.is_none());
    headless.send(Intent::Invoke { action: "gitlab.repos.nothing".into() });
    assert!(gitlab(headless.host()).filter_input.is_none(), "an id nobody lists does nothing");
    headless.send(Intent::Invoke { action: "gitlab.repos.filter".into() });
    assert!(gitlab(headless.host()).filter_input.is_some(), "the id ran the filter row");
}

#[test]
fn i_on_an_open_merge_request_row_starts_its_review_and_a_merged_one_offers_none() {
    use radar_core::agent_context::REVIEW_LOCALLY_ACTION;
    let mut p = plugin();
    select(&mut p, 263);
    let ids = |p: &GitlabPlugin| p.rows(Pane::Middle).into_iter().map(|(id, _)| id).collect::<Vec<_>>();
    assert!(ids(&p).contains(&REVIEW_LOCALLY_ACTION.to_string()), "{:?}", ids(&p));
    let fx = press(&mut p, 'i');
    let [Effect::StartReview(r)] = fx.as_slice() else { panic!("{fx:?}") };
    assert_eq!(r.changes.len(), 1);
    assert_eq!((r.changes[0].provider.as_str(), r.changes[0].id.as_str()), ("gitlab", "acme/team-a/api!263"));
    // The id runs the same key; an id nobody lists runs nothing.
    let invoke = |p: &GitlabPlugin, id: &str| {
        p.resolve_action("repos", Pane::Middle, &radar_core::view::Intent::Invoke { action: id.into() })
    };
    assert!(invoke(&p, REVIEW_LOCALLY_ACTION).is_some());
    assert!(invoke(&p, "agent.nothing").is_none());
    // A merged one is not offered, and its id runs nothing.
    for d in p.data.values_mut() {
        for m in d.mrs.iter_mut().filter(|m| m.iid == 263) {
            m.state = crate::model::MrState::Merged;
        }
    }
    assert!(!ids(&p).contains(&REVIEW_LOCALLY_ACTION.to_string()));
    assert!(invoke(&p, REVIEW_LOCALLY_ACTION).is_none());
}

mod mnemonic_tests {
    use super::*;

    #[test]
    fn action_producers_have_mnemonics_in_enabled_blocked_and_busy_states() {
        let (mut p, fixture) = crate::test_support::hosted_plugin(vec![]).unwrap();
        let mut item = mr(44, "vimes", "ankh-morpork/watch");
        item.target_branch = "trunk".into();
        item.source_branch = "watch/changes".into();
        item.pipeline_url = Some("https://gitlab.example/ankh-morpork/watch/-/pipelines/44".into());
        for state in 0..4 {
            item.rebase = state == 0;
            item.merge_status = if state == 0 { "NEED_REBASE" } else { "MERGEABLE" }.into();
            item.state = if state == 2 { MrState::Closed } else { MrState::Opened };
            p.acting.clear();
            if state == 3 {
                p.acting.insert(mr_key(&item.project_path, item.iid), crate::mr_actions::Act {
                    what: "rebase", state: Acting::Running, at: p.now,
                });
            }
            let offered = rows(&p, &item);
            for row in &offered {
                let key = row.what.key().to_string();
                assert!(radar_core::mnemonic::at(&key, &row.words()).is_some(),
                    "{state}: {key} / {}", row.words());
            }
            let rebase = offered.iter().find(|row| row.what == Do::Rebase).unwrap();
            assert!(rebase.label.starts_with("update by rebasing"));
            assert_eq!(rebase.why.is_none(), state == 0);
            let bindings = bindings(&p, &item);
            let (_, rebase) = bindings.iter().find(|(id, _)| id == "gitlab.repos.rebase").unwrap();
            assert_eq!(rebase.keys, "u");
            assert!(rebase.change.is_some());
            let action = radar_core::view::ActionSpec::named("gitlab.repos.rebase", rebase);
            assert_eq!(action.mnemonic_at, Some(0));
            assert!(!action.mnemonic_shift);
        }
        fixture.assert_finished();
    }

    #[test]
    fn rich_detail_action_producer_carries_mnemonics_and_original_targets() {
        fn check(blocks: &[radar_core::view::Block], count: &mut usize) {
            use radar_core::view::Block;
            for block in blocks {
                match block {
                    Block::Section { blocks, .. } => check(blocks, count),
                    Block::Line { text, key: Some(key), target, mnemonic_at, .. }
                        if radar_core::mnemonic::letter(key).is_some() => {
                        assert!(target.is_none(), "fallback captions do not bypass host admission");
                        assert!(mnemonic_at.is_some(), "{key}: {}", text.words());
                        // Disabled reasons are suffixes; the caption's underline stays valid.
                        assert_eq!(*mnemonic_at, radar_core::mnemonic::at_text(key, text));
                        if key == "u" { assert!(text.words().starts_with("update")); }
                        *count += 1;
                    }
                    _ => {}
                }
            }
        }
        let (mut p, fixture) = crate::test_support::hosted_plugin(vec![]).unwrap();
        p.me = "vimes".into();
        p.cfg.sources.push(crate::config::Source {
            group: "ankh-morpork".into(), code: "watch".into(), color: "cyan".into(), hide: vec![],
        });
        let mut item = crate::test_support::MrBuilder::default()
            .identity("https://gitlab.example", "ankh-morpork/watch", 410).build();
        item.rebase = true;
        item.merge_status = "NEED_REBASE".into();
        p.data.insert("ankh-morpork".into(), GroupData { mrs: vec![item], ..Default::default() });
        select(&mut p, 410);
        let view = crate::screen::neutral::view(&p, &radar_core::view::ViewCx {
            now: p.now, focus: radar_core::view::PaneId::Detail, width: 160, height: 40,
            parts: radar_core::compose::Parts::none(), icon_lead: 2, setup: false,
        });
        let pane = view.pane(radar_core::view::PaneId::Detail).unwrap();
        let canonical = crate::actions(&p.rows(Pane::Detail));
        let foot = pane.foot_actions.as_ref().expect("shared pinned actions");
        for control in &foot.actions {
            let original = canonical.iter().find(|action| action.id == control.action.id).unwrap();
            assert_eq!(control.action.key, original.key);
            assert_eq!(control.action.change, original.change);
            assert!(control.target.is_none(), "owner never issues live targets");
            let key = original.key.unwrap().name();
            assert_eq!(control.action.mnemonic_at, radar_core::mnemonic::at(&key, &control.action.label));
        }
        let radar_core::view::Node::Extension { fallback, .. } = &pane.body else { panic!("shared MR detail") };
        let radar_core::view::Node::Rich { blocks, .. } = fallback.as_ref() else { panic!("complete rich fallback") };
        let mut count = 0;
        check(blocks, &mut count);
        assert!(count >= 5);
        fixture.assert_finished();
    }
}
