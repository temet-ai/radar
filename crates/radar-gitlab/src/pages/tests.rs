use super::*;
use crate::GlMsg;
use crate::model::fixtures;
use crate::tests::sample;
use radar_core::Plugin;
use radar_core::ratatui::crossterm::event::KeyModifiers;

/// The page's menu rows as bindings.
fn keys(p: &GitlabPlugin, pane: Pane) -> Vec<Binding> {
    crate::bindings_of(super::keys(p, pane))
}

mod busy;
mod log_refresh;
mod cached_refresh;
mod detail_refresh;
mod held;
mod detail_pane_map;

fn press(p: &mut GitlabPlugin, code: KeyCode) -> Vec<Effect> {
    press_in(p, Pane::Middle, code)
}

/// A key in `pane`: the Changes tab's files are `Nav`, beside the diff.
fn press_in(p: &mut GitlabPlugin, pane: Pane, code: KeyCode) -> Vec<Effect> {
    match p.key("repos", pane, KeyEvent::new(code, KeyModifiers::NONE)) {
        Outcome::Handled(fx) => fx,
        Outcome::Ignored => vec![],
    }
}

/// Draw the screen body as the app does (the tree needs a frame to move).
fn draw(p: &GitlabPlugin, w: u16, h: u16) -> String {
    draw_in(p, Pane::Middle, w, h)
}

/// Drawn with `pane` focused: the Changes tab's files keep their cursor
/// only while they are (`Nav`); otherwise it follows the open file.
fn draw_in(p: &GitlabPlugin, pane: Pane, w: u16, h: u16) -> String {
    crate::tests::view_text_in(p, "repos", pane, w, h)
}

/// Whether the Changes tab's view lays the files out beside the diff (a
/// split page layout) rather than the diff alone. The view still holds both
/// panes; the layout says which are shown.
fn files_beside_the_diff(p: &GitlabPlugin, width: u16) -> bool {
    let vcx = radar_core::view::ViewCx {
        now: p.now,
        focus: Pane::Middle.into(),
        width,
        height: 40,
        parts: radar_core::Parts::none(),
        icon_lead: 0,
        setup: false,
    };
    let view = Plugin::view(p, "repos", &vcx).expect("the screen has a view");
    let layout = view.page.expect("a page").layout.expect("a layout");
    layout != radar_core::view::PageLayout::pane(1)
}

