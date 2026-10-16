//! The `git` settings sections. `checkouts`: the folders radar looks in for
//! checkouts, how far down, and the last look. `terminal`: which terminal
//! opens in a checkout, what radar found and how.
//! `editors`: the project types, the marker files that tell them apart,
//! and the editor each opens in. Rows explain themselves in the detail
//! panel, validate as you type, and a change to where radar looks offers to
//! look again (the check of these settings).

use radar_core::design::help_bar::KeyHint;
use radar_core::input::{Edit, TextInput};
use radar_core::plugin::{Binding, Effect, Menu, MenuItem, Outcome, SectionSpec, key_text};
#[cfg(test)]
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent};
use radar_core::system;
use radar_core::time;

use radar_core::ui;
use radar_core::view::{self as v, Ink, Seg, Text};

use crate::GitPlugin;
use crate::branch::{self, Convention, Ticket};
use crate::local::{self, DEPTH_MAX, Link, ProjectType};

/// Where the input on a `+ add …` row starts: past the row's mark.
const ADD_AT: u16 = 4;

pub const SECTIONS: [SectionSpec; 5] = [
    SectionSpec {
        id: "checkouts",
        name: "checkouts",
        group: "git",
    },
    SectionSpec {
        id: "terminal",
        name: "terminal",
        group: "git",
    },
    SectionSpec {
        id: "editors",
        name: "editors",
        group: "git",
    },
    SectionSpec {
        id: "links",
        name: "links",
        group: "git",
    },
    SectionSpec {
        id: "branches",
        name: "branches",
        group: "git",
    },
];

/// What is being typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field {
    RootAdd,
    Root(usize),
    Depth,
    Terminal,
    TypeAdd,
    TypeName(usize),
    Markers(usize),
    Editor(usize),
    /// A link's repo: a new one (then its folder is asked) or link `i`.
    LinkRepo(Option<usize>),
    /// A link's folder: for a new link of the repo, or link `i`.
    LinkDir(Option<usize>, String),
    LinkBranch(usize),
    LinkBase(usize),
    /// A new convention: `under template [base]`.
    ConvAdd,
    ConvUnder(usize),
    ConvBranch(usize),
    ConvBase(usize),
    Me,
    Branch,
    Worktree,
    AfterWorktree,
    TypeMap,
}

/// A row of the checkouts section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Add,
    Root(usize),
    /// A folder in the home folder found holding checkouts, not looked in
    /// yet (`p.local.places[i]`): ⏎ looks in it.
    Found(usize),
    Depth,
    Scan,
}

/// The found places not looked in yet, by their index in `p.local.places`.
fn found(p: &GitPlugin) -> Vec<usize> {
    let roots: Vec<String> = p.cfg.roots.iter().map(|r| local::clean_folder(r)).collect();
    (0..p.local.places.len())
        .filter(|&i| !roots.contains(&local::clean_folder(&p.local.places[i].0)))
        .collect()
}

fn rows(p: &GitPlugin) -> Vec<Row> {
    let mut rows = vec![Row::Add];
    rows.extend((0..p.cfg.roots.len()).map(Row::Root));
    rows.extend(found(p).into_iter().map(Row::Found));
    rows.extend([Row::Depth, Row::Scan]);
    rows
}

/// Whether the section's first row is a `+ add …` row.
fn has_add(section: &str) -> bool {
    section != "terminal"
}

/// The terminal that opens in a checkout and how it was chosen: the
/// command you set, else what radar found here and how.
fn terminal_now(p: &GitPlugin) -> (String, String) {
    let chosen = p.local.terminals.as_ref().map(|t| t.chosen.clone());
    if !p.cfg.terminal.is_empty() {
        let how = match chosen {
            Some(Some((prog, _))) if prog == "tmux" => "set by you; inside tmux a tmux window opens",
            _ => "set by you",
        };
        return (p.cfg.terminal.clone(), how.into());
    }
    match chosen {
        Some(Some(found)) => found,
        Some(None) => ("none found".into(), "radar copies `cd <folder>` instead".into()),
        None => ("radar finds one".into(), "when it opens".into()),
    }
}

/// Look in found place `i` from now on, and look now.
fn follow(p: &mut GitPlugin, i: usize) -> Vec<Effect> {
    let Some((place, _)) = p.local.places.get(i).cloned() else { return vec![] };
    p.cfg.roots.push(local::clean_folder(&place));
    let mut fx = vec![Effect::Notify(format!("looking in {place} from now on"))];
    p.local.started = true;
    fx.extend(local::scan_now(p, true));
    fx
}

fn at(section: &str) -> usize {
    match section {
        "editors" => 1,
        "links" => 2,
        "branches" => 3,
        "terminal" => 4,
        _ => 0,
    }
}

fn row_count(p: &GitPlugin, section: &str) -> usize {
    match section {
        "terminal" => 1,
        "editors" => p.cfg.types.len() + 1,
        "links" => p.cfg.links.len() + 1,
        "branches" => brows(p).len(),
        _ => rows(p).len(),
    }
}

/// A row of the branches section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BRow {
    Add,
    Conv(usize),
    Me,
    Branch,
    Worktree,
    After,
    Types,
}

fn brows(p: &GitPlugin) -> Vec<BRow> {
    let mut rows = vec![BRow::Add];
    rows.extend((0..p.cfg.conventions.len()).map(BRow::Conv));
    rows.extend([
        BRow::Me,
        BRow::Branch,
        BRow::Worktree,
        BRow::After,
        BRow::Types,
    ]);
    rows
}

fn selected_brow(p: &GitPlugin) -> BRow {
    let rows = brows(p);
    rows[p.local.rows[3].min(rows.len() - 1)]
}

/// The link the links section's cursor is on; `None` on `+ add`.
fn selected_link(p: &GitPlugin) -> Option<usize> {
    p.local.rows[2]
        .checked_sub(1)
        .filter(|i| *i < p.cfg.links.len())
}

// --- branch names ----------------------------------------------------------------

/// What a branch name template is shown against.
const SAMPLE: Ticket<'static> = Ticket {
    key: "ABC-12",
    title: "Add a stats endpoint",
    kind: "Story",
};

/// The sample ticket's branch name under `template`.
fn sample_branch(p: &GitPlugin, template: &str) -> String {
    let me = if p.cfg.me.is_empty() {
        "you"
    } else {
        &p.cfg.me
    };
    branch::render(template, me, &SAMPLE, &p.cfg.type_map)
}

/// A branch name template: refused when git would refuse the names it makes
/// (empty is fine where `empty_ok`: "not set here").
fn template_verdict(p: &GitPlugin, text: &str, empty_ok: Option<&str>) -> Verdict {
    if text.is_empty() {
        return match empty_ok {
            Some(said) => Verdict::Good(said.into()),
            None => Verdict::Bad("needs a template with {key} in it, e.g. {me}/{key}".into()),
        };
    }
    match branch::problem(text) {
        Some(why) => Verdict::Bad(why.into()),
        None => Verdict::Good(format!(
            "ABC-12 'Add a stats endpoint' (Story) → {}",
            sample_branch(p, text)
        )),
    }
}

fn base_verdict(text: &str) -> Verdict {
    if text.is_empty() {
        Verdict::Good("origin's default branch, else main".into())
    } else if text.contains(char::is_whitespace) {
        Verdict::Bad("a branch name is one word".into())
    } else if let Err(why) = branch::valid_ref(text) {
        Verdict::Bad(why.into())
    } else {
        Verdict::Good(format!("new branches start from origin/{text}"))
    }
}

/// `story=feature bug=fix` as a map, keys lower case.
fn parse_types(text: &str) -> Result<std::collections::BTreeMap<String, String>, &'static str> {
    let mut map = std::collections::BTreeMap::new();
    for word in text.split_whitespace() {
        match word.split_once('=') {
            Some((k, v)) if !k.is_empty() && !v.is_empty() && !v.contains('=') => {
                map.insert(k.to_lowercase(), v.to_string());
            }
            _ => return Err("pairs like story=feature, space between"),
        }
    }
    Ok(map)
}

fn types_text(p: &GitPlugin) -> String {
    p.cfg
        .type_map
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// How a link stands: the clone's branch, or why it is not usable.
fn link_state(p: &GitPlugin, l: &Link) -> Verdict {
    if crate::scoped::is_scoped(&l.repo) {
        if crate::scoped::repository(&l.repo).is_none() {
            return Verdict::Bad("use a canonical HTTPS repository URL".into());
        }
        let dir = local::expand(&l.dir).display().to_string();
        return match p.local.linked.iter().find(|c| c.dir == dir) {
            Some(c) if crate::scoped::remote(&c.remote) == crate::scoped::repository(&l.repo) => {
                Verdict::Good(c.branch.clone())
            }
            Some(_) => Verdict::Bad("folder origin does not match this host and repository".into()),
            None => {
                Verdict::Warn("folder has not been verified; scan or save the link again".into())
            }
        };
    }
    let dir = local::expand(&l.dir);
    match p
        .local
        .linked
        .iter()
        .find(|c| c.dir == dir.display().to_string())
    {
        None if !dir.is_dir() => Verdict::Bad("no such folder".into()),
        None => Verdict::Bad("not a clone".into()),
        Some(c) if c.project.is_empty() || c.project.eq_ignore_ascii_case(&l.repo) => {
            Verdict::Good(c.branch.clone())
        }
        Some(c) => Verdict::Warn(format!("origin: {}", c.project)),
    }
}

/// The same, in the words a typed menu takes (a repo page's `link to a
/// local folder…`).
pub(crate) fn link_dir_typed(repo: &str, text: &str) -> radar_core::Verdict {
    match link_dir_verdict(repo, text) {
        Verdict::Good(_) => radar_core::Verdict::Good(
            match local::read_checkout(&local::expand(&local::clean_folder(text))) {
                Some(c) => format!("a clone of it, on {} · ⏎ links it", c.branch),
                None => "⏎ links it".into(),
            },
        ),
        Verdict::Warn(w) => radar_core::Verdict::Unsure(w),
        Verdict::Bad(w) => radar_core::Verdict::Bad(w),
    }
}

/// Typing a link's folder: a clone, and does its origin name the repo?
fn link_dir_verdict(repo: &str, text: &str) -> Verdict {
    if crate::scoped::is_scoped(repo) {
        return match crate::scoped::folder_problem(text) {
            Some(why) => Verdict::Bad(why.into()),
            None => {
                Verdict::Warn("Enter verifies the folder and matching origin before saving".into())
            }
        };
    }
    let folder = local::clean_folder(text);
    if folder.is_empty() {
        return Verdict::Bad("type or paste the folder of your clone".into());
    }
    let path = local::expand(&folder);
    if !path.is_absolute() {
        return Verdict::Bad("a full path, or one starting with ~/".into());
    }
    if !path.is_dir() {
        return Verdict::Bad("not a folder on this machine".into());
    }
    match local::read_checkout(&path) {
        None => Verdict::Bad("not a git clone with an origin remote".into()),
        Some(c) if c.project.eq_ignore_ascii_case(repo) => {
            Verdict::Good(format!("a clone of {repo}, on {}", c.branch))
        }
        Some(c) if c.project.is_empty() => {
            Verdict::Warn("its origin names no repo radar can read; linked anyway".into())
        }
        Some(c) => Verdict::Warn(format!(
            "its origin names {}, not {repo}; linked anyway",
            c.project
        )),
    }
}

fn selected_row(p: &GitPlugin) -> Row {
    let rows = rows(p);
    rows[p.local.rows[0].min(rows.len() - 1)]
}

/// The project type the editors section's cursor is on; `None` on `+ add`.
fn selected_type(p: &GitPlugin) -> Option<usize> {
    p.local.rows[1]
        .checked_sub(1)
        .filter(|i| *i < p.cfg.types.len())
}

// --- validation ------------------------------------------------------------------

/// What typed text would do: taken, taken with a warning, or refused.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    Good(String),
    Warn(String),
    Bad(String),
}

fn name_problem(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("needs a name, e.g. elixir")
    } else if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '#'))
    {
        Some("a name is one word: letters, digits, - _ + #")
    } else {
        None
    }
}

fn markers_problem(markers: &[&str]) -> Option<&'static str> {
    if markers.is_empty() {
        Some("needs at least one marker file, e.g. mix.exs")
    } else if markers.iter().any(|m| m.contains(['/', '\\'])) {
        Some("marker files are names at a checkout's top, without folders")
    } else {
        None
    }
}

/// How many known checkouts have one of `markers` at their top.
fn having(p: &GitPlugin, markers: &[&str]) -> usize {
    p.local
        .index
        .iter()
        .flat_map(|i| i.checkouts.iter())
        .filter(|c| {
            markers
                .iter()
                .any(|m| c.files.iter().any(|f| local::marker_matches(m, f)))
        })
        .count()
}

