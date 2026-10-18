//! Settings are text fields and explicit read checks.

use radar_core::input::{Edit, TextInput};
use radar_core::plugin::{Binding, Effect, Menu, MenuItem, Outcome};
use radar_core::ui::{connection, rows::Move, status::Check};
use radar_core::view::{self as v, Chord, Key};

use crate::{GithubPlugin, auth, config, model::RepositoryId};
pub(crate) mod neutral;
use std::collections::BTreeMap;

pub(crate) struct Editor {
    pub input: TextInput,
    field: Field,
}
/// Group/remove and move menus retain their existing wire ids, but only
/// the exact configuration that produced a menu can consume its choice.
pub(crate) struct MenuLease {
    generation: u64,
    id: String,
    groups: Vec<config::Group>,
}

fn lease_menu(p: &mut GithubPlugin, id: String) {
    p.settings_menu = Some(MenuLease { generation: p.generation, id, groups: p.cfg.groups.clone() });
}

fn consume_menu(p: &mut GithubPlugin, id: &str) -> bool {
    p.settings_menu.take().is_some_and(|lease| {
        lease.generation == p.generation && lease.id == id && lease.groups == p.cfg.groups
    })
}

#[derive(Clone)]
enum Field {
    Api,
    Token,
    Add,
    RenameGroup(String),
    Repository(String, String),
    MoveToNewGroup(String),
}

/// Configuration refreshes can remove the row an editor captured. Cancel that
/// editor rather than retain invisible keyboard capture or paint over a fallback.
/// Reordering preserves valid identity-based edits.
pub(crate) fn reconcile_editor(p: &mut GithubPlugin) {
    let valid = p.editor.as_ref().is_none_or(|editor| match &editor.field {
        Field::RenameGroup(name) => p.cfg.groups.iter().any(|group| group.name == *name),
        Field::Repository(name, repo) => p.cfg.groups.iter().any(|group| group.name == *name && group.repositories.contains(repo)),
        Field::MoveToNewGroup(repo) => p.cfg.groups.iter().any(|group| group.repositories.contains(repo)),
        Field::Api | Field::Token | Field::Add => true,
    });
    if !valid { p.editor = None; }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Add,
    Group(usize),
    Repository(usize, usize),
}

fn all_rows(p: &GithubPlugin) -> Vec<Row> {
    let mut rows = vec![Row::Add];
    for (group, item) in p.cfg.groups.iter().enumerate() {
        rows.push(Row::Group(group));
        rows.extend((0..item.repositories.len()).map(|repo| Row::Repository(group, repo)));
    }
    rows
}

fn group_id(name: &str) -> String {
    format!("group:{}", name.replace('%', "%25").replace(v::PATH_SEP, "%1F"))
}

fn path(p: &GithubPlugin, row: Row) -> Vec<String> {
    match row {
        Row::Add => vec!["add".into()],
        Row::Group(g) => vec![group_id(&p.cfg.groups[g].name)],
        Row::Repository(g, r) => vec![group_id(&p.cfg.groups[g].name), format!("repo:{}", p.cfg.groups[g].repositories[r])],
    }
}

fn order(p: &GithubPlugin) -> Vec<Row> {
    all_rows(p).into_iter().filter(|row| match row {
        Row::Repository(g, _) => !p.repository_folded.contains(&group_id(&p.cfg.groups[*g].name)),
        _ => true,
    }).collect()
}

fn row_at(p: &GithubPlugin, target: &[String]) -> Option<Row> {
    order(p).into_iter().find(|row| path(p, *row) == target)
}

fn selected(p: &GithubPlugin) -> Row {
    let mut target = p.repository_selected.clone();
    while !target.is_empty() {
        if let Some(row) = row_at(p, &target) { return row; }
        target.pop();
    }
    Row::Add
}

fn select(p: &mut GithubPlugin, row: Row) {
    p.repository_selected = path(p, row);
    if let Row::Repository(g, _) = row {
        p.repository_folded.remove(&group_id(&p.cfg.groups[g].name));
    }
}

#[cfg(test)]
pub(crate) fn select_repository_row(p: &mut GithubPlugin, index: usize) {
    if let Some(row) = order(p).get(index).copied() { select(p, row); }
}

fn fold(p: &mut GithubPlugin, target: &v::Target, open: bool) -> Outcome {
    let target = v::TreeRow::path_of(target);
    let Some(Row::Group(g)) = row_at(p, &target) else { return Outcome::Ignored; };
    if p.cfg.groups[g].repositories.is_empty() { return Outcome::Ignored; }
    if open {
        p.repository_folded.remove(&target[0]);
    } else {
        let current = path(p, selected(p));
        p.repository_folded.insert(target[0].clone());
        if current.starts_with(&target) { p.repository_selected = target; }
    }
    Outcome::done()
}

fn target_group(p: &GithubPlugin) -> Option<usize> {
    match selected(p) {
        Row::Group(g) | Row::Repository(g, _) => Some(g),
        Row::Add => None,
    }
}

