//! GitHub settings as renderer-neutral rows and intents.

use radar_core::plugin::Outcome;
use radar_core::ui::connection;
use radar_core::ui::icon::Icon;
use radar_core::view::{self as v, Ink, ListRow, Node, SectionView, Seg, Text};

use super::{ConnRow, GithubPlugin, Row};

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
        "repositories" => {
            let rows = super::order(p)
                .into_iter()
                .enumerate()
                .map(|(i, row)| {
                    let label = match row {
                        Row::Add => "create a group or add a repository".to_owned(),
                        Row::Group(g) => format!("◆  {}", p.cfg.groups[g].name),
                        Row::Repository(g, r) => {
                            let group = &p.cfg.groups[g];
                            let full = &group.repositories[r];
                            let name = group
                                .owner
                                .as_ref()
                                .and_then(|owner| full.strip_prefix(&format!("{owner}/")))
                                .unwrap_or(full);
                            let branch = if r + 1 == group.repositories.len() {
                                "└─ "
                            } else {
                                "├─ "
                            };
                            format!("  {branch}{name}")
                        }
                    };
                    ListRow::new(format!("setting:{i}"), Text::plain(label))
                })
                .collect();
            (rows, p.repository_row)
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

pub fn intent(p: &mut GithubPlugin, section: &str, intent: &v::Intent) -> Outcome {
    if !matches!(section, "connection" | "repositories") {
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
        v::Intent::Fold { .. }
        | v::Intent::Show { .. }
        | v::Intent::End { .. }
        | v::Intent::EditSelect { .. }
        | v::Intent::EditUnselect
        | v::Intent::Dock(_) | v::Intent::Copy { .. } => Outcome::Ignored,
    }
}