fn editor_verdict(text: &str) -> Verdict {
    let Some(prog) = system::split_command(text).into_iter().next() else {
        return Verdict::Good("radar asks which installed editor to use, each time".into());
    };
    if system::on_path(&prog) {
        let kind = if system::runs_in_terminal(text) {
            "in a terminal"
        } else {
            "in its own window"
        };
        Verdict::Good(format!("{prog} is installed; it opens {kind}"))
    } else {
        Verdict::Warn(format!(
            "{prog} is not installed here; radar will offer the installed ones instead"
        ))
    }
}

fn validate(p: &GitPlugin, field: &Field, text: &str) -> Verdict {
    let text = text.trim();
    match field {
        Field::RootAdd | Field::Root(_) => {
            let folder = local::clean_folder(text);
            if folder.is_empty() {
                return Verdict::Bad("type or paste a folder".into());
            }
            let path = local::expand(&folder);
            let mine = match field {
                Field::Root(i) => Some(*i),
                _ => None,
            };
            if !path.is_absolute() {
                Verdict::Bad("a full path, or one starting with ~/".into())
            } else if !path.is_dir() {
                Verdict::Bad("not a folder on this machine".into())
            } else if p
                .cfg
                .roots
                .iter()
                .enumerate()
                .any(|(j, r)| Some(j) != mine && local::expand(r) == path)
            {
                Verdict::Bad("already in the list".into())
            } else {
                Verdict::Good(format!(
                    "radar will look in {}",
                    local::tilde(&path.display().to_string())
                ))
            }
        }
        Field::Depth => match text.parse::<usize>() {
            Ok(n) if (1..=DEPTH_MAX).contains(&n) => Verdict::Good(format!(
                "{n} folder{} down from each",
                if n == 1 { "" } else { "s" }
            )),
            _ => Verdict::Bad(format!("a whole number from 1 to {DEPTH_MAX}")),
        },
        Field::Terminal => {
            let Some(prog) = system::split_command(text).into_iter().next() else {
                return Verdict::Good(
                    "radar finds one: a tmux window inside tmux, else your desktop's terminal".into(),
                );
            };
            if system::on_path(&prog) {
                Verdict::Good(format!("{prog} is installed"))
            } else {
                Verdict::Warn(format!(
                    "{prog} is not installed here; radar will find another terminal"
                ))
            }
        }
        Field::TypeAdd => {
            let mut words = text.split_whitespace();
            let name = words.next().unwrap_or_default();
            let markers: Vec<&str> = words.collect();
            if let Some(why) = name_problem(name) {
                Verdict::Bad(why.into())
            } else if p.cfg.types.iter().any(|t| t.name == name) {
                Verdict::Bad(format!("{name} is already a type"))
            } else if let Some(why) = markers_problem(&markers) {
                Verdict::Bad(why.into())
            } else {
                Verdict::Good(format!(
                    "{name}: a checkout with {} · {} here",
                    markers.join(" or "),
                    having(p, &markers)
                ))
            }
        }
        Field::TypeName(i) => match name_problem(text) {
            Some(why) => Verdict::Bad(why.into()),
            None if p
                .cfg
                .types
                .iter()
                .enumerate()
                .any(|(j, t)| j != *i && t.name == text) =>
            {
                Verdict::Bad("another type has that name".into())
            }
            None => Verdict::Good("a project type".into()),
        },
        Field::Markers(_) => {
            let markers: Vec<&str> = text.split_whitespace().collect();
            match markers_problem(&markers) {
                Some(why) => Verdict::Bad(why.into()),
                None => {
                    let n = having(p, &markers);
                    Verdict::Good(format!(
                        "{} {} one",
                        plural(n, "known checkout", "known checkouts"),
                        if n == 1 { "has" } else { "have" }
                    ))
                }
            }
        }
        Field::Editor(_) => editor_verdict(text),
        Field::LinkRepo(mine) => {
            if crate::scoped::is_scoped(text) && crate::scoped::repository(text).is_none() {
                return Verdict::Bad("use a canonical lowercase HTTPS repository URL without credentials, query or fragment".into());
            }
            if text.is_empty() || !text.contains('/') || text.contains(char::is_whitespace) {
                Verdict::Bad("the repo's full path, e.g. acme/team-a/api".into())
            } else if p
                .cfg
                .links
                .iter()
                .enumerate()
                .any(|(j, l)| Some(j) != *mine && l.repo.eq_ignore_ascii_case(text))
            {
                Verdict::Bad("that repo is already linked".into())
            } else {
                Verdict::Good(match local::checkouts_for(p, text).len() {
                    0 => format!("{text}: radar found no clone of it; you name one next"),
                    n => format!(
                        "{text}: {} found by origin; the link wins",
                        plural(n, "checkout", "checkouts")
                    ),
                })
            }
        }
        Field::LinkDir(_, repo) => link_dir_verdict(repo, text),
        Field::LinkBranch(_) => template_verdict(p, text, Some("uses this repo's convention")),
        Field::LinkBase(_) | Field::ConvBase(_) => base_verdict(text),
        Field::ConvAdd => {
            let mut words = text.split_whitespace();
            let (under, template, base) = (
                words.next().unwrap_or_default(),
                words.next().unwrap_or_default(),
                words.next().unwrap_or_default(),
            );
            if let Some(why) = under_problem(p, under, None) {
                Verdict::Bad(why)
            } else if let Verdict::Bad(why) = template_verdict(p, template, None) {
                Verdict::Bad(why)
            } else if let Verdict::Bad(why) = base_verdict(base) {
                Verdict::Bad(why)
            } else if words.next().is_some() {
                Verdict::Bad("under, template, base: three words at most".into())
            } else {
                template_verdict(p, template, None)
            }
        }
        Field::ConvUnder(i) => match under_problem(p, text, Some(*i)) {
            Some(why) => Verdict::Bad(why),
            None => Verdict::Good(format!(
                "{} here",
                plural(known_under(p, text), "known checkout", "known checkouts")
            )),
        },
        Field::ConvBranch(_) => template_verdict(p, text, Some("uses the default template")),
        Field::Me => {
            if text.is_empty() {
                Verdict::Good("{me} stays as written until you set it".into())
            } else if let Err(why) = branch::valid_ref(text) {
                Verdict::Bad(why.into())
            } else {
                Verdict::Good(format!("{{me}} is {text}"))
            }
        }
        Field::Branch => template_verdict(p, text, None),
        Field::Worktree => {
            if text.is_empty() {
                Verdict::Bad("needs a template, e.g. {clone}-{topic}".into())
            } else if !text.contains("{topic}") {
                Verdict::Warn("without {topic} two worktrees of one clone would clash".into())
            } else {
                Verdict::Good(format!("→ {}", worktree_sample(text)))
            }
        }
        Field::AfterWorktree => match system::split_command(text).into_iter().next() {
            None => Verdict::Good("nothing runs in a new worktree".into()),
            Some(prog) if system::on_path(&prog) => Verdict::Good(format!("{prog} is installed")),
            Some(prog) => Verdict::Warn(format!("{prog} is not installed here")),
        },
        Field::TypeMap => match parse_types(text) {
            Ok(m) => Verdict::Good(format!("{} mapped", plural(m.len(), "type", "types"))),
            Err(why) => Verdict::Bad(why.into()),
        },
    }
}

fn worktree_sample(template: &str) -> String {
    template
        .replace("{clone}", "~/code/api")
        .replace("{topic}", "ABC-12")
}

/// How many known checkouts have a repo path under `under`.
fn known_under(p: &GitPlugin, under: &str) -> usize {
    p.local
        .index
        .iter()
        .flat_map(|i| i.checkouts.iter())
        .filter(|c| {
            c.main.is_empty() && (c.project == under || c.project.starts_with(&format!("{under}/")))
        })
        .count()
}

fn under_problem(p: &GitPlugin, under: &str, mine: Option<usize>) -> Option<String> {
    if under.is_empty() || under.contains(char::is_whitespace) || under.starts_with('/') {
        Some("a repo path or a folder of them, e.g. acme/team-a".into())
    } else if p
        .cfg
        .conventions
        .iter()
        .enumerate()
        .any(|(j, c)| Some(j) != mine && c.under == under)
    {
        Some("already has a convention".into())
    } else {
        None
    }
}

// --- drawing ---------------------------------------------------------------------

/// How many known checkouts are under a root.
fn under(p: &GitPlugin, root: &str) -> usize {
    let base = local::expand(root);
    p.local
        .index
        .iter()
        .flat_map(|i| i.checkouts.iter())
        .filter(|c| std::path::Path::new(&c.dir).starts_with(&base))
        .count()
}

fn missing(p: &GitPlugin, root: &str) -> bool {
    p.local
        .index
        .as_ref()
        .is_some_and(|i| i.missing.iter().any(|m| m == root))
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The last look, in words; `full` adds what the row has no room for.
fn last_look(p: &GitPlugin, now: i64, full: bool) -> String {
    if p.local.scanning {
        return "looking now…".into();
    }
    match &p.local.index {
        None => "not yet".into(),
        Some(i) => {
            let mut s = format!("{} ago", time::ago(i.at, now));
            if full {
                s.push_str(&format!(
                    " · {}, {} with a remote, in {}",
                    plural(i.checkouts.len(), "checkout", "checkouts"),
                    i.checkouts.iter().filter(|c| !c.project.is_empty()).count(),
                    plural(i.folders, "folder", "folders")
                ));
            }
            if i.cut {
                s.push_str(" · stopped early: too many folders");
            }
            s
        }
    }
}

type Drawn = (Text, Option<Text>);

/// The checkouts section's label column.
const LABEL: usize = 11;
/// Where an input on a labelled row starts: the row's margin, the labels.
const VALUE_AT: u16 = 1 + LABEL as u16;

fn chars(s: &str) -> usize {
    s.chars().count()
}

/// `s` cut to `n` cells, keeping its start and marking the cut with `…`.
fn cut_head(s: &str, n: usize) -> String {
    if chars(s) <= n {
        s.to_string()
    } else {
        let keep: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{keep}…")
    }
}

/// `s` cut to `n` cells, keeping its end (a repo path's last part is the
/// part that tells repos apart) and marking the cut with `…`.
fn cut_tail(s: &str, n: usize) -> String {
    let len = chars(s);
    if len <= n {
        s.to_string()
    } else {
        let keep: String = s.chars().skip(len - n.saturating_sub(1)).collect();
        format!("…{keep}")
    }
}

/// The shortest a convention's repo path is cut to, to keep its template whole.
const LABEL_LEAST: usize = 5;

/// A row `label → value` with a status on the right. Link rows share space
/// between repo and folder; convention rows keep the template whole first.
fn pair_row(
    label: &str,
    value: String,
    value_ink: Ink,
    right: Option<(String, Ink)>,
    width: u16,
    balanced: bool,
) -> Drawn {
    // The row's margins and the gap before the status.
    let avail = usize::from(width).saturating_sub(4);
    let right = right.map(|(s, slot)| {
        (
            cut_head(&s, (avail / 3).max(if balanced { 8 } else { 10 })),
            slot,
        )
    });
    let right_w = right.as_ref().map_or(0, |(s, _)| chars(s) + 2);
    let (label_text, value_text, keep_right) = if balanced {
        // Only very narrow link rows lose the status. A long diagnostic cannot
        // crowd out either path.
        let keep_right = right.is_some() && avail >= 21;
        let room = avail.saturating_sub((if keep_right { right_w } else { 0 }) + 3);
        let label_w = chars(label)
            .min(room.div_ceil(2))
            .max(room.saturating_sub(chars(&value)));
        let value_w = room.saturating_sub(label_w);
        (
            cut_tail(label, label_w),
            cut_head(&value, value_w),
            keep_right,
        )
    } else {
        'fit: {
            for keep in [true, false] {
                if keep && right.is_none() {
                    continue;
                }
                let room = avail.saturating_sub(if keep { right_w } else { 0 });
                if chars(label) + 3 + chars(&value) <= room {
                    break 'fit (label.to_string(), value.clone(), keep);
                }
                let for_label = room.saturating_sub(3 + chars(&value));
                if for_label >= LABEL_LEAST {
                    break 'fit (cut_tail(label, for_label), value.clone(), keep);
                }
            }
            let for_value = avail.saturating_sub(3 + LABEL_LEAST);
            (
                cut_tail(label, LABEL_LEAST),
                cut_head(&value, for_value),
                false,
            )
        }
    };
    let line = Text::new()
        .push(Seg::new(label_text, Ink::Emphasis).strong())
        .push(Seg::new(" → ", Ink::Muted))
        .push(Seg::new(value_text, value_ink));
    let right = right
        .filter(|_| keep_right)
        .map(|(s, ink)| Text::of(s, ink));
    (line, right)
}

