//! The `workspace` settings section `workspaces`: the workspaces you work
//! from, one row each, with the ones radar found on this machine offered as a
//! question first. Rows explain themselves in the detail panel, validate as
//! you type, and show what was checked (the folder, its clones, where the
//! notes are read from).

use std::collections::BTreeSet;

use radar_core::design::help_bar::KeyHint;
use radar_core::input::{Edit, TextInput};
use radar_core::plugin::{Binding, Effect, Menu, MenuItem, Outcome, SectionSpec, key_text};
use radar_core::system;
use radar_core::ui::connection::Found;
use radar_core::view::{self as v, Ink, Seg, Text};
use radar_core::{time, ui};

use crate::WorkspacePlugin;
use crate::config::{OPEN_AT, Space, clean_folder, expand, tilde};
use crate::find;

pub const SECTIONS: [SectionSpec; 1] = [SectionSpec {
    id: "workspaces",
    name: "workspaces",
    group: "workspace",
}];

/// What is being typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field {
    Add,
    Dir(usize),
    Name(usize),
    Tickets(usize),
    Repos(usize),
    AfterWorktree(usize),
    Notes(usize),
    Editor(usize),
}

/// A row of the section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// A workspace radar found, offered as a question.
    Found(usize),
    Add,
    Space(usize),
}

/// The workspaces found that are not in use and were not passed on.
fn offers(p: &WorkspacePlugin) -> Vec<find::Offer> {
    let known: Vec<_> = p.cfg.spaces.iter().map(Space::top).collect();
    p.state
        .offers
        .iter()
        .flatten()
        .filter(|o| !known.contains(&expand(&o.dir)) && !p.state.passed.contains(&o.dir))
        .cloned()
        .collect()
}

fn rows(p: &WorkspacePlugin) -> Vec<Row> {
    let mut rows: Vec<Row> = (0..offers(p).len()).map(Row::Found).collect();
    rows.push(Row::Add);
    rows.extend((0..p.cfg.spaces.len()).map(Row::Space));
    rows
}

fn selected(p: &WorkspacePlugin) -> Row {
    let rows = rows(p);
    rows[p.state.row.min(rows.len() - 1)]
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// --- validation ------------------------------------------------------------------

/// What typed text would do: taken, taken with a warning, or refused.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    Good(String),
    Warn(String),
    Bad(String),
}

fn editor_verdict(text: &str) -> Verdict {
    let Some(prog) = system::split_command(text).into_iter().next() else {
        return Verdict::Good("radar asks which installed editor to use, the first time".into());
    };
    if system::on_path(&prog) {
        Verdict::Good(format!("{prog} is installed"))
    } else {
        Verdict::Warn(format!(
            "{prog} is not installed here; radar will offer the installed ones instead"
        ))
    }
}

fn folder_verdict(p: &WorkspacePlugin, mine: Option<usize>, text: &str) -> Verdict {
    let folder = clean_folder(text);
    if folder.is_empty() {
        return Verdict::Bad("type or paste a folder".into());
    }
    let path = expand(&folder);
    if !path.is_absolute() {
        Verdict::Bad("a full path, or one starting with ~/".into())
    } else if !path.is_dir() {
        Verdict::Bad("not a folder on this machine".into())
    } else if p
        .cfg
        .spaces
        .iter()
        .enumerate()
        .any(|(j, s)| Some(j) != mine && s.top() == path)
    {
        Verdict::Bad("already a workspace".into())
    } else if !find::is_clone(&path) {
        Verdict::Warn(format!(
            "{} is not a git repo; it works, but radar only offers folders that are",
            tilde(&path.display().to_string())
        ))
    } else {
        Verdict::Good(format!(
            "radar will work from {}",
            tilde(&path.display().to_string())
        ))
    }
}

fn relative_problem(text: &str) -> Option<&'static str> {
    if text.starts_with('/') || text.starts_with('~') {
        Some("a folder under the workspace, not a full path")
    } else if text.split('/').any(|c| c == "..") {
        Some("no `..`: it stays under the workspace")
    } else {
        None
    }
}