#[allow(dead_code)]
fn check(p: &GithubPlugin) -> &Check {
    p.connection_check.as_ref().unwrap_or(&Check::NotYet)
}

/// How radar signs in to GitHub: the connection section's three choices,
/// each a value of `[github] token`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    /// No token: public repositories only.
    Public,
    /// `gh`: the gh program's sign-in, asked when radar reads.
    Gh,
    /// `env:NAME`, or a token pasted.
    Token,
}

impl Access {
    pub(crate) const ALL: [Self; 3] = [Self::Public, Self::Gh, Self::Token];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Public => "public only",
            Self::Gh => "gh's sign-in",
            Self::Token => "a token",
        }
    }

    pub(crate) const fn about(self) -> &'static str {
        match self {
            Self::Public => "public repositories, no token",
            Self::Gh => "asks gh for its token",
            Self::Token => "env:NAME, or one pasted",
        }
    }

    /// The choice a token reference is.
    pub(crate) fn of(token: &str) -> Self {
        match token {
            "" => Self::Public,
            auth::GH => Self::Gh,
            _ => Self::Token,
        }
    }
}

/// A row of the connection section: how radar signs in, where, the token,
/// the check, how often; then the repositories followed (the repositories
/// section's groups, as setup's GitHub step shows them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnRow {
    /// A gap before a heading: not selectable.
    Gap(u8),
    /// `SIGN IN`: not selectable.
    AccessHeading,
    Access(Access),
    Server,
    Token,
    Check,
    Refresh,
    /// `ACCESS`: not selectable.
    GrantHeading,
    /// What a token is made to allow, `[github] access`.
    Grant(config::TokenAccess),
    /// `REPOSITORIES`: not selectable.
    FollowHeading,
    /// `+ add a repository or group`: the repositories section's add field.
    FollowAdd,
    /// A group.
    Follow(usize),
}

impl ConnRow {
    pub(crate) const fn selectable(self) -> bool {
        !matches!(self, Self::Gap(_) | Self::AccessHeading | Self::GrantHeading | Self::FollowHeading)
    }
}

/// What an access choice lets a token do, as its row says it.
pub(crate) const fn grant_about(access: config::TokenAccess) -> &'static str {
    match access {
        config::TokenAccess::Read => "pull requests, checks, runs",
        config::TokenAccess::Write => "adds comment, approve, merge, re-run",
    }
}

/// The access row's detail: the permissions the new-token page fills in;
/// for write, that each write is still asked and the mode must allow it.
pub(crate) fn grant_detail(access: config::TokenAccess) -> String {
    match access {
        config::TokenAccess::Read => {
            "The token reads pull requests, their checks and Actions runs. The new-token link fills in contents, pull requests, actions and statuses: read.".into()
        }
        config::TokenAccess::Write => format!(
            "The token may also comment, approve, merge and re-run: the new-token link fills in pull requests, contents and actions: read and write. radar sends no GitHub writes yet. Each write is still asked first, and only while the mode allows writes; the mode is {} now.",
            radar_core::writes::mode().0.word()
        ),
    }
}

pub(crate) fn conn_rows(p: &GithubPlugin) -> Vec<ConnRow> {
    let mut rows = vec![ConnRow::AccessHeading];
    rows.extend(Access::ALL.map(ConnRow::Access));
    // As a setup step the card leaves out the server and refresh rows, which
    // only Settings needs; the plugin remembers how the card was last drawn,
    // so keys and clicks walk what is drawn.
    let setup = p.setup_card.get();
    rows.push(ConnRow::Gap(0));
    if !setup {
        rows.push(ConnRow::Server);
    }
    rows.extend([ConnRow::Token, ConnRow::Check]);
    if !setup {
        rows.push(ConnRow::Refresh);
    }
    rows.extend([ConnRow::Gap(1), ConnRow::GrantHeading]);
    rows.extend(config::TokenAccess::ALL.map(ConnRow::Grant));
    rows.extend([ConnRow::Gap(2), ConnRow::FollowHeading, ConnRow::FollowAdd]);
    rows.extend((0..p.cfg.groups.len()).map(ConnRow::Follow));
    rows
}

/// The connection row the cursor is on: the first that can be selected
/// at or after `connection_row`.
pub(crate) fn conn_at(p: &GithubPlugin) -> usize {
    let rows = conn_rows(p);
    (p.connection_row.min(rows.len() - 1)..rows.len())
        .find(|&i| rows[i].selectable())
        .or_else(|| rows.iter().rposition(|r| r.selectable()))
        .unwrap_or(0)
}

pub(crate) fn conn_row(p: &GithubPlugin) -> ConnRow {
    conn_rows(p)[conn_at(p)]
}

pub(crate) const API_ABOUT: &str = "The GitHub API radar talks to: api.github.com for github.com, or your own GitHub Enterprise Server or a local server. The token is sent only to this address.";
pub(crate) const API_ACCEPTS: &str = "https://api.github.com · a GitHub Enterprise Server's https://HOST/api/v3 · http://127.0.0.1:PORT for a local server";