/// `label` in a muted column `width` cells wide, then `value`.
fn labelled(label: &str, width: usize, value: Text) -> Text {
    let mut line = Text::of(ui::pad(label, width), Ink::Muted);
    line.extend(value);
    line
}

/// A labelled row (`branch  {me}/{key}`) with a sample on the right: the
/// value stays whole, and the sample goes when there is no room for both.
fn kv_row(
    label: &str,
    value: &str,
    fallback: &str,
    sample: Option<(String, Ink)>,
    width: u16,
) -> Drawn {
    let avail = usize::from(width).saturating_sub(4 + LABEL + 1);
    let (text, ink) = if value.is_empty() {
        (fallback.to_string(), Ink::Muted)
    } else {
        (value.to_string(), Ink::Primary)
    };
    let keep = sample
        .as_ref()
        .is_some_and(|(s, _)| chars(&text) + 2 + chars(s) <= avail);
    let line = labelled(label, LABEL, Text::of(cut_head(&text, avail), ink));
    let right = sample.filter(|_| keep).map(|(s, ink)| Text::of(s, ink));
    (line, right)
}

/// A section's rows as view data (each row's target is `row:{n}`), and
/// where the field being typed into sits: its row and how far in.
fn section_rows(
    p: &GitPlugin,
    section: &str,
    width: u16,
    now: i64,
) -> (Vec<v::ListRow>, Option<(usize, u16)>) {
    let editing = |f: &Field| p.local.edit.as_ref().is_some_and(|(e, _)| e == f);
    let mut drawn: Vec<Drawn> = Vec::new();
    let mut input_at: Option<(usize, u16)> = None;
    if section == "links" {
        let typing = |f: &Field| match (f, &p.local.edit) {
            (Field::LinkRepo(None) | Field::LinkDir(None, _), Some((e, _))) => {
                matches!(e, Field::LinkRepo(None) | Field::LinkDir(None, _))
            }
            _ => editing(f),
        };
        if typing(&Field::LinkRepo(None)) {
            input_at = Some((0, ADD_AT));
        }
        drawn.push((
            add_line(
                "link a repo to your clone",
                "the repo's path, then the folder",
                width,
            ),
            None,
        ));
        for (i, l) in p.cfg.links.iter().enumerate() {
            let n = i + 1;
            if editing(&Field::LinkRepo(Some(i)))
                || matches!(&p.local.edit, Some((Field::LinkDir(Some(j), _), _)) if *j == i)
                || editing(&Field::LinkBranch(i))
                || editing(&Field::LinkBase(i))
            {
                input_at = Some((n, 1));
            }
            let state = match link_state(p, l) {
                Verdict::Good(b) => (format!("✓ {b}"), Ink::Good),
                Verdict::Warn(w) => (format!("⚠ {w}"), Ink::Warn),
                Verdict::Bad(w) => (format!("✗ {w}"), Ink::Bad),
            };
            drawn.push(pair_row(
                &l.repo,
                local::tilde(&l.dir),
                Ink::Secondary,
                Some(state),
                width,
                true,
            ));
        }
    } else if section == "branches" {
        for (n, row) in brows(p).into_iter().enumerate() {
            let (f, (line, right)): (Option<Field>, Drawn) = match row {
                BRow::Add => (
                    Some(Field::ConvAdd),
                    (
                        add_line(
                            "add a convention for a repo",
                            "a repo path, a template, a base",
                            width,
                        ),
                        None,
                    ),
                ),
                BRow::Conv(i) => {
                    let c = &p.cfg.conventions[i];
                    let (value, ink) = if c.branch.is_empty() {
                        ("default template".to_string(), Ink::Muted)
                    } else {
                        (c.branch.clone(), Ink::Secondary)
                    };
                    let right = (!c.base.is_empty())
                        .then(|| (format!("from {}", c.base), Ink::AccentSecondary));
                    (
                        Some(Field::ConvUnder(i)),
                        pair_row(&c.under, value, ink, right, width, false),
                    )
                }
                BRow::Me => (
                    Some(Field::Me),
                    kv_row("me", &p.cfg.me, "not set", None, width),
                ),
                BRow::Branch => {
                    let sample = (sample_branch(p, &p.cfg.branch), Ink::Number);
                    (
                        Some(Field::Branch),
                        kv_row("branch", &p.cfg.branch, "", Some(sample), width),
                    )
                }
                BRow::Worktree => {
                    let sample = (worktree_sample(&p.cfg.worktree), Ink::Muted);
                    (
                        Some(Field::Worktree),
                        kv_row("worktree", &p.cfg.worktree, "", Some(sample), width),
                    )
                }
                BRow::After => (
                    Some(Field::AfterWorktree),
                    kv_row("after", &p.cfg.after_worktree, "nothing", None, width),
                ),
                BRow::Types => (
                    Some(Field::TypeMap),
                    kv_row(
                        "types",
                        &types_text(p),
                        "as the tracker names them",
                        None,
                        width,
                    ),
                ),
            };
            // An input sits where the row starts its value (labelled rows),
            // or at the row's start (a convention is typed whole).
            let at = match row {
                BRow::Add => ADD_AT,
                BRow::Conv(_) => 1,
                _ => VALUE_AT,
            };
            let typing = p.local.edit.as_ref().is_some_and(|(e, _)| match (row, e) {
                (
                    BRow::Conv(i),
                    Field::ConvUnder(j) | Field::ConvBranch(j) | Field::ConvBase(j),
                ) => i == *j,
                (_, e) => f.as_ref() == Some(e),
            });
            if typing {
                input_at = Some((n, at));
            }
            drawn.push((line, right));
        }
    } else if section == "editors" {
        if editing(&Field::TypeAdd) {
            input_at = Some((0, ADD_AT));
        }
        drawn.push((
            add_line("add a project type", "a name, then its marker files", width),
            None,
        ));
        for (i, ty) in p.cfg.types.iter().enumerate() {
            let n = i + 1;
            if editing(&Field::TypeName(i)) {
                input_at = Some((n, 1));
            }
            if editing(&Field::Markers(i)) || editing(&Field::Editor(i)) {
                input_at = Some((n, 11));
            }
            let line = Text::new()
                .push(Seg::new(format!("{:<10}", ty.name), Ink::Emphasis).strong())
                .push(Seg::new(ty.markers.join(" "), Ink::Secondary));
            let right = if ty.editor.is_empty() {
                Text::of("asks", Ink::Muted)
            } else {
                Text::of(ty.editor.clone(), Ink::AccentSecondary)
            };
            drawn.push((line, Some(right)));
        }
    } else if section == "terminal" {
        if editing(&Field::Terminal) {
            input_at = Some((0, VALUE_AT));
        }
        let (what, how) = terminal_now(p);
        let value = Text::new()
            .push(Seg::new(what, Ink::Primary))
            .push(Seg::new(format!(" · {how}"), Ink::Muted));
        drawn.push((labelled("terminal", LABEL, value), None));
    } else {
        for (n, row) in rows(p).into_iter().enumerate() {
            // Labels in a column; an input starts where the value does.
            let kv = |label: &str, value: Text| labelled(label, LABEL, value);
            drawn.push(match row {
                Row::Add => {
                    if editing(&Field::RootAdd) {
                        input_at = Some((n, ADD_AT));
                    }
                    (
                        add_line("add a folder to look in", "paste or type a folder", width),
                        None,
                    )
                }
                Row::Root(i) => {
                    if editing(&Field::Root(i)) {
                        input_at = Some((n, VALUE_AT));
                    }
                    let root = &p.cfg.roots[i];
                    let right = if missing(p, root) {
                        Text::of("not here", Ink::Bad)
                    } else if p.local.index.is_some() {
                        Text::of(format!("{} found", under(p, root)), Ink::Number)
                    } else {
                        Text::of("not looked yet", Ink::Muted)
                    };
                    (
                        kv("folder", Text::of(local::tilde(root), Ink::Primary)),
                        Some(right),
                    )
                }
                Row::Found(i) => {
                    let (place, n) = &p.local.places[i];
                    (
                        kv(
                            "found",
                            Text::new()
                                .push(Seg::icon_or_lead(radar_core::ui::icon::Icon::Todo, "[ ] ", Ink::Muted))
                                .push(Seg::new(local::tilde(place), Ink::Secondary)),
                        ),
                        Some(Text::of(plural(*n, "checkout", "checkouts"), Ink::Number)),
                    )
                }
                Row::Depth => {
                    if editing(&Field::Depth) {
                        input_at = Some((n, VALUE_AT));
                    }
                    let d = p.cfg.depth;
                    let value = format!("{d} folder{}", if d == 1 { "" } else { "s" });
                    (kv("look down", Text::of(value, Ink::Primary)), None)
                }
                Row::Scan => {
                    let mark = match (&p.local.index, p.local.scanning) {
                        (_, true) => Some(Text::of("looking", Ink::Info)),
                        (Some(i), _) => {
                            Some(Text::of(format!("{} found", i.checkouts.len()), Ink::Good))
                        }
                        (None, _) => None,
                    };
                    (
                        kv(
                            "last look",
                            Text::of(last_look(p, now, false), Ink::Secondary),
                        ),
                        mark,
                    )
                }
            });
        }
    }
    let rows = drawn
        .into_iter()
        .enumerate()
        .map(|(n, (line, right))| {
            let row = v::ListRow::new(format!("row:{n}"), line);
            match right {
                Some(word) => row.right(word),
                None => row,
            }
        })
        .collect();
    (rows, input_at)
}

pub fn detail_title(p: &GitPlugin, section: &str) -> String {
    if section == "links" {
        return selected_link(p).map_or_else(
            || "link to a local folder".into(),
            |i| p.cfg.links[i].repo.clone(),
        );
    }
    if section == "branches" {
        return match selected_brow(p) {
            BRow::Add => "add a convention".into(),
            BRow::Conv(i) => p.cfg.conventions[i].under.clone(),
            BRow::Me => "me".into(),
            BRow::Branch => "branch name".into(),
            BRow::Worktree => "worktree folder".into(),
            BRow::After => "after a worktree".into(),
            BRow::Types => "ticket types".into(),
        };
    }
    if section == "editors" {
        return selected_type(p).map_or_else(
            || "add a project type".into(),
            |i| p.cfg.types[i].name.clone(),
        );
    }
    if section == "terminal" {
        return "terminal".into();
    }
    match selected_row(p) {
        Row::Add => "add a folder".into(),
        Row::Root(i) => local::tilde(&p.cfg.roots[i]),
        Row::Found(i) => local::tilde(&p.local.places[i].0),
        Row::Depth => "look down".into(),
        Row::Scan => "last look".into(),
    }
}

/// `✓ checked` or `⚠ why`: an icon where the pack has one, else the symbol.
fn checked(good: bool, said: &str) -> Text {
    let (icon, symbol, ink) = if good {
        (radar_core::ui::icon::Icon::RunPassed, "✓", Ink::Good)
    } else {
        (radar_core::ui::icon::Icon::Warning, "⚠", Ink::Warn)
    };
    Text::new()
        .push(Seg::icon_or(icon, symbol, ink))
        .push(Seg::new(format!(" {said}"), ink))
}