/// Move the tree's cursor down until `want` is selected.
fn select(p: &mut GitlabPlugin, want: impl Fn(&Selected<'_>) -> bool) {
    p.view = 0; // Everything
    for _ in 0..30 {
        draw(p, 160, 40);
        if screen::selected(p, p.visible()).as_ref().is_some_and(&want) {
            return;
        }
        press(p, KeyCode::Down);
    }
    panic!("never got there");
}

fn on_mr(p: &mut GitlabPlugin, iid: u64) {
    select(p, |s| matches!(s, Selected::Mr(it) if it.mr.iid == iid));
}

/// The sample's !263 page, open, with its data arrived.
fn mr_page() -> GitlabPlugin {
    let mut p = sample();
    on_mr(&mut p, 263);
    press(&mut p, KeyCode::Enter);
    let mr = find_mr(&p, "acme/team-a/api", 263).unwrap().clone();
    on_mr_page(
        &mut p,
        mr_key("acme/team-a/api", 263),
        Ok((mr, fixtures::detail())),
    );
    p
}

/// Through the real host, chrome and all.
fn hosted(p: GitlabPlugin, w: u16, h: u16) -> String {
    let mut host = radar_core::Host::new(vec![Box::new(sample())], p.now).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    crate::tests::shown(&host, w, h)
}

#[test]
fn enter_opens_a_merge_requests_page_and_fetches_what_it_shows() {
    let mut p = sample();
    on_mr(&mut p, 263);
    let fx = press(&mut p, KeyCode::Enter);
    assert!(matches!(fx[0], Effect::Focus(Pane::Middle)));
    assert!(
        fx.iter().any(|e| matches!(e, Effect::SpawnAction { action, .. }
            if action.kind.as_str() == "gitlab.page.detail")),
        "{fx:?}"
    );
    assert_eq!(
        p.pages.last().map(|pg| pg.kind.clone()),
        Some(Kind::Mr {
            project: "acme/team-a/api".into(),
            iid: 263
        })
    );
    assert!(p.busy("repos"), "busy while the page loads");
    let text = draw(&p, 160, 40);
    assert!(text.contains("Repositories / a / api / !263"), "{text}");
    assert!(text.contains("loading…"), "{text}");
    assert_eq!(p.panes("repos"), vec![Pane::Middle], "a page is one pane");
}

#[test]
fn an_error_shows_in_the_page() {
    let mut p = sample();
    on_mr(&mut p, 263);
    press(&mut p, KeyCode::Enter);
    on_mr_page(
        &mut p,
        mr_key("acme/team-a/api", 263),
        Err("GitLab: boom".into()),
    );
    assert!(!p.busy("repos"));
    let text = draw(&p, 160, 40);
    assert!(text.contains(":error: merge request could not sync: GitLab: boom"), "{text}");
}

#[test]
fn the_merge_request_page_shows_every_tab() {
    let mut p = mr_page();
    let text = draw(&p, 160, 40);
    for want in [
        "ABC-12: Add a stats endpoint for the dashboard",
        "alice · :branch: feature/ABC-263 → main",
        "<:info: Overview>  :file: Changes  :thread: Threads !  :pipeline: Pipeline  :commit: Commits",
        "can it merge?",
        ":needs_approval: needs 1 approval",
        ":run_passed: pipeline passed",
        ":run_passed: no conflicts, up to date",
        ":needs_approval: needs 1 approval (0 of 1)",
        ":thread: 1 open thread",
        ":run_passed: not a draft",
        "• adds `GET /stats`",
        "LATEST ACTIVITY",
        "Alice Example · 46m ago · on StatsController.java:21",
    ] {
        assert!(text.contains(want), "overview lacks {want:?}:\n{text}");
    }
    assert!(!text.contains("<!--") && !text.contains("**"), "{text}");
    let tabs: [&[&str]; 5] = [
        &[
            "<:file: Changes>",
            "files · 1 of 5 seen",
            "main/java/acme/stats",
            "+42 −0",
            "+33 −3",
            "StatsController.java :thread: 1",
            "loading…",
        ],
        &[
            "<:thread: Threads !>",
            "+ :comment: comment on !263",
            ":thread: open · StatsController.java:21",
            "│ ↳ :person: Alice Example · 46m ago",
            ":resolved: resolved · StatsController.java:9",
        ],
        &[
            "<:pipeline: Pipeline>",
            "pipeline #1843",
            ":run_passed: build",
            "build-artifacts",
            "2m 03s",
            ":run_failed: test",
            "allowed to fail",
        ],
        &[
            "<:commit: Commits>",
            "9612f87c",
            "Resolve merge conflicts",
            "Alice Example",
        ],
        &["<:info: Overview>"],
    ];
    for wants in tabs {
        press(&mut p, KeyCode::Char(']'));
        let text = draw(&p, 160, 40);
        for want in wants {
            assert!(text.contains(want), "lacks {want:?}:\n{text}");
        }
    }
    press(&mut p, KeyCode::Char('['));
    assert!(
        draw(&p, 160, 40).contains("<:commit: Commits>"),
        "[ goes back, round"
    );
}

#[test]
fn mr_page_overview() {
    insta::assert_snapshot!(hosted(mr_page(), 160, 40));
}

/// The sample's !263 page on its Changes tab, its diffs arrived.
fn changes_page() -> GitlabPlugin {
    let mut p = mr_page();
    let fx = press(&mut p, KeyCode::Char(']'));
    assert!(
        fx.iter().any(|e| matches!(e, Effect::SpawnAction { .. })),
        "opening the tab asks for the open file's diff: {fx:?}"
    );
    let ds = &p.diffs[&mr_key("acme/team-a/api", 263)];
    assert_eq!(
        ds.asked.iter().copied().collect::<Vec<_>>(),
        [1],
        "the first page of files"
    );
    let head = ds.head.clone();
    let fx = changes::on_diffs(
        &mut p,
        mr_key("acme/team-a/api", 263),
        head,
        1,
        Ok(fixtures::diffs()),
    );
    assert!(fx.is_empty(), "everything is held now: {fx:?}");
    // The tree's first file is the README; the review is on the controller, the next.
    press(&mut p, KeyCode::Char('n'));
    assert_eq!(p.pages[0].file.get(), 0);
    draw(&p, 160, 40); // Coordinate-dependent review actions use a painted payload.
    p
}

#[test]
fn mr_page_changes() {
    insta::assert_snapshot!(hosted(changes_page(), 160, 40));
}

#[test]
fn mr_page_changes_narrow() {
    insta::assert_snapshot!(hosted(changes_page(), 80, 24));
}

#[test]
fn the_diff_moves_by_line_hunk_and_file() {
    let mut p = changes_page();
    let text = draw(&p, 160, 40);
    for want in [
        "@@ -0,0 +1,24 @@",
        "1 +package acme.stats;",
        "StatsController.java · new file",
    ] {
        assert!(text.contains(want), "lacks {want:?}:\n{text}");
    }
    press(&mut p, KeyCode::Down);
    assert_eq!(p.pages[0].diff.cursor(), 1);
    // Past the only hunk: the next file in the tree (the service), at its first line.
    press(&mut p, KeyCode::Char('}'));
    assert_eq!(p.pages[0].file.get(), 2);
    assert_eq!(p.pages[0].diff.cursor(), 0);
    press(&mut p, KeyCode::Char('}'));
    assert_eq!(p.pages[0].file.get(), 1);
    let text = draw(&p, 160, 40);
    assert!(text.contains("files · 4 of 5 seen"), "{text}");
    assert!(text.contains("10   10      @Test"), "{text}");
    assert!(
        text.contains("12      -        assertEquals(0, totals());"),
        "{text}"
    );
    // The build file is the last in the tree, though not in the list.
    press(&mut p, KeyCode::Char('n'));
    assert_eq!(p.pages[0].file.get(), 4);
    let fx = press(&mut p, KeyCode::Char('n'));
    assert!(
        matches!(&fx[..], [Effect::Notify(m)] if m == "that was the last file"),
        "{fx:?}"
    );
    for want in [1, 2, 0] {
        press(&mut p, KeyCode::Char('N'));
        assert_eq!(p.pages[0].file.get(), want);
    }
    press(&mut p, KeyCode::Char('G'));
    assert_eq!(p.pages[0].diff.cursor(), 24);
    press(&mut p, KeyCode::Char('{'));
    assert_eq!(p.pages[0].diff.cursor(), 0);
}

fn stat(path: &str, additions: u32, deletions: u32) -> crate::model::FileStat {
    crate::model::FileStat {
        path: path.into(),
        additions,
        deletions,
    }
}

#[test]
fn the_files_are_a_folder_tree_with_common_prefixes_joined_and_sums_on_folders() {
    use changes::Entry;
    let files = [
        stat("src/main/java/acme/stats/B.java", 5, 1),
        stat("src/main/java/acme/stats/a.java", 2, 0),
        stat("src/test/java/T.java", 7, 3),
        stat("docs/README.md", 6, 1),
        stat("Cargo.toml", 1, 1),
        stat("z/only/chain/f.rs", 4, 4),
    ];
    let tops = changes::entries(&files);
    let names: Vec<String> = tops
        .iter()
        .map(|e| match e {
            Entry::Dir { name, .. } | Entry::File { name, .. } => name.clone(),
        })
        .collect();
    // Folders first, then files; a folder with one folder in it is one row.
    assert_eq!(names, ["docs", "src", "z/only/chain", "Cargo.toml"]);
    let Entry::Dir {
        kids,
        adds,
        dels,
        id,
        ..
    } = &tops[1]
    else {
        panic!("src is a folder")
    };
    assert_eq!(
        (id.as_str(), *adds, *dels),
        ("d:src", 14, 4),
        "sums of everything under it"
    );
    let [
        Entry::Dir {
            name: main,
            id: main_id,
            adds: ma,
            dels: md,
            kids: in_main,
        },
        Entry::Dir { name: test, .. },
    ] = &kids[..]
    else {
        panic!("two folders under src: {kids:?}");
    };
    assert_eq!(
        (main.as_str(), test.as_str()),
        ("main/java/acme/stats", "test/java")
    );
    assert_eq!(
        (main_id.as_str(), *ma, *md),
        ("d:src/main/java/acme/stats", 7, 1)
    );
    // Files in a folder by name, whatever the case.
    let files_in: Vec<_> = in_main.iter().map(|e| e.id()).collect();
    assert_eq!(files_in, ["f:1", "f:0"]);
    // n and N step through the files as shown, folders left out.
    assert_eq!(changes::order(&tops), [3, 1, 0, 2, 5, 4]);
    assert_eq!(
        changes::path_to(&tops, "f:0"),
        Some(vec![
            "d:src".into(),
            "d:src/main/java/acme/stats".into(),
            "f:0".into()
        ])
    );
}

#[test]
fn space_folds_a_folder_and_n_and_n_skip_folders() {
    let mut p = changes_page();
    let text = draw(&p, 160, 40);
    for want in [
        "docs",
        "README.md",
        "src",
        "main/java/acme/stats",
        "test/java/acme/stats",
        "StatsService.java",
        "build.gradle",
    ] {
        assert!(text.contains(want), "lacks {want:?}:\n{text}");
    }
    // The folders add up their files: the stats folder has the controller and the service.
    assert!(text.contains("+60 −2"), "{text}");
    // Space in the files pane folds a folder (on the diff it folds a thread).
    // The cursor starts on the open file; up is the folder over it.
    press_in(&mut p, Pane::Nav, KeyCode::Up);
    draw_in(&p, Pane::Nav, 160, 40);
    press_in(&mut p, Pane::Nav, KeyCode::Char(' '));
    let text = draw_in(&p, Pane::Nav, 160, 40);
    assert!(
        !text.contains("StatsService.java"),
        "the folder is folded:\n{text}"
    );
    assert!(text.contains("main/java/acme/stats  +60 −2  ⋯ 2"), "the two held files: {text}");
    press_in(&mut p, Pane::Nav, KeyCode::Enter);
    assert!(
        draw_in(&p, Pane::Nav, 160, 40).contains("StatsService.java"),
        "⏎ unfolds it"
    );
    // ⏎ on a file opens it; the page stays.
    press_in(&mut p, Pane::Nav, KeyCode::Down);
    press_in(&mut p, Pane::Nav, KeyCode::Down);
    draw_in(&p, Pane::Nav, 160, 40);
    press_in(&mut p, Pane::Nav, KeyCode::Enter);
    assert_eq!(p.pages[0].file.get(), 2);
    assert_eq!(p.pages.len(), 1);
    // Off the files' keys their cursor follows the open file. Fold the
    // folder over it, then n goes to the next file, not a folder.
    draw(&p, 160, 40);
    press_in(&mut p, Pane::Nav, KeyCode::Up);
    press_in(&mut p, Pane::Nav, KeyCode::Up);
    draw_in(&p, Pane::Nav, 160, 40);
    press_in(&mut p, Pane::Nav, KeyCode::Char(' '));
    // (Its name is still in the diff's title.)
    // The open file is the tree's selected row, drawn with the selection's `>`.
    let open_file = |text: &str| text.lines().any(|l| l.starts_with('>') && l.contains(":file_java: StatsService.java"));
    assert!(!open_file(&draw(&p, 160, 40)));
    press(&mut p, KeyCode::Char('n'));
    assert_eq!(p.pages[0].file.get(), 1, "from the service to the test");
    press(&mut p, KeyCode::Char('N'));
    assert_eq!(p.pages[0].file.get(), 2);
    assert!(
        open_file(&draw(&p, 160, 40)),
        "the folder over the open file opens"
    );
    // Back at the top: the README, then nothing before it.
    press(&mut p, KeyCode::Char('N'));
    press(&mut p, KeyCode::Char('N'));
    assert_eq!(p.pages[0].file.get(), 3);
    let fx = press(&mut p, KeyCode::Char('N'));
    assert!(
        matches!(&fx[..], [Effect::Notify(m)] if m == "this is the first file"),
        "{fx:?}"
    );
}

#[test]
fn neutral_changes_fold_selects_the_collapsed_parent_without_changing_the_file() {
    // One real owner projection establishes the valid loaded file hierarchy;
    // the rest exercises intents without additional frame rendering.
    let mut p = changes_page();
    draw(&p, 160, 40);
    let selected = p.pages[0].tree.borrow().selected().to_vec();
    assert!(selected.len() > 1);
    let parent = selected[..selected.len() - 1].to_vec();
    let file = p.pages[0].file.get();
    let target = radar_core::view::TreeRow::target_of(&parent);
    let _ = neutral::intent(&mut p, Pane::Nav, &radar_core::view::Intent::Fold {
        target: target.clone(), open: false,
    });
    assert_eq!(p.pages[0].tree.borrow().selected(), parent.as_slice());
    assert_eq!(p.pages[0].file.get(), file, "folding does not open another file");
    let _ = neutral::intent(&mut p, Pane::Nav, &radar_core::view::Intent::Fold {
        target, open: true,
    });
    assert_eq!(p.pages[0].tree.borrow().selected(), parent.as_slice());
    assert_eq!(p.pages[0].file.get(), file);
}

#[test]
fn host_open_all_expands_nested_changes_folders_without_changing_selection_or_file() {
    use radar_core::host::Event;
    use radar_core::view::{Intent, Node, PaneId};

    let p = changes_page();
    p.pages[0].tree.borrow_mut().close_all();
    p.pages[0].tree.borrow_mut().select(vec!["d:src".into()]);
    let now = p.now;
    let mut host = radar_core::Host::new(vec![Box::new(p)], now).unwrap();
    host.focus = Pane::Nav;
    host.set_view_room(160, 40);
    let page = |host: &radar_core::Host| {
        host.screen_view(0, "repos")
            .expect("the GitLab screen")
            .page
            .expect("the Changes page")
    };
    let before = page(&host);
    assert_eq!(before.panes[0].pane, PaneId::Nav);
    let Node::Tree(before_tree) = &before.panes[0].body else {
        panic!("Changes has a files tree")
    };
    let mut groups = before_tree.groups();
    assert_eq!(groups.len(), 4, "all loaded folders, including nested ones");
    assert!(groups.iter().any(|path| path.len() > 1));
    assert!(before_tree.open.is_empty());
    assert_eq!(before_tree.selected, ["d:src"]);
    for path in &groups {
        assert_eq!(before_tree.row(path).unwrap().name.0[0].icon, Some(Icon::Folder));
    }
    assert_eq!(before.panes[1].pane, PaneId::Middle);
    assert!(before.panes[1].title.contains("StatsController.java"));
    let subject = host.plugins[0].subject("repos", Pane::Nav).expect("the merge request");
    let context = host.motion_context().expect("the Changes page identity");

    host.update(Event::Intent(Intent::OpenAll));

    let after = page(&host);
    let Node::Tree(after_tree) = &after.panes[0].body else {
        panic!("Changes keeps its files tree")
    };
    groups.sort();
    assert_eq!(after_tree.open, groups, "opening the first folder must not stop the rest");
    assert_eq!(after_tree.selected, before_tree.selected);
    for path in &groups {
        assert_eq!(after_tree.row(path).unwrap().name.0[0].icon, Some(Icon::FolderOpen));
    }
    assert_eq!(after.panes[1], before.panes[1], "the open file and diff stay unchanged");
    assert_eq!(host.plugins[0].subject("repos", Pane::Nav), Some(subject));
    assert_eq!(host.motion_context(), Some(context));
    assert_eq!(host.focus, Pane::Nav);
}

#[test]
fn plus_and_minus_fold_the_files_folders_in_the_files_pane() {
    let mut p = changes_page();
    draw(&p, 160, 40);
    assert!(
        !radar_core::Plugin::folds(&p, "repos", Pane::Middle),
        "the diff folds nothing"
    );
    assert!(radar_core::Plugin::folds(&p, "repos", Pane::Nav));
    // On the open file: `-` folds its folder and lands on it.
    press_in(&mut p, Pane::Nav, KeyCode::Char('-'));
    let text = draw(&p, 160, 40);
    assert!(text.contains("main/java/acme/stats  +60 −2  ⋯ 2"), "the two held files: {text}");
    assert!(
        p.pages[0]
            .tree
            .borrow()
            .selected()
            .last()
            .is_some_and(|s| s.starts_with("d:"))
    );
    press_in(&mut p, Pane::Nav, KeyCode::Char('+'));
    assert!(
        draw(&p, 160, 40).contains("StatsService.java"),
        "+ opens it again"
    );
    press_in(&mut p, Pane::Nav, KeyCode::Char('-'));
    draw(&p, 160, 40);
    // '=' belongs to the shared host contract, including the files pane.
    let now = p.now;
    let mut host = radar_core::Host::new(vec![Box::new(p)], now).unwrap();
    host.focus = Pane::Nav;
    host.set_view_room(160, 40);
    let before = host.screen_view(0, "repos").unwrap().page.unwrap();
    assert!(host.update(radar_core::host::Event::Key(KeyEvent::new(
        KeyCode::Char('='), KeyModifiers::NONE,
    ))).is_empty(), "opening folders has no external effects");
    let after = host.screen_view(0, "repos").unwrap().page.unwrap();
    let radar_core::view::Node::Tree(tree) = &after.panes[0].body else {
        panic!("files tree")
    };
    assert!(tree.groups().iter().all(|path| tree.open.contains(path)),
        "= opens every held folder");
    let radar_core::view::Node::Tree(before_tree) = &before.panes[0].body else {
        panic!("files tree before opening")
    };
    assert_eq!(tree.selected, before_tree.selected);
    assert_eq!(after.panes[1], before.panes[1], "opening keeps the selected file and diff");
    assert!(crate::tests::shown(&host, 160, 40).contains("StatsService.java"), "= too");
}

#[test]
fn the_changes_tab_is_two_panes_and_up_past_the_top_of_the_files_is_the_hosts() {
    let mut p = changes_page();
    draw(&p, 160, 40);
    assert_eq!(
        radar_core::Plugin::panes(&p, "repos"),
        vec![Pane::Nav, Pane::Middle]
    );
    assert_eq!(
        radar_core::Plugin::tabs(&p, "repos").map(|t| (t.names.len(), t.on)),
        Some((5, 1)),
        "the page's tabs, the Changes tab showing"
    );
    // The x menu is about the pane: the files' keys there, the diff's on the diff.
    let keys = |p: &GitlabPlugin, pane| -> Vec<String> {
        radar_core::Plugin::keys(p, "repos", pane)
            .into_iter()
            .map(|b| b.keys)
            .collect()
    };
    let files = keys(&p, Pane::Nav);
    assert!(
        files.contains(&"space".to_string()) && !files.contains(&"n".to_string()),
        "{files:?}"
    );
    let diff = keys(&p, Pane::Middle);
    assert!(
        diff.contains(&"n".to_string()) && diff.contains(&"v".to_string()),
        "{diff:?}"
    );
    assert!(
        !files.contains(&"v".to_string()),
        "side by side is the diff's: {files:?}"
    );
    // The host shows a tab: from its top, on the main pane.
    let fx = radar_core::Plugin::show_tab(&mut p, "repos", 2);
    assert!(matches!(fx[0], Effect::Focus(Pane::Middle)));
    assert_eq!(p.pages[0].tab, 2);
    assert_eq!(radar_core::Plugin::panes(&p, "repos"), vec![Pane::Middle]);
    // Narrow: the diff alone, one pane.
    radar_core::Plugin::show_tab(&mut p, "repos", 1);
    draw(&p, 80, 24);
    assert_eq!(radar_core::Plugin::panes(&p, "repos"), vec![Pane::Middle]);
}

/// The Changes tab never traps the arrows: ↑ from the files reaches the
/// tab row, and ← / → fold the files tree only while there is something to
/// fold or step into; past that they go to the pane and the tab beside, as
/// on every item page.
#[test]
fn the_arrows_leave_the_changes_tab_for_the_tab_row_and_the_tabs_beside() {
    use radar_core::host::Event;
    let press = |host: &mut radar_core::Host, code: KeyCode, times: usize| {
        for _ in 0..times {
            let _ = crate::tests::shown(host, 160, 40);
            host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        }
    };
    let tab = |host: &radar_core::Host| radar_core::Plugin::tabs(host.plugins[0].as_ref(), "repos").map(|t| t.on);
    let hosted = || {
        let p = changes_page();
        let now = p.now;
        let mut host = radar_core::Host::new(vec![Box::new(p)], now).unwrap();
        host.focus = Pane::Nav;
        host.set_view_room(160, 40);
        host
    };

    let mut host = hosted();
    press(&mut host, KeyCode::Up, 12);
    assert!(host.tabs, "↑ past the top of the files is the page's tab row");
    press(&mut host, KeyCode::Left, 1);
    assert_eq!(tab(&host), Some(0), "← on the tab row: Overview");

    // Pressed until the tab changes: a few presses fold and climb first.
    let until_the_tab_changes = |code: KeyCode| {
        let mut host = hosted();
        for _ in 0..12 {
            press(&mut host, code, 1);
            if tab(&host) != Some(1) {
                break;
            }
        }
        tab(&host)
    };
    assert_eq!(until_the_tab_changes(KeyCode::Left), Some(0), "← folds and climbs, then the tab beside: Overview");
    assert_eq!(until_the_tab_changes(KeyCode::Right), Some(2), "→ crosses to the diff, then the tab beside: Threads");
}

#[test]
fn v_shows_the_diff_side_by_side_and_is_kept_in_the_config() {
    let mut p = changes_page();
    let one = draw(&p, 160, 40);
    assert!(!p.pages[0].split_shown.get());
    press(&mut p, KeyCode::Char('v'));
    assert!(p.cfg.side_by_side);
    let two = draw(&p, 160, 40);
    assert!(p.pages[0].split_shown.get(), "wide enough:\n{two}");
    assert_ne!(one, two);
    let cfg = toml::to_string(&p.cfg).unwrap();
    assert!(cfg.contains("side_by_side = true"), "{cfg}");
    // Narrow: one column, whatever is wanted.
    draw(&p, 80, 24);
    assert!(!p.pages[0].split_shown.get());
    // The x menu says what v does now.
    draw(&p, 160, 40);
    let keys = radar_core::Plugin::keys(&p, "repos", Pane::Middle);
    assert!(
        keys.iter()
            .any(|b| b.keys == "v" && b.what == "view the diff in one column"),
        "{keys:?}"
    );
    press(&mut p, KeyCode::Char('v'));
    assert!(!p.cfg.side_by_side && !toml::to_string(&p.cfg).unwrap().contains("side_by_side"));
}

#[test]
fn f_collapses_the_files_and_the_diff_takes_the_width() {
    let mut p = changes_page();
    draw(&p, 160, 40);
    assert!(files_beside_the_diff(&p, 160));
    assert_eq!(
        radar_core::Plugin::panes(&p, "repos"),
        vec![Pane::Nav, Pane::Middle]
    );
    // From the files pane: collapsed, the keys go to the diff.
    let fx = press_in(&mut p, Pane::Nav, KeyCode::Char('f'));
    assert!(
        fx.iter().any(|e| matches!(e, Effect::Focus(Pane::Middle))),
        "{fx:?}"
    );
    assert!(!files_beside_the_diff(&p, 160));
    assert_eq!(radar_core::Plugin::panes(&p, "repos"), vec![Pane::Middle]);
    assert!(
        toml::to_string(&p.cfg)
            .unwrap()
            .contains("hide_files = true")
    );
    let keys = radar_core::Plugin::keys(&p, "repos", Pane::Middle);
    assert!(
        keys.iter()
            .any(|b| b.keys == "f" && b.what == "show the files beside the diff"),
        "{keys:?}"
    );
    // From the diff: back again.
    press(&mut p, KeyCode::Char('f'));
    assert!(files_beside_the_diff(&p, 160));
    assert_eq!(
        radar_core::Plugin::panes(&p, "repos"),
        vec![Pane::Nav, Pane::Middle]
    );
}

#[test]
fn enter_or_a_click_on_a_lines_numbers_starts_a_comment_on_it() {
    let mut p = changes_page();
    draw(&p, 160, 40);
    // The first line is the hunk's heading: down to a line of code.
    press(&mut p, KeyCode::Down);
    press(&mut p, KeyCode::Enter);
    let c = p.compose.as_ref().expect("⏎ writes a comment");
    assert!(
        matches!(c.target, crate::write::Target::Line(_)),
        "{:?}",
        c.target
    );
    p.compose = None;
    // Side by side, a click on the numbers of a line does the same.
    press(&mut p, KeyCode::Char('v'));
    // The numbers of a shown line send `comment:<line>`; the first line is
    // the hunk's heading, which has none.
    let text = draw(&p, 160, 40);
    assert!(text.contains("diff "), "{text}");
    let target = "comment:2".to_string();
    click(&mut p, &target);
    assert!(p.compose.is_some(), "a press on {target} writes a comment");
}

#[test]
fn clicking_a_row_selects_it_and_a_file_opens() {
    let mut p = changes_page();
    // The files are rows with targets; the tree sends a pointer press as
    // `node:<target>` (`pointer_action`), so a folder only moves the cursor.
    let vcx = radar_core::view::ViewCx {
        now: p.now,
        focus: radar_core::view::PaneId::Nav,
        width: 160,
        height: 40,
        parts: radar_core::Parts::none(),
        icon_lead: 0,
        setup: false,
    };
    let view = Plugin::view(&p, "repos", &vcx).expect("the screen has a view");
    let files = view.page.as_ref().and_then(|page| page.panes.first()).expect("the files pane");
    assert!(matches!(&files.body, radar_core::view::Node::Tree(tree) if tree.pointer_action));
    let text = draw_in(&p, Pane::Nav, 160, 40);
    assert!(text.contains("{d:docs}"), "{text}");
    assert!(text.contains("{f:4}"), "{text}");
    click(&mut p, "node:d:src");
    assert_eq!(p.pages[0].tree.borrow().selected(), ["d:src".to_string()]);
    assert_eq!(p.pages[0].file.get(), 0, "a folder opens no file");
    click(&mut p, "node:f:4");
    assert_eq!(p.pages[0].file.get(), 4);
    assert_eq!(
        p.pages[0]
            .tree
            .borrow()
            .selected()
            .last()
            .map(String::as_str),
        Some("f:4")
    );
}

#[test]
fn mr_page_changes_folded() {
    let mut p = changes_page();
    draw(&p, 160, 40);
    press_in(&mut p, Pane::Nav, KeyCode::Up);
    draw_in(&p, Pane::Nav, 160, 40);
    press_in(&mut p, Pane::Nav, KeyCode::Char(' '));
    insta::assert_snapshot!(hosted(p, 160, 40));
}

#[test]
fn threads_sit_under_their_lines_open_ones_open_and_resolved_ones_folded() {
    let mut p = changes_page();
    let text = draw(&p, 160, 40);
    let at = |text: &str, want: &str| {
        text.lines()
            .position(|l| l.contains(want))
            .unwrap_or_else(|| panic!("lacks {want:?}:\n{text}"))
    };
    // Under line 9, folded to one line.
    let resolved = at(&text, ":resolved: resolved · 1 note · Bob E.: Typo in the name.");
    assert_eq!(resolved, at(&text, "9 +public class StatsControler {") + 1);
    // Under line 21, with its notes.
    press(&mut p, KeyCode::Char('G'));
    let text = draw(&p, 160, 40);
    let open = at(&text, ":thread: open thread · 2 notes");
    assert_eq!(open, at(&text, "21 +    public Totals totals()") + 1);
    assert!(text.contains("│ :person: Bob Example · 1h ago"), "{text}");
    assert!(
        text.contains("│ Should this read from the cached view?"),
        "{text}"
    );
    assert!(text.contains("│ ↳ :person: Alice Example · 46m ago"), "{text}");
    // space folds the thread under the cursor's line, and unfolds it.
    for _ in 0..3 {
        press(&mut p, KeyCode::Up);
    }
    assert_eq!(p.pages[0].diff.cursor(), 21);
    let fx = press(&mut p, KeyCode::Char(' '));
    assert!(fx.is_empty(), "{fx:?}");
    let text = draw(&p, 160, 40);
    assert!(
        text.contains(":thread: open thread · 2 notes · Bob E.: Should this read"),
        "{text}"
    );
    press(&mut p, KeyCode::Down);
    let fx = press(&mut p, KeyCode::Char(' '));
    assert!(
        matches!(&fx[..], [Effect::Notify(m)] if m == "no thread on this line"),
        "{fx:?}"
    );
}

fn ctrl_s(p: &mut GitlabPlugin) -> Vec<Effect> {
    match p.key(
        "repos",
        Pane::Middle,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    ) {
        Outcome::Handled(fx) => fx,
        Outcome::Ignored => vec![],
    }
}

fn typed(p: &mut GitlabPlugin, text: &str) {
    for c in text.chars() {
        press(
            p,
            if c == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(c)
            },
        );
    }
}