fn validate(p: &WorkspacePlugin, field: &Field, text: &str) -> Verdict {
    let text = text.trim();
    match field {
        Field::Add => folder_verdict(p, None, text),
        Field::Dir(i) => folder_verdict(p, Some(*i), text),
        Field::Name(i) => {
            if text.is_empty() {
                Verdict::Good("named after its folder".into())
            } else if p
                .cfg
                .spaces
                .iter()
                .enumerate()
                .any(|(j, s)| j != *i && s.label() == text)
            {
                Verdict::Bad("another workspace has that name".into())
            } else {
                Verdict::Good("what this workspace is called".into())
            }
        }
        Field::Tickets(_) => {
            let words: Vec<&str> = text.split([' ', ',']).filter(|w| !w.is_empty()).collect();
            match words.iter().find(|w| {
                !w.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                    || !w.chars().all(|c| c.is_ascii_alphanumeric())
            }) {
                Some(w) => Verdict::Bad(format!(
                    "{w}: a project key is letters and digits, e.g. ABC"
                )),
                None if words.is_empty() => Verdict::Good(
                    "none listed: it is used for tickets no workspace lists, when it is the first"
                        .into(),
                ),
                None => Verdict::Good(format!(
                    "tickets of {} are worked here",
                    words
                        .iter()
                        .map(|w| w.to_ascii_uppercase())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
        }
        Field::Repos(i) => {
            let folder = text.trim_matches('/');
            if let Some(why) = relative_problem(folder) {
                Verdict::Bad(why.into())
            } else if folder.is_empty() {
                Verdict::Good("the clones are in the workspace's own folder".into())
            } else if p
                .cfg
                .spaces
                .get(*i)
                .is_some_and(|s| !s.top().join(folder).is_dir())
            {
                Verdict::Warn(format!("{folder} is not a folder there yet"))
            } else {
                Verdict::Good(format!("the clones are under {folder}"))
            }
        }
        Field::AfterWorktree(_) => match system::split_command(text).into_iter().next() {
            None => Verdict::Good("nothing runs after a worktree is made".into()),
            Some(prog) if system::on_path(&prog) => {
                Verdict::Good(format!("{prog} runs after a worktree is made"))
            }
            Some(prog) => Verdict::Warn(format!(
                "{prog} is not installed here; it runs in the workspace's folder"
            )),
        },
        Field::Notes(_) => {
            if text.is_empty() || text == "okf" {
                Verdict::Good(
                    "notes are read where .okf.yaml says (else a folder named by the key)".into(),
                )
            } else if let Some(why) = relative_problem(text) {
                Verdict::Bad(why.into())
            } else if !text.contains("{key}") {
                Verdict::Warn("a template should name {key}: where a ticket's notes go".into())
            } else {
                Verdict::Good(format!(
                    "notes are in {} (or a folder named by the key)",
                    crate::okf::static_prefix(text)
                ))
            }
        }
        Field::Editor(_) => editor_verdict(text),
    }
}

// --- drawing ---------------------------------------------------------------------

/// Where the input on a `+ add …` row starts: past the row's mark.
const ADD_AT: u16 = 4;
/// A workspace row's name column, and where a value starts after the margin.
const NAME_W: usize = 12;
const VALUE_AT: u16 = 1 + NAME_W as u16;

/// The section as renderer-neutral data: its rows (each `row:{n}`), the
/// field being typed into on its row, the detail of the selected row and
/// the actions on it.
pub fn section_view(p: &WorkspacePlugin, section: &str, cx: &v::ViewCx<'_>) -> Option<v::SectionView> {
    if !SECTIONS.iter().any(|s| s.id == section) {
        return None;
    }
    let width = cx.width;
    let editing = |f: &Field| p.state.edit.as_ref().is_some_and(|(e, _)| e == f);
    let found = offers(p);
    let mut rows: Vec<v::ListRow> = Vec::new();
    let mut input_at: Option<(usize, u16)> = None;
    for (n, row) in self::rows(p).into_iter().enumerate() {
        let target = format!("row:{n}");
        rows.push(match row {
            Row::Found(i) => v::ListRow::new(target, found_of(&found[i]).text(width)),
            Row::Add => {
                if editing(&Field::Add) {
                    input_at = Some((n, ADD_AT));
                }
                v::ListRow::new(target, add_line("add a workspace", "a folder of clones", width))
            }
            Row::Space(i) => {
                let s = &p.cfg.spaces[i];
                for f in [
                    Field::Dir(i),
                    Field::Name(i),
                    Field::Tickets(i),
                    Field::Repos(i),
                    Field::AfterWorktree(i),
                    Field::Notes(i),
                    Field::Editor(i),
                ] {
                    if editing(&f) {
                        input_at = Some((n, if f == Field::Name(i) { 1 } else { VALUE_AT }));
                    }
                }
                let line = Text::new()
                    .push(Seg::new(format!("{:<NAME_W$}", s.label()), Ink::Emphasis).strong())
                    .push(Seg::new(tilde(&s.dir), Ink::Secondary));
                let right = match p.state.indexes.get(&s.dir) {
                    _ if p.state.indexing.contains(&s.dir) => Text::of("looking", Ink::Info),
                    Some(i) if i.missing => Text::of("not here", Ink::Bad),
                    Some(i) => Text::of(plural(i.clones, "clone", "clones"), Ink::Number),
                    None => Text::of("not looked yet", Ink::Muted),
                };
                v::ListRow::new(target, line).right(right)
            }
        });
    }
    let editing = match (input_at, &p.state.edit) {
        (Some((row, at)), Some((_, input))) => Some(v::Editing {
            row,
            at,
            text: input.value(),
            caret: input.cursor(),
        }),
        _ => None,
    };
    Some(v::SectionView {
        version: v::VERSION,
        scope: section.to_string(),
        body: v::Node::List {
            rows,
            selected: Some(p.state.row),
            editing,
        },
        detail_title: detail_title(p, section),
        detail: detail(p, cx.now),
        detail_blocks: vec![],
        actions: section_actions(p, section),
    })
}

/// The actions of the workspaces section as data, each under its own id. A menu row
/// takes the menu item's value as its verb (`workspace.workspaces.dir`); the add row
/// is named by hand from its key.
fn section_actions(p: &WorkspacePlugin, section: &str) -> Vec<v::ActionSpec> {
    if selected(p) == Row::Add {
        return keys(p, section)
            .iter()
            .map(|b| v::ActionSpec::named(action_id(b), b))
            .collect();
    }
    let Some(menu) = row_menu(p) else {
        return vec![];
    };
    let mut out = vec![v::ActionSpec::named(String::new(), &Binding::heading(menu.title))];
    out.extend(menu.items.iter().filter_map(|i| {
        let key = i.key?;
        let id = format!("workspace.{section}.{}", i.value);
        Some(v::ActionSpec::named(id, &Binding::new(key_text(key), &i.label)))
    }));
    out
}

/// The stable id of the add row, chosen here by hand from its key. A heading has no id.
fn action_id(b: &Binding) -> String {
    if b.is_heading() {
        return String::new();
    }
    match b.keys.as_str() {
        "⏎" => "workspace.workspaces.add".into(),
        other => unreachable!("workspaces has no stable id for key {other:?}"),
    }
}

/// The `+ …` row that starts adding, and what it takes where that fits.
fn add_line(action: &str, takes: &str, width: u16) -> Text {
    let mut line = Text::new()
        .push(Seg::new(" + ", Ink::Accent).strong())
        .push(Seg::new(action, Ink::AccentSecondary).strong());
    let hint = format!("   {takes}");
    if 3 + ui::width(action) + ui::width(&hint) + 2 <= usize::from(width) {
        line.add(Seg::new(hint, Ink::Muted));
    }
    line
}

fn heading(text: &str) -> Text {
    Text::new().push(Seg::new(text.to_uppercase(), Ink::Muted).strong())
}

/// `label` in a muted column `width` cells wide, then `value`.
fn labelled(label: &str, width: usize, value: Text) -> Text {
    let mut line = Text::of(ui::pad(label, width), Ink::Muted);
    line.extend(value);
    line
}

/// A found workspace as the question it is asked as.
fn found_of(o: &find::Offer) -> Found {
    Found {
        what: format!("a workspace with {}", plural(o.clones, "clone", "clones")),
        names: Vec::new(),
        file: Some(tilde(&o.dir)),
        does: "adds it as a workspace".into(),
        note: None,
    }
}

pub fn detail_title(p: &WorkspacePlugin, _section: &str) -> String {
    match selected(p) {
        Row::Found(i) => offers(p)
            .get(i)
            .map_or_else(String::new, |o| found_of(o).title()),
        Row::Add => "add a workspace".into(),
        Row::Space(i) => p.cfg.spaces[i].label(),
    }
}

fn check_line(good: bool, text: String) -> Text {
    let (icon, symbol, ink) = if good {
        (ui::icon::Icon::RunPassed, "✓", Ink::Good)
    } else {
        (ui::icon::Icon::Warning, "⚠", Ink::Warn)
    };
    Text::new()
        .push(Seg::icon_or(icon, symbol, ink))
        .push(Seg::new(format!(" {text}"), ink))
}

/// The detail beside the rows: the live verdict on what is typed first,
/// then what the selected row is.
fn detail(p: &WorkspacePlugin, now: i64) -> Vec<Text> {
    let text = Ink::Primary;
    let mut lines: Vec<Text> = Vec::new();
    if let Some((field, input)) = &p.state.edit {
        lines.push(heading("as you type"));
        lines.push(match validate(p, field, &input.value()) {
            Verdict::Good(m) => Text::of(format!("✓ {m}"), Ink::Good),
            Verdict::Warn(m) => Text::of(format!("⚠ {m}"), Ink::Warn),
            Verdict::Bad(m) => Text::of(format!("✗ {m}"), Ink::Warn),
        });
        lines.push(Text::new());
    }
    let mut key = "";
    match selected(p) {
        Row::Found(i) => {
            if let Some(o) = offers(p).get(i) {
                lines.push(Text::of(
                    format!(
                        "radar found a workspace at {}. Do you want to use it?",
                        tilde(&o.dir)
                    ),
                    text,
                ));
                lines.push(Text::new());
                lines.push(heading("found"));
                lines.push(labelled("folder", 10, Text::of(tilde(&o.dir), text)));
                let clones = format!(
                    "{} {}",
                    plural(o.clones, "clone", "clones"),
                    if o.repos.is_empty() {
                        "in it".to_string()
                    } else {
                        format!("under {}", o.repos)
                    }
                );
                lines.push(labelled("clones", 10, Text::of(clones, Ink::Number)));
                lines.push(Text::new());
                lines.push(Text::of(
                    "A git repo whose folder holds other clones. Using it makes it a workspace: its tickets' notes are read from it, and the editor and a terminal open at its top.",
                    text,
                ));
                key = "[[workspace.space]]";
            }
        }
        Row::Add => {
            lines.push(Text::of(
                "A workspace is the top folder you work from: a git repo that holds your clones, where you and your agents see every repo at once, and where a ticket's notes are kept.",
                text,
            ));
            lines.push(Text::new());
            lines.push(heading("accepts"));
            lines.push(Text::of(
                "a full path, or one starting with ~/ (pasted links to files work too)",
                Ink::Secondary,
            ));
            lines.push(Text::new());
            lines.push(heading("now"));
            lines.push(Text::new().push(
                Seg::new(plural(p.cfg.spaces.len(), "workspace", "workspaces"), Ink::Emphasis).strong(),
            ));
            key = "[[workspace.space]] dir";
        }
        Row::Space(i) => {
            let s = &p.cfg.spaces[i];
            lines.push(heading("checked"));
            match p.state.indexes.get(&s.dir) {
                None if p.state.indexing.contains(&s.dir) => {
                    lines.push(Text::of("looking now…", Ink::Info));
                }
                None => lines.push(Text::of("not looked yet", Ink::Muted)),
                Some(ix) if ix.missing => {
                    lines.push(check_line(
                        false,
                        format!("{} is not a folder on this machine", tilde(&s.dir)),
                    ));
                }
                Some(ix) => {
                    lines.push(check_line(true, plural(ix.clones, "clone found", "clones found")));
                    if ix.okf {
                        lines.push(check_line(
                            true,
                            format!(
                                ".okf.yaml read: knowledge in {}",
                                if ix.roots.is_empty() {
                                    "the workspace".into()
                                } else {
                                    ix.roots.join(", ")
                                }
                            ),
                        ));
                    } else {
                        lines.push(check_line(
                            false,
                            "no .okf.yaml: a ticket's notes are the files in a folder named by its key".into(),
                        ));
                    }
                    let tickets: BTreeSet<&str> = ix.notes.iter().map(|n| n.key.as_str()).collect();
                    lines.push(check_line(
                        !ix.notes.is_empty(),
                        format!(
                            "{} for {} · {} read · {} ago",
                            plural(ix.notes.len(), "note", "notes"),
                            plural(tickets.len(), "ticket", "tickets"),
                            plural(ix.files, "file", "files"),
                            time::ago(ix.at, now)
                        ),
                    ));
                }
            }
            lines.push(Text::new());
            lines.push(Text::of(
                format!(
                    "{} is a workspace: tickets of {} are worked from {}, and the editor and a terminal open at its {}.",
                    s.label(),
                    if s.tickets.is_empty() { "the projects no workspace lists".to_string() } else { s.tickets.join(", ") },
                    tilde(&s.dir),
                    if s.open_at == "repo" { "ticket's notes folder" } else { "top" }
                ),
                text,
            ));
            lines.push(Text::new());
            let kv = |label: &str, value: String| labelled(label, 16, Text::of(value, text));
            lines.push(kv("folder", tilde(&s.dir)));
            lines.push(kv("name", s.label()));
            lines.push(kv(
                "tickets",
                if s.tickets.is_empty() {
                    "—".into()
                } else {
                    s.tickets.join(" ")
                },
            ));
            lines.push(kv(
                "clones in",
                if s.repos.is_empty() {
                    "the folder itself".into()
                } else {
                    s.repos.clone()
                },
            ));
            lines.push(kv("open at", s.open_at.clone()));
            lines.push(kv(
                "editor",
                if s.editor.is_empty() {
                    "asks".into()
                } else {
                    s.editor.clone()
                },
            ));
            lines.push(kv(
                "after worktree",
                if s.after_worktree.is_empty() {
                    "—".into()
                } else {
                    s.after_worktree.clone()
                },
            ));
            lines.push(kv("notes", s.notes.clone()));
            key = "[[workspace.space]] dir, name, tickets, repos, open_at, editor, after_worktree, notes";
        }
    }
    if !key.is_empty() {
        lines.push(Text::new());
        lines.push(Text::of(format!("config: {key}"), Ink::Muted));
    }
    lines
}

// --- menus -----------------------------------------------------------------------

/// The options of the selected row; their keys also work from the row.
fn row_menu(p: &WorkspacePlugin) -> Option<Menu> {
    match selected(p) {
        Row::Add => None,
        Row::Found(i) => {
            let o = offers(p).into_iter().nth(i)?;
            Some(Menu::new(
                format!("ws-found:{i}"),
                tilde(&o.dir),
                vec![
                    MenuItem::new('y', "use it", "add it as a workspace", "use"),
                    MenuItem::new('n', "not now", "stop asking this time", "pass"),
                ],
            ))
        }
        Row::Space(i) => Some(Menu::new(
            format!("ws-space:{i}"),
            p.cfg.spaces[i].label(),
            vec![
                MenuItem::new(
                    'f',
                    "change its folder",
                    "type or paste another folder",
                    "dir",
                ),
                MenuItem::new('n', "rename", "what it is called", "name"),
                MenuItem::new(
                    't',
                    "tickets…",
                    "the tracker projects worked here",
                    "tickets",
                ),
                MenuItem::new(
                    'c',
                    "clones folder…",
                    "where its clones are, under it",
                    "repos",
                ),
                MenuItem::new(
                    'o',
                    "open at…",
                    "the top, or the ticket's notes folder",
                    "open",
                ),
                MenuItem::new('e', "editor…", "the editor it opens in", "editor"),
                MenuItem::new(
                    'p',
                    "after a worktree…",
                    "a command run after one is made",
                    "after",
                ),
                MenuItem::new('m', "notes…", "okf, or a path template", "notes"),
                MenuItem::new(
                    's',
                    "look again",
                    "find its clones and notes again",
                    "reindex",
                ),
                MenuItem::new('a', "add a workspace", "another folder to work from", "add"),
                MenuItem::new('d', "stop using it…", "remove this workspace", "remove"),
            ],
        )),
    }
}

fn edit(p: &mut WorkspacePlugin, field: Field, value: &str) -> Vec<Effect> {
    p.state.edit = Some((field, TextInput::new(value)));
    vec![]
}

/// The menu that asks for a workspace's folder.
pub(crate) const ADD_MENU: &str = "ws-add";

/// Adding a workspace: a field for its folder that offers the workspaces
/// radar found, then the folders the typed path goes on to, and says what the
/// folder holds as you type (`find::describe`).
fn ask_add() -> Vec<Effect> {
    vec![Effect::Menu(
        radar_core::Menu::typed(ADD_MENU, "add a workspace: the folder you work from", "")
            .icon(radar_core::ui::icon::Icon::Folder),
    )]
}

/// What the typed folder is, as a workspace, and what to offer.
pub(crate) fn typing(p: &WorkspacePlugin, menu: &str, text: &str) -> radar_core::Typing {
    if menu != ADD_MENU {
        return radar_core::Typing::default();
    }
    let offers: Vec<MenuItem> = if text.trim().is_empty() {
        offers(p)
            .into_iter()
            .map(|o| {
                let d = tilde(&o.dir);
                MenuItem::pick(d.clone(), format!("found · {}", plural(o.clones, "clone", "clones")), d)
            })
            .chain(std::iter::once(MenuItem::pick("~/", "", "~/")))
            .collect()
    } else {
        system::folders_like(text, 8)
            .into_iter()
            .map(|d| {
                let hint = if find::is_clone(&expand(&clean_folder(&d))) { "a git repo" } else { "" };
                MenuItem::pick(d.clone(), hint, d)
            })
            .collect()
    };
    let verdict = (!text.trim().is_empty()).then(|| match folder_verdict(p, None, text) {
        Verdict::Bad(w) => radar_core::Verdict::Bad(w),
        Verdict::Warn(_) | Verdict::Good(_) => {
            let top = expand(&clean_folder(text));
            if !find::worth_describing(&top) {
                return radar_core::Verdict::Unsure(
                    "not a git repo and no repositories folder: pick the folder that holds your clones".into(),
                );
            }
            let d = find::describe(&top);
            if d.clones == 0 {
                radar_core::Verdict::Unsure(format!("{}: radar works from it, with no repos", d.words()))
            } else {
                radar_core::Verdict::Good(d.words())
            }
        }
    });
    radar_core::Typing { verdict, offers }
}

/// The folder typed was kept: added as described, its clones' folder and
/// ticket projects filled in.
fn add_typed(p: &mut WorkspacePlugin, text: &str) -> Vec<Effect> {
    let dir = clean_folder(text);
    let d = find::describe(&expand(&dir));
    let fx = add_space(p, &dir, &d.repos);
    if let Some(s) = p.cfg.spaces.last_mut() {
        s.tickets = d.projects.clone();
    }
    let mut out = vec![Effect::Notify(format!("added {} · {}", tilde(&dir), d.words()))];
    out.extend(fx);
    out
}

fn add_space(p: &mut WorkspacePlugin, dir: &str, repos: &str) -> Vec<Effect> {
    let mut s = Space::new(dir);
    s.repos = repos.to_string();
    p.cfg.spaces.push(s);
    p.state.row = rows(p).len() - 1;
    p.reindex(dir, false)
}

pub fn on_menu(p: &mut WorkspacePlugin, menu: &str, value: &str) -> Option<Vec<Effect>> {
    if menu == "bulk" {
        return (value == "ws-reindex").then(|| {
            let dirs: Vec<String> = p.cfg.spaces.iter().map(|s| s.dir.clone()).collect();
            dirs.iter().flat_map(|d| p.reindex(d, true)).collect()
        });
    }
    if menu == ADD_MENU {
        return Some(add_typed(p, value));
    }
    let (kind, arg) = menu.split_once(':').unwrap_or((menu, ""));
    if !kind.starts_with("ws-") {
        return None;
    }
    let idx = arg.parse::<usize>().ok();
    Some(match (kind, value) {
        ("ws-found", v) => {
            let o = offers(p).into_iter().nth(idx?)?;
            if v == "use" {
                p.state.passed.remove(&o.dir);
                let fx = add_space(p, &o.dir, &o.repos);
                let mut out = vec![Effect::Notify(format!("using {}", tilde(&o.dir)))];
                out.extend(fx);
                out
            } else {
                p.state.passed.insert(o.dir);
                vec![]
            }
        }
        ("ws-space", "add") => {
            p.state.row = rows(p).iter().position(|r| *r == Row::Add).unwrap_or(0);
            ask_add()
        }
        ("ws-space", v) => {
            let i = idx.filter(|i| *i < p.cfg.spaces.len())?;
            let s = p.cfg.spaces[i].clone();
            match v {
                "dir" => edit(p, Field::Dir(i), &s.dir),
                "name" => edit(p, Field::Name(i), &s.name),
                "tickets" => edit(p, Field::Tickets(i), &s.tickets.join(" ")),
                "repos" => edit(p, Field::Repos(i), &s.repos),
                "editor" => edit(p, Field::Editor(i), &s.editor),
                "after" => edit(p, Field::AfterWorktree(i), &s.after_worktree),
                "notes" => edit(p, Field::Notes(i), &s.notes),
                "open" => vec![Effect::Menu(Menu::new(
                    format!("ws-open:{i}"),
                    format!("{} opens at…", s.label()),
                    OPEN_AT
                        .iter()
                        .map(|w| {
                            let hint = if *w == "top" {
                                "the workspace's folder: every repo in view"
                            } else {
                                "the ticket's notes folder, when it has one"
                            };
                            let hint = if *w == s.open_at {
                                format!("{hint} · in use")
                            } else {
                                hint.to_string()
                            };
                            MenuItem::pick((*w).to_string(), hint, (*w).to_string())
                        })
                        .collect(),
                ))],
                "reindex" => p.reindex(&s.dir, true),
                "remove" => vec![Effect::Menu(
                    Menu::confirm(
                        format!("ws-remove:{i}"),
                        format!("stop using the workspace {}?", s.label()),
                        "stop using it",
                        "keep it",
                    )
                    .saying("its folder and notes are not touched"),
                )],
                _ => vec![],
            }
        }
        ("ws-open", v) => {
            let i = idx.filter(|i| *i < p.cfg.spaces.len())?;
            if OPEN_AT.contains(&v) {
                p.cfg.spaces[i].open_at = v.to_string();
                vec![Effect::Notify(format!("opens at {v}"))]
            } else {
                vec![]
            }
        }
        ("ws-remove", "yes") => {
            let i = idx.filter(|i| *i < p.cfg.spaces.len())?;
            let s = p.cfg.spaces.remove(i);
            p.forget(&s.dir);
            p.state.row = 0;
            vec![
                Effect::PersistCache,
                Effect::Notify(format!("stopped using {}", s.label())),
            ]
        }
        ("ws-remove", _) => vec![],
        _ => return None,
    })
}

// --- keys ------------------------------------------------------------------------

fn commit(p: &mut WorkspacePlugin, field: Field, text: String) -> Vec<Effect> {
    match field {
        Field::Add => {
            let dir = clean_folder(&text);
            let repos = if expand(&dir).join("repositories").is_dir() {
                "repositories"
            } else {
                ""
            };
            let fx = add_space(p, &dir, repos);
            let mut out = vec![Effect::Notify(format!("added {}", tilde(&dir)))];
            out.extend(fx);
            out
        }
        Field::Dir(i) => {
            let old = std::mem::replace(&mut p.cfg.spaces[i].dir, clean_folder(&text));
            p.forget(&old);
            let new = p.cfg.spaces[i].dir.clone();
            p.reindex(&new, false)
        }
        Field::Name(i) => {
            p.cfg.spaces[i].name = text;
            vec![]
        }
        Field::Tickets(i) => {
            p.cfg.spaces[i].tickets = text
                .split([' ', ','])
                .filter(|w| !w.is_empty())
                .map(str::to_ascii_uppercase)
                .collect();
            vec![]
        }
        Field::Repos(i) => {
            p.cfg.spaces[i].repos = text.trim_matches('/').to_string();
            let dir = p.cfg.spaces[i].dir.clone();
            p.forget(&dir);
            p.reindex(&dir, false)
        }
        Field::AfterWorktree(i) => {
            p.cfg.spaces[i].after_worktree = text;
            vec![]
        }
        Field::Notes(i) => {
            p.cfg.spaces[i].notes = if text.is_empty() { "okf".into() } else { text };
            let dir = p.cfg.spaces[i].dir.clone();
            p.forget(&dir);
            p.reindex(&dir, false)
        }
        Field::Editor(i) => {
            p.cfg.spaces[i].editor = text;
            vec![Effect::Notify("changed".into())]
        }
    }
}

/// What the user did in the section, from any frontend: a row chosen, `⏎`
/// on it, a key, or a key typed into its field.
pub fn intent(p: &mut WorkspacePlugin, section: &str, intent: &v::Intent) -> Outcome {
    use v::Intent;
    match intent {
        Intent::Select { target } | Intent::Context { target } => click(p, section, target.as_str()),
        Intent::Activate { target } => {
            click(p, section, target.as_str());
            press(p, v::Chord::new(v::Key::Enter))
        }
        Intent::Fold { .. } | Intent::Show { .. } | Intent::End { .. } | Intent::EditSelect { .. } | Intent::EditUnselect | Intent::Dock(_) | Intent::Copy { .. } => Outcome::Ignored,
        Intent::Key { .. } | Intent::Invoke { .. } => match intent.chord() {
            Some(chord) => press(p, chord),
            None => Outcome::Ignored,
        },
    }
}

/// A key: typed into the field being edited, or on the selected row (`⏎`
/// adds, uses or changes it, an option's key does it, `a` adds).
fn press(p: &mut WorkspacePlugin, key: v::Chord) -> Outcome {
    if let Some((field, mut input)) = p.state.edit.take() {
        return match input.chord(key) {
            Edit::Cancel => Outcome::done(),
            Edit::Typing => {
                p.state.edit = Some((field, input));
                Outcome::done()
            }
            Edit::Submit(text) => match validate(p, &field, &text) {
                Verdict::Bad(why) => {
                    p.state.edit = Some((field, input));
                    Outcome::Handled(vec![Effect::Notify(format!("not yet: {why}"))])
                }
                Verdict::Good(_) | Verdict::Warn(_) => {
                    Outcome::Handled(commit(p, field, text.trim().to_string()))
                }
            },
        };
    }
    if key.ctrl || key.alt {
        return Outcome::Ignored;
    }
    let on = selected(p);
    if key.key == v::Key::Enter {
        return match on {
            Row::Add => Outcome::Handled(ask_add()),
            Row::Found(_) => Outcome::Handled(
                on_menu(p, &row_menu(p).map(|m| m.id).unwrap_or_default(), "use")
                    .unwrap_or_default(),
            ),
            Row::Space(i) => Outcome::Handled(edit(p, Field::Dir(i), &p.cfg.spaces[i].dir.clone())),
        };
    }
    if let v::Key::Char(c) = key.key
        && let Some(menu) = row_menu(p)
        && let Some(item) = menu.items.iter().find(|i| i.key == Some(c))
    {
        let value = item.value.clone();
        return Outcome::Handled(on_menu(p, &menu.id, &value).unwrap_or_default());
    }
    match key.key {
        v::Key::Char('a') => {
            p.state.row = rows(p).iter().position(|r| *r == Row::Add).unwrap_or(0);
            Outcome::Handled(ask_add())
        }
        // `x` shows the options; `⏎` on a workspace changes its folder.
        _ => Outcome::Ignored,
    }
}

fn click(p: &mut WorkspacePlugin, _section: &str, id: &str) -> Outcome {
    if p.state.edit.is_none()
        && let Some(i) = id
            .strip_prefix("row:")
            .and_then(|n| n.parse::<usize>().ok())
    {
        p.state.row = i.min(rows(p).len().saturating_sub(1));
    }
    Outcome::done()
}

pub fn hints(p: &WorkspacePlugin) -> Vec<KeyHint<'static>> {
    if p.state.edit.is_some() {
        vec![KeyHint::new("⏎", "keep"), KeyHint::new("esc", "cancel")]
    } else {
        vec![]
    }
}

/// The actions menu (`x`) for the selected row.
pub fn keys(p: &WorkspacePlugin, _section: &str) -> Vec<Binding> {
    if selected(p) == Row::Add {
        return vec![
            Binding::heading("add a workspace"),
            Binding::new("⏎", "type or paste the folder you work from"),
        ];
    }
    let Some(menu) = row_menu(p) else {
        return vec![];
    };
    let mut b = vec![Binding::heading(menu.title)];
    b.extend(
        menu.items
            .iter()
            .filter_map(|i| Some(Binding::new(key_text(i.key?), &i.label))),
    );
    b
}

pub fn bulk(p: &WorkspacePlugin, _scope: &str) -> Vec<MenuItem> {
    if p.cfg.spaces.is_empty() {
        vec![]
    } else {
        vec![MenuItem::new(
            's',
            "look for notes and clones again",
            format!(
                "in {}",
                plural(p.cfg.spaces.len(), "workspace", "workspaces")
            ),
            "ws-reindex",
        )]
    }
}