/// The detail beside a section's rows: the live verdict on what is typed
/// first, then what the selected row is, what it accepts, its value and
/// its config key.
fn detail(p: &GitPlugin, section: &str, now: i64) -> Vec<Text> {
    let strong = |s: String, ink: Ink| Text::new().push(Seg::new(s, ink).strong());
    let mut lines: Vec<Text> = Vec::new();
    if let Some((field, input)) = &p.local.edit {
        lines.push(view_heading("as you type"));
        lines.push(match validate(p, field, &input.value()) {
            Verdict::Good(m) => Text::of(format!("✓ {m}"), Ink::Good),
            Verdict::Warn(m) => Text::of(format!("⚠ {m}"), Ink::Warn),
            Verdict::Bad(m) => Text::of(format!("✗ {m}"), Ink::Warn),
        });
        lines.push(Text::new());
    }
    let (about, accepts, value, key): (&str, &str, String, &str) = if section == "links" {
        match selected_link(p) {
            None => (
                "Tie a repo to the clone you work in. A link wins over the clone radar finds by its origin, and the clone need not be under a folder radar looks in.",
                "a canonical HTTPS repository URL or legacy full path (acme/team-a/api), then the folder of your clone",
                plural(p.cfg.links.len(), "link", "links"),
                "[[git.link]]",
            ),
            Some(i) => {
                let l = &p.cfg.links[i];
                lines.push(view_heading("checked"));
                let (good, said) = match link_state(p, l) {
                    Verdict::Good(b) => (true, format!("a clone, on {b}")),
                    Verdict::Warn(m) | Verdict::Bad(m) => (false, m),
                };
                lines.push(checked(good, &said));
                lines.push(Text::new());
                lines.push(Text::of(
                    format!("{} is worked on in {}.", l.repo, local::tilde(&l.dir)),
                    Ink::Primary,
                ));
                lines.push(Text::new());
                let value = |s: &str, fallback: &str| {
                    if s.is_empty() {
                        Text::of(fallback, Ink::Muted)
                    } else {
                        strong(s.to_string(), Ink::Emphasis)
                    }
                };
                lines.push(labelled(
                    "branch",
                    10,
                    value(&l.branch, "the repo's convention"),
                ));
                lines.push(labelled(
                    "base",
                    10,
                    value(&l.base, "the convention's, else origin's default"),
                ));
                let others = p
                    .local
                    .linked
                    .iter()
                    .filter(|c| {
                        !c.main.is_empty() && local::expand(&l.dir).display().to_string() == c.main
                    })
                    .count();
                lines.push(labelled(
                    "worktrees",
                    10,
                    Text::of(format!("{others} known"), Ink::Number),
                ));
                (
                    "",
                    "repo: HTTPS URL or legacy path · folder: your clone · branch: a name template · base: a branch",
                    String::new(),
                    "[[git.link]] repo, dir, branch, base",
                )
            }
        }
    } else if section == "branches" {
        match selected_brow(p) {
            BRow::Add => (
                "A branch name template and base branch for the repos under a path. The longest matching path wins; a link's own template wins over it.",
                "a repo path or folder of repos, a template with {key}, and a base: acme/team-a {type}/{key}-{slug} develop",
                plural(p.cfg.conventions.len(), "convention", "conventions"),
                "[[git.convention]]",
            ),
            BRow::Conv(i) => {
                let c: &Convention = &p.cfg.conventions[i];
                (
                    "Repos under this path name their branches by this template and start from this base.",
                    "under: a repo path or folder · template: needs {key} · base: a branch (empty: origin's default)",
                    format!(
                        "{} → {} from {}",
                        c.under,
                        if c.branch.is_empty() {
                            "the default template".to_string()
                        } else {
                            sample_branch(p, &c.branch)
                        },
                        if c.base.is_empty() {
                            "origin's default"
                        } else {
                            &c.base
                        }
                    ),
                    "[[git.convention]] under, branch, base",
                )
            }
            BRow::Me => (
                "Your name in branch names: {me}.",
                "one word, letters digits - _ .",
                if p.cfg.me.is_empty() {
                    "not set".into()
                } else {
                    p.cfg.me.clone()
                },
                "[git] me",
            ),
            BRow::Branch => (
                "The branch name template for repos no convention names. Variables: {me} {key} {project} {num} {slug} {type}. Shown against ABC-12 'Add a stats endpoint' (Story).",
                "a template with {key} in it, e.g. {me}/{key}",
                format!("{} → {}", p.cfg.branch, sample_branch(p, &p.cfg.branch)),
                "[git] branch",
            ),
            BRow::Worktree => (
                "Where a branch parked on a worktree goes, beside its clone. {clone} is the clone's folder, {topic} the ticket key.",
                "a path template, e.g. {clone}-{topic}",
                format!("{} → {}", p.cfg.worktree, worktree_sample(&p.cfg.worktree)),
                "[git] worktree",
            ),
            BRow::After => (
                "A command run in a new worktree, for example to install dependencies.",
                "empty for nothing, or a command line",
                if p.cfg.after_worktree.is_empty() {
                    "nothing".into()
                } else {
                    p.cfg.after_worktree.clone()
                },
                "[git] after_worktree",
            ),
            BRow::Types => (
                "How a ticket's type is written in {type}: the tracker's name, then what you want in the branch. Types not listed are written in lower case.",
                "pairs like story=feature bug=fix, space between",
                if p.cfg.type_map.is_empty() {
                    "as the tracker names them".into()
                } else {
                    types_text(p)
                },
                "[git.types]",
            ),
        }
    } else if section == "editors" {
        match selected_type(p) {
            None => (
                "Another kind of project, told apart by a file at the top of its checkout, with the editor it opens in.",
                "a name, then its marker files: elixir mix.exs · *.csproj matches by ending",
                plural(p.cfg.types.len(), "type", "types"),
                "[[git.type]]",
            ),
            Some(i) => {
                let ty = &p.cfg.types[i];
                if !ty.editor.is_empty() {
                    let (good, said) = match editor_verdict(&ty.editor) {
                        Verdict::Good(m) => (true, m),
                        Verdict::Warn(m) | Verdict::Bad(m) => (false, m),
                    };
                    lines.push(view_heading("checked"));
                    lines.push(checked(good, &said));
                    lines.push(Text::new());
                }
                let markers: Vec<&str> = ty.markers.iter().map(String::as_str).collect();
                lines.push(Text::of(
                    format!(
                        "A checkout with {} at its top is a {} project, and opens in {}.",
                        ty.markers.join(" or "),
                        ty.name,
                        if ty.editor.is_empty() {
                            "the editor you pick each time"
                        } else {
                            &ty.editor
                        }
                    ),
                    Ink::Primary,
                ));
                lines.push(Text::new());
                lines.push(labelled(
                    "markers",
                    10,
                    Text::of(ty.markers.join(" "), Ink::Primary),
                ));
                let editor = if ty.editor.is_empty() {
                    "asks each time".to_string()
                } else {
                    ty.editor.clone()
                };
                lines.push(labelled("editor", 10, strong(editor, Ink::Emphasis)));
                let here = format!(
                    "{} known",
                    plural(having(p, &markers), "checkout", "checkouts")
                );
                lines.push(labelled("here", 10, Text::of(here, Ink::Number)));
                (
                    "",
                    "markers: file names, space between · editor: a command line",
                    String::new(),
                    "[[git.type]] name, markers, editor",
                )
            }
        }
    } else if section == "terminal" {
        terminal_about(p)
    } else {
        checkout_about(p, now)
    };
    if !about.is_empty() {
        lines.push(Text::of(about, Ink::Primary));
    }
    if !accepts.is_empty() {
        lines.push(Text::new());
        lines.push(view_heading("accepts"));
        lines.push(Text::of(accepts, Ink::Secondary));
    }
    if !value.is_empty() {
        lines.push(Text::new());
        lines.push(view_heading("now"));
        lines.push(strong(value, Ink::Emphasis));
    }
    if section == "terminal" {
        lines.extend(terminal_found(p));
    }
    if !key.is_empty() {
        lines.push(Text::new());
        lines.push(Text::of(format!("config: {key}"), Ink::Muted));
    }
    lines
}

/// What the selected checkouts row is, what it accepts, its value now and
/// its config key.
fn checkout_about(p: &GitPlugin, now: i64) -> (&'static str, &'static str, String, &'static str) {
    match selected_row(p) {
        Row::Add => (
            "Another folder to look for git checkouts in. radar matches each checkout to its repo by its origin remote.",
            "a full path, or one starting with ~/ (pasted links to files work too)",
            plural(p.cfg.roots.len(), "folder", "folders"),
            "[git] roots",
        ),
        Row::Root(i) => {
            let root = &p.cfg.roots[i];
            (
                "A folder radar looks in for checkouts, and in the folders under it.",
                "a full path, or one starting with ~/",
                if missing(p, root) {
                    format!("{} · not a folder on this machine", local::tilde(root))
                } else {
                    format!(
                        "{} · {} found",
                        local::tilde(root),
                        plural(under(p, root), "checkout", "checkouts")
                    )
                },
                "[git] roots",
            )
        }
        Row::Found(i) => {
            let (place, n) = &p.local.places[i];
            (
                "A folder in your home folder that holds git checkouts, found when you let setup look. radar does not look in it yet; ⏎ looks in it from now on.",
                "",
                format!("{} · {}", local::tilde(place), plural(*n, "checkout", "checkouts")),
                "[git] roots",
            )
        }
        Row::Depth => (
            "How many folders down from each folder radar looks. Hidden folders and build output (node_modules, target, build, …) are never looked in.",
            "a number from 1 to 6, typed in place: ⏎ keeps it, esc leaves it, ↑↓ step it",
            format!("{}", p.cfg.depth),
            "[git] depth",
        ),
        Row::Scan => (
            "When radar last looked for checkouts. It looks once at start-up, and again when you ask.",
            "",
            last_look(p, now, true),
            "",
        ),
    }
}

/// What the terminal row is, what it accepts, what opens now, its key.
fn terminal_about(p: &GitPlugin) -> (&'static str, &'static str, String, &'static str) {
    let (what, how) = terminal_now(p);
    (
        "What opens a terminal in a checkout (t on a repo or a merge request). Inside tmux it is a new tmux window; on a desktop your terminal app; over ssh without tmux, radar puts `cd <folder>` on your clipboard instead.",
        "⏎ picks one of the terminals found here, or type a command line: {dir} is the folder, {cmd} the command it runs (else -e is added)",
        format!("{what} · {how}"),
        "[git] terminal",
    )
}

/// The terminals installed on this machine, for the terminal section's
/// detail: the one radar runs in first, marked.
fn terminal_found(p: &GitPlugin) -> Vec<Text> {
    let Some(t) = &p.local.terminals else { return vec![] };
    let mut lines = vec![Text::new(), view_heading("installed here")];
    if t.found.is_empty() {
        lines.push(Text::of("No terminal radar knows was found. Type a command instead.", Ink::Muted));
        return lines;
    }
    let w = t.found.iter().map(|k| ui::width(k.name)).max().unwrap_or(0) + 2;
    for k in &t.found {
        let pad = " ".repeat(w.saturating_sub(ui::width(k.name)));
        let mut line = Text::new().push(Seg::new(format!("{}{pad}", k.name), Ink::Primary));
        if t.current == Some(k.name) {
            line.add(Seg::new("radar is running in it", Ink::Good));
        }
        lines.push(line);
    }
    lines
}

// --- the checkouts section as view data ------------------------------------------

/// A section as renderer-neutral data: its rows (the field being typed
/// into on its row), the detail of the selected row, and the actions on it.
pub fn section_view(p: &GitPlugin, section: &str, cx: &v::ViewCx<'_>) -> Option<v::SectionView> {
    if !SECTIONS.iter().any(|s| s.id == section) {
        return None;
    }
    let (rows, input_at) = section_rows(p, section, cx.width, cx.now);
    let editing = match (input_at, &p.local.edit) {
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
            selected: Some(p.local.rows[at(section)]),
            editing,
        },
        detail_title: detail_title(p, section),
        detail_blocks: if section == "checkouts" {
            // The row's detail, then what local code is (setup's sheet).
            let mut blocks: Vec<v::Block> = Vec::new();
            if p.local.home == local::HomeWalk::Walking {
                blocks.push(v::Block::line(Text::of("Looking in your home folder for folders that hold checkouts…", Ink::Info)));
                blocks.push(v::Block::line(Text::new()));
            }
            blocks.extend(detail(p, section, cx.now).into_iter().map(v::Block::line));
            blocks.push(v::Block::line(Text::new()));
            blocks.extend(crate::setup::help());
            blocks
        } else {
            vec![]
        },
        detail: detail(p, section, cx.now),
        actions: section_actions(p, section),
    })
}

/// The actions of `section` as data, each under its own id. A menu row takes the
/// menu item's value as its verb (`git.checkouts.dir`); a row without a menu is
/// named by hand from its key.
fn section_actions(p: &GitPlugin, section: &str) -> Vec<v::ActionSpec> {
    if has_add(section) && p.local.rows[at(section)] == 0 {
        return keys(p, section)
            .iter()
            .map(|b| v::ActionSpec::named(action_id(section, b), b))
            .collect();
    }
    let Some(menu) = row_menu(p, section) else {
        return vec![];
    };
    let mut out = vec![v::ActionSpec::named(String::new(), &Binding::heading(menu.title))];
    out.extend(menu.items.iter().filter_map(|i| {
        let key = i.key?;
        let id = format!("git.{section}.{}", i.value);
        Some(v::ActionSpec::named(id, &Binding::new(key_text(key), &i.label)))
    }));
    out
}

/// The stable id of a row without a menu, chosen here by hand from its key
/// (`git.<section>.<verb>`). A heading has no id.
fn action_id(section: &str, b: &Binding) -> String {
    if b.is_heading() {
        return String::new();
    }
    let verb = match b.keys.as_str() {
        "⏎" => "add",
        "s" => "look_now",
        other => unreachable!("section {section} has no stable id for key {other:?}"),
    };
    format!("git.{section}.{verb}")
}