pub(crate) fn title(p: &GithubPlugin, section: &str) -> String {
    if section == "connection" {
        match conn_row(p) {
            ConnRow::Access(a) => a.label().into(),
            ConnRow::Grant(a) => a.label().into(),
            ConnRow::GrantHeading => "access".into(),
            ConnRow::Server => "server".into(),
            ConnRow::Token => connection::TOKEN.into(),
            ConnRow::Check => connection::CHECK.into(),
            ConnRow::Refresh => connection::REFRESH.into(),
            ConnRow::FollowAdd | ConnRow::FollowHeading | ConnRow::Gap(_) | ConnRow::AccessHeading => {
                "add a repository or group".into()
            }
            ConnRow::Follow(g) => p.cfg.groups[g].name.clone(),
        }
    } else {
        match selected(p) {
            Row::Add => "create a group or add a repository".into(),
            Row::Group(g) => p.cfg.groups[g].name.clone(),
            Row::Repository(g, r) => p.cfg.groups[g].repositories[r].clone(),
        }
    }
}

/// A bare word is a personal grouping label. Only a GitHub URL creates a
/// group restricted to that account's repositories.
fn typed_owner(value: &str) -> Option<String> {
    let value = value.trim();
    (value.starts_with("github.com/") || value.starts_with("https://github.com/"))
        .then(|| config::owner(value).ok())
        .flatten()
}

fn validate(field: &Field, value: &str) -> Result<String, String> {
    match field {
        Field::Api => config::Endpoint::parse(value)
            .map(|api| {
                if api.rest() == config::DEFAULT_API {
                    "GitHub.com".into()
                } else if api.base_path() == "/api/v3" {
                    format!("GitHub Enterprise Server, GraphQL at {}", api.graphql())
                } else if api.is_https() {
                    format!("GitHub API at {}", api.rest())
                } else {
                    format!(
                        "{} is plain http; the token travels unencrypted",
                        api.rest()
                    )
                }
            })
            .map_err(|e| e.to_string()),
        Field::Token => auth::validate_reference(value.trim())
            .map(|_| {
                if value.trim().is_empty() {
                    "public read access without authentication".into()
                } else {
                    if value.trim() == auth::GH {
                        "gh's sign-in, asked with gh auth token when radar reads".into()
                    } else if value.trim().starts_with("env:") {
                        "token read from the named environment variable".into()
                    } else {
                        "typed token; saved in config.toml and hidden on screen".into()
                    }
                }
            })
            .map_err(|e| e.to_string()),
        Field::Add => {
            let value = value.trim();
            if value.is_empty() {
                Err("enter a group name, owner, or repository".into())
            } else if let Ok(repo) = config::repository(value) {
                Ok(format!("add {}", repo.full_name()))
            } else if let Some(owner) = typed_owner(value) {
                Ok(format!("create github.com/{owner} group"))
            } else if value.contains('/') || value.contains("://") {
                Err("enter a valid github.com owner or owner/repo URL".into())
            } else {
                Ok(format!("create group {value}"))
            }
        }
        Field::RenameGroup(_) | Field::MoveToNewGroup(_) => {
            if value.trim().is_empty() {
                Err("group name cannot be empty".into())
            } else {
                Ok(format!("rename group to {}", value.trim()))
            }
        }
        Field::Repository(..) => config::repository(value)
            .map(|r| format!("GitHub.com repository {}", r.full_name()))
            .map_err(|e| e.to_string()),
    }
}

fn start_edit(p: &mut GithubPlugin, section: &str) -> Outcome {
    let (field, value) = if section == "connection" {
        match conn_row(p) {
            ConnRow::Server => (Field::Api, p.cfg.api.clone()),
            ConnRow::Token | ConnRow::Access(Access::Token) => (
                Field::Token,
                if auth::is_typed(&p.cfg.token) {
                    String::new()
                } else {
                    p.cfg.token.clone()
                },
            ),
            ConnRow::FollowAdd => (Field::Add, String::new()),
            _ => {
                return Outcome::Handled(vec![Effect::Notify(
                    "this setting is not typed; the server and the token are editable".into(),
                )]);
            }
        }
    } else {
        match selected(p) {
            Row::Add => (Field::Add, String::new()),
            Row::Group(g) => (Field::RenameGroup(p.cfg.groups[g].name.clone()), p.cfg.groups[g].name.clone()),
            Row::Repository(g, r) => (
                Field::Repository(p.cfg.groups[g].name.clone(), p.cfg.groups[g].repositories[r].clone()),
                p.cfg.groups[g].repositories[r].clone(),
            ),
        }
    };
    p.editor = Some(Editor {
        input: TextInput::new(&value),
        field,
    });
    Outcome::done()
}

