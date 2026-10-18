//! GitHub settings as renderer-neutral rows and intents.

use radar_core::plugin::Outcome;
use radar_core::ui::connection;
use radar_core::ui::icon::Icon;
use radar_core::view::{self as v, Ink, ListRow, Node, SectionView, Seg, Text};

use super::{ConnRow, GithubPlugin};

/// A muted capitals heading and what it takes, as a list row draws it.
fn heading(words: &str, about: &str) -> Text {
    Text::plain(" ")
        .push(Seg::new(words.to_uppercase(), Ink::Muted).strong())
        .push(Seg::new(format!("  {about}"), Ink::Muted))
}

/// A choice's mark: the done icon on the one in use, else the to-do one;
/// with icons off, `[x]` and `[ ]`.
fn radio(on: bool) -> Seg {
    if on {
        Seg::icon_or_lead(Icon::Done, "[x] ", Ink::Good)
    } else {
        Seg::icon_or_lead(Icon::Todo, "[ ] ", Ink::Muted)
    }
}

/// The token as the connection shows it: by what it is, never its value.
fn token_words(token: &str) -> String {
    match super::Access::of(token) {
        super::Access::Public => "none: public access".into(),
        super::Access::Gh => "gh: gh's sign-in".into(),
        super::Access::Token if token.starts_with("env:") => token.into(),
        super::Access::Token => "a token typed in (hidden)".into(),
    }
}