/// The `+ …` row that starts adding, and what it takes where that fits.
fn add_line(action: &str, takes: &str, width: u16) -> Text {
    let mut line = Text::new()
        .push(Seg::new(" + ", Ink::Accent).strong())
        .push(Seg::new(action, Ink::AccentSecondary).strong());
    let hint = format!("   {takes}");
    if 3 + chars(action) + chars(&hint) + 2 <= usize::from(width) {
        line.add(Seg::new(hint, Ink::Muted));
    }
    line
}

fn view_heading(text: &str) -> Text {
    Text::new().push(Seg::new(text.to_uppercase(), Ink::Muted).strong())
}

/// What the user did in a section drawn from its view, from any frontend:
/// a row chosen, `⏎` on it, a key, or a key typed into its field.
pub fn intent(p: &mut GitPlugin, section: &str, intent: &v::Intent) -> Outcome {
    use v::Intent;
    match intent {
        Intent::Select { target } | Intent::Context { target } => {
            click(p, section, target.as_str())
        }
        Intent::Activate { target } => {
            click(p, section, target.as_str());
            press(p, section, v::Chord::new(v::Key::Enter))
        }
        Intent::Fold { .. } | Intent::Show { .. } | Intent::End { .. } | Intent::EditSelect { .. } | Intent::EditUnselect | Intent::Dock(_) | Intent::Copy { .. } => Outcome::Ignored,
        Intent::Key { .. } | Intent::Invoke { .. } => {
            let Some(chord) = intent.chord() else {
                return Outcome::Ignored;
            };
            if let Some(outcome) = depth_key(p, chord) {
                outcome
            } else if p.local.edit.is_some() {
                typed(p, |input| input.chord(chord))
            } else {
                press(p, section, chord)
            }
        }
    }
}

/// A key while look down is typed into: a digit takes the field's place
/// (it holds one), ↑↓ step it from 1 to 6, other letters are not taken;
/// ⏎, esc and the editing keys go to the field.
fn depth_key(p: &mut GitPlugin, chord: v::Chord) -> Option<Outcome> {
    let Some((Field::Depth, input)) = p.local.edit.as_mut() else { return None };
    if chord.ctrl || chord.alt {
        return None;
    }
    let now = input.value().trim().parse::<usize>().unwrap_or(p.cfg.depth);
    let set = match chord.key {
        v::Key::Char(c) if depth_digit(c) => c.to_string(),
        v::Key::Char(_) => return Some(Outcome::done()),
        v::Key::Up => now.saturating_add(1).clamp(1, DEPTH_MAX).to_string(),
        v::Key::Down => now.saturating_sub(1).clamp(1, DEPTH_MAX).to_string(),
        _ => return None,
    };
    *input = TextInput::new(&set);
    Some(Outcome::done())
}

/// A digit look down takes: 1 to [`DEPTH_MAX`].
pub(crate) fn depth_digit(c: char) -> bool {
    c.to_digit(10).is_some_and(|d| (1..=DEPTH_MAX).contains(&(d as usize)))
}

/// Look down's field, opened on its row with `value` in it.
fn edit_depth(p: &mut GitPlugin, value: &str) -> Outcome {
    p.local.edit = Some((Field::Depth, TextInput::new(value)));
    Outcome::done()
}

/// The menu of the terminal row: the terminals installed here, the one
/// radar runs in first, then a command of your own, and back to radar's
/// choice.
fn terminal_menu(p: &GitPlugin) -> Menu {
    let (found, current) = match &p.local.terminals {
        Some(t) => (t.found.clone(), t.current),
        None => (vec![], None),
    };
    let mut items: Vec<MenuItem> = found
        .into_iter()
        .map(|k| {
            let hint = if p.cfg.terminal == k.command {
                "in use"
            } else if current == Some(k.name) {
                "radar is running in it"
            } else {
                ""
            };
            MenuItem::pick(k.name, hint, format!("use:{}", k.name))
        })
        .collect();
    items.push(MenuItem::new('e', "type a command…", "any terminal's command line", "edit"));
    if !p.cfg.terminal.is_empty() {
        items.push(MenuItem::new('r', "let radar find one", "clear it", "reset"));
    }
    Menu::new("lc-term", "terminal", items)
}

// --- menus -----------------------------------------------------------------------

/// The options of the selected row; their keys also work from the row.
fn row_menu(p: &GitPlugin, section: &str) -> Option<Menu> {
    if section == "links" {
        let i = selected_link(p)?;
        return Some(Menu::new(
            format!("lc-link:{i}"),
            p.cfg.links[i].repo.clone(),
            vec![
                MenuItem::new('e', "change the folder", "the clone you work in", "dir"),
                MenuItem::new('r', "change the repo", "its full path", "repo"),
                MenuItem::new(
                    't',
                    "branch template…",
                    "names for new branches here",
                    "branch",
                ),
                MenuItem::new('m', "base branch…", "new branches start from it", "base"),
                MenuItem::new('a', "link another repo", "a repo, then its clone", "add"),
                MenuItem::new('d', "unlink…", "forget this link", "remove"),
            ],
        ));
    }
    if section == "branches" {
        let reset = |on: bool, what: &str| {
            on.then(|| MenuItem::new('r', "back to the default", what.to_string(), "reset"))
        };
        let edit = |hint: &str| MenuItem::new('e', "change it", hint.to_string(), "edit");
        let (id, title, mut items) = match selected_brow(p) {
            BRow::Add => return None,
            BRow::Conv(i) => {
                return Some(Menu::new(
                    format!("lc-conv:{i}"),
                    p.cfg.conventions[i].under.clone(),
                    vec![
                        MenuItem::new('u', "change the path", "the repos it covers", "under"),
                        MenuItem::new('t', "branch template…", "needs {key}", "branch"),
                        MenuItem::new('m', "base branch…", "new branches start from it", "base"),
                        MenuItem::new('a', "add a convention", "another repo path", "add"),
                        MenuItem::new('d', "remove…", "forget this convention", "remove"),
                    ],
                ));
            }
            BRow::Me => ("me", "me", vec![edit("your name in branch names")]),
            BRow::Branch => ("branch", "branch name", vec![edit("a template with {key}")]),
            BRow::Worktree => ("worktree", "worktree folder", vec![edit("a path template")]),
            BRow::After => ("after", "after a worktree", vec![edit("a command line")]),
            BRow::Types => ("types", "ticket types", vec![edit("story=feature bug=fix")]),
        };
        let non_default = match id {
            "me" => !p.cfg.me.is_empty(),
            "branch" => p.cfg.branch != local::DEFAULT_BRANCH,
            "worktree" => p.cfg.worktree != local::DEFAULT_WORKTREE,
            "after" => !p.cfg.after_worktree.is_empty(),
            _ => !p.cfg.type_map.is_empty(),
        };
        items.extend(reset(non_default, "clear it"));
        items.push(MenuItem::new(
            'a',
            "add a convention",
            "a repo path, a template, a base",
            "add",
        ));
        return Some(Menu::new(format!("lc-br:{id}"), title, items));
    }
    if section == "editors" {
        let i = selected_type(p)?;
        let ty = &p.cfg.types[i];
        return Some(Menu::new(
            format!("lc-type:{i}"),
            ty.name.clone(),
            vec![
                MenuItem::new('e', "editor…", "which editor this type opens in", "editor"),
                MenuItem::new(
                    'm',
                    "marker files",
                    "the files that make a checkout this type",
                    "markers",
                ),
                MenuItem::new('n', "rename", "the type's name", "rename"),
                MenuItem::new(
                    'a',
                    "add a project type",
                    "a name, then its marker files",
                    "add",
                ),
                MenuItem::new('d', "remove…", "forget this type", "remove"),
            ],
        ));
    }
    if section == "terminal" {
        return Some(terminal_menu(p));
    }
    let look = MenuItem::new(
        's',
        "look now",
        "look for checkouts in every folder",
        "scan",
    );
    Some(match selected_row(p) {
        Row::Add => return None,
        Row::Found(i) => Menu::new(
            format!("lc-found:{i}"),
            local::tilde(&p.local.places[i].0),
            vec![
                MenuItem::new(' ', "look in it", "from now on, and now", "follow"),
                MenuItem::new('a', "add a folder", "another folder to look in", "add"),
            ],
        ),
        Row::Root(i) => Menu::new(
            format!("lc-root:{i}"),
            local::tilde(&p.cfg.roots[i]),
            vec![
                MenuItem::new('e', "change it", "type or paste another folder", "edit"),
                look,
                MenuItem::new('a', "add a folder", "another folder to look in", "add"),
                MenuItem::new('d', "stop looking here…", "remove this folder", "remove"),
            ],
        ),
        Row::Depth => Menu::new(
            "lc-depth",
            "look down",
            vec![
                MenuItem::new('e', "change it", "a number from 1 to 6", "edit"),
                look,
            ],
        ),
        Row::Scan => Menu::new("lc-scan", "last look", vec![look]),
    })
}

fn ask_to_scan() -> Effect {
    Effect::Menu(
        Menu::confirm(
            "lc-ask-scan",
            "look for checkouts now?",
            "look now",
            "later",
        )
        .saying("looks in every folder in the background"),
    )
}