fn keep(p: &mut GithubPlugin, value: String) -> Outcome {
    let Some(editor) = p.editor.take() else {
        return Outcome::Ignored;
    };
    if let Err(reason) = validate(&editor.field, &value) {
        p.editor = Some(editor);
        return Outcome::Handled(vec![Effect::error(None, format!("not yet: {reason}"))]);
    }
    let field = editor.field.clone();
    let mut cfg = p.cfg.clone();
    let connection_edit = matches!(&editor.field, Field::Token | Field::Api);
    let moving_to_new = matches!(&editor.field, Field::MoveToNewGroup(_));
    let adding = matches!(&editor.field, Field::Add) && config::repository(&value).is_err();
    let added_repo = matches!(&editor.field, Field::Add)
        .then(|| config::repository(&value).ok())
        .flatten()
        .map(|repo| repo.full_name());
    if !connection_edit {
        cfg.repositories.clear();
    }
    let mut edited_row = None;
    match editor.field {
        Field::Api => cfg.api = value.trim().into(),
        Field::Token => cfg.token = value.trim().into(),
        Field::Add => {
            if let Ok(repo) = config::repository(&value) {
                let name = format!("github.com/{}", repo.owner);
                let group = cfg
                    .groups
                    .iter()
                    .position(|g| g.name == name)
                    .unwrap_or_else(|| {
                        cfg.groups.push(config::Group {
                            name,
                            owner: Some(repo.owner.clone()),
                            repositories: vec![],
                        });
                        cfg.groups.len() - 1
                    });
                cfg.groups[group].repositories.push(repo.full_name());
            } else {
                let owner = typed_owner(&value);
                let name = owner
                    .as_ref()
                    .map_or_else(|| value.trim().to_string(), |o| format!("github.com/{o}"));
                cfg.groups.push(config::Group {
                    name,
                    owner,
                    repositories: vec![],
                });
            }
        }
        Field::RenameGroup(name) => {
            let Some(g) = cfg.groups.iter().position(|group| group.name == name) else {
                return Outcome::Handled(vec![Effect::Notify("group changed; reopen edit".into())]);
            };
            cfg.groups[g].name = value.trim().into();
            cfg.groups[g].owner = typed_owner(&value);
            edited_row = Some(Row::Group(g));
        }
        Field::MoveToNewGroup(repo) => {
            let Some(from) = cfg
                .groups
                .iter()
                .position(|g| g.repositories.contains(&repo))
            else {
                return Outcome::Handled(vec![Effect::Notify(
                    "repository changed; reopen move".into(),
                )]);
            };
            cfg.groups[from].repositories.retain(|r| r != &repo);
            cfg.groups.push(config::Group {
                name: value.trim().into(),
                owner: None,
                repositories: vec![repo],
            });
        }
        Field::Repository(group, repo) => {
            let Some((g, r)) = cfg.groups.iter().enumerate().find_map(|(g, item)| {
                (item.name == group).then(|| item.repositories.iter().position(|name| *name == repo)).flatten().map(|r| (g, r))
            }) else {
                return Outcome::Handled(vec![Effect::Notify("repository changed; reopen edit".into())]);
            };
            cfg.groups[g].repositories[r] = config::repository(&value).unwrap().full_name();
            edited_row = Some(Row::Repository(g, r));
        }
    }
    let cfg = match cfg.validated() {
        Ok(cfg) => cfg,
        Err(error) => {
            p.editor = Some(Editor {
                input: TextInput::new(&value),
                field,
            });
            return Outcome::Handled(vec![Effect::error(None, error.to_string())]);
        }
    };
    if cfg == p.cfg {
        return Outcome::done();
    }
    let new_group = adding && cfg.groups.len() > p.cfg.groups.len();
    let renamed_fold = match &field {
        Field::RenameGroup(name) if p.repository_folded.contains(&group_id(name)) => Some(group_id(name)),
        _ => None,
    };
    p.apply_config(cfg);
    if let Some(old) = renamed_fold {
        p.repository_folded.remove(&old);
        p.repository_folded.insert(group_id(value.trim()));
    }
    p.started = true;
    if let Some(row) = edited_row { select(p, row); }
    if let Some(repo) = added_repo
        && let Some(row) = all_rows(p).into_iter().find(|row| match row {
            Row::Repository(g, r) => p.cfg.groups[*g].repositories[*r] == repo,
            _ => false,
        })
    {
        select(p, row);
    }
    if moving_to_new {
        let row = all_rows(p).last().copied().unwrap_or(Row::Add);
        select(p, row);
        Outcome::Handled(vec![Effect::Notify(
            "repository moved into new group".into(),
        )])
    } else if connection_edit {
        Outcome::Handled(p.check_connection())
    } else if new_group {
        let group = p.cfg.groups.len() - 1;
        select(p, Row::Group(group));
        if let Some(owner) = p.cfg.groups[group].owner.clone() {
            p.discovery_group = Some(group);
            Outcome::Handled(p.discover_repositories(Some(owner)))
        } else {
            Outcome::Handled(vec![Effect::Notify(
                "group created; select a repository and press m to move it here".into(),
            )])
        }
    } else {
        p.check_menu = Some(p.generation);
        Outcome::Handled(vec![Effect::Menu(
            Menu::confirm(
                "check",
                "read configured GitHub repositories now?",
                "read them",
                "later",
            )
            .saying("This makes read-only requests for the configured repositories."),
        )])
    }
}