pub fn view(p: &GithubPlugin, section: &str, cx: &v::ViewCx<'_>) -> Option<SectionView> {
    if section == "repositories" {
        let typing = p.editor.as_ref().map(|editor| super::validate(&editor.field, &editor.input.value()));
        let mut detail = vec![Text::of(
            "Create groups and choose repositories. Press a to browse, e to edit, d to remove, or m to move a repository.",
            Ink::Secondary,
        )];
        if let Some(result) = typing {
            detail.push(Text::of("as you type", Ink::Muted));
            detail.push(Text::of(result.unwrap_or_else(|reason| reason), Ink::Info));
        }
        return Some(SectionView {
            version: v::VERSION,
            scope: section.into(),
            body: Node::Tree(repositories(p)),
            detail_title: super::title(p, section),
            detail_blocks: vec![],
            detail,
            actions: super::keys(p, section).iter().map(crate::action_spec).collect(),
        });
    }
    if section == "connection" {
        p.setup_card.set(cx.setup);
    }
    let (rows, selected) = match section {
        "connection" => {
            let width = cx.width;
            let now = super::Access::of(&p.cfg.token);
            let rows = super::conn_rows(p)
                .into_iter()
                .enumerate()
                .map(|(i, row)| {
                    let target = format!("setting:{i}");
                    let field = |label: &str, value: Text| {
                        ListRow::new(target.clone(), connection::field_text(Text::plain(" "), label, value))
                    };
                    match row {
                        ConnRow::Gap(_) => ListRow::heading(target, Text::new()),
                        ConnRow::AccessHeading => ListRow::heading(target, heading("sign in", "how radar reads GitHub")),
                        ConnRow::FollowHeading => {
                            ListRow::heading(target, heading("repositories", "github.com/owner, owner/repo or a group name"))
                        }
                        ConnRow::Access(a) => ListRow::new(
                            target,
                            Text::plain(" ")
                                .push(radio(a == now))
                                .push(Seg::new(radar_core::ui::pad(a.label(), 16), Ink::Primary))
                                .push(Seg::new(a.about(), Ink::Muted)),
                        ),
                        ConnRow::GrantHeading => ListRow::heading(target, heading("access", "what radar may do on GitHub")),
                        ConnRow::Grant(a) => ListRow::new(
                            target,
                            Text::plain(" ")
                                .push(radio(a == p.cfg.access))
                                .push(Seg::new(radar_core::ui::pad(a.label(), 16), Ink::Primary))
                                .push(Seg::new(super::grant_about(a), Ink::Muted)),
                        ),
                        ConnRow::Server => field("server", Text::of(p.cfg.api.clone(), Ink::Primary)),
                        ConnRow::Token => field(connection::TOKEN, Text::of(token_words(&p.cfg.token), Ink::Primary)),
                        ConnRow::Check => ListRow::new(
                            target,
                            connection::check_field_text(
                                Text::plain(" "),
                                p.connection_check.as_ref().unwrap_or(&radar_core::ui::status::Check::NotYet),
                                "GitHub",
                            ),
                        ),
                        ConnRow::Refresh => field(connection::REFRESH, Text::of("once on start, then on request", Ink::Primary)),
                        ConnRow::FollowAdd => ListRow::new(
                            target,
                            v::kit::add_line("add a repository or group", "paste or type it", width),
                        ),
                        ConnRow::Follow(g) => {
                            let group = &p.cfg.groups[g];
                            let n = group.repositories.len();
                            ListRow::new(
                                target,
                                Text::plain("   ")
                                    .push(Seg::icon(Icon::Group))
                                    .push(Seg::new(group.name.clone(), Ink::Emphasis).strong())
                                    .push(Seg::new(
                                        format!("   {n} repositor{}", if n == 1 { "y" } else { "ies" }),
                                        Ink::Muted,
                                    )),
                            )
                        }
                    }
                })
                .collect();
            (rows, super::conn_at(p))
        }
        _ => return None,
    };
    let editing = p.editor.as_ref().map(|e| v::Editing {
        text: e.input.value(),
        caret: e.input.cursor(),
        row: selected,
        at: match (section, super::conn_row(p)) {
            ("connection", ConnRow::FollowAdd) => 4,
            ("connection", _) => 1 + connection::value_x(1),
            _ => 1,
        },
    });
    let typing = p
        .editor
        .as_ref()
        .map(|e| super::validate(&e.field, &e.input.value()));
    let row = super::conn_row(p);
    let detail_lines = if section == "connection" {
        let grant;
        let mut d = connection::DetailText::new(match row {
            ConnRow::Grant(a) => {
                grant = super::grant_detail(a);
                grant.as_str()
            }
            ConnRow::Access(super::Access::Public) => "No token: radar reads public repositories only.",
            ConnRow::Access(super::Access::Gh) => {
                "The gh program's sign-in: radar asks gh auth token when it reads, holds the answer a minute, and keeps only the word gh in its config."
            }
            ConnRow::Access(super::Access::Token) | ConnRow::Token => {
                "Paste a GitHub token, use env:NAME, gh for gh's sign-in, or leave empty for public access. Private repositories require read permission."
            }
            ConnRow::Server => super::API_ABOUT,
            ConnRow::Check => "Check the signed-in account or public API access.",
            ConnRow::Refresh => "Repositories load on start and when explicitly refreshed.",
            ConnRow::FollowAdd | ConnRow::FollowHeading | ConnRow::Gap(_) | ConnRow::AccessHeading | ConnRow::GrantHeading => {
                "Follow an owner's repositories, one repository, or a group of your own."
            }
            ConnRow::Follow(_) => "A group: ⏎ browses what the token can read in it; d removes it.",
        });
        match row {
            ConnRow::Server => {
                d.accepts = Some(super::API_ACCEPTS.into());
                d.now = Some(p.cfg.api.clone());
                d.config = Some("[github] api".into());
            }
            ConnRow::Token | ConnRow::Access(_) => {
                d.accepts = Some("empty, gh, env:NAME, or a token".into());
                d.now = Some(token_words(&p.cfg.token));
                d.config = Some("[github] token".into());
            }
            ConnRow::Grant(_) => {
                d.accepts = Some("read or write".into());
                d.now = Some(p.cfg.access.label().into());
                d.config = Some("[github] access".into());
            }
            _ => {}
        }
        d.typing = typing;
        d.text()
    } else {
        let mut lines = vec![Text::of(
            "Create groups and choose repositories. Press a to browse, e to edit, d to remove, or m to move a repository.",
            Ink::Secondary,
        )];
        if let Some(result) = typing {
            lines.push(Text::of("as you type", Ink::Muted));
            lines.push(Text::of(result.unwrap_or_else(|reason| reason), Ink::Info));
        }
        lines
    };
    Some(SectionView {
        version: v::VERSION,
        scope: section.into(),
        body: Node::List {
            rows,
            selected: Some(selected),
            editing,
        },
        detail_title: super::title(p, section),
        detail_blocks: if section == "connection" {
            // The row's detail, then the help that row needs.
            let help = match row {
                ConnRow::Server | ConnRow::Refresh => vec![],
                ConnRow::FollowAdd | ConnRow::FollowHeading | ConnRow::Follow(_) | ConnRow::Gap(2..) => {
                    crate::setup::follow_help()
                }
                _ => crate::setup::access_help(p),
            };
            let mut blocks: Vec<v::Block> = detail_lines.iter().cloned().map(v::Block::line).collect();
            if !help.is_empty() {
                blocks.push(v::Block::line(Text::new()));
                blocks.extend(help);
            }
            blocks
        } else {
            vec![]
        },
        detail: detail_lines,
        actions: super::keys(p, section)
            .iter()
            .map(crate::action_spec)
            .collect(),
    })
}