pub fn on_menu(p: &mut GitPlugin, menu: &str, value: &str) -> Option<Vec<Effect>> {
    let (kind, arg) = menu.split_once(':').unwrap_or((menu, ""));
    if !kind.starts_with("lc-") {
        return None;
    }
    let idx = arg.parse::<usize>().ok();
    let edit = |p: &mut GitPlugin, f: Field, v: &str| {
        p.local.edit = Some((f, TextInput::new(v)));
        vec![]
    };
    Some(match (kind, value) {
        ("lc-ask-scan", "yes") | (_, "scan") => local::scan_now(p, true),
        ("lc-ask-scan", _) => vec![],
        ("lc-root", "edit") => {
            let i = idx.filter(|i| *i < p.cfg.roots.len())?;
            let v = p.cfg.roots[i].clone();
            edit(p, Field::Root(i), &v)
        }
        ("lc-found", "follow") => {
            let i = idx.filter(|i| *i < p.local.places.len())?;
            follow(p, i)
        }
        ("lc-root" | "lc-type" | "lc-found", "add") => {
            let (row, f) = if kind != "lc-type" {
                (0, Field::RootAdd)
            } else {
                (1, Field::TypeAdd)
            };
            p.local.rows[row] = 0;
            edit(p, f, "")
        }
        ("lc-root", "remove") => {
            let i = idx.filter(|i| *i < p.cfg.roots.len())?;
            vec![Effect::Menu(
                Menu::confirm(
                    format!("lc-root-remove:{i}"),
                    format!("stop looking in {}?", local::tilde(&p.cfg.roots[i])),
                    "stop looking there",
                    "keep it",
                )
                .saying("its checkouts are forgotten at the next look"),
            )]
        }
        ("lc-root-remove", "yes") => {
            let i = idx.filter(|i| *i < p.cfg.roots.len())?;
            let root = p.cfg.roots.remove(i);
            p.local.rows[0] = 0;
            let mut fx = vec![Effect::Notify(format!(
                "stopped looking in {}",
                local::tilde(&root)
            ))];
            if !p.cfg.roots.is_empty() {
                fx.extend(local::scan_now(p, false));
            } else {
                p.local.index = None;
            }
            fx
        }
        ("lc-depth", "edit") => {
            let v = p.cfg.depth.to_string();
            edit(p, Field::Depth, &v)
        }
        ("lc-term", "edit") => {
            let v = p.cfg.terminal.clone();
            edit(p, Field::Terminal, &v)
        }
        ("lc-term", "reset") => {
            p.cfg.terminal.clear();
            vec![Effect::Notify("radar finds a terminal for you".into())]
        }
        ("lc-term", v) if v.starts_with("use:") => {
            let k = crate::terminal::by_name(&v["use:".len()..])?;
            p.cfg.terminal = k.command.to_string();
            vec![Effect::Notify(format!("{} opens in a checkout from now on", k.name))]
        }
        ("lc-type", v) => {
            let i = idx.filter(|i| *i < p.cfg.types.len())?;
            let ty = p.cfg.types[i].clone();
            match v {
                "editor" => {
                    let mut items: Vec<MenuItem> = p
                        .local
                        .installed
                        .iter()
                        .map(|(cmd, name)| {
                            let hint = if *cmd == ty.editor {
                                format!("{cmd} · in use")
                            } else {
                                cmd.clone()
                            };
                            MenuItem::pick(name.clone(), hint, cmd.clone())
                        })
                        .collect();
                    items.push(MenuItem::pick(
                        "type a command…",
                        "any command line that takes a folder",
                        "@type",
                    ));
                    items.push(MenuItem::pick(
                        "ask each time",
                        "radar offers the installed editors",
                        "@ask",
                    ));
                    vec![Effect::Menu(Menu::new(
                        format!("lc-editor:{i}"),
                        format!("{} opens in…", ty.name),
                        items,
                    ))]
                }
                "markers" => edit(p, Field::Markers(i), &ty.markers.join(" ")),
                "rename" => edit(p, Field::TypeName(i), &ty.name),
                "remove" => vec![Effect::Menu(
                    Menu::confirm(
                        format!("lc-type-remove:{i}"),
                        format!("forget the project type {}?", ty.name),
                        "forget it",
                        "keep it",
                    )
                    .saying("its checkouts are then of no type"),
                )],
                _ => vec![],
            }
        }
        ("lc-editor", v) => {
            let i = idx.filter(|i| *i < p.cfg.types.len())?;
            match v {
                "@type" => {
                    let e = p.cfg.types[i].editor.clone();
                    edit(p, Field::Editor(i), &e)
                }
                "@ask" => {
                    p.cfg.types[i].editor.clear();
                    vec![Effect::Notify("asks each time".into())]
                }
                cmd => {
                    p.cfg.types[i].editor = cmd.to_string();
                    vec![Effect::Notify(format!(
                        "{} opens in {cmd}",
                        p.cfg.types[i].name
                    ))]
                }
            }
        }
        ("lc-type-remove", "yes") => {
            let i = idx.filter(|i| *i < p.cfg.types.len())?;
            let ty = p.cfg.types.remove(i);
            p.local.rows[1] = 0;
            vec![Effect::Notify(format!("forgot {}", ty.name))]
        }
        ("lc-link", "add") => {
            p.local.rows[2] = 0;
            edit(p, Field::LinkRepo(None), "")
        }
        ("lc-conv" | "lc-br", "add") => {
            p.local.rows[3] = 0;
            edit(p, Field::ConvAdd, "")
        }
        ("lc-link", v) => {
            let i = idx.filter(|i| *i < p.cfg.links.len())?;
            let l = p.cfg.links[i].clone();
            match v {
                "dir" => edit(p, Field::LinkDir(Some(i), l.repo.clone()), &l.dir),
                "repo" => edit(p, Field::LinkRepo(Some(i)), &l.repo),
                "branch" => edit(p, Field::LinkBranch(i), &l.branch),
                "base" => edit(p, Field::LinkBase(i), &l.base),
                "remove" => vec![Effect::Menu(
                    Menu::confirm(
                        format!("lc-link-remove:{i}"),
                        format!("unlink {}?", l.repo),
                        "unlink it",
                        "keep it",
                    )
                    .saying("radar goes back to the clone it finds by origin"),
                )],
                _ => vec![],
            }
        }
        ("lc-link-remove", "yes") => {
            let i = idx.filter(|i| *i < p.cfg.links.len())?;
            let l = p.cfg.links.remove(i);
            p.local.rows[2] = 0;
            local::refresh_links(p);
            vec![Effect::Notify(format!("unlinked {}", l.repo))]
        }
        ("lc-conv", v) => {
            let i = idx.filter(|i| *i < p.cfg.conventions.len())?;
            let c = p.cfg.conventions[i].clone();
            match v {
                "under" => edit(p, Field::ConvUnder(i), &c.under),
                "branch" => edit(p, Field::ConvBranch(i), &c.branch),
                "base" => edit(p, Field::ConvBase(i), &c.base),
                "remove" => vec![Effect::Menu(
                    Menu::confirm(
                        format!("lc-conv-remove:{i}"),
                        format!("remove the convention for {}?", c.under),
                        "remove it",
                        "keep it",
                    )
                    .saying("its repos use the default template again"),
                )],
                _ => vec![],
            }
        }
        ("lc-conv-remove", "yes") => {
            let i = idx.filter(|i| *i < p.cfg.conventions.len())?;
            let c = p.cfg.conventions.remove(i);
            p.local.rows[3] = 0;
            vec![Effect::Notify(format!(
                "removed the convention for {}",
                c.under
            ))]
        }
        ("lc-br", v) => {
            let (field, now) = match arg {
                "me" => (Field::Me, p.cfg.me.clone()),
                "branch" => (Field::Branch, p.cfg.branch.clone()),
                "worktree" => (Field::Worktree, p.cfg.worktree.clone()),
                "after" => (Field::AfterWorktree, p.cfg.after_worktree.clone()),
                "types" => (Field::TypeMap, types_text(p)),
                _ => return None,
            };
            match v {
                "edit" => edit(p, field, &now),
                "reset" => {
                    let d = local::Local::default();
                    match field {
                        Field::Me => p.cfg.me = d.me,
                        Field::Branch => p.cfg.branch = d.branch,
                        Field::Worktree => p.cfg.worktree = d.worktree,
                        Field::AfterWorktree => p.cfg.after_worktree = d.after_worktree,
                        _ => p.cfg.type_map = d.type_map,
                    }
                    vec![Effect::Notify("back to the default".into())]
                }
                _ => vec![],
            }
        }
        ("lc-root-remove" | "lc-type-remove" | "lc-link-remove" | "lc-conv-remove", _) => vec![],
        _ => return None,
    })
}

// --- keys ------------------------------------------------------------------------

fn commit(p: &mut GitPlugin, field: Field, text: String) -> Vec<Effect> {
    if let Field::LinkDir(_, repo) = &field
        && crate::scoped::is_scoped(repo)
    {
        return crate::scoped::request_link(p, repo, &text);
    }
    if let Field::LinkRepo(Some(i)) = &field
        && (crate::scoped::is_scoped(&text) || crate::scoped::is_scoped(&p.cfg.links[*i].repo))
    {
        let old = p.cfg.links[*i].clone();
        return crate::scoped::request_relink(p, &text, &old.dir, Some(old.repo));
    }
    let cfg = &mut p.cfg;
    match field {
        Field::RootAdd => {
            cfg.roots.push(local::clean_folder(&text));
            p.local.rows[0] = cfg.roots.len();
            vec![ask_to_scan()]
        }
        Field::Root(i) => {
            cfg.roots[i] = local::clean_folder(&text);
            vec![ask_to_scan()]
        }
        Field::Depth => {
            let n = text.parse().unwrap_or(cfg.depth);
            if n == cfg.depth {
                return vec![];
            }
            cfg.depth = n;
            let mut fx = vec![Effect::Notify(format!("looks {} down from now on", plural(n, "folder", "folders")))];
            if !cfg.roots.is_empty() {
                fx.extend(local::scan_now(p, false));
            }
            fx
        }
        Field::Terminal => {
            cfg.terminal = text;
            vec![Effect::Notify("changed".into())]
        }
        Field::TypeAdd => {
            let mut words = text.split_whitespace();
            let name = words.next().unwrap_or_default().to_string();
            cfg.types.push(ProjectType {
                name,
                markers: words.map(str::to_string).collect(),
                editor: String::new(),
            });
            p.local.rows[1] = cfg.types.len();
            vec![Effect::Notify("added · e picks its editor".into())]
        }
        Field::TypeName(i) => {
            cfg.types[i].name = text;
            vec![]
        }
        Field::Markers(i) => {
            cfg.types[i].markers = text.split_whitespace().map(str::to_string).collect();
            vec![]
        }
        Field::Editor(i) => {
            cfg.types[i].editor = text;
            vec![Effect::Notify("changed".into())]
        }
        Field::LinkRepo(None) => {
            // The folder comes next, from a clone found by origin when there is one.
            let guess = local::checkouts_for(p, &text)
                .first()
                .map(|c| local::tilde(&c.dir))
                .unwrap_or_default();
            p.local.edit = Some((Field::LinkDir(None, text), TextInput::new(&guess)));
            vec![]
        }
        Field::LinkRepo(Some(i)) => {
            p.cfg.links[i].repo = text;
            local::refresh_links(p);
            vec![]
        }
        Field::LinkDir(None, repo) => {
            cfg.links.push(Link {
                repo: repo.clone(),
                dir: local::clean_folder(&text),
                ..Link::default()
            });
            p.local.rows[2] = cfg.links.len();
            local::refresh_links(p);
            vec![Effect::Notify(format!("linked {repo}"))]
        }
        Field::LinkDir(Some(i), _) => {
            cfg.links[i].dir = local::clean_folder(&text);
            local::refresh_links(p);
            vec![]
        }
        Field::LinkBranch(i) => {
            cfg.links[i].branch = text;
            vec![]
        }
        Field::LinkBase(i) => {
            cfg.links[i].base = text;
            vec![]
        }
        Field::ConvAdd => {
            let mut words = text.split_whitespace().map(str::to_string);
            cfg.conventions.push(Convention {
                under: words.next().unwrap_or_default(),
                branch: words.next().unwrap_or_default(),
                base: words.next().unwrap_or_default(),
            });
            p.local.rows[3] = cfg.conventions.len();
            vec![Effect::Notify("added".into())]
        }
        Field::ConvUnder(i) => {
            cfg.conventions[i].under = text;
            vec![]
        }
        Field::ConvBranch(i) => {
            cfg.conventions[i].branch = text;
            vec![]
        }
        Field::ConvBase(i) => {
            cfg.conventions[i].base = text;
            vec![]
        }
        Field::Me => {
            cfg.me = text;
            vec![]
        }
        Field::Branch => {
            cfg.branch = text;
            vec![]
        }
        Field::Worktree => {
            cfg.worktree = text;
            vec![]
        }
        Field::AfterWorktree => {
            cfg.after_worktree = text;
            vec![]
        }
        Field::TypeMap => {
            cfg.type_map = parse_types(&text).unwrap_or_default();
            vec![]
        }
    }
}

/// A terminal key in a section, as the Ratatui frontend turns it into an
/// intent (tests that press keys).
#[cfg(test)]
pub(crate) fn key(p: &mut GitPlugin, section: &str, key: KeyEvent) -> Outcome {
    let fe = radar_core::view_tui::Frontend::default();
    let scope = radar_core::view_tui::Scope::Section(section);
    radar_core::view_tui::press_part(
        &fe,
        p,
        scope,
        radar_core::Pane::Middle,
        key,
        radar_core::plugin::DEMO_NOW,
    )
}

/// A key typed into the field being edited: kept, cancelled, or the text
/// committed when it is taken.
fn typed(p: &mut GitPlugin, key: impl FnOnce(&mut TextInput) -> Edit) -> Outcome {
    let Some((field, mut input)) = p.local.edit.take() else {
        return Outcome::Ignored;
    };
    match key(&mut input) {
        Edit::Cancel => Outcome::done(),
        Edit::Typing => {
            p.local.edit = Some((field, input));
            Outcome::done()
        }
        Edit::Submit(text) => match validate(p, &field, &text) {
            Verdict::Bad(why) => {
                p.local.edit = Some((field, input));
                Outcome::Handled(vec![Effect::Notify(format!("not yet: {why}"))])
            }
            Verdict::Good(_) | Verdict::Warn(_) => {
                Outcome::Handled(commit(p, field, text.trim().to_string()))
            }
        },
    }
}

/// A key on the selected row (not typing, not moving): `⏎` adds or opens
/// the row's options, an option's key does it, `a` adds, `s` looks again.
fn press(p: &mut GitPlugin, section: &str, key: v::Chord) -> Outcome {
    let a = at(section);
    let on_add = has_add(section) && p.local.rows[a] == 0;
    let add = |p: &mut GitPlugin| {
        p.local.rows[a] = 0;
        p.local.edit = Some((
            match a {
                0 => Field::RootAdd,
                1 => Field::TypeAdd,
                2 => Field::LinkRepo(None),
                _ => Field::ConvAdd,
            },
            TextInput::new(""),
        ));
        Outcome::done()
    };
    if key.ctrl || key.alt {
        return Outcome::Ignored;
    }
    if key.key == v::Key::Enter && on_add {
        return add(p);
    }
    // ⏎ on a found place looks in it, as its space does.
    if key.key == v::Key::Enter
        && a == 0
        && let Row::Found(i) = selected_row(p)
    {
        return Outcome::Handled(follow(p, i));
    }
    // Look down is a number typed in place: ⏎ opens it with the value
    // now, a digit opens it with that digit.
    if a == 0 && section != "terminal" && selected_row(p) == Row::Depth {
        let now = p.cfg.depth.to_string();
        match key.key {
            v::Key::Enter => return edit_depth(p, &now),
            v::Key::Char(c) if depth_digit(c) => return edit_depth(p, &c.to_string()),
            _ => {}
        }
    }
    if key.key == v::Key::Enter {
        return Outcome::Handled(row_menu(p, section).map(Effect::Menu).into_iter().collect());
    }
    if let v::Key::Char(c) = key.key
        && let Some(menu) = row_menu(p, section)
        && let Some(item) = menu.items.iter().find(|i| i.key == Some(c))
    {
        let value = item.value.clone();
        return Outcome::Handled(on_menu(p, &menu.id, &value).unwrap_or_default());
    }
    match key.key {
        v::Key::Char('a') if has_add(section) => add(p),
        v::Key::Char('s') if a == 0 => Outcome::Handled(local::scan_now(p, true)),
        _ => Outcome::Ignored,
    }
}