pub(crate) fn key(p: &mut GithubPlugin, section: &str, key: Chord) -> Outcome {
    if let Some(editor) = &mut p.editor {
        return match editor.input.key(radar_core::view_tui::key_of(key)) {
            Edit::Submit(value) => keep(p, value),
            Edit::Cancel => {
                p.editor = None;
                Outcome::done()
            }
            Edit::Typing => Outcome::done(),
        };
    }
    if key.ctrl || key.alt {
        return Outcome::Ignored;
    }
    if section == "repositories"
        && let Some(step) = v::tree_step(&neutral::repositories(p), key)
    {
        return neutral::intent(p, section, &step);
    }
    if section == "connection" {
        match (key.key, conn_row(p)) {
            (Key::Enter, ConnRow::Access(a)) => return choose(p, a),
            (Key::Enter, ConnRow::Grant(a)) => return choose_grant(p, a),
            (Key::Enter, ConnRow::Check) => return run_check(p),
            (Key::Enter, ConnRow::Follow(g)) => {
                p.discovery_group = Some(g);
                let owner = p.cfg.groups[g].owner.clone();
                return Outcome::Handled(p.discover_repositories(owner));
            }
            (Key::Char('d'), ConnRow::Follow(g)) => return Outcome::Handled(vec![remove_group(p, g)]),
            (Key::Char('o'), _) => return Outcome::Handled(vec![Effect::Open(crate::setup::token_page(p))]),
            (Key::Char('y'), _) => {
                return Outcome::Handled(vec![
                    Effect::Copy(crate::setup::token_page(p)),
                    Effect::Notify("copied the new-token page's link".into()),
                ]);
            }
            _ => {}
        }
    }
    match key.key {
        Key::Char('c') => return run_check(p),
        Key::Char('a') if section == "repositories" => {
            p.discovery_group = target_group(p);
            let owner = p
                .discovery_group
                .and_then(|g| p.cfg.groups[g].owner.clone());
            return Outcome::Handled(p.discover_repositories(owner));
        }
        Key::Enter if section == "repositories" && matches!(selected(p), Row::Group(_)) => {
            p.discovery_group = target_group(p);
            let owner = p
                .discovery_group
                .and_then(|g| p.cfg.groups[g].owner.clone());
            return Outcome::Handled(p.discover_repositories(owner));
        }
        Key::Enter | Key::Char('e') => return start_edit(p, section),
        Key::Char('d') if section == "repositories" && selected(p) != Row::Add => {
            return match selected(p) {
                Row::Group(g) => Outcome::Handled(vec![remove_group(p, g)]),
                Row::Repository(g, r) => {
                    let id = RepositoryId::parse(&p.cfg.groups[g].repositories[r]).unwrap();
                    p.remove_menu = Some((p.generation, id.clone()));
                    Outcome::Handled(vec![Effect::Menu(Menu::confirm("remove", format!("stop tracking {}?", id.full_name()), "remove it", "keep it").saying("This removes it from radar settings and drops its held data. It does not change GitHub."))])
                }
                Row::Add => Outcome::Ignored,
            };
        }
        Key::Char('m') if section == "repositories" => {
            if let Row::Repository(from, repo) = selected(p) {
                let name = &p.cfg.groups[from].repositories[repo];
                let mut items: Vec<MenuItem> = p
                    .cfg
                    .groups
                    .iter()
                    .enumerate()
                    .filter(|(i, group)| {
                        *i != from
                            && group
                                .owner
                                .as_ref()
                                .is_none_or(|owner| name.starts_with(&format!("{owner}/")))
                    })
                    .map(|(i, group)| {
                        MenuItem::pick(&group.name, "move into this group", i.to_string())
                    })
                    .collect();
                items.push(MenuItem::pick(
                    "Create a new group…",
                    "name a group and move this repository into it",
                    "new",
                ));
                let menu = format!("move:{from}:{repo}");
                lease_menu(p, menu.clone());
                return Outcome::Handled(vec![Effect::Menu(Menu::new(
                    menu,
                    "Move repository to group",
                    items,
                ))]);
            }
        }
        _ => {}
    }
    let Some(movement) = Move::from_key(&radar_core::view_tui::key_of(key), p.section_scroll.height())
    else {
        return Outcome::Ignored;
    };
    let order: Vec<usize> = if section == "connection" {
        let rows = conn_rows(p);
        (0..rows.len()).filter(|&i| rows[i].selectable()).collect()
    } else {
        (0..order(p).len()).collect()
    };
    let current = if section == "connection" {
        conn_at(p)
    } else {
        self::order(p).iter().position(|row| *row == selected(p)).unwrap_or(0)
    };
    let at = order.iter().position(|row| *row == current).unwrap_or(0);
    let to = radar_core::ui::rows::select(order.len(), at, movement, |_| true);
    if to == at {
        Outcome::Ignored
    } else {
        if section == "connection" {
            p.connection_row = order[to];
        } else {
            let row = self::order(p)[order[to]];
            select(p, row);
        }
        Outcome::done()
    }
}