/// The menu a list of effects opens: its title and its lines.
fn menu_of(fx: &[Effect]) -> (String, Vec<String>) {
    fx.iter()
        .find_map(|e| match e {
            Effect::Menu(m) => Some((
                m.title.clone(),
                m.items.iter().map(|i| i.label.clone()).collect(),
            )),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no menu in {fx:?}"))
}

#[test]
fn a_comment_on_a_line_is_shown_exactly_asked_first_and_sent_once() {
    let mut p = changes_page();
    for _ in 0..3 {
        press(&mut p, KeyCode::Down);
    }
    assert!(press(&mut p, KeyCode::Char('c')).is_empty());
    assert!(p.capturing(), "typing");
    typed(&mut p, "Cap the range\nat 90 days?");
    let text = draw(&p, 160, 40);
    assert!(
        text.contains("✎ comment on StatsController.java line 3"),
        "{text}"
    );
    assert!(text.contains("Cap the range"), "{text}");
    let fx = ctrl_s(&mut p);
    let (title, lines) = menu_of(&fx);
    assert_eq!(title, "post this on StatsController.java line 3?");
    assert_eq!(lines, ["Cap the range", "at 90 days?", "yes", "edit", "no"]);
    let Some(crate::write::Change::Line { at, text, .. }) = p.change.clone() else {
        panic!("{:?}", p.change)
    };
    assert_eq!(
        (at.new, at.old, text.as_str()),
        (Some(3), None, "Cap the range\nat 90 days?")
    );
    // Nothing is sent before yes; yes sends it once.
    let fx = p.on_menu("gl-write", "yes");
    assert!(
        matches!(
            &fx[..],
            [
                Effect::Report(_),
                Effect::Report(_),
                Effect::Notify(_),
                Effect::Spawn(_)
            ]
        ),
        "{fx:?}"
    );
    assert!(p.compose.is_none() && p.busy("repos"));
    let fx = press(&mut p, KeyCode::Char('c'));
    choose_recovery(&mut p, &fx, "new");
    typed(&mut p, "again");
    ctrl_s(&mut p);
    let fx = p.on_menu("gl-write", "yes");
    assert!(
        matches!(&fx[..], [Effect::Report(_), Effect::Report(_), Effect::Notify(m)] if m.contains("has not answered")),
        "{fx:?}"
    );
    assert_eq!(
        p.compose.as_ref().unwrap().text(),
        "again",
        "busy keeps the newer composer"
    );
    assert_eq!(
        p.change.as_ref().and_then(crate::write::Change::text),
        Some("again")
    );
    // The string-only error cannot prove execution: keep its text and uncertainty.
    let change = crate::write::Change::Line {
        project: "acme/team-a/api".into(),
        iid: 263,
        at: at.clone(),
        text: "Cap the range\nat 90 days?".into(),
    };
    let fx = crate::write::on_wrote(
        &mut p,
        crate::write::WroteMsg {
            operation: 1,
            connection: Default::default(),
            change,
            result: Err("GitLab says this token may not do that here (403)".into()),
        },
    );
    assert!(
        matches!(&fx[..], [Effect::Report(radar_core::journal::Outcome::Unknown), Effect::Notice(m)] if m.tone == radar_core::view::chrome::NoticeTone::Error && m.text == "✗ Could not confirm the result of asking GitLab to post a comment on StatsController.java line 3: GitLab says this token may not do that here (403) Check GitLab before trying again. · c has your text"),
        "{fx:?}"
    );
    p.compose = None;
    p.change = None;
    let fx = press(&mut p, KeyCode::Char('c'));
    let fx = choose_recovery(&mut p, &fx, "entry:1");
    choose_recovery(&mut p, &fx, "edit");
    assert_eq!(
        p.compose.as_ref().unwrap().text(),
        "Cap the range\nat 90 days?"
    );
    // esc with text asks first.
    let fx = press(&mut p, KeyCode::Esc);
    assert_eq!(menu_of(&fx).0, "stop: drop the comment?");
}

#[test]
fn replies_resolves_and_approvals_are_asked_first() {
    let mut p = changes_page();
    for _ in 0..21 {
        press(&mut p, KeyCode::Down);
    }
    press(&mut p, KeyCode::Char('c'));
    assert_eq!(
        p.compose.as_ref().unwrap().title(),
        "✎ reply in the thread on StatsController.java:21"
    );
    typed(&mut p, "Done.");
    let (title, lines) = menu_of(&ctrl_s(&mut p));
    assert_eq!(title, "post this in the thread on StatsController.java:21?");
    assert_eq!(lines, ["Done.", "yes", "edit", "no"]);
    assert!(
        matches!(&p.change, Some(crate::write::Change::Reply { discussion, .. }) if discussion == "6a9c1750b37d513a43987b574953fceb50b03ce7")
    );
    // e: back to the text, as it was.
    p.on_menu("gl-write", "edit");
    assert!(p.change.is_none());
    assert_eq!(p.compose.as_ref().unwrap().text(), "Done.");
    ctrl_s(&mut p);
    // n: nothing sent, the composer closes, and c has the text back.
    let fx = p.on_menu("gl-write", "no");
    assert!(
        matches!(&fx[..], [Effect::Report(_), Effect::Notify(m)] if m == "nothing sent · c has your text"),
        "{fx:?}"
    );
    assert!(p.change.is_none() && p.compose.is_none(), "no closes it");
    let fx = press(&mut p, KeyCode::Char('c'));
    let fx = choose_recovery(&mut p, &fx, "entry:1");
    choose_recovery(&mut p, &fx, "edit");
    assert_eq!(p.compose.as_ref().unwrap().text(), "Done.");
    p.compose = None;
    assert_eq!(
        changes::split("src/a/AVeryLongNameIndeed.java", 12).1,
        "AVeryLongNa…"
    );
    let (title, lines) = menu_of(&press(&mut p, KeyCode::Char('d')));
    assert_eq!(title, "resolve the thread on StatsController.java:21?");
    assert_eq!(lines, ["marks it resolved for everyone", "yes", "no"]);
    let (title, lines) = menu_of(&press(&mut p, KeyCode::Char('a')));
    assert_eq!(title, "approve !263?");
    assert_eq!(lines[0], "as me, of commit 9612f87c");
    // On a line without a thread there is nothing to resolve.
    press(&mut p, KeyCode::Down);
    let fx = press(&mut p, KeyCode::Char('d'));
    assert!(
        matches!(&fx[..], [Effect::Notify(m)] if m == "no thread here to resolve"),
        "{fx:?}"
    );
}

#[test]
fn the_threads_tab_comments_on_the_merge_request_and_opens_a_thread_at_its_line() {
    let mut p = changes_page();
    press(&mut p, KeyCode::Char(']'));
    let text = draw(&p, 160, 40);
    assert!(text.contains("+ :comment: comment on !263"), "{text}");
    press(&mut p, KeyCode::Char('c'));
    assert_eq!(p.compose.as_ref().unwrap().title(), "✎ comment on !263");
    let text = draw(&p, 160, 40);
    assert!(
        text.contains("✎ comment on !263"),
        "the composer at the foot:\n{text}"
    );
    press(&mut p, KeyCode::Esc);
    assert!(p.compose.is_none(), "nothing typed: esc just stops");
    press(&mut p, KeyCode::Down);
    let (title, _) = menu_of(&press(&mut p, KeyCode::Char('d')));
    assert_eq!(title, "resolve the thread on StatsController.java:21?");
    press(&mut p, KeyCode::Enter);
    assert_eq!(p.pages[0].tab, mr::CHANGES);
    draw(&p, 160, 40);
    assert_eq!(p.pages[0].diff.cursor(), 21, "at the thread's line");
}

#[test]
fn each_mode_sends_only_the_writes_it_allows_and_the_merge_says_what_it_will_do() {
    use radar_core::writes::{Change, Mode, set_mode};
    let sent = |fx: &[Effect]| fx.iter().any(|e| matches!(e, Effect::Spawn(_)));
    let said = |fx: &[Effect]| {
        fx.iter()
            .find_map(|e| match e {
                Effect::Notify(m) => Some(m.clone()),
                Effect::Notice(m) => Some(m.text.clone()),
                _ => None,
            })
            .unwrap_or_default()
    };
    for mode in Mode::ALL {
        set_mode(mode, None);
        // A comment adds: write and edit send it.
        let mut p = changes_page();
        press(&mut p, KeyCode::Char(']'));
        press(&mut p, KeyCode::Char('c'));
        typed(&mut p, "Looks right.");
        let fx = ctrl_s(&mut p);
        assert!(
            matches!(&fx[..], [Effect::Journal(_), Effect::Menu(m)] if m.change == Some(Change::Add)),
            "a comment adds"
        );
        let fx = p.on_menu("gl-write", "yes");
        assert_eq!(sent(&fx), mode != Mode::ReadOnly, "{mode:?}: comment");
        if mode == Mode::ReadOnly {
            assert_eq!(
                said(&fx),
                "✗ GitLab did not post a comment on !263: not sent: radar is read only · c has your text"
            );
            let fx = press(&mut p, KeyCode::Char('c'));
            let fx = choose_recovery(&mut p, &fx, "entry:1");
            choose_recovery(&mut p, &fx, "edit");
            assert_eq!(
                p.compose.as_ref().unwrap().text(),
                "Looks right.",
                "kept for c"
            );
        }
        // Approving changes what is there: only edit sends it.
        let mut p = changes_page();
        let fx = press(&mut p, KeyCode::Char('a'));
        assert!(
            matches!(&fx[..], [Effect::Journal(_), Effect::Menu(m)] if m.change == Some(Change::Edit)),
            "approving edits"
        );
        let fx = p.on_menu("gl-write", "yes");
        assert_eq!(sent(&fx), mode == Mode::Edit, "{mode:?}: approve");
        match mode {
            Mode::ReadOnly => assert_eq!(
                said(&fx),
                "✗ GitLab did not approve !263: not sent: radar is read only"
            ),
            Mode::Write => assert_eq!(
                said(&fx),
                "✗ GitLab did not approve !263: not sent: radar is in write mode; this needs edit mode"
            ),
            Mode::Edit => assert_eq!(said(&fx), "asking GitLab to approve !263…"),
        }
        // Resolving, rebase and merge too.
        let mut p = changes_page();
        for _ in 0..21 {
            press(&mut p, KeyCode::Down);
        }
        let fx = press(&mut p, KeyCode::Char('d'));
        assert!(
            matches!(&fx[..], [Effect::Journal(_), Effect::Menu(m)] if m.change == Some(Change::Edit)),
            "resolving edits"
        );
        assert_eq!(
            sent(&p.on_menu("gl-write", "yes")),
            mode == Mode::Edit,
            "{mode:?}: resolve"
        );
        let mut p = changes_page();
        let fx = crate::mr_actions::start(&mut p, "acme/team-a/api", 263, "rebase");
        assert_eq!(sent(&fx), mode == Mode::Edit, "{mode:?}: rebase");
    }
    set_mode(Mode::Edit, None);
    let p = changes_page();
    let mr = find_mr(&p, "acme/team-a/api", 263).unwrap().clone();
    assert!(
        matches!(&act::ask_rebase(&p, &mr)[..], [Effect::Journal(_), Effect::Menu(m)] if m.change == Some(Change::Edit)),
        "a rebase edits"
    );
    let settings = changes::detail(&p, "acme/team-a/api", 263).map(|d| d.merge);
    let m = act::merge_menu("acme/team-a/api!263", &mr, settings);
    assert_eq!(m.change, Some(Change::Edit), "a merge edits");
    assert!(
        m.question,
        "a merge is asked as a dialog, like every yes/no"
    );
    let lines: Vec<&str> = m.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(m.title, "merge !263 into main? It cannot be undone.");
    assert_eq!(
        lines[..4],
        [
            "feature/ABC-263 → main",
            "squash: yes, its commits become one",
            "feature/ABC-263: deleted after the merge",
            "GitLab checks it once more, then merges that commit",
        ]
    );
    assert_eq!(
        m.items.iter().find(|i| i.label == "yes").unwrap().value,
        "yes:1:1"
    );
}

#[test]
fn mr_page_changes_comment_multiline() {
    let mut p = changes_page();
    for _ in 0..21 {
        press(&mut p, KeyCode::Down);
    }
    press(&mut p, KeyCode::Char('c'));
    typed(&mut p, "Switched in 4f2a.\nThanks");
    insta::assert_snapshot!(hosted(p, 160, 40));
}

#[test]
fn mr_page_changes_comment() {
    let mut p = changes_page();
    for _ in 0..3 {
        press(&mut p, KeyCode::Down);
    }
    press(&mut p, KeyCode::Char('c'));
    typed(&mut p, "Cap the range at 90 days?");
    insta::assert_snapshot!(hosted(p, 160, 40));
}

#[test]
fn mr_page_threads_list() {
    let mut p = changes_page();
    press(&mut p, KeyCode::Char(']'));
    press(&mut p, KeyCode::Down);
    insta::assert_snapshot!(hosted(p, 160, 40));
}

#[test]
fn mr_page_changes_threads() {
    let mut p = changes_page();
    for _ in 0..21 {
        press(&mut p, KeyCode::Down);
    }
    insta::assert_snapshot!(hosted(p, 160, 40));
}

#[test]
fn a_file_is_fetched_when_opened_and_a_new_head_drops_the_old_diffs() {
    let mut p = mr_page();
    press(&mut p, KeyCode::Char(']'));
    let key = mr_key("acme/team-a/api", 263);
    let head = p.diffs[&key].head.clone();
    assert!(p.busy("repos"), "busy while the diff loads");
    // A page for another head is dropped.
    changes::on_diffs(&mut p, key.clone(), "old".into(), 1, Ok(fixtures::diffs()));
    assert!(p.diffs[&key].files.is_empty());
    // A failed page says why and is asked again on the next look.
    changes::on_diffs(
        &mut p,
        key.clone(),
        head.clone(),
        1,
        Err("GitLab answered 500".into()),
    );
    let text = draw(&p, 160, 40);
    assert!(
        text.contains(":error: could not load: GitLab answered 500"),
        "{text}"
    );
    let fx = press(&mut p, KeyCode::Char('n'));
    assert!(
        fx.iter().any(|e| matches!(e, Effect::SpawnAction { .. })),
        "{fx:?}"
    );
    // Clicks open a file and put the cursor on a line.
    changes::on_diffs(&mut p, key.clone(), head, 1, Ok(fixtures::diffs()));
    draw(&p, 160, 40);
    click(&mut p, "node:f:0");
    assert_eq!(p.pages[0].file.get(), 0);
    click(&mut p, "line:5");
    assert_eq!(p.pages[0].diff.cursor(), 5);
}

#[test]
fn mr_page_threads() {
    let mut p = mr_page();
    press(&mut p, KeyCode::Char(']'));
    press(&mut p, KeyCode::Char(']'));
    insta::assert_snapshot!(hosted(p, 160, 40));
}

#[test]
fn esc_goes_back_to_the_tree_with_its_cursor_where_it_was() {
    let mut p = sample();
    on_mr(&mut p, 263);
    let before = p.tree.borrow().selected().to_vec();
    press(&mut p, KeyCode::Enter);
    press(&mut p, KeyCode::Char(']'));
    press(&mut p, KeyCode::Down);
    let fx = press(&mut p, KeyCode::Esc);
    assert!(p.pages.is_empty());
    assert!(matches!(fx[..], [Effect::Focus(Pane::Middle)]));
    draw(&p, 160, 40);
    assert_eq!(p.tree.borrow().selected(), before);
    assert_eq!(p.panes("repos").len(), 3, "the tree's panes again");
}

#[test]
fn a_repo_page_opens_a_merge_request_and_esc_comes_back_to_it() {
    let mut p = sample();
    select(
        &mut p,
        |s| matches!(s, Selected::Repo(pr, _) if pr.name == "web"),
    );
    press(&mut p, KeyCode::Enter);
    // Its main is red, so it opens on the pipelines: back to the first tab.
    press(&mut p, KeyCode::Char('['));
    let text = draw(&p, 160, 40);
    assert!(text.contains("Repositories / a / web"), "{text}");
    assert!(text.contains("<:change: Merge requests>"), "{text}");
    assert!(text.contains("Pipelines on main"), "{text}");
    assert!(text.contains("!187") && text.contains("!198"), "{text}");
    press(&mut p, KeyCode::Down);
    let row = p.pages[0].row.get();
    let want = repo::mrs(&p, "acme/team-a/web")[row].mr.iid;
    press(&mut p, KeyCode::Enter);
    assert_eq!(p.pages.len(), 2);
    assert!(matches!(p.pages[1].kind, Kind::Mr { iid, .. } if iid == want));
    assert!(draw(&p, 160, 40).contains(&format!("Repositories / a / web / !{want}")));
    let fx = press(&mut p, KeyCode::Esc);
    assert!(fx.is_empty(), "back to the repo page, not the tree");
    assert_eq!(p.pages.len(), 1);
    assert_eq!(p.pages[0].row.get(), row, "on the row it was on");
    assert_eq!(
        keys(&p, Pane::Middle)
            .last()
            .map(|b| b.what.clone())
            .as_deref(),
        Some("back to the middle pane")
    );
}

#[test]
fn a_repo_page_lists_mains_pipelines() {
    let mut p = sample();
    select(
        &mut p,
        |s| matches!(s, Selected::Repo(pr, _) if pr.name == "api"),
    );
    let fx = press(&mut p, KeyCode::Enter);
    assert!(
        fx.iter().any(|e| matches!(e, Effect::SpawnAction { action, .. }
            if action.kind.as_str() == "gitlab.page.runs")),
        "fetches the pipelines"
    );
    press(&mut p, KeyCode::Char(']'));
    assert!(draw(&p, 160, 40).contains("loading…"));
    on_runs(&mut p, "acme/team-a/api".into(), Ok(fixtures::runs()));
    let text = draw(&p, 160, 40);
    for want in [
        "<:pipeline: Pipelines on main>",
        ":run_failed: failed",
        "#1842",
        "2c2ea8c3",
        "feat: bulk import",
        "push · Alice E.",
        "12m 28s",
    ] {
        assert!(text.contains(want), "lacks {want:?}:\n{text}");
    }
    let fx = press(&mut p, KeyCode::Enter);
    assert!(
        matches!(&fx[..], [Effect::Open(u)] if u.ends_with("/pipelines/2")),
        "⏎ opens the run: {fx:?}"
    );
    press(&mut p, KeyCode::Down);
    let fx = press(&mut p, KeyCode::Char('p'));
    assert!(
        matches!(&fx[..], [Effect::Open(u)] if u.ends_with("/pipelines/1")),
        "{fx:?}"
    );
}

#[test]
fn repo_page() {
    let mut p = sample();
    select(
        &mut p,
        |s| matches!(s, Selected::Repo(pr, _) if pr.name == "web"),
    );
    press(&mut p, KeyCode::Enter);
    insta::assert_snapshot!(hosted(p, 160, 40));
}

#[test]
fn a_tenancy_page_is_a_table_of_its_repos_and_opens_one() {
    let mut p = sample();
    select(&mut p, |s| matches!(s, Selected::Tenancy(1)));
    press(&mut p, KeyCode::Enter);
    let text = draw(&p, 160, 40);
    for want in [
        "Repositories / a",
        "acme/team-a",
        "3 repos · 3 merge requests",
        "<:repo: Repos>",
        "repo  main  mrs  last activity",
        ":run_failed: failed",
        "docs",
        "1 repo hidden: tools/renovate.",
    ] {
        assert!(text.contains(want), "lacks {want:?}:\n{text}");
    }
    assert!(!text.contains("renovate "), "hidden repos are not rows");
    let first = tenancy::repos(&p, "team-a")[0].project.full_path.clone();
    press(&mut p, KeyCode::Enter);
    assert!(matches!(&p.pages[1].kind, Kind::Repo(path) if *path == first));
}

#[test]
fn a_repo_whose_main_is_red_opens_on_its_pipelines() {
    let mut p = sample();
    let _ = super::push(&mut p, Kind::Repo("acme/team-a/web".into()));
    assert_eq!(p.pages.last().unwrap().tab, repo::PIPELINES);
    let _ = super::push(&mut p, Kind::Repo("acme/team-a/api".into()));
    assert_eq!(
        p.pages.last().unwrap().tab,
        0,
        "green main: merge requests first"
    );
}

#[test]
fn tenancy_page() {
    let mut p = sample();
    select(&mut p, |s| matches!(s, Selected::Tenancy(1)));
    press(&mut p, KeyCode::Enter);
    insta::assert_snapshot!(hosted(p, 160, 40));
}

#[test]
fn the_actions_menu_is_the_pages_titled_with_the_item() {
    let p = mr_page();
    // Drawn once: the overview's links are gathered while drawing.
    draw(&p, 160, 40);
    let b = keys(&p, Pane::Middle);
    assert_eq!(b[0].what, "api !263");
    assert!(b[0].is_heading());
    let rows: Vec<&str> = b[1..].iter().map(|b| b.keys.as_str()).collect();
    // ⏎ follows a link on the page; headings ("") group GitLab's actions
    // and the page's. This machine's are another part's.
    assert_eq!(
        rows,
        ["⏎", "o", "y", "r", "", "u", "m", "a", "", "]", "[", "esc"]
    );
    assert_eq!(
        b.iter().find(|b| b.keys == "⏎").unwrap().what,
        "follow a link on this page… (1 link)"
    );
    let what = |k: &str| b.iter().find(|b| b.keys == k).unwrap().what.clone();
    assert_eq!(what("u"), "update by rebasing onto main…");
    assert_eq!(what("m"), "merge · needs 1 approval");
    assert_eq!(what("]"), "next tab: Changes");
    assert_eq!(what("["), "previous tab: Commits");
}

#[test]
fn clicking_a_tab_switches_to_it() {
    let mut p = mr_page();
    click(&mut p, "tab:3");
    assert_eq!(p.pages[0].tab, 3);
}

// --- why it is red, and a job's log ---------------------------------------------

/// The sample's !342 (its pipeline red), on its Pipeline tab.
fn red_pipeline_tab() -> GitlabPlugin {
    let mut p = sample();
    on_mr(&mut p, 342);
    press(&mut p, KeyCode::Enter);
    for _ in 0..3 {
        press(&mut p, KeyCode::Char(']'));
    }
    p
}

/// Its first failed job's page, opened with ⏎.
fn job_page() -> GitlabPlugin {
    let mut p = red_pipeline_tab();
    draw(&p, 160, 40);
    press(&mut p, KeyCode::Enter);
    p
}

const WORKER: &str = "acme/team-b/worker";

#[test]
fn a_red_pipeline_leads_with_its_failed_jobs_and_the_lines_that_say_why() {
    let p = red_pipeline_tab();
    let text = draw(&p, 160, 40);
    for want in [
        "<:pipeline: Pipeline !>",
        ":run_failed: failed · 6m 12s",
        "WHY IT IS RED",
        ":run_failed: unit-test",
        "test · 1m 14s · the script failed",
        "│ > Task :compileJava",
        "│ StatsControllerTest > returnsTheCachedView() FAILED",
        "│     org.opentest4j.AssertionFailedError: expected: <200> but was: <500>",
        "│ 3 tests completed, 1 failed",
        ":run_failed: lint",
        "the script failed · allowed to fail",
    ] {
        assert!(text.contains(want), "lacks {want:?}:\n{text}");
    }
    assert!(
        !text.contains("ERROR: Job failed"),
        "not the runner's sign-off:\n{text}"
    );
    // The one that made it red comes first, though GitLab listed lint first.
    assert!(text.find(":run_failed: unit-test").unwrap() < text.find(":run_failed: lint").unwrap());
    // Two rows to choose, each opening its job.
    assert_eq!(list_len(&p, p.pages.last().unwrap()), Some(2));
    let b = keys(&p, Pane::Middle);
    assert_eq!(b[1].keys, "⏎");
    assert_eq!(b[1].what, "open unit-test's log");
}

#[test]
fn the_pipeline_tab_is_still_its_jobs_when_none_failed() {
    let mut p = mr_page();
    for _ in 0..3 {
        press(&mut p, KeyCode::Char(']'));
    }
    let text = draw(&p, 160, 40);
    assert!(
        text.contains("ALL JOBS") && text.contains(":run_passed: build"),
        "{text}"
    );
    assert!(!text.contains("WHY IT IS RED"), "it passed:\n{text}");
    // Every job is a row; ⏎ opens one's log page.
    assert_eq!(list_len(&p, p.pages.last().unwrap()), Some(3));
    press(&mut p, KeyCode::Down);
    let fx = press(&mut p, KeyCode::Enter);
    assert!(
        fx.iter().any(|e| matches!(e, Effect::SpawnAction { action, .. }
            if action.kind.as_str() == "gitlab.job.log")),
        "its log is fetched: {fx:?}"
    );
    assert_eq!(
        p.pages.last().unwrap().kind,
        Kind::Job {
            project: "acme/team-a/api".into(),
            id: 1002
        }
    );
    // Its title is what the list knew until GitLab answers.
    let text = draw(&p, 160, 40);
    assert!(
        text.contains("unit-test") && text.contains(":run_passed: success · stage test · 3m 42s"),
        "{text}"
    );
    assert!(text.contains("loading…"), "{text}");
}

#[test]
fn a_jobs_page_shows_its_log_at_the_first_error_and_its_sections() {
    let p = job_page();
    assert_eq!(
        p.pages.last().unwrap().kind,
        Kind::Job {
            project: WORKER.into(),
            id: 5101
        }
    );
    let text = draw(&p, 160, 40);
    for want in [
        "Repositories / b / worker / !342 / unit-test",
        ":run_failed: failed · stage test · 1m 14s · the script failed",
        "<:log: Log>",
        "Running with gitlab-runner 17.0",
        "─ Preparing the \"docker\" executor · 4s ⋯ 1 line",
        "┌ Executing \"step_script\" stage of the job script · 1m 14s",
        "│ $ ./gradlew test",
        "│ StatsControllerTest > returnsTheCachedView() FAILED",
        "ERROR: Job failed: exit code 1",
    ] {
        assert!(text.contains(want), "lacks {want:?}:\n{text}");
    }
    assert!(
        !text.contains("Using docker image"),
        "the runner's set-up is folded:\n{text}"
    );
    let page = p.pages.last().unwrap();
    let log = &p.logs[&JobKey::new(WORKER, 5101)].value.as_ref().unwrap().text;
    assert_eq!(
        page.log.line(log),
        Some(5),
        "the cursor is on the first error"
    );
}

#[test]
fn a_jobs_page_searches_its_log_and_esc_clears_the_search_before_going_back() {
    let mut p = job_page();
    press(&mut p, KeyCode::Char('/'));
    assert!(p.capturing(), "typing goes to the search");
    for c in "TASK".chars() {
        press(&mut p, KeyCode::Char(c));
    }
    let fx = press(&mut p, KeyCode::Enter);
    assert!(
        matches!(&fx[..], [Effect::Notify(m)] if m == "2 lines match · n and N go between them"),
        "{fx:?}"
    );
    let text = draw(&p, 160, 40);
    assert!(text.contains("log · /TASK · 1 of 2"), "{text}");
    press(&mut p, KeyCode::Char('n'));
    assert!(draw(&p, 160, 40).contains("log · /TASK · 2 of 2"));
    let b = keys(&p, Pane::Middle);
    assert!(
        b.iter()
            .any(|b| b.keys == "N" && b.what == "previous line with TASK")
    );
    let previous = b.iter().find(|binding| binding.keys == "N").unwrap();
    let action = radar_core::view::ActionSpec::named("gitlab.job.previous_match", previous);
    assert_eq!(action.mnemonic_at, Some(11));
    assert!(action.mnemonic_shift);
    assert_eq!(b.last().map(|b| b.what.as_str()), Some("clear the search"));
    press(&mut p, KeyCode::Esc);
    assert_eq!(p.pages.len(), 2, "esc cleared the search first");
    assert!(draw(&p, 160, 40).contains("middle · log"));
    press(&mut p, KeyCode::Esc);
    assert_eq!(p.pages.len(), 1, "then back to !342");
    assert_eq!(p.pages[0].tab, 3, "on its Pipeline tab");
}

#[test]
fn a_jobs_actions_open_it_its_pipeline_and_copy_its_link_and_excerpt() {
    let mut p = job_page();
    let fx = press(&mut p, KeyCode::Char('o'));
    assert!(
        matches!(&fx[..], [Effect::Open(u)] if u == "https://gitlab.com/acme/team-b/worker/-/jobs/5101"),
        "{fx:?}"
    );
    let fx = press(&mut p, KeyCode::Char('p'));
    assert!(
        matches!(&fx[..], [Effect::Open(u)] if u == "https://gitlab.com/acme/team-b/worker/-/pipelines/9001"),
        "{fx:?}"
    );
    let fx = press(&mut p, KeyCode::Char('y'));
    assert!(
        matches!(&fx[0], Effect::Copy(u) if u.ends_with("/-/jobs/5101")),
        "{fx:?}"
    );
    let fx = press(&mut p, KeyCode::Char('c'));
    let Effect::Copy(text) = &fx[0] else {
        panic!("{fx:?}")
    };
    assert!(
        text.starts_with("> Task :compileJava\n> Task :test\nStatsControllerTest"),
        "{text}"
    );
    assert!(text.ends_with("3 tests completed, 1 failed"), "{text}");
    let fx = press(&mut p, KeyCode::Char('r'));
    assert!(
        matches!(&fx[..], [Effect::SpawnAction { action, .. }]
            if action.kind.as_str() == "gitlab.job.log"),
        "r loads the log again: {fx:?}"
    );
    let rows: Vec<String> = keys(&p, Pane::Middle)
        .iter()
        .map(|b| b.keys.clone())
        .collect();
    assert_eq!(
        rows,
        ["", "o", "p", "y", "r", "c", "", "/", "space", "", "esc"]
    );
}

#[test]
fn mains_red_pipeline_leads_the_repo_pages_pipelines_and_a_long_log_pages_in() {
    let mut p = sample();
    select(
        &mut p,
        |s| matches!(s, Selected::Repo(pr, _) if pr.name == "web"),
    );
    let fx = press(&mut p, KeyCode::Enter);
    assert!(
        fx.iter().any(|e| matches!(e, Effect::SpawnAction { action, .. }
            if action.kind.as_str() == "gitlab.page.runs")),
        "main's runs are fetched"
    );
    // It opens on the pipelines tab, main being red.
    on_runs(&mut p, "acme/team-a/web".into(), Ok(fixtures::runs()));
    let text = draw(&p, 160, 40);
    for want in [
        "why main is red",
        ":run_failed: unit-test",
        "test · 1m 14s · it ran out of time",
        "│ no error recognised; its last lines:",
        "│ still waiting (20m)",
        "pipelines on main",
        "#1842",
    ] {
        assert!(text.contains(want), "lacks {want:?}:\n{text}");
    }
    // The failed job, then the runs: ⏎ on the job opens its page.
    assert_eq!(repo::len(&p, "acme/team-a/web", repo::PIPELINES), 3);
    press(&mut p, KeyCode::Down);
    let fx = press(&mut p, KeyCode::Enter);
    assert!(
        matches!(&fx[..], [Effect::Open(u)] if u.ends_with("/pipelines/2")),
        "{fx:?}"
    );
    press(&mut p, KeyCode::Up);
    press(&mut p, KeyCode::Enter);
    assert_eq!(
        p.pages.last().unwrap().kind,
        Kind::Job {
            project: "acme/team-a/web".into(),
            id: 5001
        }
    );
    let text = draw(&p, 160, 40);
    assert!(text.contains("<:log: Log>"), "{text}");
    assert!(
        text.contains("⋯ 878 KB earlier in the log, loaded when you reach the top"),
        "{text}"
    );
    // Reaching the top asks for the part before, once.
    let fx = press(&mut p, KeyCode::Char('g'));
    assert!(matches!(&fx[..], [Effect::SpawnAction { .. }]), "{fx:?}");
    assert!(
        press(&mut p, KeyCode::Up).is_empty(),
        "not again while it comes"
    );
    assert!(draw(&p, 160, 40).contains("⋯ loading the part before…"));
    let before = p
        .pages
        .last()
        .unwrap()
        .log
        .line(&p.logs[&JobKey::new("acme/team-a/web", 5001)].value.as_ref().unwrap().text);
    let key = JobKey::new("acme/team-a/web", 5001);
    let earlier = p.log_reads[&key].earlier.unwrap();
    let request = crate::log_ownership::LogRead {
        owner: earlier.owner, earlier: Some(earlier),
        action: crate::log_ownership::action(&p.cfg.url, &key), key,
    };
    crate::why::on_log(
        &mut p,
        request,
        None,
        Ok(crate::model::TracePart {
            // Its last line goes on in the part held: `… cut line`.
            text: "set\nstarting the deploy\nthe line that was ".into(),
            start: 0,
            total: 900_131,
        }),
    );
    let log = p.logs[&JobKey::new("acme/team-a/web", 5001)].value.as_ref().unwrap();
    assert!(!log.earlier());
    let after = p.pages.last().unwrap().log.line(&log.text);
    assert_eq!(after, before.map(|l| l + 3), "the cursor stays on its line");
    assert!(draw(&p, 160, 40).contains("<:log: Log>"));
}

#[test]
fn the_preview_of_a_red_merge_request_says_which_job_failed_and_why() {
    let mut p = sample();
    on_mr(&mut p, 342);
    let text = draw(&p, 160, 40);
    assert!(
        text.contains(":error: unit-test and 1 more failed: the script"),
        "{text}"
    );
    // Not known yet: moving onto it asks GitLab which jobs failed (no logs).
    let mut p = sample();
    p.failed.clear();
    p.view = 0;
    draw(&p, 160, 40);
    let mut asked = false;
    for _ in 0..10 {
        let fx = press(&mut p, KeyCode::Down);
        draw(&p, 160, 40);
        asked |= fx.iter().any(|e| matches!(e, Effect::SpawnAction { action, .. } if action.kind.as_str() == "gitlab.pipeline.diagnosis"));
        if matches!(screen::selected(&p, p.visible()), Some(Selected::Mr(it)) if it.mr.iid == 342) {
            break;
        }
    }
    assert!(!asked, "not while it is passed over");
    // Once the selection has rested on the red row, its jobs are asked.
    let mut asked = false;
    for _ in 0..3 {
        asked |= p.tick(p.now).iter().any(|e| matches!(e, Effect::SpawnAction { action, .. } if action.kind.as_str() == "gitlab.pipeline.diagnosis"));
    }
    assert!(asked, "the red one's failed jobs were asked for");
    assert!(p.failed.contains_key("acme/team-b/worker#9001"));
}

#[test]
fn the_red_pipeline_tab() {
    insta::assert_snapshot!(hosted(red_pipeline_tab(), 160, 40));
}

#[test]
fn the_red_pipeline_tab_narrow() {
    insta::assert_snapshot!(hosted(red_pipeline_tab(), 80, 24));
}

#[test]
fn the_job_page() {
    insta::assert_snapshot!(hosted(job_page(), 160, 40));
}

#[test]
fn neutral_job_log_keeps_search_and_fold_keys_and_row_targets() {
    let p = job_page();
    let mut host = radar_core::Host::new(vec![Box::new(p)], 1).unwrap();
    host.view = radar_core::host::View::Screen(0);
    host.focus = Pane::Middle;
    let shown = crate::tests::shown(&host, 80, 24);
    assert!(shown.contains("StatsControllerTest"), "{shown}");
    // A shown log row has a target: `log:<row>`, as the text form numbers it.
    let view = host.screen_view(0, "repos").expect("the screen");
    let row = view
        .page
        .iter()
        .flat_map(|page| page.panes.iter())
        .find_map(|pane| match &pane.body {
            radar_core::view::Node::Log { lines, .. } => lines.first().map(|line| line.row),
            _ => None,
        })
        .expect("a visible log row has a target");
    host.update(radar_core::host::Event::Intent(radar_core::view::Intent::Select {
        target: radar_core::view::Target::new(format!("log:{row}")),
    }));
    host.update(radar_core::host::Event::Key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)));
    host.update(radar_core::host::Event::Paste("TASK".into()));
    host.update(radar_core::host::Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
    let shown = crate::tests::shown(&host, 80, 24);
    assert!(shown.contains("log · /TASK · 1 of 2"), "{shown}");
    host.update(radar_core::host::Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)));
    assert!(crate::tests::shown(&host, 80, 24).contains("log · /TASK · 2 of 2"));
    host.update(radar_core::host::Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    host.update(radar_core::host::Event::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE)));
    host.update(radar_core::host::Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)));
    host.update(radar_core::host::Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)));
    assert!(crate::tests::shown(&host, 80, 24).contains("Using docker image"));
}