fn click(p: &mut GitPlugin, section: &str, id: &str) -> Outcome {
    if p.local.edit.is_none()
        && let Some(i) = id
            .strip_prefix("row:")
            .and_then(|n| n.parse::<usize>().ok())
    {
        p.local.rows[at(section)] = i.min(row_count(p, section).saturating_sub(1));
    }
    Outcome::done()
}

pub fn hints(p: &GitPlugin) -> Vec<KeyHint<'static>> {
    if matches!(p.local.edit, Some((Field::Depth, _))) {
        vec![KeyHint::new("1-6", "look down"), KeyHint::new("↑↓", "step"), KeyHint::new("⏎", "keep"), KeyHint::new("esc", "cancel")]
    } else if p.local.edit.is_some() {
        vec![KeyHint::new("⏎", "keep"), KeyHint::new("esc", "cancel")]
    } else {
        vec![]
    }
}

/// The actions menu (`x`) for the selected row.
pub fn keys(p: &GitPlugin, section: &str) -> Vec<Binding> {
    if has_add(section) && p.local.rows[at(section)] == 0 {
        return if section == "links" {
            vec![
                Binding::heading("link a repo to your clone"),
                Binding::new("⏎", "type the repo's path, then the folder"),
            ]
        } else if section == "branches" {
            vec![
                Binding::heading("add a convention"),
                Binding::new("⏎", "type a repo path, a template, a base"),
            ]
        } else if section == "editors" {
            vec![
                Binding::heading("add a project type"),
                Binding::new("⏎", "type its name, then its marker files"),
            ]
        } else {
            vec![
                Binding::heading("add a folder"),
                Binding::new("⏎", "type or paste a folder to look in"),
                Binding::new("s", "look for checkouts now"),
            ]
        };
    }
    let Some(menu) = row_menu(p, section) else {
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

pub fn bulk(p: &GitPlugin, section: &str) -> Vec<MenuItem> {
    if section == "checkouts" && !p.cfg.roots.is_empty() {
        vec![MenuItem::new(
            's',
            "look for checkouts now",
            format!("in {} folders", p.cfg.roots.len()),
            "local-scan",
        )]
    } else {
        vec![]
    }
}

pub fn on_bulk(p: &mut GitPlugin, value: &str) -> Option<Vec<Effect>> {
    (value == "local-scan").then(|| local::scan_now(p, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::sample;
    use radar_core::ratatui::crossterm::event::KeyModifiers;

    fn press(p: &mut GitPlugin, section: &str, code: KeyCode) -> Vec<Effect> {
        match key(p, section, KeyEvent::new(code, KeyModifiers::NONE)) {
            Outcome::Handled(fx) => fx,
            Outcome::Ignored => vec![],
        }
    }

    fn typed(p: &mut GitPlugin, section: &str, text: &str) -> Vec<Effect> {
        for c in text.chars() {
            press(p, section, KeyCode::Char(c));
        }
        press(p, section, KeyCode::Enter)
    }

    #[test]
    fn adding_a_folder_refuses_one_that_is_not_here_then_offers_a_look() {
        let mut p = sample();
        p.local.rows[0] = 0;
        press(&mut p, "checkouts", KeyCode::Enter);
        let fx = typed(&mut p, "checkouts", "/radar/no/such/folder");
        assert!(
            matches!(&fx[..], [Effect::Notify(m)] if m == "not yet: not a folder on this machine"),
            "{fx:?}"
        );
        assert!(p.local.edit.is_some(), "still typing after a refusal");
        p.local.edit = Some((Field::RootAdd, TextInput::new("")));
        let tmp = std::env::temp_dir().display().to_string();
        let fx = typed(&mut p, "checkouts", &format!("'{tmp}/'"));
        assert_eq!(p.cfg.roots.last(), Some(&local::clean_folder(&tmp)));
        let Some(Effect::Menu(m)) = fx.first() else {
            panic!("{fx:?}")
        };
        assert_eq!(m.id, "lc-ask-scan");
        let fx = on_menu(&mut p, "lc-ask-scan", "yes").unwrap();
        assert!(fx.iter().any(|e| matches!(e, Effect::Spawn(_))));
        assert!(p.local.scanning);
    }

    #[test]
    fn depth_is_a_number_in_range() {
        let p = sample();
        assert!(matches!(validate(&p, &Field::Depth, "0"), Verdict::Bad(_)));
        assert!(matches!(validate(&p, &Field::Depth, "7"), Verdict::Bad(_)));
        assert!(matches!(validate(&p, &Field::Depth, "x"), Verdict::Bad(_)));
        assert_eq!(
            validate(&p, &Field::Depth, "3"),
            Verdict::Good("3 folders down from each".into())
        );
    }

    fn cx() -> v::ViewCx<'static> {
        v::ViewCx {
            now: 0,
            focus: v::PaneId::Middle,
            width: 80,
            height: 24,
            parts: radar_core::compose::Parts::none(),
            icon_lead: 0,
            setup: false,
        }
    }

    /// The field being typed into: its row, where it starts, its text.
    fn field(p: &GitPlugin, section: &str) -> Option<(usize, u16, String)> {
        match section_view(p, section, &cx()).unwrap().body {
            v::Node::List { editing, .. } => editing.map(|e| (e.row, e.at, e.text)),
            _ => None,
        }
    }

    #[test]
    fn look_down_is_a_number_typed_in_place() {
        let mut p = sample();
        p.local.rows[0] = rows(&p).iter().position(|r| *r == Row::Depth).unwrap();
        let row = p.local.rows[0];
        // ⏎ opens the field on the row with the value now; no menu.
        let fx = press(&mut p, "checkouts", KeyCode::Enter);
        assert!(fx.is_empty(), "{fx:?}");
        assert_eq!(field(&p, "checkouts"), Some((row, VALUE_AT, "4".into())));
        // A digit takes the field's place; out of range and letters are not taken.
        for (c, want) in [('2', "2"), ('9', "2"), ('x', "2"), ('0', "2"), ('6', "6")] {
            press(&mut p, "checkouts", KeyCode::Char(c));
            assert_eq!(field(&p, "checkouts").unwrap().2, want, "after {c}");
        }
        // ↑↓ step it within 1 to 6.
        let step = |p: &mut GitPlugin, k: v::Key| intent(p, "checkouts", &v::Intent::Key { chord: v::Chord::new(k) });
        step(&mut p, v::Key::Up);
        assert_eq!(field(&p, "checkouts").unwrap().2, "6");
        step(&mut p, v::Key::Down);
        assert_eq!(field(&p, "checkouts").unwrap().2, "5");
        // ⏎ keeps it and looks again; esc leaves it as it was.
        let fx = press(&mut p, "checkouts", KeyCode::Enter);
        assert_eq!(p.cfg.depth, 5);
        assert!(p.local.edit.is_none() && !fx.iter().any(|e| matches!(e, Effect::Menu(_))), "{fx:?}");
        press(&mut p, "checkouts", KeyCode::Char('3'));
        assert_eq!(field(&p, "checkouts").unwrap().2, "3", "a digit on the row opens it with that digit");
        press(&mut p, "checkouts", KeyCode::Esc);
        assert_eq!((p.cfg.depth, p.local.edit.is_none()), (5, true));
    }

    fn found_here() -> crate::terminal::Terminals {
        let k = |n: &str| *crate::terminal::by_name(n).unwrap();
        crate::terminal::Terminals {
            chosen: Some(("ghostty".into(), "from $TERMINAL".into())),
            found: vec![k("ghostty"), k("foot")],
            current: Some("ghostty"),
        }
    }

    #[test]
    fn the_terminal_section_says_what_was_found_and_offers_the_ones_here() {
        let mut p = sample();
        p.local.terminals = Some(found_here());
        let view = section_view(&p, "terminal", &cx()).unwrap();
        let body = format!("{:?}", view.body);
        assert!(body.contains("ghostty") && body.contains(" · from $TERMINAL"), "{body}");
        let detail: Vec<String> = view.detail.iter().map(|t| t.0.iter().map(|s| s.text.as_str()).collect()).collect();
        let detail = detail.join("\n");
        for want in ["INSTALLED HERE", "ghostty", "radar is running in it", "foot", "config: [git] terminal"] {
            assert!(detail.contains(want), "{want} in\n{detail}");
        }
        assert!(!detail.contains("not installed"), "only what is installed: {detail}");
        // ⏎ offers the installed ones, the one radar runs in first, then a command of your own.
        let fx = press(&mut p, "terminal", KeyCode::Enter);
        let [Effect::Menu(m)] = &fx[..] else { panic!("{fx:?}") };
        let items: Vec<(&str, &str, bool)> = m.items.iter().map(|i| (i.label.as_str(), i.hint.as_str(), i.muted)).collect();
        assert_eq!(items, [
            ("ghostty", "radar is running in it", false),
            ("foot", "", false),
            ("type a command…", "any terminal's command line", false),
        ]);
        let ids: Vec<String> = view.actions.iter().map(|a| a.id.clone()).collect();
        assert!(ids.contains(&"git.terminal.edit".to_string()), "{ids:?}");
        // A pick is kept in [git] terminal, and the row says you set it.
        on_menu(&mut p, "lc-term", "use:foot").unwrap();
        assert_eq!(p.cfg.terminal, "foot --working-directory={dir} {cmd}");
        assert_eq!(terminal_now(&p).1, "set by you");
        assert!(radar_core::Plugin::config(&p).unwrap()["terminal"].as_str().is_some());
        assert!(terminal_menu(&p).items.iter().any(|i| i.value == "reset"));
        // `a` adds nothing here: the section has no add row.
        assert!(press(&mut p, "terminal", KeyCode::Char('a')).is_empty() && p.local.edit.is_none());
        // Not read yet (the sandbox): radar says it finds one when it opens.
        let mut p = sample();
        p.local.terminals = None;
        assert_eq!(terminal_now(&p), ("radar finds one".into(), "when it opens".into()));
    }

    #[test]
    fn a_project_type_is_added_with_its_markers_and_an_editor_picked_from_a_menu() {
        let mut p = sample();
        p.local.installed = vec![("code".into(), "VS Code".into())];
        p.local.rows[1] = 0;
        press(&mut p, "editors", KeyCode::Enter);
        let fx = typed(&mut p, "editors", "elixir");
        assert!(matches!(&fx[..], [Effect::Notify(m)] if m.contains("marker file")));
        typed(&mut p, "editors", " mix.exs");
        let ty = p.cfg.types.last().unwrap();
        assert_eq!(
            (ty.name.as_str(), ty.markers.as_slice()),
            ("elixir", &["mix.exs".to_string()][..])
        );
        assert_eq!(selected_type(&p), Some(p.cfg.types.len() - 1));
        let fx = press(&mut p, "editors", KeyCode::Char('e'));
        let Some(Effect::Menu(m)) = fx.first() else {
            panic!("{fx:?}")
        };
        assert_eq!(m.items[0].value, "code");
        on_menu(&mut p, &m.id.clone(), "code");
        assert_eq!(p.cfg.types.last().unwrap().editor, "code");
        // Removing asks first.
        let fx = press(&mut p, "editors", KeyCode::Char('d'));
        let Some(Effect::Menu(c)) = fx.first() else {
            panic!("{fx:?}")
        };
        let n = p.cfg.types.len();
        on_menu(&mut p, &c.id.clone(), "no");
        assert_eq!(p.cfg.types.len(), n);
        on_menu(&mut p, &c.id.clone(), "yes");
        assert_eq!(p.cfg.types.len(), n - 1);
    }

    #[test]
    fn an_editor_not_installed_is_kept_with_a_warning() {
        let p = sample();
        assert!(matches!(
            validate(&p, &Field::Editor(0), "radar-no-such-editor --wait"),
            Verdict::Warn(m) if m.contains("not installed here")
        ));
        assert!(matches!(
            validate(&p, &Field::Editor(0), "sh"),
            Verdict::Good(_)
        ));
        assert!(
            matches!(validate(&p, &Field::Editor(0), ""), Verdict::Good(m) if m.contains("asks"))
        );
        assert!(matches!(
            validate(&p, &Field::Markers(0), "src/pom.xml"),
            Verdict::Bad(_)
        ));
        assert!(
            matches!(validate(&p, &Field::TypeAdd, "java pom.xml"), Verdict::Bad(m) if m.contains("already"))
        );
    }

    /// A folder that reads as a clone whose origin is `url`, without git.
    fn fake_clone(name: &str, url: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("radar-links-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(
            dir.join(".git/config"),
            format!("[remote \"origin\"]\n\turl = {url}\n"),
        )
        .unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        dir
    }

    #[test]
    fn a_link_asks_for_the_repo_then_the_folder_and_warns_when_origin_names_another() {
        let mut p = sample();
        let clone = fake_clone("add", "git@example.com:acme/team-a/api.git");
        let other = fake_clone("other", "git@example.com:acme/team-a/web.git");
        // The repo first: a path, not already linked.
        assert!(matches!(
            validate(&p, &Field::LinkRepo(None), "api"),
            Verdict::Bad(_)
        ));
        assert!(
            matches!(validate(&p, &Field::LinkRepo(None), "acme/team-a/api"), Verdict::Good(m) if m.contains("1 checkout"))
        );
        assert!(matches!(
            validate(&p, &Field::LinkDir(None, "acme/team-a/api".into()), "/radar/no/such"),
            Verdict::Bad(m) if m.contains("not a folder")
        ));
        assert!(matches!(
            validate(&p, &Field::LinkDir(None, "acme/team-a/api".into()), &std::env::temp_dir().display().to_string()),
            Verdict::Bad(m) if m.contains("not a git clone")
        ));
        assert!(matches!(
            validate(&p, &Field::LinkDir(None, "acme/team-a/api".into()), &other.display().to_string()),
            Verdict::Warn(m) if m.contains("acme/team-a/web")
        ));
        p.local.rows[2] = 0;
        press(&mut p, "links", KeyCode::Enter);
        typed(&mut p, "links", "acme/team-a/api");
        assert!(
            matches!(&p.local.edit, Some((Field::LinkDir(None, r), _)) if r == "acme/team-a/api")
        );
        // The folder of a clone found by origin is offered.
        assert_eq!(p.local.edit.as_ref().unwrap().1.value(), "/work/acme/api");
        for _ in 0.."/work/acme/api".len() {
            press(&mut p, "links", KeyCode::Backspace);
        }
        let fx = typed(&mut p, "links", &format!("{}/", clone.display()));
        assert!(
            matches!(&fx[..], [Effect::Notify(m)] if m == "linked acme/team-a/api"),
            "{fx:?}"
        );
        assert_eq!(p.cfg.links[0].dir, clone.display().to_string());
        assert_eq!(
            link_state(&p, &p.cfg.links[0]),
            Verdict::Good("main".into())
        );
        assert_eq!(selected_link(&p), Some(0));
        // The same repo twice is refused; unlinking asks first.
        assert!(
            matches!(validate(&p, &Field::LinkRepo(None), "ACME/team-a/api"), Verdict::Bad(m) if m.contains("already"))
        );
        let fx = press(&mut p, "links", KeyCode::Char('d'));
        let Some(Effect::Menu(c)) = fx.first() else {
            panic!("{fx:?}")
        };
        on_menu(&mut p, &c.id.clone(), "no");
        assert_eq!(p.cfg.links.len(), 1);
        on_menu(&mut p, &c.id.clone(), "yes");
        assert!(p.cfg.links.is_empty() && p.local.linked.is_empty());
        let _ = (
            std::fs::remove_dir_all(clone),
            std::fs::remove_dir_all(other),
        );
    }

    #[test]
    fn a_template_is_checked_against_the_sample_ticket_and_conventions_are_added_and_removed() {
        let mut p = sample();
        p.cfg.me = "alice".into();
        assert_eq!(
            validate(&p, &Field::Branch, "{me}/{key}"),
            Verdict::Good("ABC-12 'Add a stats endpoint' (Story) → alice/ABC-12".into())
        );
        p.cfg.type_map.insert("story".into(), "feature".into());
        assert!(matches!(
            validate(&p, &Field::Branch, "{type}/{key}-{slug}"),
            Verdict::Good(m) if m.ends_with("feature/ABC-12-add-a-stats-endpoint")
        ));
        assert!(
            matches!(validate(&p, &Field::Branch, "{me}/work"), Verdict::Bad(m) if m.contains("{key}"))
        );
        assert!(matches!(validate(&p, &Field::Branch, ""), Verdict::Bad(_)));
        assert!(matches!(
            validate(&p, &Field::Branch, "{key}..{me}"),
            Verdict::Bad(_)
        ));
        assert!(matches!(
            validate(&p, &Field::LinkBranch(0), ""),
            Verdict::Good(_)
        ));
        assert!(matches!(
            validate(&p, &Field::Worktree, "{clone}"),
            Verdict::Warn(_)
        ));
        assert!(matches!(
            validate(&p, &Field::TypeMap, "story"),
            Verdict::Bad(_)
        ));
        assert!(matches!(
            validate(&p, &Field::ConvAdd, "acme/team-b"),
            Verdict::Bad(_)
        ));
        assert!(matches!(
            validate(&p, &Field::ConvAdd, "acme/team-b {key} bad..base"),
            Verdict::Bad(_)
        ));
        p.local.rows[3] = 0;
        press(&mut p, "branches", KeyCode::Enter);
        typed(&mut p, "branches", "acme/team-b {type}/{key} develop");
        assert_eq!(
            p.cfg.conventions,
            [Convention {
                under: "acme/team-b".into(),
                branch: "{type}/{key}".into(),
                base: "develop".into()
            }]
        );
        assert!(
            matches!(validate(&p, &Field::ConvAdd, "acme/team-b {key}"), Verdict::Bad(m) if m.contains("already"))
        );
        // The scalar rows: the cursor goes past the convention to `me`.
        press(&mut p, "branches", KeyCode::Down);
        assert_eq!(selected_brow(&p), BRow::Me);
        press(&mut p, "branches", KeyCode::Char('e'));
        for _ in 0.."alice".len() {
            press(&mut p, "branches", KeyCode::Backspace);
        }
        typed(&mut p, "branches", "bob");
        assert_eq!(p.cfg.me, "bob");
        press(&mut p, "branches", KeyCode::Char('r'));
        assert!(p.cfg.me.is_empty(), "back to the default");
        press(&mut p, "branches", KeyCode::Up);
        let fx = press(&mut p, "branches", KeyCode::Char('d'));
        let Some(Effect::Menu(c)) = fx.first() else {
            panic!("{fx:?}")
        };
        on_menu(&mut p, &c.id.clone(), "no");
        assert_eq!(p.cfg.conventions.len(), 1);
        on_menu(&mut p, &c.id.clone(), "yes");
        assert!(p.cfg.conventions.is_empty());
    }

    #[test]
    fn every_row_of_the_new_sections_is_its_own_and_acts() {
        use radar_core::check::{Check, Place};
        let make = || {
            let mut p = sample();
            p.cfg.me = "alice".into();
            p.cfg.links.push(Link {
                repo: "acme/team-a/api".into(),
                dir: "/work/acme/api".into(),
                ..Link::default()
            });
            p.cfg.conventions.push(Convention {
                under: "acme/team-b".into(),
                branch: "{key}".into(),
                base: String::new(),
            });
            Box::new(p) as Box<dyn radar_core::Plugin>
        };
        let mut check = Check::new(&make);
        let mut places = Vec::new();
        for id in ["links", "branches"] {
            for row in 0..8 {
                places.push(Place::section(id).down(row));
            }
        }
        let gaps = check.places(&places);
        assert!(gaps.is_empty(), "{} gaps:\n{}", gaps.len(), gaps.join("\n"));
    }

    fn hosted() -> radar_core::Host {
        let mut host =
            radar_core::Host::new(vec![Box::new(custom())], radar_core::plugin::DEMO_NOW).unwrap();
        host.load_config(Some("")).unwrap();
        // load_config re-configures from the (empty) file: put the sample back.
        host.plugins[0] = Box::new(custom());
        host
    }

    fn custom() -> GitPlugin {
        let mut p = sample();
        p.cfg.me = "alice".into();
        p.cfg.type_map.insert("story".into(), "feature".into());
        // The cursor on the link that is not a clone, and on the branch template.
        p.local.rows[2] = 2;
        p.local.rows[3] = 3;
        p.cfg.links.push(Link {
            repo: "acme/team-a/api".into(),
            dir: "/work/acme/api".into(),
            ..Link::default()
        });
        p.cfg.links.push(Link {
            repo: "acme/team-b/web".into(),
            dir: "/radar/no/such/web".into(),
            branch: "{type}/{key}-{slug}".into(),
            base: "develop".into(),
        });
        p.cfg.conventions.push(Convention {
            under: "acme/team-b".into(),
            branch: "{type}/{key}-{slug}".into(),
            base: "develop".into(),
        });
        // Repo paths of ~50 cells, as real ones are.
        p.cfg.links.push(Link {
            repo: "acme/platform-services/billing/payments-gateway".into(),
            dir: "/work/acme/payments-gateway".into(),
            ..Link::default()
        });
        p.cfg.conventions.push(Convention {
            under: "acme/platform-services/billing/payments-gateway".into(),
            branch: "{type}/{key}-{slug}".into(),
            base: "develop".into(),
        });
        p.local.linked = vec![
            crate::local::Checkout {
                dir: "/work/acme/api".into(),
                project: "acme/team-a/api".into(),
                branch: "main".into(),
                ..Default::default()
            },
            // Its origin names another, long, repo: a warning as long as that.
            crate::local::Checkout {
                dir: "/work/acme/payments-gateway".into(),
                project: "acme/platform-services/billing/payments-gateway-v2".into(),
                branch: "main".into(),
                ..Default::default()
            },
        ];
        p
    }

    fn shot(section: &str, w: u16, h: u16) -> String {
        let mut host = hosted();
        host.view = radar_core::host::View::Settings;
        host.section = host
            .sections
            .iter()
            .position(|(_, s)| s.name == section)
            .unwrap();
        radar_core::shell::to_text(&host, w, h)
    }

    #[test]
    fn settings_links_80x24() {
        insta::assert_snapshot!(shot("links", 80, 24));
    }

    #[test]
    fn settings_branches_160x40() {
        insta::assert_snapshot!(shot("branches", 160, 40));
    }

    #[test]
    fn settings_links_160x40() {
        insta::assert_snapshot!(shot("links", 160, 40));
    }

    #[test]
    fn settings_branches_80x24() {
        insta::assert_snapshot!(shot("branches", 80, 24));
    }

    /// Long paths share the row and leave room for the status.
    #[test]
    fn a_long_repo_path_is_cut_before_what_was_set() {
        let text = |l: &Text| l.words();
        let repo = "acme/platform-services/billing/payments-gateway";
        let status = Some(("✓ main".to_string(), Ink::Good));
        let (line, right) = pair_row(
            repo,
            "{type}/{key}-{slug}".into(),
            Ink::Secondary,
            status.clone(),
            44,
            false,
        );
        assert!(
            text(&line).ends_with("{type}/{key}-{slug}"),
            "{}",
            text(&line)
        );
        assert!(
            text(&line).starts_with('…'),
            "the cut is marked: {}",
            text(&line)
        );
        assert!(chars(&text(&line)) + right.map_or(0, |r| chars(&r.words()) + 2) <= 40);
        // Wide enough: nothing is cut.
        let (line, right) = pair_row(
            repo,
            "~/work/payments".into(),
            Ink::Secondary,
            status.clone(),
            160,
            true,
        );
        assert_eq!(text(&line), format!("{repo} → ~/work/payments"));
        assert!(right.is_some());
        // No room even for a short repo: the status goes before the value is cut.
        let (line, right) = pair_row(
            repo,
            "~/work/payments-gateway".into(),
            Ink::Secondary,
            status,
            40,
            true,
        );
        assert!(
            right.is_some() && text(&line).contains(" → "),
            "{}",
            text(&line)
        );
        // The branch row keeps its template and drops the sample first.
        let (line, right) = kv_row(
            "branch",
            "{me}/{key}",
            "",
            Some(("alice/ABC-12".into(), Ink::Number)),
            30,
        );
        assert!(
            text(&line).contains("{me}/{key}") && right.is_none(),
            "{}",
            text(&line)
        );
        let (_, right) = kv_row(
            "branch",
            "{me}/{key}",
            "",
            Some(("alice/ABC-12".into(), Ink::Number)),
            60,
        );
        assert!(right.is_some());
    }
}