fn run_check(p: &mut GithubPlugin) -> Outcome {
    Outcome::Handled(p.check_connection())
}

/// Asks before a group and its repositories go.
fn remove_group(p: &mut GithubPlugin, g: usize) -> Effect {
    let menu = format!("remove-group:{g}");
    lease_menu(p, menu.clone());
    Effect::Menu(Menu::confirm(
        menu,
        format!("remove group {} and its repositories?", p.cfg.groups[g].name),
        "remove group",
        "keep it",
    ))
}

/// A sign-in choice taken: public and gh's set the token at once (and
/// check it); a token is typed into the token row.
fn choose(p: &mut GithubPlugin, access: Access) -> Outcome {
    let value = match access {
        Access::Public => "",
        Access::Gh => auth::GH,
        Access::Token => {
            p.connection_row = conn_rows(p).iter().position(|r| *r == ConnRow::Token).unwrap_or(0);
            return start_edit(p, "connection");
        }
    };
    p.editor = Some(Editor { input: TextInput::new(value), field: Field::Token });
    keep(p, value.into())
}

/// An access choice taken: kept in the config (no data depends on it, so
/// nothing is read again), and a check already made asked again, so the
/// check row says whether the token allows it.
fn choose_grant(p: &mut GithubPlugin, access: config::TokenAccess) -> Outcome {
    if p.cfg.access == access {
        return Outcome::done();
    }
    p.cfg.access = access;
    let mut fx = vec![Effect::Notify(format!("the token's access: {}", access.label()))];
    if p.connection_check.is_some() {
        fx.extend(p.check_connection());
    }
    Outcome::Handled(fx)
}

pub(crate) fn click(p: &mut GithubPlugin, section: &str, id: &str) -> Outcome {
    if section == "repositories" {
        let target = v::TreeRow::path_of(&v::Target::new(id));
        let Some(row) = row_at(p, &target) else { return Outcome::Ignored; };
        select(p, row);
        return Outcome::done();
    }
    let Some(index) = id
        .strip_prefix("setting:")
        .and_then(|v| v.parse::<usize>().ok())
    else {
        return Outcome::Ignored;
    };
    if section == "connection" && conn_rows(p).get(index).is_some_and(|r| r.selectable()) {
        p.connection_row = index;
        Outcome::done()
    } else {
        Outcome::Ignored
    }
}

pub(crate) fn keys(p: &GithubPlugin, section: &str) -> Vec<crate::Act> {
    let act = |verb: &str, b: Binding| crate::act(section, verb, b);
    let mut rows = vec![crate::head(Binding::heading(title(p, section)))];
    if section == "connection" {
        match conn_row(p) {
            ConnRow::Access(_) => rows.push(act("choose", Binding::new("⏎", "sign in this way"))),
            ConnRow::Grant(_) => rows.push(act("choose", Binding::new("⏎", "give the token this access"))),
            ConnRow::Server | ConnRow::Token => {
                rows.push(act("edit", Binding::new("e", "change it")));
                rows.push(act("activate_setting", Binding::new("⏎", "change it")));
            }
            ConnRow::Check => rows.push(act("activate_check", Binding::new("⏎", "check GitHub connection"))),
            ConnRow::FollowAdd | ConnRow::FollowHeading | ConnRow::Gap(_) | ConnRow::AccessHeading | ConnRow::GrantHeading => {
                rows.push(act("follow", Binding::new("⏎", "add a repository or group")));
            }
            ConnRow::Follow(_) => {
                rows.push(act("activate_group", Binding::new("⏎", "browse repositories for group")));
                rows.push(act("remove", Binding::new("d", "remove selected group or repository…")));
            }
            ConnRow::Refresh => {}
        }
        rows.push(act("check", Binding::new("c", "check GitHub connection")));
        rows.push(act("token_page", Binding::new("o", "open GitHub's new-token page")));
        rows.push(act("copy_token_link", Binding::new("y", "copy the new-token page's link")));
        return rows;
    }
    if section == "repositories" {
        rows.push(act("edit", Binding::new("e", "change it")));
        let (verb, what) = match selected(p) {
            Row::Add => ("activate_add", "create group or add repository"),
            Row::Group(_) => ("activate_group", "browse repositories for group"),
            Row::Repository(..) => ("activate_repository", "change repository"),
        };
        rows.push(act(verb, Binding::new("⏎", what)));
    }
    if section == "repositories" {
        rows.push(act("check", Binding::new("c", "check GitHub connection")));
        rows.push(act(
            "browse",
            Binding::new("a", "browse repositories this token can access"),
        ));
    }
    if section == "repositories" && selected(p) != Row::Add {
        rows.push(act(
            "remove",
            Binding::new("d", "remove selected group or repository…"),
        ));
        if matches!(selected(p), Row::Repository(..)) {
            rows.push(act("move", Binding::new("m", "move to another group")));
        }
    }
    rows
}

