//! Settings are text fields and explicit read checks.

use radar_core::input::{Edit, TextInput};
use radar_core::plugin::{Binding, Effect, Menu, MenuItem, Outcome};
use radar_core::ui::{connection, rows::Move, status::Check};
use radar_core::view::{Chord, Key};

use crate::{GithubPlugin, auth, config, model::RepositoryId};
pub(crate) mod neutral;
use std::collections::BTreeMap;

pub(crate) struct Editor {
    pub input: TextInput,
    field: Field,
}
#[derive(Clone)]
enum Field {
    Api,
    Token,
    Add,
    RenameGroup(usize),
    Repository(usize, usize),
    MoveToNewGroup(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Add,
    Group(usize),
    Repository(usize, usize),
}

fn order(p: &GithubPlugin) -> Vec<Row> {
    let mut rows = vec![Row::Add];
    for (group, item) in p.cfg.groups.iter().enumerate() {
        rows.push(Row::Group(group));
        rows.extend((0..item.repositories.len()).map(|repo| Row::Repository(group, repo)));
    }
    rows
}

fn selected(p: &GithubPlugin) -> Row {
    order(p).get(p.repository_row).copied().unwrap_or(Row::Add)
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

/// The connection section's rows, in order: where, the token, the check,
/// how often.
pub(crate) const CONNECTION: [&str; 4] =
    ["server", "token", connection::CHECK, connection::REFRESH];

pub(crate) const API_ABOUT: &str = "The GitHub API radar talks to: api.github.com for github.com, or your own GitHub Enterprise Server or a local server. The token is sent only to this address.";
pub(crate) const API_ACCEPTS: &str = "https://api.github.com · a GitHub Enterprise Server's https://HOST/api/v3 · http://127.0.0.1:PORT for a local server";

pub(crate) fn title(p: &GithubPlugin, section: &str) -> String {
    if section == "connection" {
        CONNECTION[p.connection_row.min(3)].into()
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
                    if value.trim().starts_with("env:") {
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
        match p.connection_row {
            0 => (Field::Api, p.cfg.api.clone()),
            1 => (
                Field::Token,
                if p.cfg.token.starts_with("env:") {
                    p.cfg.token.clone()
                } else {
                    String::new()
                },
            ),
            _ => {
                return Outcome::Handled(vec![Effect::Notify(
                    "this setting is not typed; the server and the token are editable".into(),
                )]);
            }
        }
    } else {
        match selected(p) {
            Row::Add => (Field::Add, String::new()),
            Row::Group(g) => (Field::RenameGroup(g), p.cfg.groups[g].name.clone()),
            Row::Repository(g, r) => (
                Field::Repository(g, r),
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
        return Outcome::Handled(vec![Effect::Notify(format!("not yet: {reason}"))]);
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
        Field::RenameGroup(g) => {
            cfg.groups[g].name = value.trim().into();
            cfg.groups[g].owner = typed_owner(&value);
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
        Field::Repository(g, r) => {
            cfg.groups[g].repositories[r] = config::repository(&value).unwrap().full_name()
        }
    }
    let cfg = match cfg.validated() {
        Ok(cfg) => cfg,
        Err(error) => {
            p.editor = Some(Editor {
                input: TextInput::new(&value),
                field,
            });
            return Outcome::Handled(vec![Effect::Notify(error.to_string())]);
        }
    };
    if cfg == p.cfg {
        return Outcome::done();
    }
    let new_group = adding && cfg.groups.len() > p.cfg.groups.len();
    p.apply_config(cfg);
    p.started = true;
    p.repository_row = p.repository_row.min(order(p).len() - 1);
    if let Some(repo) = added_repo {
        p.repository_row = order(p)
            .iter()
            .position(|row| match row {
                Row::Repository(g, r) => p.cfg.groups[*g].repositories[*r] == repo,
                _ => false,
            })
            .unwrap_or(0);
    }
    if moving_to_new {
        p.repository_row = order(p).len() - 1;
        Outcome::Handled(vec![Effect::Notify(
            "repository moved into new group".into(),
        )])
    } else if connection_edit {
        Outcome::Handled(p.check_connection())
    } else if new_group {
        let group = p.cfg.groups.len() - 1;
        p.repository_row = order(p)
            .iter()
            .position(|r| *r == Row::Group(group))
            .unwrap_or(0);
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
    match key.key {
        Key::Char('c') => return run_check(p),
        Key::Char('a') if section == "repositories" => {
            p.discovery_group = target_group(p);
            let owner = p
                .discovery_group
                .and_then(|g| p.cfg.groups[g].owner.clone());
            return Outcome::Handled(p.discover_repositories(owner));
        }
        Key::Enter if section == "connection" && p.connection_row == 2 => return run_check(p),
        Key::Enter if section == "repositories" && matches!(selected(p), Row::Group(_)) => {
            p.discovery_group = target_group(p);
            let owner = p
                .discovery_group
                .and_then(|g| p.cfg.groups[g].owner.clone());
            return Outcome::Handled(p.discover_repositories(owner));
        }
        Key::Enter | Key::Char('e') => return start_edit(p, section),
        Key::Char('d') if section == "repositories" && p.repository_row > 0 => {
            return match selected(p) {
                Row::Group(g) => Outcome::Handled(vec![Effect::Menu(Menu::confirm(
                    format!("remove-group:{g}"),
                    format!(
                        "remove group {} and its repositories?",
                        p.cfg.groups[g].name
                    ),
                    "remove group",
                    "keep it",
                ))]),
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
                return Outcome::Handled(vec![Effect::Menu(Menu::new(
                    format!("move:{from}:{repo}"),
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
        (0..4).collect()
    } else {
        (0..order(p).len()).collect()
    };
    let current = if section == "connection" {
        p.connection_row
    } else {
        p.repository_row
    };
    let at = order.iter().position(|row| *row == current).unwrap_or(0);
    let to = radar_core::ui::rows::select(order.len(), at, movement, |_| true);
    if to == at {
        Outcome::Ignored
    } else {
        if section == "connection" {
            p.connection_row = order[to];
        } else {
            p.repository_row = order[to];
        }
        Outcome::done()
    }
}

fn run_check(p: &mut GithubPlugin) -> Outcome {
    Outcome::Handled(p.check_connection())
}

pub(crate) fn click(p: &mut GithubPlugin, section: &str, id: &str) -> Outcome {
    let Some(index) = id
        .strip_prefix("setting:")
        .and_then(|v| v.parse::<usize>().ok())
    else {
        return Outcome::Ignored;
    };
    if section == "connection" && index < 4 {
        p.connection_row = index;
        Outcome::done()
    } else if section == "repositories" && index < order(p).len() {
        p.repository_row = index;
        Outcome::done()
    } else {
        Outcome::Ignored
    }
}

pub(crate) fn keys(p: &GithubPlugin, section: &str) -> Vec<crate::Act> {
    let act = |verb: &str, b: Binding| crate::act(section, verb, b);
    let mut rows = vec![crate::head(Binding::heading(title(p, section)))];
    if section == "repositories" || p.connection_row <= 1 {
        rows.push(act("edit", Binding::new("e", "change it")));
        rows.push(if section == "repositories" {
            let (verb, what) = match selected(p) {
                Row::Add => ("activate_add", "create group or add repository"),
                Row::Group(_) => ("activate_group", "browse repositories for group"),
                Row::Repository(..) => ("activate_repository", "change repository"),
            };
            act(verb, Binding::new("⏎", what))
        } else {
            act("activate_setting", Binding::new("⏎", "change it"))
        });
    }
    if section == "connection" || section == "repositories" {
        rows.push(act("check", Binding::new("c", "check GitHub connection")));
    }
    if section == "repositories" {
        rows.push(act(
            "browse",
            Binding::new("a", "browse repositories this token can access"),
        ));
    }
    if section == "connection" && p.connection_row == 2 {
        rows.push(act(
            "activate_check",
            Binding::new("⏎", "check GitHub connection"),
        ));
    }
    if section == "repositories" && p.repository_row > 0 {
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
    if matches!(key.key, Key::Char('e' | 'd' | 'c' | 'a' | 'm') | Key::Enter) {
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
            p.repository_row = p.repository_row.min(order(p).len() - 1);
            vec![Effect::Notify("repository removed from radar".into())]
        }
        other if other.starts_with("remove-group:") => {
            if value != "yes" {
                return vec![];
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
            p.repository_row = p.repository_row.min(order(p).len() - 1);
            vec![Effect::Notify("group removed from radar".into())]
        }
        other if other.starts_with("move:") => {
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
            p.repository_row = order(p)
                .iter()
                .position(|r| *r == Row::Group(to))
                .unwrap_or(0);
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