#[test]
fn refresh_refetches_and_the_merge_request_is_updated_in_the_list_too() {
    let mut p = mr_page();
    let fx = press(&mut p, KeyCode::Char('r'));
    assert!(
        matches!(&fx[..], [Effect::SpawnAction { action, .. }]
            if action.kind.as_str() == "gitlab.page.detail"),
        "{fx:?}"
    );
    let mut mr = find_mr(&p, "acme/team-a/api", 263).unwrap().clone();
    mr.title = "renamed".into();
    on_mr_page(
        &mut p,
        mr_key("acme/team-a/api", 263),
        Ok((mr, fixtures::detail())),
    );
    assert_eq!(
        find_mr(&p, "acme/team-a/api", 263).unwrap().title,
        "renamed"
    );
    assert!(p.visible().iter().any(|it| it.mr.title == "renamed"));
}

/// A dependency update's description, as such bots write it: a comment, a
/// table of badges, a folded section of release notes.
const UPDATE: &str = r#"<!-- bot:header -->
This merge request updates one dependency.

| Package | Change | Age | Confidence |
|---|---|---|---|
| [acme/widget-lib](https://example.com/widget-lib) | `1.2.2` -> `1.2.3` | [![age](https://img.example.com/age.svg)](https://example.com/age) | [![confidence](https://img.example.com/c.svg)](https://example.com/confidence) |

---

### Release Notes

<details>
<summary>acme/widget-lib (widget-lib)</summary>

#### [`v1.2.3`](https://example.com/v1.2.3)

- Fixes a crash when the shelf is empty

</details>

<!--bot:debug:{"x":1}-->
"#;

#[test]
fn a_dependency_updates_description_reads_as_text_with_its_table_as_rows() {
    let mut p = mr_page();
    let key = mr_key("acme/team-a/api", 263);
    if let Some((_, d)) = p.mr_pages.get_mut(&key).and_then(|f| f.value.as_mut()) {
        d.description = UPDATE.into();
    }
    let text = draw(&p, 160, 40);
    for want in [
        "This merge request updates one dependency.",
        "Package | Change | Age | Confidence",
        "| `1.2.2` -> `1.2.3` |",
        "Release Notes",
        "acme/widget-lib (widget-lib)",
        "• Fixes a crash when the shelf is empty",
    ] {
        assert!(text.contains(want), "lacks {want:?}:\n{text}");
    }
    for never in ["<!--", "<details>", "<summary>", "](https", "![", "|---"] {
        assert!(!text.contains(never), "shows {never:?}:\n{text}");
    }
    // Its links are one key away (⏎), each shown the radar way: a plain link
    // opens as a link.
    let fx = press(&mut p, KeyCode::Enter);
    let Some(Effect::Menu(m)) = fx.first() else {
        panic!("no menu: {fx:?}");
    };
    let labels: Vec<&str> = m.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(
        labels[0].trim_end(),
        "acme/widget-lib",
        "named by its words: {labels:?}"
    );
    let links = p.pages[0].links.borrow().clone();
    let at = links
        .iter()
        .position(|s| s.url.as_deref() == Some("https://example.com/v1.2.3"))
        .expect("the release link");
    assert_eq!(links.len(), m.items.len());
    let id = m.id.clone();
    let fx = p.on_menu(&id, &at.to_string());
    assert!(
        matches!(fx.as_slice(), [Effect::Show(s)] if s.url.as_deref() == Some("https://example.com/v1.2.3")),
        "{fx:?}"
    );
    assert!(
        radar_core::Plugin::keys(&p, "repos", Pane::Middle)
            .iter()
            .any(|b| b.keys == "⏎" && b.what.starts_with("follow a link")),
        "the x menu says so"
    );
    insta::assert_snapshot!("mr_page_update_description", text);
}

fn choose_recovery(p: &mut GitlabPlugin, effects: &[Effect], value: &str) -> Vec<Effect> {
    let menu = effects
        .iter()
        .find_map(|e| {
            if let Effect::Menu(m) = e {
                Some(m)
            } else {
                None
            }
        })
        .expect("recovery menu");
    p.on_menu(&menu.id, value)
}

mod mnemonic_tests {
    use super::*;

    #[test]
    fn page_action_producers_have_mnemonics_across_review_and_diff_states() {
        let (mut p, fixture) = crate::test_support::hosted_plugin(vec![]).unwrap();
        let item = crate::test_support::MrBuilder::default().build();
        let detail = crate::model::fixtures::detail();
        let head = detail.refs.as_ref().unwrap().head.clone();
        let key = mr_key(&item.project_path, item.iid);
        p.mr_pages.insert(key.clone(), crate::pages::Fetched {
            value: Some((item.clone(), detail)), at: p.now, ..Default::default()
        });
        p.diffs.insert(key, crate::pages::changes::Diffs {
            head,
            files: crate::model::fixtures::diffs().into_iter().map(|file| (file.new_path.clone(), file)).collect(),
            asked: [1].into(), ended: Some(1), ..Default::default()
        });
        p.pages.push(Page::new(Kind::Mr { project: item.project_path.clone(), iid: item.iid }));
        let mut checked = 0;
        let mut shifted = false;
        for split in [false, true] {
            p.cfg.side_by_side = split;
            for tab in 0..mr::TABS.len() {
                p.pages.last_mut().unwrap().tab = tab;
                for file in 0..3 {
                    p.pages.last().unwrap().file.set(file);
                    for pane in [Pane::Nav, Pane::Middle, Pane::Detail] {
                        for (id, binding) in crate::pages::keys(&p, pane) {
                            if radar_core::mnemonic::letter(&binding.keys).is_some() {
                                assert!(radar_core::mnemonic::at(&binding.keys, &binding.what).is_some(),
                                    "{id}: {} / {}", binding.keys, binding.what);
                                let action = radar_core::view::ActionSpec::named(id, &binding);
                                assert_eq!(action.mnemonic_shift, radar_core::mnemonic::shifted(&binding.keys));
                                shifted |= binding.keys == "N";
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(checked > 20);
        assert!(shifted, "exercise the previous-file uppercase binding");
        p.acting.insert(mr_key(&item.project_path, item.iid), crate::mr_actions::Act {
            what: "rebase", state: crate::mr_actions::Acting::Running, at: p.now,
        });
        let bindings = crate::pages::act::bindings(&p, p.pages.last().unwrap());
        let (_, rebase) = bindings.iter().find(|(id, _)| id == "gitlab.mr.rebase").unwrap();
        assert_eq!((&*rebase.keys, &*rebase.what), ("u", "update by rebasing · asked, waiting for GitLab"));
        fixture.assert_finished();
    }
}