pub(crate) fn context(p: &GithubPlugin, section: &str, key: Chord) -> Option<String> {
    if matches!(key.key, Key::Char('e' | 'd' | 'c' | 'a' | 'm' | 'o' | 'y') | Key::Enter) {
        Some(format!("{}:{section}:{}", p.generation, title(p, section)))
    } else {
        None
    }
}

pub(crate) fn on_menu(p: &mut GithubPlugin, menu: &str, value: &str) -> Vec<Effect> {
    match menu {
        "discover" => {
            if value == "done" {
                return p.refresh_all();
            }
            let choices = p.available.as_ref().map(|r| &r.items);
            let mut add: Vec<String> = Vec::new();
            if let Some(owner) = value.strip_prefix("owner:") {
                if let Some(choices) = choices {
                    add.extend(
                        choices
                            .iter()
                            .filter(|r| r.owner == owner)
                            .map(RepositoryId::full_name),
                    );
                }
            } else if let Some(repo) = value.strip_prefix("repo:")
                && choices.is_some_and(|choices| choices.iter().any(|r| r.full_name() == repo))
            {
                add.push(repo.to_string());
            }
            if add.is_empty() {
                return vec![Effect::Notify(
                    "repository selection changed; browse again".into(),
                )];
            }
            let mut cfg = p.cfg.clone();
            add.retain(|r| !cfg.repositories.contains(r));
            if cfg.repositories.len() + add.len() > config::MAX_REPOSITORIES {
                return vec![Effect::Notify(format!(
                    "that selection exceeds {} tracked repositories; choose individual repos",
                    config::MAX_REPOSITORIES
                ))];
            }
            let count = add.len();
            for repo in add {
                let group = p
                    .discovery_group
                    .filter(|g| *g < cfg.groups.len())
                    .unwrap_or_else(|| {
                        let owner = repo.split_once('/').unwrap().0;
                        let name = format!("github.com/{owner}");
                        cfg.groups
                            .iter()
                            .position(|g| g.name == name)
                            .unwrap_or_else(|| {
                                cfg.groups.push(config::Group {
                                    name,
                                    owner: Some(owner.into()),
                                    repositories: vec![],
                                });
                                cfg.groups.len() - 1
                            })
                    });
                cfg.groups[group].repositories.push(repo);
            }
            cfg.repositories.clear();
            let Ok(cfg) = cfg.validated() else {
                return vec![Effect::Notify(
                    "repository selection is no longer valid".into(),
                )];
            };
            p.apply_config(cfg);
            p.started = true;
            let mut effects = vec![Effect::Notify(format!(
                "tracking {count} more GitHub repositor{}",
                if count == 1 { "y" } else { "ies" }
            ))];
            if let Some(menu) = discovery_menu(p) {
                effects.push(Effect::Menu(menu));
            } else {
                effects.extend(p.refresh_all());
            }
            effects
        }
        "check" => {
            let generation = p.check_menu.take();
            if value == "yes" && generation == Some(p.generation) {
                p.refresh_all()
            } else {
                vec![]
            }
        }
        "remove" => {
            let Some((generation, id)) = p.remove_menu.take() else {
                return vec![];
            };
            if value != "yes" {
                return vec![];
            }
            if generation != p.generation {
                return vec![Effect::Notify(
                    "settings changed; reopen the repository action".into(),
                )];
            }
            let mut cfg = p.cfg.clone();
            for group in &mut cfg.groups {
                group.repositories.retain(|r| r != &id.full_name());
            }
            cfg.repositories.clear();
            p.apply_config(cfg.validated().unwrap());
            // Removing a row is a settings edit, not a request to read the
            // remaining repositories again on the next tick.
            p.started = true;
            let row = selected(p);
            select(p, row);
            vec![Effect::Notify("repository removed from radar".into())]
        }
        other if other.starts_with("remove-group:") => {
            let valid = consume_menu(p, other);
            if value != "yes" {
                return vec![];
            }
            if !valid {
                return vec![Effect::Notify("settings changed; reopen the group action".into())];
            }
            let Some(group) = other
                .strip_prefix("remove-group:")
                .and_then(|s| s.parse::<usize>().ok())
            else {
                return vec![];
            };
            if group >= p.cfg.groups.len() {
                return vec![];
            }
            let mut cfg = p.cfg.clone();
            cfg.groups.remove(group);
            cfg.repositories.clear();
            p.apply_config(cfg.validated().unwrap());
            p.started = true;
            let row = selected(p);
            select(p, row);
            vec![Effect::Notify("group removed from radar".into())]
        }
        other if other.starts_with("move:") => {
            if !consume_menu(p, other) {
                return vec![Effect::Notify("settings changed; reopen move".into())];
            }
            let Some((from, repo)) = other
                .strip_prefix("move:")
                .and_then(|s| s.split_once(':'))
                .and_then(|(g, r)| Some((g.parse::<usize>().ok()?, r.parse::<usize>().ok()?)))
            else {
                return vec![];
            };
            if value == "new" {
                let Some(name) = p
                    .cfg
                    .groups
                    .get(from)
                    .and_then(|g| g.repositories.get(repo))
                    .cloned()
                else {
                    return vec![Effect::Notify("repository changed; reopen move".into())];
                };
                p.editor = Some(Editor {
                    input: TextInput::new(""),
                    field: Field::MoveToNewGroup(name),
                });
                return vec![];
            }
            let Some(to) = value.parse::<usize>().ok() else {
                return vec![];
            };
            let mut cfg = p.cfg.clone();
            if from >= cfg.groups.len()
                || to >= cfg.groups.len()
                || repo >= cfg.groups[from].repositories.len()
            {
                return vec![];
            }
            let name = cfg.groups[from].repositories.remove(repo);
            cfg.groups[to].repositories.push(name);
            cfg.repositories.clear();
            let Ok(cfg) = cfg.validated() else {
                return vec![Effect::Notify(
                    "this repository cannot be moved to that owner group".into(),
                )];
            };
            p.apply_config(cfg);
            p.started = true;
            select(p, Row::Group(to));
            vec![Effect::Notify("repository moved".into())]
        }
        _ => vec![],
    }
}