/// The complete held settings hierarchy. Folding changes visibility only;
/// identity paths keep edits and delete confirmations tied to the same item.
pub(super) fn repositories(p: &GithubPlugin) -> v::Tree {
    let mut rows = vec![v::TreeRow::new("add", Text::plain("create a group or add a repository"))];
    let mut open = Vec::new();
    for group in &p.cfg.groups {
        let id = super::group_id(&group.name);
        if !p.repository_folded.contains(&id) { open.push(vec![id.clone()]); }
        let mut row = v::TreeRow::new(id, Text::from(vec![
            Seg::icon(Icon::Group), Seg::new(&group.name, Ink::Emphasis),
        ]));
        row.children = group.repositories.iter().map(|full| {
            let name = group.owner.as_ref()
                .and_then(|owner| full.strip_prefix(&format!("{owner}/")))
                .unwrap_or(full);
            v::TreeRow::new(format!("repo:{full}"), Text::plain(name))
        }).collect();
        rows.push(row);
    }
    let selected = super::path(p, super::selected(p));
    let editing = p.editor.as_ref().and_then(|editor| {
        let path = match &editor.field {
            super::Field::Add => vec!["add".into()],
            super::Field::RenameGroup(name) => vec![super::group_id(name)],
            super::Field::Repository(group, repo) => vec![super::group_id(group), format!("repo:{repo}")],
            super::Field::MoveToNewGroup(repo) => super::all_rows(p).into_iter().find_map(|row| match row {
                super::Row::Repository(g, r) if p.cfg.groups[g].repositories[r] == *repo => Some(super::path(p, row)),
                _ => None,
            })?,
            super::Field::Api | super::Field::Token => return None,
        };
        Some(Box::new(v::TreeEditing { path, at: 0, text: editor.input.value(), caret: editor.input.cursor() }))
    });
    v::Tree { rows, selected, open, editing, ..v::Tree::default() }
}