/// A bounded, owner-grouped picker from the authenticated user's access list.
pub(crate) fn discovery_menu(p: &GithubPlugin) -> Option<Menu> {
    let available = p.available.as_ref()?;
    let mut groups: BTreeMap<&str, Vec<&RepositoryId>> = BTreeMap::new();
    for repo in &available.items {
        if p.discovery_owner
            .as_deref()
            .is_some_and(|owner| owner != repo.owner)
            || p.cfg.repositories.contains(&repo.full_name())
        {
            continue;
        }
        groups.entry(&repo.owner).or_default().push(repo);
    }
    if groups.is_empty() {
        return None;
    }
    let mut items = Vec::new();
    if !available.completion.complete {
        items.push(MenuItem::note(
            "Partial list: more accessible repositories may exist.",
        ));
    }
    for (owner, repos) in groups {
        items.push(MenuItem::heading(format!("github.com/{owner}")));
        items.push(MenuItem::pick(
            format!("track all {} shown here", repos.len()),
            "add this owner’s accessible repositories",
            format!("owner:{owner}"),
        ));
        for repo in repos {
            items.push(MenuItem::pick(
                &repo.name,
                "track this repository",
                format!("repo:{}", repo.full_name()),
            ));
        }
    }
    items.push(MenuItem::pick(
        "done · read selected repositories",
        "load their pull requests",
        "done",
    ));
    Some(Menu::new(
        "discover",
        "GitHub repositories you can access",
        items,
    ))
}

#[cfg(test)]
mod mnemonic_tests {
    use super::*;


    fn check_setup_link(blocks: &[radar_core::view::Block], url: &str, copy: &str, key: &str) {
        use radar_core::view::{Block, Text};
        fn find<'a>(blocks: &'a [Block], url: &str) -> Option<&'a Text> {
            blocks.iter().find_map(|block| match block {
                Block::Line { text, .. } if text.0.iter().any(|seg| seg.link.as_deref() == Some(url)) => Some(text),
                Block::Section { blocks, .. } => find(blocks, url),
                _ => None,
            })
        }
        let text = find(blocks, url).expect("actual setup token link");
        assert_eq!(text.0[0].link.as_deref(), Some(url));
        assert!(text.words().ends_with(&format!("   open · {copy}")));
        assert!(text.plain_words().ends_with(&format!("   open [o] · {copy} [{key}]")));
        let underlined: Vec<_> = text.0.iter().filter(|seg| seg.look.as_deref().is_some_and(|look| look.contains("underline"))).map(|seg| seg.text.as_str()).collect();
        assert_eq!(underlined, ["o", key]);
        let hints: Vec<_> = text.0.iter().filter_map(|seg| seg.key_hint.as_deref()).collect();
        assert_eq!(hints, ["o", key]);
        assert!(text.0[1..].iter().all(|seg| seg.link.is_none() && seg.target.is_none()));
    }

    #[test]
    fn github_settings_action_producers_have_mnemonics() {
        let mut p = crate::demo::sample();
        check_setup_link(&crate::setup::access_help(&p), &crate::setup::token_page(&p), "copy", "y");
        let mut checked = 0;
        for section in ["connection", "repositories"] {
            let count = if section == "connection" { conn_rows(&p).len() } else { order(&p).len() };
            for row in 0..count {
                if section == "connection" { p.connection_row = row; } else { select_repository_row(&mut p, row); }
                for (_, binding) in keys(&p, section) {
                    if radar_core::mnemonic::letter(&binding.keys).is_some() {
                        assert!(radar_core::mnemonic::at(&binding.keys, &binding.what).is_some(),
                            "{section}: {} / {}", binding.keys, binding.what);
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 10);
    }
}