pub fn intent(p: &mut GithubPlugin, section: &str, intent: &v::Intent) -> Outcome {
    if !matches!(section, "connection" | "repositories") {
        return Outcome::Ignored;
    }
    if p.editor.is_some() && !matches!(intent, v::Intent::Key { .. } | v::Intent::Invoke { .. }) {
        return Outcome::Ignored;
    }
    match intent {
        v::Intent::Select { target }
        | v::Intent::Context { target }
        | v::Intent::Activate { target } => {
            let out = super::click(p, section, target.as_str());
            if matches!(intent, v::Intent::Activate { .. }) && matches!(out, Outcome::Handled(_)) {
                return super::key(p, section, v::Chord::new(v::Key::Enter));
            }
            out
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => {
            let rows = super::keys(p, section);
            crate::chord_for(&rows, intent)
                .map_or(Outcome::Ignored, |chord| super::key(p, section, chord))
        }
        v::Intent::Fold { target, open } if section == "repositories" => super::fold(p, target, *open),
        v::Intent::OpenAll | v::Intent::Fold { .. }
        | v::Intent::Show { .. }
        | v::Intent::End { .. }
        | v::Intent::EditSelect { .. }
        | v::Intent::EditUnselect
        | v::Intent::Dock(_) | v::Intent::Copy { .. } | v::Intent::NoticeInput { .. } | v::Intent::DismissNotice { .. } => Outcome::Ignored,
    }
}

#[cfg(test)]
mod tree_tests {
    use super::*;
    use crate::{config::{Group, Settings}, settings};
    use radar_core::input::TextInput;
    use radar_core::plugin::Effect;

    fn fixture() -> GithubPlugin {
        let mut p = GithubPlugin::default();
        p.apply_config(Settings {
            groups: vec![
                Group { name: "Watch".into(), owner: None, repositories: vec!["ankh-morpork/watch".into(), "ankh-morpork/guilds".into()] },
                Group { name: "Witches".into(), owner: None, repositories: vec!["lancre/witches".into()] },
            ],
            ..Settings::default()
        });
        p
    }

    fn target(parts: &[&str]) -> v::Target {
        v::TreeRow::target_of(&parts.iter().map(|part| (*part).into()).collect::<Vec<_>>())
    }

    fn select(p: &mut GithubPlugin, target: &v::Target) -> Outcome {
        intent(p, "repositories", &v::Intent::Select { target: target.clone() })
    }

    fn press(p: &mut GithubPlugin, key: v::Key) -> Outcome {
        intent(p, "repositories", &v::Intent::Key { chord: v::Chord::new(key) })
    }

    #[test]
    fn settings_paths_selection_and_folds_survive_reorder_and_refresh() {
        let mut p = fixture();
        let repo = target(&["group:Watch", "repo:ankh-morpork/guilds"]);
        assert!(matches!(select(&mut p, &repo), Outcome::Handled(_)));
        let mut cfg = p.cfg.clone();
        cfg.groups.reverse();
        cfg.groups[1].repositories.reverse();
        p.apply_config(cfg);
        assert_eq!(v::TreeRow::target_of(&repositories(&p).selected), repo);
        let before = repositories(&p);
        let step = v::tree_step(&before, v::Chord::char('-')).unwrap();
        assert!(matches!(intent(&mut p, "repositories", &step), Outcome::Handled(_)));
        let folded = repositories(&p);
        assert_eq!(folded.rows, before.rows);
        assert_eq!(folded.selected, ["group:Watch"]);
        assert!(!folded.open.contains(&vec!["group:Watch".into()]));
        assert!(matches!(select(&mut p, &repo), Outcome::Ignored));
        p.apply_config(p.cfg.clone());
        assert_eq!(repositories(&p), folded);
        assert!(matches!(intent(&mut p, "repositories", &v::Intent::Fold { target: target(&["group:Watch"]), open: true }), Outcome::Handled(_)));
        assert_eq!(repositories(&p).selected, ["group:Watch"]);
        assert!(matches!(select(&mut p, &repo), Outcome::Handled(_)));
    }

    #[test]
    fn settings_reject_wrong_ancestors_removed_targets_and_leaf_folds() {
        let mut p = fixture();
        let repo = target(&["group:Watch", "repo:ankh-morpork/watch"]);
        assert!(matches!(select(&mut p, &target(&["group:Witches", "repo:ankh-morpork/watch"])), Outcome::Ignored));
        assert!(matches!(intent(&mut p, "repositories", &v::Intent::Fold { target: repo.clone(), open: false }), Outcome::Ignored));
        select(&mut p, &repo);
        let mut cfg = p.cfg.clone();
        cfg.groups[0].repositories.remove(0);
        p.apply_config(cfg);
        assert!(matches!(select(&mut p, &repo), Outcome::Ignored));
        assert_eq!(repositories(&p).selected, ["group:Watch"]);
        p.apply_config(Settings::default());
        assert_eq!(repositories(&p).rows.len(), 1);
        assert_eq!(repositories(&p).selected, ["add"]);
        assert!(matches!(intent(&mut p, "repositories", &v::Intent::Fold { target: target(&["add"]), open: false }), Outcome::Ignored));
    }

    #[test]
    fn repository_editor_keeps_identity_and_captures_fold_keys_after_reordering() {
        let mut p = fixture();
        let repo = target(&["group:Watch", "repo:ankh-morpork/watch"]);
        select(&mut p, &repo);
        assert!(matches!(press(&mut p, v::Key::Char('e')), Outcome::Handled(_)));
        let editing = repositories(&p).editing.unwrap();
        assert_eq!(v::TreeRow::target_of(&editing.path), repo);
        assert_eq!(editing.text, "ankh-morpork/watch");
        assert!(matches!(intent(&mut p, "repositories", &v::Intent::Fold { target: target(&["group:Watch"]), open: false }), Outcome::Ignored));
        assert!(matches!(select(&mut p, &target(&["group:Witches"])), Outcome::Ignored));
        let mut cfg = p.cfg.clone();
        cfg.groups.reverse();
        cfg.groups[1].repositories.reverse();
        p.apply_config(cfg);
        p.editor.as_mut().unwrap().input = TextInput::new("ankh-morpork/patrol");
        press(&mut p, v::Key::Enter);
        assert_eq!(p.cfg.groups[1].repositories, ["ankh-morpork/guilds", "ankh-morpork/patrol"]);
        assert_eq!(repositories(&p).selected, ["group:Watch", "repo:ankh-morpork/patrol"]);
        assert!(p.editor.is_none());
    }

    #[test]
    fn removed_editor_identity_cancels_capture_without_retargeting() {
        for kind in ["repository", "group", "move"] {
            let mut p = fixture();
            let row = if kind == "group" { target(&["group:Watch"]) }
                else { target(&["group:Watch", "repo:ankh-morpork/watch"]) };
            select(&mut p, &row);
            if kind == "move" {
                let menu = menu_id(&mut p, 'm');
                settings::on_menu(&mut p, &menu, "new");
            } else {
                press(&mut p, v::Key::Char('e'));
            }
            assert!(radar_core::Plugin::capturing(&p));
            let mut cfg = p.cfg.clone();
            if kind == "group" { cfg.groups.remove(0); }
            else { cfg.groups[0].repositories.remove(0); }
            p.apply_config(cfg);
            assert!(p.editor.is_none(), "{kind} editor must disappear with its target");
            assert!(!radar_core::Plugin::capturing(&p), "{kind} editor cannot consume invisible typing");
            assert!(repositories(&p).editing.is_none());
            assert!(!p.cfg.repositories.contains(&"ankh-morpork/watch".into()));
        }
    }

    #[test]
    fn stale_editor_commit_still_refuses_an_unreconciled_replacement() {
        let mut p = fixture();
        select(&mut p, &target(&["group:Watch", "repo:ankh-morpork/watch"]));
        press(&mut p, v::Key::Char('e'));
        // The commit boundary remains defensive even if a caller bypasses the
        // normal configuration reconciliation above.
        p.cfg.groups[0].repositories.remove(0);
        assert_eq!(repositories(&p).editing.unwrap().path, ["group:Watch", "repo:ankh-morpork/watch"]);
        p.editor.as_mut().unwrap().input = TextInput::new("ankh-morpork/patrol");
        let Outcome::Handled(effects) = press(&mut p, v::Key::Enter) else { panic!("stale editor feedback") };
        assert!(effects.iter().any(|effect| matches!(effect, Effect::Notify(text) if text == "repository changed; reopen edit")));
        assert_eq!(p.cfg.groups[0].repositories, ["ankh-morpork/guilds"]);
    }

    #[test]
    fn settings_delete_payload_and_generation_guard_stay_on_the_selected_repository() {
        let mut p = fixture();
        let repo = target(&["group:Watch", "repo:ankh-morpork/guilds"]);
        select(&mut p, &repo);
        let Outcome::Handled(effects) = press(&mut p, v::Key::Char('d')) else { panic!("delete confirmation") };
        let Some(Effect::Menu(menu)) = effects.first() else { panic!("delete confirmation") };
        assert_eq!(menu.id, "remove");
        assert_eq!(p.remove_menu.as_ref().unwrap().1.full_name(), "ankh-morpork/guilds");
        select(&mut p, &target(&["group:Witches"]));
        settings::on_menu(&mut p, &menu.id, "yes");
        assert_eq!(p.cfg.groups[0].repositories, ["ankh-morpork/watch"]);
        assert_eq!(p.cfg.groups[1].repositories, ["lancre/witches"]);

        select(&mut p, &target(&["group:Watch", "repo:ankh-morpork/watch"]));
        press(&mut p, v::Key::Char('d'));
        p.apply_config(p.cfg.clone());
        settings::on_menu(&mut p, "remove", "yes");
        assert_eq!(p.cfg.groups[0].repositories, ["ankh-morpork/watch"]);
    }

    #[test]
    fn renaming_a_folded_group_keeps_selection_and_fold_and_escapes_identity() {
        let mut p = fixture();
        let group = target(&["group:Watch"]);
        select(&mut p, &group);
        intent(&mut p, "repositories", &v::Intent::Fold { target: group, open: false });
        press(&mut p, v::Key::Char('e'));
        p.editor.as_mut().unwrap().input = TextInput::new("Watch\u{1f}%1F");
        press(&mut p, v::Key::Enter);
        let model = repositories(&p);
        assert_eq!(model.selected, ["group:Watch%1F%251F"]);
        assert!(!model.open.contains(&model.selected));
        assert_eq!(v::TreeRow::path_of(&v::TreeRow::target_of(&model.selected)), model.selected);
    }
    fn menu_id(p: &mut GithubPlugin, key: char) -> String {
        let Outcome::Handled(effects) = press(p, v::Key::Char(key)) else { panic!("settings menu") };
        effects.iter().find_map(|effect| match effect {
            Effect::Menu(menu) => Some(menu.id.clone()),
            _ => None,
        }).expect("settings menu")
    }

    #[test]
    fn group_delete_keeps_its_payload_and_refuses_reordered_or_reset_configuration() {
        let mut p = fixture();
        select(&mut p, &target(&["group:Watch"]));
        let menu = menu_id(&mut p, 'd');
        assert_eq!(menu, "remove-group:0");
        select(&mut p, &target(&["group:Witches"]));
        settings::on_menu(&mut p, &menu, "yes");
        assert_eq!(p.cfg.groups.len(), 1);
        assert_eq!(p.cfg.groups[0].name, "Witches");

        for change in 0..3 {
            let mut p = fixture();
            select(&mut p, &target(&["group:Watch"]));
            let menu = menu_id(&mut p, 'd');
            let mut cfg = p.cfg.clone();
            if change != 1 { cfg.groups.reverse(); }
            if change == 2 { p.cfg = cfg; } else { p.apply_config(cfg); }
            let before = p.cfg.clone();
            let effects = settings::on_menu(&mut p, &menu, "yes");
            assert_eq!(p.cfg, before);
            assert!(effects.iter().any(|effect| matches!(effect, Effect::Notify(text) if text == "settings changed; reopen the group action")));
        }
    }

    #[test]
    fn repository_move_refuses_stale_source_destination_reset_and_new_group_choices() {
        for choice in ["1", "new"] {
            for change in 0..3 {
                let mut p = fixture();
                select(&mut p, &target(&["group:Watch", "repo:ankh-morpork/watch"]));
                let menu = menu_id(&mut p, 'm');
                assert_eq!(menu, "move:0:0");
                let mut cfg = p.cfg.clone();
                if change != 1 { cfg.groups.reverse(); }
                if change == 2 { p.cfg = cfg; } else { p.apply_config(cfg); }
                let before = p.cfg.clone();
                let effects = settings::on_menu(&mut p, &menu, choice);
                assert_eq!(p.cfg, before);
                assert!(p.editor.is_none());
                assert!(effects.iter().any(|effect| matches!(effect, Effect::Notify(text) if text == "settings changed; reopen move")));
            }
        }
    }

    #[test]
    fn hidden_removed_and_legacy_context_targets_do_not_open_fallback_actions() {
        let mut p = fixture();
        let repo = target(&["group:Watch", "repo:ankh-morpork/watch"]);
        select(&mut p, &target(&["group:Watch"]));
        intent(&mut p, "repositories", &v::Intent::Fold { target: target(&["group:Watch"]), open: false });
        let before = repositories(&p).selected;
        for target in [repo.clone(), v::Target::new("setting:2")] {
            assert!(matches!(intent(&mut p, "repositories", &v::Intent::Context { target }), Outcome::Ignored));
            assert_eq!(repositories(&p).selected, before);
        }
        p.apply_config(Settings::default());
        assert!(matches!(intent(&mut p, "repositories", &v::Intent::Context { target: repo }), Outcome::Ignored));
    }

}
