//! The Jira screen: a nav pane of views in words │ a middle pane (a tree) of site → status →
//! tickets (a parent's children under it), striped by site and banded like
//! Repositories │ a detail pane of whatever is selected. Described as view data
//! ([`view`]) and driven by intents ([`intent`]).

use std::collections::{BTreeMap, HashSet};

use radar_core::design::help_bar::KeyHint;
use radar_core::input::{Edit, TextInput};
use radar_core::plugin::{Binding, Effect, MenuItem, Outcome, Pane, Tone};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent};
use radar_core::ui::icon::{self, Icon};
use radar_core::ui;
use radar_core::view::{self as v, Block, ColumnSpec, Ink, Intent, Node, PaneId, PaneView, Seg, Text, TreeRow, kit};
use radar_core::time;
use radar_core::view::detail::{self, DetailContent, DetailFact, DetailHeader, FootAction, FootActions};

use crate::config::Site;
use crate::model::{Category, Ticket};
use crate::{Goto, Item, JiraPlugin, NOBODY, join, page, view, write};
use radar_core::Parts;

// --- what the tree holds -------------------------------------------------------

/// A ticket and the complete ancestry held in this filtered view. A child
/// stays with its parent even when their statuses differ.
pub(crate) struct TicketNode<'a> {
    pub ticket: &'a Ticket,
    pub children: Vec<TicketNode<'a>>,
}

/// Only roots are grouped by status; the count includes all held descendants.
pub(crate) struct GroupNode<'a> {
    pub status: String,
    pub category: Category,
    pub tops: Vec<TicketNode<'a>>,
}

impl<'a> GroupNode<'a> {
    fn nodes(&self) -> impl Iterator<Item = &TicketNode<'a>> {
        let mut pending: Vec<_> = self.tops.iter().rev().collect();
        std::iter::from_fn(move || {
            let node = pending.pop()?;
            pending.extend(node.children.iter().rev());
            Some(node)
        })
    }

    pub fn tickets(&self) -> impl Iterator<Item = &'a Ticket> + '_ {
        self.nodes().map(|node| node.ticket)
    }

    pub fn len(&self) -> usize {
        self.tickets().count()
    }
}

/// Use only loaded, filtered tickets. Repeated query keys refer to one ticket;
/// the first occurrence keeps Jira's ordering. A missing parent makes a root.
/// Break each cycle at its smallest key, independent of response order, so
/// every distinct ticket appears once without inventing or fetching a parent.
fn forest(tickets: Vec<&Ticket>) -> Vec<GroupNode<'_>> {
    let mut seen = HashSet::new();
    let tickets: Vec<_> = tickets.into_iter().filter(|t| seen.insert(t.key.as_str())).collect();
    let indices: BTreeMap<_, _> = tickets.iter().enumerate().map(|(i, t)| (t.key.as_str(), i)).collect();
    let mut parents: Vec<_> = tickets.iter().map(|t| {
        t.parent.as_ref().and_then(|parent| indices.get(parent.key.as_str()).copied())
    }).collect();
    let mut finished = vec![false; tickets.len()];
    for start in 0..tickets.len() {
        let mut path: Vec<usize> = Vec::new();
        let mut positions = BTreeMap::new();
        let mut next = Some(start);
        while let Some(i) = next {
            if finished[i] {
                break;
            }
            if let Some(&at) = positions.get(&i) {
                let root = *path[at..].iter().min_by_key(|&&j| tickets[j].key.as_str()).expect("cycle is nonempty");
                parents[root] = None;
                break;
            }
            positions.insert(i, path.len());
            path.push(i);
            next = parents[i];
        }
        for i in path {
            finished[i] = true;
        }
    }
    let mut children = vec![Vec::new(); tickets.len()];
    for (i, parent) in parents.iter().enumerate() {
        if let Some(parent) = parent {
            children[*parent].push(i);
        }
    }
    fn branch<'a>(i: usize, tickets: &[&'a Ticket], children: &[Vec<usize>]) -> TicketNode<'a> {
        TicketNode {
            ticket: tickets[i],
            children: children[i].iter().map(|&child| branch(child, tickets, children)).collect(),
        }
    }
    let mut by: BTreeMap<(Category, String), Vec<TicketNode<'_>>> = BTreeMap::new();
    for (i, ticket) in tickets.iter().enumerate() {
        if parents[i].is_none() {
            by.entry((ticket.category, ticket.status.clone())).or_default().push(branch(i, &tickets, &children));
        }
    }
    by.into_iter().map(|((category, status), tops)| GroupNode { status, category, tops }).collect()
}

/// Walk every held ticket with its current structural path, regardless of folds.
fn visit_tickets<'a>(nodes: &[TicketNode<'a>], site: &str, prefix: &[String], visit: &mut impl FnMut(&'a Ticket, &[String], bool)) {
    for node in nodes {
        let mut path = prefix.to_vec();
        path.push(ticket_id(site, &node.ticket.key));
        visit(node.ticket, &path, !node.children.is_empty());
        visit_tickets(&node.children, site, &path, visit);
    }
}

pub(crate) struct SiteNode<'a> {
    pub index: usize,
    pub site: &'a Site,
    pub groups: Vec<GroupNode<'a>>,
    /// The view asks this site something.
    pub asked: bool,
    /// An answer for this view has come.
    pub loaded: bool,
}

pub(crate) enum Selected<'a> {
    Site(usize),
    Group(usize, String),
    Ticket(Item<'a>),
}

pub(crate) fn site_id(name: &str) -> String {
    format!("s:{name}")
}
fn group_id(site: &str, status: &str) -> String {
    format!("g:{site}:{status}")
}
pub(crate) fn ticket_id(site: &str, key: &str) -> String {
    format!("t:{site}:{key}")
}

pub(crate) fn nodes(p: &JiraPlugin) -> Vec<SiteNode<'_>> {
    // What you typed (`/`) or the assignee picked narrows the tree: a site
    // with nothing left in it is not part of the answer (one that failed to
    // sync stays, to say so).
    let narrowed = !p.filter.trim().is_empty() || p.assignee.is_some();
    p.cfg
        .sites
        .iter()
        .enumerate()
        .map(|(index, site)| {
            let asked = p
                .current_view()
                .is_some_and(|v| p.jql_for(index, v).is_some());
            let held = p.site_tickets(index);
            let loaded = held.is_some();
            let tickets: Vec<&Ticket> = held
                .unwrap_or_default()
                .into_iter()
                .filter(|t| p.shown(t))
                .collect();
            let groups = forest(tickets);
            SiteNode {
                index,
                site,
                groups,
                asked,
                loaded,
            }
        })
        .filter(|sn| !narrowed || !sn.groups.is_empty() || p.sync.errors.contains_key(&sn.site.name))
        .collect()
}

pub(crate) fn selected(p: &JiraPlugin) -> Option<Selected<'_>> {
    selection(p).map(|(selected, _)| selected)
}

/// Resolve the subject and its held children together from the repaired forest,
/// so action hints never scan unrelated sites or follow a cut cycle edge.
fn selection(p: &JiraPlugin) -> Option<(Selected<'_>, bool)> {
    let path = p.tree.borrow().selected().to_vec();
    let last = path.last()?.clone();
    for sn in nodes(p) {
        if last == site_id(&sn.site.name) {
            return Some((Selected::Site(sn.index), !sn.groups.is_empty()));
        }
        for g in &sn.groups {
            if last == group_id(&sn.site.name, &g.status) {
                return Some((Selected::Group(sn.index, g.status.clone()), !g.tops.is_empty()));
            }
            for node in g.nodes() {
                if last == ticket_id(&sn.site.name, &node.ticket.key) {
                    return Some((Selected::Ticket(Item { site: sn.site, t: node.ticket }), !node.children.is_empty()));
                }
            }
        }
    }
    None
}

/// Leave `/` for the tree, on the first ticket found (or, with `keep`, where
/// the selection was if it is still there). Jira says nothing when the
/// selection moves; its preview is drawn from the tree.
fn to_results(p: &mut JiraPlugin, keep: bool) -> Vec<Effect> {
    let before = p.filter_before.take().unwrap_or_else(|| p.tree.borrow().selected().to_vec());
    let nodes = nodes(p);
    let kept = keep.then(|| {
        paths(&nodes).into_iter().find(|(path, _, _)| path.last() == before.last()).map(|(path, _, _)| path)
    }).flatten();
    open_new_nodes(p, &nodes);
    let next = kept.or_else(|| targets(p).into_iter().next().map(|(path, ..)| path));
    if let Some(path) = next {
        let mut state = p.tree.borrow_mut();
        for depth in 1..path.len() {
            state.open(path[..depth].to_vec());
        }
        state.select(path);
    }
    vec![Effect::Focus(Pane::Middle)]
}

/// The tree path to every ticket shown, with the words go to matches.
fn targets(p: &JiraPlugin) -> Vec<(Vec<String>, String, Vec<String>)> {
    let mut out = Vec::new();
    for sn in nodes(p) {
        let sid = site_id(&sn.site.name);
        for g in &sn.groups {
            let gid = group_id(&sn.site.name, &g.status);
            visit_tickets(&g.tops, &sn.site.name, &[sid.clone(), gid], &mut |ticket, path, _| {
                out.push((path.to_vec(), format!("{} {}", ticket.key, ticket.summary), vec![ticket.key.clone(), ticket.summary.clone()]));
            });
        }
    }
    out
}

// --- top bar and status line ------------------------------------------------------

pub fn top_right(p: &JiraPlugin) -> Option<String> {
    if p.cfg.sites.is_empty() {
        return None;
    }
    Some(radar_core::ui::status::sync_words(
        "Jira",
        !p.sync.in_flight.is_empty(),
        p.sync.last_ok,
        p.now,
        Some(p.cfg.refresh_secs),
    ))
}

pub fn status(p: &JiraPlugin) -> Vec<(String, Tone)> {
    // The whole part's view, whatever the filter or the assignee hides.
    let mut out = Vec::new();
    if !p.cfg.sites.is_empty() {
        let held: Vec<&Ticket> = (0..p.cfg.sites.len()).filter_map(|i| p.site_tickets(i)).flatten().collect();
        let doing = held.iter().filter(|t| t.category == Category::Doing).count();
        out.push((format!("{} ticket{}", held.len(), if held.len() == 1 { "" } else { "s" }), Tone::Plain));
        if doing > 0 {
            out.push((format!("{doing} doing"), Tone::Info));
        }
    }
    if !p.sync.errors.is_empty() {
        let n = p.sync.errors.len();
        out.push((
            format!("{n} site{} could not sync", if n == 1 { "" } else { "s" }),
            Tone::Bad,
        ));
    }
    out
}

// --- the screen as view data -------------------------------------------------------

/// The screen's nav, middle and detail panes as view data.
pub fn view(p: &JiraPlugin, cx: &v::ViewCx<'_>) -> v::ScreenView {
    let mut nav = PaneView::new(
        PaneId::Nav,
        "views",
        Node::List {
            rows: p
                .cfg
                .views
                .iter()
                .enumerate()
                .map(|(i, view)| {
                    v::ListRow::new(
                        format!("view:{i}"),
                        Text::new().push(Seg::icon(view_icon(view))).push(Seg::new(view.name.clone(), Ink::Plain)),
                    )
                })
                .collect(),
            selected: Some(p.view),
            editing: None,
        },
    );
    nav.width = Some(26);
    v::ScreenView::new(SCREEN_ID, vec![nav, middle_pane(p, cx), detail_pane(p, cx)])
}

const SCREEN_ID: &str = crate::SCREEN.id;

/// The foot of the tree: a site that could not load, then what the view
/// shows, in words, and the sites it asks nothing of (the nav pane is too
/// narrow for any of it).
fn shows_lines(p: &JiraPlugin) -> Vec<Text> {
    let mut lines: Vec<Text> = p.sync.errors.iter().map(|(site, why)| kit::sync_failed(site, why)).collect();
    if let Some(v) = p.current_view() {
        lines.push(Text::of(view::describe_view(v).join(" · "), Ink::Secondary));
        // Sites the view has nothing to ask, and why.
        for (i, s) in p.cfg.sites.iter().enumerate() {
            if p.jql_for(i, v).is_none() {
                lines.push(Text::of(format!("nothing asked of {}: {}", s.name, view::why_not(s, v)), Ink::Muted));
            }
        }
    }
    lines
}

/// A view's icon, from what its conditions ask.
fn view_icon(v: &crate::config::View) -> Icon {
    let has = |c: &str| v.when.iter().any(|w| w == c);
    if !v.jql.trim().is_empty() {
        Icon::Search
    } else if has("assignee:me") {
        Icon::You
    } else if has("assignee:none") {
        Icon::Person
    } else if has("sprint:open") {
        Icon::Board
    } else if has("status:doing") {
        Icon::InProgress
    } else if has("reporter:me") {
        Icon::Comment
    } else {
        Icon::Issue
    }
}

// --- the tree -----------------------------------------------------------------------

/// The tree's columns: type goes first as the width shrinks, then updated;
/// the names keep 28 cells.
fn columns(lead: u16) -> Vec<ColumnSpec> {
    vec![
        ColumnSpec::new("type", 10, 1),
        ColumnSpec::new("status", 16, 3),
        // Room for the icon before its heading when icons are on.
        ColumnSpec::new("updated", 8 + lead, 2).icon(Icon::Updated),
    ]
}

const NAME_ROOM: u16 = 28;

/// The columns other parts add to the tickets shown: each heading once, as
/// wide as its widest value (at most 20), in the order first seen. Only
/// when some ticket has a value.
fn added_columns(p: &JiraPlugin, nodes: &[SiteNode<'_>], parts: &Parts<'_>) -> Vec<ColumnSpec> {
    let mut out: Vec<ColumnSpec> = Vec::new();
    for sn in nodes {
        for g in &sn.groups {
            for tk in g.tickets() {
                for cell in parts.cells(&join::ticket(p, &sn.site.name, &tk.key)) {
                    let lead = cell.icon.map_or(0, |i| ui::width(&icon::plain(i)));
                    let w = ((lead + ui::width(&cell.text)).max(ui::width(&cell.heading)) + 1).min(20) as u16;
                    match out.iter_mut().find(|c| c.heading == cell.heading) {
                        Some(c) => c.width = c.width.max(w),
                        None => out.push(ColumnSpec::extra(cell.heading, w)),
                    }
                }
            }
        }
    }
    out
}

/// How old a change is, as a look word: fresh under 2 days, aging under 2
/// weeks, stale after.
pub(crate) fn age_word(updated: &str, now: i64) -> &'static str {
    match time::parse_rfc3339(&iso(updated)) {
        Some(at) if now - at < 2 * 86_400 => "fresh",
        Some(at) if now - at < 14 * 86_400 => "aging",
        Some(_) => "stale",
        None => "stale",
    }
}

/// Jira writes `2026-10-06T09:00:00.000+0000`; radar's parser wants `+00:00`.
pub(crate) fn iso(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() > 5 {
        let (head, tail) = s.split_at(s.len() - 5);
        if (tail.starts_with('+') || tail.starts_with('-')) && tail[1..].chars().all(|c| c.is_ascii_digit()) {
            return format!("{head}{}:{}", &tail[..3], &tail[3..]);
        }
    }
    s.to_string()
}

/// Jira's time as RFC 3339 in UTC, for the CLI's JSON (`updated`); what
/// does not parse stays as Jira wrote it.
pub(crate) fn rfc3339(s: &str) -> String {
    time::parse_rfc3339(&iso(s)).map_or_else(|| s.to_string(), time::to_rfc3339)
}

/// A person for the CLI's JSON: `{name, handle}`, the handle being Jira's
/// account id (`null` when unknown); `null` for nobody.
pub(crate) fn person(name: &str, account_id: &str) -> serde_json::Value {
    if name.is_empty() {
        return serde_json::Value::Null;
    }
    serde_json::json!({ "name": name, "handle": (!account_id.is_empty()).then_some(account_id) })
}

/// `2h`, from Jira's time.
pub(crate) fn ago(s: &str, now: i64) -> String {
    time::ago_str(&iso(s), now)
}

/// `◐ In Review`, in its category's look.
pub(crate) fn status_text(p: &JiraPlugin, status: &str, c: Category) -> Text {
    let look = p.cfg.style_for(c.word());
    let mut mark = Seg::icon_or(c.icon(), c.symbol(), Ink::Plain);
    mark.look = Some(look.clone());
    Text::new().push(mark).push(Seg::styled(format!(" {status}"), look))
}

/// The icon before a priority's name, when it is one we know; an empty
/// run otherwise, so the runs after it keep their places.
pub(crate) fn priority_seg(priority: &str) -> Seg {
    crate::model::priority_icon(priority).map_or_else(Seg::default, Seg::icon)
}

/// A ticket's assignee as an icon and a name: "you" in the `you` look with
/// the you icon when the ticket is assigned to the site's `myself`; the
/// name (given, as the place shows it) in `base` with the person icon for
/// anyone else; nothing but `name` for a ticket nobody has.
pub(crate) fn assignee_segs(p: &JiraPlugin, site: &str, tk: &Ticket, name: String, base: Ink) -> [Seg; 2] {
    if tk.assignee.is_empty() {
        [Seg::default(), Seg::new(name, base)]
    } else if p.is_mine(site, tk) {
        [Seg::icon(Icon::You), Seg::styled("you", p.cfg.style_for("you"))]
    } else {
        [Seg::icon(Icon::Person), Seg::new(name, base)]
    }
}

/// Site → status → tickets, a parent's children under it; the site's
/// colour is its stripe.
fn tree_rows(p: &JiraPlugin, nodes: &[SiteNode<'_>], now: i64, parts: &Parts<'_>) -> Vec<TreeRow> {
    let ticket = |site: &str, tk: &Ticket| {
        let subject = join::ticket(p, site, &tk.key);
        let who = if tk.assignee.is_empty() {
            "unassigned".to_string()
        } else {
            ui::short_name(&tk.assignee)
        };
        let [who_icon, who_name] = assignee_segs(p, site, tk, who, Ink::Muted);
        let name = Text::from(vec![
            Seg::icon(tk.icon()),
            Seg::styled(format!("{} ", tk.key), p.cfg.style_for("key")),
            priority_seg(&tk.priority),
            Seg::new(tk.summary.clone(), Ink::Primary),
            Seg::new("  ", Ink::Muted),
            who_icon,
            who_name,
        ]);
        let mut row = TreeRow::new(ticket_id(site, &tk.key), name)
            .chips(3, join::chip_text(parts, &subject))
            .cell("type", Text::from(vec![Seg::styled(tk.kind.clone(), p.cfg.style_for(tk.type_word()))]))
            .cell("status", status_text(p, &tk.status, tk.category))
            .cell(
                "updated",
                Text::from(vec![Seg::styled(ago(&tk.updated, now), p.cfg.style_for(age_word(&tk.updated, now)))]),
            )
            .glow(join::motion_key(&tk.key));
        // Other parts' columns: their words in the secondary text look.
        for cell in parts.cells(&subject) {
            let mut cv = v::cell_of(&cell);
            if let Some(value) = cv.text.0.last_mut()
                && value.ink == Ink::Plain
                && !value.strong
            {
                value.ink = Ink::Secondary;
            }
            row = row.cell_view(cv);
        }
        row
    };
    fn branch(node: &TicketNode<'_>, site: &str, ticket: &impl Fn(&str, &Ticket) -> TreeRow) -> TreeRow {
        ticket(site, node.ticket).children(node.children.iter().map(|child| branch(child, site, ticket)))
    }
    nodes
        .iter()
        .map(|sn| {
            let site = sn.site.name.as_str();
            let groups = sn.groups.iter().map(|g| {
                TreeRow::new(group_id(site, &g.status), status_text(p, &g.status, g.category)).children(
                    g.tops
                        .iter()
                        .map(|node| branch(node, site, &ticket)),
                )
            });
            TreeRow::new(site_id(site), site_name(p, sn))
                .stripe(format!("jira.{}", sn.site.color))
                .children(groups)
        })
        .collect()
}

/// A site's row: its name in its colour, its host, how many tickets.
fn site_name(p: &JiraPlugin, sn: &SiteNode<'_>) -> Text {
    let mut text = Text::from(vec![
        Seg::icon(Icon::Site),
        Seg::styled(sn.site.name.clone(), format!("jira.{} {}", sn.site.color, p.cfg.style_for("site"))),
        Seg::new(format!("  {}", sn.site.host()), Ink::Secondary),
    ]);
    if p.sync.errors.contains_key(&sn.site.name) {
        let err = p.cfg.style_for("failed");
        text.add(Seg::styled("   ", err.clone()));
        let mut mark = Seg::icon_or(Icon::Error, "✗", Ink::Plain);
        mark.look = Some(err.clone());
        text.add(mark);
        text.add(Seg::styled(" could not load", err));
    } else if !sn.asked {
        text.add(Seg::new("   nothing to ask here in this view", Ink::Muted));
    } else if !sn.loaded {
        text.add(Seg::new("   loading…", Ink::Muted));
    } else {
        text.add(Seg::default());
    }
    text
}

/// The tree's look as view data: lines, ticket bands and site headings.
fn tree_look_view(p: &JiraPlugin) -> v::TreeLook {
    v::TreeLook {
        lines: p.cfg.style_for("lines"),
        band: p.cfg.style_for("band"),
        heading: p.cfg.style_for("heading"),
        band_level: 2,
    }
}

/// Paths to all held rows, including groups hidden inside a closed ancestor.
fn paths(nodes: &[SiteNode<'_>]) -> Vec<(Vec<String>, bool, bool)> {
    let mut out = Vec::new();
    for sn in nodes {
        let sid = site_id(&sn.site.name);
        out.push((vec![sid.clone()], !sn.groups.is_empty(), true));
        for group in &sn.groups {
            let path = vec![sid.clone(), group_id(&sn.site.name, &group.status)];
            out.push((path.clone(), !group.tops.is_empty(), group.category != Category::Done));
            visit_tickets(&group.tops, &sn.site.name, &path, &mut |_, path, children| {
                out.push((path.to_vec(), children, true));
            });
        }
    }
    out
}

/// Open each group once; afterwards fold choices follow its identity across
/// status/ancestry changes. Selected tickets follow the same identity, opening
/// their new ancestry when a refresh or filter moves them to another branch.
fn open_new_nodes(p: &JiraPlugin, nodes: &[SiteNode<'_>]) {
    let paths = paths(nodes);
    let by_id: BTreeMap<_, _> = paths.iter().filter_map(|(path, _, _)| path.last().map(|id| (id, path))).collect();
    let mut seen = p.opened_once.borrow_mut();
    let mut state = p.tree.borrow_mut();
    for open in &mut state.open {
        if let Some(path) = open.last().and_then(|id| by_id.get(id)) {
            *open = (*path).clone();
        }
    }
    for (path, children, initially_open) in &paths {
        if *children && seen.insert(path.last().expect("row path").clone()) && *initially_open {
            state.open(path.clone());
        }
    }
    let previous = state.selected.clone();
    let current = previous.last().and_then(|id| by_id.get(id)).copied();
    if let Some(path) = current {
        if *path != previous {
            for depth in 1..path.len() {
                state.open(path[..depth].to_vec());
            }
        }
        state.select(path.clone());
    } else {
        state.select(previous.iter().rev().find_map(|id| by_id.get(id).map(|path| (*path).clone()))
            .or_else(|| paths.first().map(|(path, _, _)| path.clone())).unwrap_or_default());
    }
    // A deliberate fold keeps selection on the closest visible ancestor.
    for depth in 1..state.selected.len() {
        if !state.is_open(&state.selected[..depth]) {
            state.selected.truncate(depth);
            break;
        }
    }
}

fn accepts_tree_target(p: &JiraPlugin, target: &v::Target, fold: bool) -> bool {
    let nodes = nodes(p);
    open_new_nodes(p, &nodes);
    let path = TreeRow::path_of(target);
    let held = paths(&nodes).into_iter().any(|(held, children, _)| held == path && (!fold || children));
    held && (fold || (1..path.len()).all(|depth| p.tree.borrow().is_open(&path[..depth])))
}

/// The middle pane: go to and the filter over the tree of tickets, what
/// the view shows at its foot.
fn middle_pane(p: &JiraPlugin, cx: &v::ViewCx<'_>) -> PaneView {
    let nodes = nodes(p);
    let title = match p.current_view() {
        Some(v) => v.name.clone(),
        None => "tickets".into(),
    };
    let body = tree_body(p, cx, &nodes);
    let mut pane = PaneView::new(PaneId::Middle, title, body);
    if let Some(g) = &p.goto {
        pane.fields.push(goto_field(p, g));
    }
    if p.filter_input.is_some() || !p.filter.is_empty() {
        let hint = if p.filter.trim().is_empty() {
            "type words: a key, part of a summary, a person, a status".to_string()
        } else {
            format!("tickets mentioning \"{}\"", p.filter.trim())
        };
        pane.fields.push(v::FieldBar {
            mark: " / ".into(),
            text: p.filter_input.as_ref().map_or_else(|| p.filter.clone(), TextInput::value),
            caret: p.filter_input.as_ref().map(TextInput::cursor),
            hint: Text::of(hint, Ink::Muted),
        });
    }
    pane.notes = shows_lines(p);
    pane.notes_most = Some(3);
    pane
}

fn tree_body(p: &JiraPlugin, cx: &v::ViewCx<'_>, nodes: &[SiteNode<'_>]) -> Node {
    if p.cfg.sites.is_empty() {
        let mut text = "No Jira site yet. Press , for settings, then add one under connection.".to_string();
        if let Some(f) = p.offers().first() {
            text.push_str(&format!(" radar found {} {}; connection offers it.", f.label, f.source));
        }
        return Node::State {
            state: v::State::Empty,
            text,
        };
    }
    if !nodes.iter().any(|n| n.asked) {
        return Node::State {
            state: v::State::Empty,
            text: "This view asks about watched projects, and no site watches one yet. Press , for settings, then add one under projects.".into(),
        };
    }
    let loaded = nodes.iter().any(|n| n.loaded);
    let note = (!loaded).then(|| {
        let (state, text) = if p.sync.in_flight.is_empty() && p.sync.errors.is_empty() {
            (v::State::Empty, "No tickets yet. Press r to refresh.")
        } else if p.sync.in_flight.is_empty() {
            (v::State::Empty, "Jira did not answer; the detail pane says why. Press r to try again.")
        } else {
            // Waiting for the first data: it shimmers (motion, full).
            (v::State::Loading, "Loading from Jira…")
        };
        // The sites stay selectable for their detail pane.
        v::Note {
            state,
            text: text.into(),
        }
    });
    open_new_nodes(p, nodes);
    let mut columns = columns(cx.icon_lead as u16);
    columns.extend(added_columns(p, nodes, &cx.parts));
    let who = match p.assignee.as_deref() {
        None => "everyone",
        Some(NOBODY) => "nobody",
        Some(a) => a,
    };
    let header = Text::from(vec![
        Seg::new("ASSIGNED TO", Ink::Plain),
        Seg::new(" ", Ink::Plain),
        Seg::icon(if p.assignee.is_some() { Icon::Person } else { Icon::Team }),
        if p.assignee.is_some() {
            Seg::new(who, Ink::AccentSecondary).strong()
        } else {
            Seg::new(who, Ink::Secondary)
        },
    ]);
    let nav = p.tree.borrow();
    Node::Tree(v::Tree {
        header,
        columns,
        ordered_columns: false,
        rows: tree_rows(p, nodes, cx.now, &cx.parts),
        selected: nav.selected.clone(),
        open: nav.open.clone(),
        name_width: NAME_ROOM,
        look: Some(tree_look_view(p)),
        // The heading's words open the assignee menu.
        header_target: Some(v::Target::new("assignees")),
        note,
        cut_names: false,
        editing: None,
        pointer_action: false,
    })
}

// --- detail pane --------------------------------------------------------------------

/// The icon before the menu's title: a ticket's kind.
fn sel_icon(sel: Option<&Selected<'_>>) -> Option<Icon> {
    match sel {
        Some(Selected::Ticket(it)) => Some(it.t.icon()),
        _ => None,
    }
}

/// What the selection is called: the detail pane's title and the menu's.
fn name_of(p: &JiraPlugin, sel: Option<&Selected<'_>>) -> String {
    match sel {
        Some(Selected::Ticket(it)) => it.t.key.clone(),
        Some(Selected::Group(i, status)) => format!("{} · {status}", p.cfg.sites[*i].name),
        Some(Selected::Site(i)) => p.cfg.sites[*i].name.clone(),
        None => "detail".into(),
    }
}

fn detail_pane(p: &JiraPlugin, cx: &v::ViewCx<'_>) -> PaneView {
    let sel = selected(p);
    let blocks = match &sel {
        Some(Selected::Ticket(it)) => {
            let mut content = ticket_detail(p, it, cx.now, &cx.parts);
            if let Some(outcome) = write::outcome(p, &it.site.name, &it.t.key) {
                content.sections.insert(
                    0,
                    Block::section_of(None, "latest change", None, vec![outcome]),
                );
            }
            content
                .sections
                .extend(cx.parts.detail_sections(&join::ticket(p, &it.site.name, &it.t.key)));
            return detail::pane(name_of(p, sel.as_ref()), content, ticket_foot(p, it));
        }
        Some(Selected::Group(i, status)) => group_detail(p, *i, status)
            .into_iter()
            .map(Block::line)
            .collect(),
        Some(Selected::Site(i)) => site_detail(p, *i),
        None => {
            let mut lines = vec![Text::of("Select a site, a status or a ticket.", Ink::Muted)];
            for (site, err) in &p.sync.errors {
                lines.push(Text::new());
                lines.push(failed_line(p, &format!(" {site}")));
                lines.push(Text::of(err.clone(), Ink::Primary));
            }
            lines.into_iter().map(Block::line).collect()
        }
    };
    PaneView::new(
        PaneId::Detail,
        name_of(p, sel.as_ref()),
        Node::Rich {
            blocks,
            pictures: None,
        },
    )
}

/// `✗ words` in the `failed` look.
fn failed_line(p: &JiraPlugin, words: &str) -> Text {
    let bad = p.cfg.style_for("failed");
    let mut mark = Seg::icon_or(Icon::Error, "✗", Ink::Plain);
    mark.look = Some(bad.clone());
    Text::new().push(mark).push(Seg::styled(words, bad))
}

/// How many lines of a description or of acceptance criteria the detail pane
/// shows; the ticket's page has them all.
const DETAIL_LINES: usize = 12;

/// Use the selected ticket's Detail actions, independent of the focused pane.
fn ticket_foot(p: &JiraPlugin, it: &Item<'_>) -> FootActions {
    let offered = p.screen_rows(Pane::Detail);
    let mut controls = Vec::new();
    for (id, caption) in [
        ("jira.tickets.comment", "comment"),
        ("jira.tickets.move", "status"),
        ("jira.tickets.assign_me", "assign to me"),
        ("jira.tickets.open_in_jira", "open"),
        ("jira.tickets.copy_link", "copy link"),
        ("jira.tickets.copy_key", "copy key"),
        ("jira.tickets.open", "page"),
    ] {
        if let Some((id, binding)) = offered.iter().find(|(offered, _)| offered == id) {
            let mut control =
                FootAction::new(v::ActionSpec::named(id.clone(), binding)).caption(caption);
            if id == "jira.tickets.assign_me"
                && let Some(reason) = write::assign_blocked(p, &it.site.name, &it.t.key)
            {
                control = control.disabled(reason);
            }
            controls.push(control);
        }
    }
    FootActions::new(controls)
}

fn ticket_detail(p: &JiraPlugin, it: &Item<'_>, now: i64, parts: &Parts<'_>) -> DetailContent {
    let tk = it.t;
    let key = p.cfg.style_for("key");
    let mut path = Text::from(vec![Seg::icon(Icon::Site)]);
    let mut has_component = false;
    for component in [
        Seg::styled(it.site.name.clone(), format!("jira.{}", it.site.color)),
        Seg::new(tk.project.clone(), Ink::Secondary),
        Seg::styled(tk.key.clone(), key.clone()),
    ] {
        if component.text.trim().is_empty() { continue; }
        if has_component { path.add(Seg::new(" / ", Ink::Secondary)); }
        path.add(component);
        has_component = true;
    }
    let mut header = DetailHeader::new(
        path,
        Text::from(vec![
            Seg::icon(tk.icon()),
            Seg::new(tk.summary.clone(), Ink::Emphasis).strong(),
        ]),
    );
    if !tk.reporter.trim().is_empty() {
        header.author = Some(Text::from(vec![
            Seg::icon(Icon::Person),
            Seg::new(tk.reporter.clone(), Ink::Secondary),
        ]));
    }
    if !tk.updated.trim().is_empty() {
        header.age = Some(Text::of(
            format!("{} ago", ago(&tk.updated, now)),
            Ink::Muted,
        ));
    }
    let who = if tk.assignee.is_empty() {
        "unassigned".to_string()
    } else {
        tk.assignee.clone()
    };
    let mut facts = vec![
        DetailFact::new(
            "status",
            if tk.status.trim().is_empty() {
                Text::new()
            } else {
                status_text(p, &tk.status, tk.category)
            },
        ),
        DetailFact::new(
            "type",
            Text::from(vec![Seg::styled(
                tk.kind.clone(),
                p.cfg.style_for(tk.type_word()),
            )]),
        ),
        DetailFact::new(
            "priority",
            Text::from(vec![
                priority_seg(&tk.priority),
                Seg::new(tk.priority.clone(), Ink::Secondary),
            ]),
        ),
        DetailFact::new(
            "assignee",
            Text::from(assignee_segs(p, &it.site.name, tk, who, Ink::Secondary).to_vec()),
        ),
    ];
    if let Some(parent) = &tk.parent {
        let mut value = Text::from(vec![Seg::styled(parent.key.clone(), key)]);
        if !parent.summary.trim().is_empty() {
            value.add(Seg::new(format!(": {}", parent.summary), Ink::Secondary));
        }
        facts.push(DetailFact::new("parent", value));
    }
    let mut chips = join::chip_text(parts, &join::ticket(p, &it.site.name, &tk.key));
    if !chips.0.is_empty() {
        chips.0.remove(0);
        facts.push(DetailFact::new("related", chips));
    }
    if !tk.labels.is_empty() {
        let mut labels = page::labels_text(p, &tk.labels);
        labels.0.insert(0, Seg::icon(Icon::Label));
        facts.push(DetailFact::new("labels", labels));
    }
    facts.extend([
        DetailFact::new(
            "components",
            Text::of(tk.components.join(", "), Ink::Secondary),
        ),
        DetailFact::new(
            "fix versions",
            Text::of(tk.fix_versions.join(", "), Ink::Secondary),
        ),
    ]);
    let mut blocks = Vec::new();
    let section =
        |blocks: &mut Vec<Block>, heading: &str, body: &radar_core::ui::rich::Doc, empty: &str| {
            let inside = if body.is_empty() {
                Block::line(Text::of(empty, Ink::Muted))
            } else {
                Block::Doc {
                    doc: body.clone(),
                    bar: None,
                    most: Some(DETAIL_LINES),
                    urls: None,
                }
            };
            blocks.push(Block::section(None, heading, None, vec![inside]));
        };
    section(
        &mut blocks,
        "description",
        &tk.description,
        "no description",
    );
    let ac_set = it.site.ac_field(&tk.project).is_some();
    section(
        &mut blocks,
        "acceptance criteria",
        &tk.ac,
        if ac_set {
            "none written"
        } else {
            "no field set for this project: settings, projects"
        },
    );
    DetailContent::new(header, facts, blocks)
}

fn group_detail(p: &JiraPlugin, i: usize, status: &str) -> Vec<Text> {
    let sn = nodes(p).into_iter().find(|n| n.index == i);
    let g = sn.as_ref().and_then(|n| n.groups.iter().find(|g| g.status == status));
    let mut lines = vec![];
    if let Some(g) = g {
        lines.push(status_text(p, &g.status, g.category));
        lines.push(Text::of(
            format!(
                "{} ticket{} on {}, in this view",
                g.len(),
                if g.len() == 1 { "" } else { "s" },
                p.cfg.sites[i].name
            ),
            Ink::Secondary,
        ));
        lines.push(Text::new());
        for tk in g.tickets() {
            lines.push(Text::from(vec![
                Seg::icon(tk.icon()),
                Seg::styled(format!("{} ", tk.key), p.cfg.style_for("key")),
                Seg::new(tk.summary.clone(), Ink::Primary),
            ]));
        }
    }
    lines
}

fn site_detail(p: &JiraPlugin, i: usize) -> Vec<Block> {
    let s = &p.cfg.sites[i];
    let data = p.data.get(&s.name);
    let lines = vec![
        Text::from(vec![
            Seg::styled("▌ ", format!("jira.{}", s.color)),
            Seg::icon(Icon::Site),
            Seg::styled(s.name.clone(), format!("jira.{} bold", s.color)),
            Seg::new(format!("  {}", s.host()), Ink::Secondary),
        ]),
        Text::new(),
        kit::labelled(
            "signed in",
            12,
            Text::of(
                data.and_then(|d| d.me.as_ref())
                    .map_or_else(|| "not known yet".to_string(), |m| m.name.clone()),
                Ink::Primary,
            ),
        ),
        kit::labelled(
            "watching",
            12,
            Text::of(
                if s.projects.is_empty() {
                    "no projects: settings, projects".to_string()
                } else {
                    s.projects.iter().map(|p| p.key.as_str()).collect::<Vec<_>>().join(", ")
                },
                Ink::Primary,
            ),
        ),
    ];
    let mut blocks: Vec<Block> = lines.into_iter().map(Block::line).collect();
    let tickets = p.site_tickets(i).unwrap_or_default();
    if !tickets.is_empty() {
        let mut counts = Vec::new();
        for c in [Category::Doing, Category::Todo, Category::Done] {
            let n = tickets.iter().filter(|tk| tk.category == c).count();
            if n > 0 {
                let look = p.cfg.style_for(c.word());
                let mut mark = Seg::icon_or(c.icon(), c.symbol(), Ink::Plain);
                mark.look = Some(look.clone());
                counts.push(Text::from(vec![
                    mark,
                    Seg::styled(format!(" {}", ui::pad(c.word(), 6)), look),
                    Seg::new(n.to_string(), Ink::Number),
                ]));
            }
        }
        blocks.push(Block::line(Text::new()));
        blocks.push(Block::section_of(None, "in this view", None, counts));
    }
    if let Some(e) = p.sync.errors.get(&s.name) {
        blocks.extend([
            Block::line(Text::new()),
            Block::line(failed_line(p, " could not load")),
            Block::line(Text::of(e.clone(), Ink::Primary)),
        ]);
    }
    blocks
}

// --- go to ------------------------------------------------------------------------

pub(crate) fn goto_start(p: &mut JiraPlugin) {
    p.goto = Some(Goto {
        input: TextInput::new(""),
        before: p.tree.borrow().selected().to_vec(),
        found: None,
    });
}

/// Go to's line over the tree: what is typed, and where it went.
fn goto_field(_p: &JiraPlugin, g: &Goto) -> v::FieldBar {
    let hint = match (&g.found, g.input.value().trim().is_empty()) {
        (_, true) => Text::of("go to: a ticket's key, or words from its summary", Ink::Muted),
        (Some(f), _) => Text::of(format!("at {f}"), Ink::Secondary),
        (None, _) if crate::config::is_ticket_key(&g.input.value().trim().to_ascii_uppercase()) => {
            Text::of("not in this view", Ink::Secondary)
        }
        (None, _) => Text::of("no ticket here matches", Ink::Warn),
    };
    v::FieldBar {
        mark: " : ".into(),
        text: g.input.value(),
        caret: Some(g.input.cursor()),
        hint,
    }
}

fn score(query: &str, words: &[String]) -> u32 {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return 0;
    }
    words
        .iter()
        .map(|w| {
            let w = w.to_lowercase();
            if w == q {
                100
            } else if w.starts_with(&q) {
                80
            } else if w.split(|c: char| !c.is_alphanumeric()).any(|x| !x.is_empty() && x.starts_with(&q)) {
                60
            } else if w.contains(&q) {
                40
            } else {
                0
            }
        })
        .max()
        .unwrap_or(0)
}

fn goto_jump(p: &JiraPlugin, g: &mut Goto) {
    let targets = targets(p);
    let q = g.input.value();
    let mut best: Option<(usize, u32)> = None;
    for (i, (_, _, words)) in targets.iter().enumerate() {
        let s = score(&q, words);
        if s > 0 && best.is_none_or(|(_, b)| s > b) {
            best = Some((i, s));
        }
    }
    let mut state = p.tree.borrow_mut();
    match best {
        Some((i, _)) => {
            let path = targets[i].0.clone();
            for n in 1..path.len() {
                state.open(path[..n].to_vec());
            }
            state.select(path);
            g.found = Some(targets[i].1.clone());
        }
        None => {
            state.select(g.before.clone());
            g.found = None;
        }
    }
}

pub(crate) fn goto_key(p: &mut JiraPlugin, key: KeyEvent) -> Outcome {
    let Some(mut g) = p.goto.take() else {
        return Outcome::Ignored;
    };
    match g.input.key(key) {
        Edit::Cancel => {
            p.tree.borrow_mut().select(g.before);
            Outcome::done()
        }
        Edit::Submit(text) => {
            let key = text.trim().to_ascii_uppercase();
            // A key that is not in the tree: its page, from the site that
            // watches its project (else the first).
            if g.found.as_deref().is_none_or(|f| !f.starts_with(&format!("{key} ")))
                && crate::config::is_ticket_key(&key)
            {
                p.tree.borrow_mut().select(g.before);
                let project = key.split('-').next().unwrap_or_default();
                let site = p
                    .cfg
                    .sites
                    .iter()
                    .find(|s| s.watches(project))
                    .or_else(|| p.cfg.sites.first())
                    .map(|s| s.name.clone());
                return match site {
                    Some(site) => Outcome::Handled(page::open(p, &site, &key)),
                    None => Outcome::done(),
                };
            }
            Outcome::done()
        }
        Edit::Typing => {
            goto_jump(p, &mut g);
            p.goto = Some(g);
            Outcome::done()
        }
    }
}

pub(crate) fn goto_paste(p: &mut JiraPlugin, text: &str) {
    if let Some(mut g) = p.goto.take() {
        g.input.paste(text);
        goto_jump(p, &mut g);
        p.goto = Some(g);
    }
}

// --- keys and intents ---------------------------------------------------------------

/// The link `o` and `y` act on for what is selected.
fn link(p: &JiraPlugin) -> Option<String> {
    match selected(p)? {
        Selected::Ticket(it) => it.site.browse(&it.t.key),
        Selected::Site(i) => p.cfg.sites[i].base_url().ok(),
        Selected::Group(..) => None,
    }
}

/// Another view: what it shows, fetched if it is not fresh.
fn pick_view(p: &mut JiraPlugin, i: usize) -> Vec<Effect> {
    let i = i.min(p.cfg.views.len().saturating_sub(1));
    if i == p.view {
        return vec![];
    }
    p.view = i;
    p.sync_view(false)
}

fn view_at(target: &v::Target) -> Option<usize> {
    target.as_str().strip_prefix("view:")?.parse().ok()
}

/// What the user did on the screen, from any frontend: a view chosen in
/// the nav pane, a row selected or folded in the tree, `⏎` on one, the
/// assignee heading, or a key.
pub fn intent(p: &mut JiraPlugin, pane: Pane, intent: &Intent) -> Outcome {
    match intent {
        Intent::Select { target } | Intent::Context { target } if view_at(target).is_some() => {
            Outcome::Handled(pick_view(p, view_at(target).unwrap_or(0)))
        }
        // `⏎` on the nav pane is the host's: it goes into the tree.
        Intent::Activate { target } if view_at(target).is_some() => Outcome::Ignored,
        Intent::Select { target } if target.as_str() == "assignees" => {
            Outcome::Handled(vec![Effect::Menu(p.assignee_menu())])
        }
        Intent::Context { target } | Intent::Activate { target } if target.as_str() == "assignees" => Outcome::Ignored,
        Intent::Select { target } | Intent::Context { target } | Intent::Fold { target, .. } => {
            if !accepts_tree_target(p, target, matches!(intent, Intent::Fold { .. })) {
                return if matches!(intent, Intent::Context { .. }) { Outcome::Ignored } else { Outcome::done() };
            }
            p.tree.borrow_mut().apply(intent);
            Outcome::done()
        }
        Intent::Activate { target } => {
            if !accepts_tree_target(p, target, false) {
                return Outcome::done();
            }
            p.tree.borrow_mut().select(TreeRow::path_of(target));
            Outcome::Handled(page::open_from_tree(p))
        }
        // The host turns a chosen cell into a selection and a show.
        Intent::EditSelect { anchor, extent } => match p.compose.as_mut() {
            Some(c) => {
                c.composer.input.select(*anchor, *extent);
                Outcome::done()
            }
            None => Outcome::Ignored,
        },
        Intent::EditUnselect => match p.compose.as_mut() {
            Some(c) => {
                c.composer.input.unselect();
                Outcome::done()
            }
            None => Outcome::Ignored,
        },
        Intent::OpenAll | Intent::Show { .. } | Intent::End { .. } | Intent::Dock(_) | Intent::Copy { .. } | Intent::NoticeInput { .. } | Intent::DismissNotice { .. } => Outcome::Ignored,
        Intent::Key { .. } | Intent::Invoke { .. } => match intent.chord() {
            Some(chord) => key(p, pane, radar_core::view_tui::key_of(chord)),
            None => Outcome::Ignored,
        },
    }
}

/// A key the frontend did not use for moving: the filter's field, `/`,
/// `r`, the links, `⏎` from the detail pane, `esc` clearing the filter.
pub fn key(p: &mut JiraPlugin, pane: Pane, key: KeyEvent) -> Outcome {
    if let Some(mut input) = p.filter_input.take() {
        // ↓ keeps the words and goes down into what they found.
        if key.code == KeyCode::Down {
            p.filter = input.value();
            return Outcome::Handled(to_results(p, false));
        }
        match input.key(key) {
            Edit::Cancel => {
                p.filter.clear();
                if let Some(before) = p.filter_before.take() {
                    p.tree.borrow_mut().select(before);
                }
                open_new_nodes(p, &nodes(p));
            }
            Edit::Submit(text) => {
                p.filter = text;
                return Outcome::Handled(to_results(p, true));
            }
            Edit::Typing => {
                p.filter = input.value();
                p.filter_input = Some(input);
            }
        }
        return Outcome::done();
    }
    match key.code {
        KeyCode::Char('/') => {
            p.filter_before = Some(p.tree.borrow().selected().to_vec());
            p.filter_input = Some(TextInput::new(&p.filter));
            return Outcome::done();
        }
        KeyCode::Char('r') => {
            let fx = p.refresh();
            return Outcome::Handled(if fx.is_empty() {
                vec![Effect::Notify("already asking Jira".into())]
            } else {
                fx
            });
        }
        KeyCode::Char('o') => return Outcome::Handled(link(p).map(Effect::Open).into_iter().collect()),
        KeyCode::Char('y') => {
            return Outcome::Handled(match link(p) {
                Some(url) => vec![Effect::Copy(url), Effect::Notify("link copied".into())],
                None => vec![],
            });
        }
        KeyCode::Char('Y') => {
            return Outcome::Handled(match selected(p) {
                Some(Selected::Ticket(it)) => vec![
                    Effect::Copy(it.t.key.clone()),
                    Effect::Notify(format!("{} copied", it.t.key)),
                ],
                _ => vec![],
            });
        }
        _ => {}
    }
    match (pane, key.code) {
        (Pane::Detail, KeyCode::Enter) if matches!(selected(p), Some(Selected::Ticket(_))) => {
            Outcome::Handled(page::open_from_tree(p))
        }
        (Pane::Middle, KeyCode::Enter) => Outcome::Handled(page::open_from_tree(p)),
        (Pane::Middle, KeyCode::Esc) if !p.filter.is_empty() => {
            p.filter.clear();
            Outcome::done()
        }
        _ => Outcome::Ignored,
    }
}

pub fn hints(p: &JiraPlugin) -> Vec<KeyHint<'static>> {
    if p.goto.is_some() {
        return vec![KeyHint::new("⏎", "stay here"), KeyHint::new("esc", "back")];
    }
    if p.filter_input.is_some() {
        return vec![KeyHint::new("⏎", "keep filter"), KeyHint::new("esc", "clear")];
    }
    if p.edit.is_some() {
        return vec![KeyHint::new("⏎", "keep"), KeyHint::new("esc", "cancel")];
    }
    vec![]
}

/// Bulk commands (`b`): done to everything the view shows.
pub fn bulk(p: &JiraPlugin) -> Vec<MenuItem> {
    if p.cfg.sites.is_empty() {
        return vec![];
    }
    let n = p.visible().len();
    let mut out = vec![
        MenuItem::new('e', "open everything", "every site and status in the tree", "j-open"),
        MenuItem::new('c', "close everything", "down to the sites", "j-close"),
    ];
    if n > 0 {
        let s = if n == 1 { "" } else { "s" };
        out.extend([
            MenuItem::new('o', "open every ticket", format!("{n} in this view, in your browser"), "j-browse"),
            MenuItem::new('Y', "copy every key", format!("{n} key{s}, one per line"), "j-keys"),
            MenuItem::new('y', "copy every link", format!("{n} ticket{s}, one per line"), "j-links"),
        ]);
    }
    out.push(MenuItem::new('r', "refresh everything", "every site, now", "j-refresh"));
    out
}

/// Run the screen's bulk command `value`; `None` when it is not one.
pub fn on_bulk(p: &mut JiraPlugin, value: &str) -> Option<Vec<Effect>> {
    Some(match value {
        "j-open" => {
            let nodes = nodes(p);
            open_new_nodes(p, &nodes);
            let mut state = p.tree.borrow_mut();
            for (path, children, _) in paths(&nodes) {
                if children {
                    state.open(path);
                }
            }
            vec![]
        }
        "j-close" => {
            let mut state = p.tree.borrow_mut();
            let keep = state.selected().first().cloned();
            state.close_all();
            if let Some(t) = keep {
                state.select(vec![t]);
            }
            vec![]
        }
        "j-browse" => p
            .visible()
            .iter()
            .filter_map(|it| it.site.browse(&it.t.key))
            .map(Effect::Open)
            .collect(),
        "j-keys" | "j-links" => {
            let rows: Vec<String> = p
                .visible()
                .iter()
                .filter_map(|it| {
                    if value == "j-keys" {
                        Some(it.t.key.clone())
                    } else {
                        it.site.browse(&it.t.key)
                    }
                })
                .collect();
            let n = rows.len();
            let what = if value == "j-keys" { "keys" } else { "links" };
            vec![Effect::Copy(rows.join("\n")), Effect::Notify(format!("{n} {what} copied"))]
        }
        "j-refresh" => p.refresh(),
        _ => return None,
    })
}

/// The actions menu (`x`) for a pane, named after what is selected.
pub fn keys(p: &JiraPlugin, pane: Pane) -> Vec<Binding> {
    let (sel, has_kids) = match selection(p) {
        Some((selected, children)) => (Some(selected), children),
        None => (None, false),
    };
    let view_rows = |b: &mut Vec<Binding>| {
        b.push(Binding::new("/", "filter the view by words"));
        b.push(Binding::new("r", "refresh now"));
    };
    let links = |b: &mut Vec<Binding>| match &sel {
        Some(Selected::Ticket(it)) => {
            if it.site.browse(&it.t.key).is_some() {
                b.push(Binding::new("o", "open it in Jira"));
                b.push(Binding::new("y", "copy its link"));
            }
            b.push(Binding::new("Y", format!("copy {}", it.t.key)));
        }
        Some(Selected::Site(i)) if p.cfg.sites[*i].base_url().is_ok() => {
            b.push(Binding::new("o", "open the site in Jira"));
            b.push(Binding::new("y", "copy the site's link"));
        }
        _ => {}
    };
    match pane {
        Pane::Nav => {
            let name = p.current_view().map_or("views", |v| v.name.as_str());
            let mut b = vec![Binding::heading(name), Binding::new("⏎", "go to its tickets")];
            view_rows(&mut b);
            b
        }
        Pane::Middle => {
            let mut b = vec![Binding::heading(name_of(p, sel.as_ref())).icon(sel_icon(sel.as_ref()))];
            if matches!(sel, Some(Selected::Ticket(_))) {
                b.push(Binding::new("⏎", "open its page"));
            }
            let state = p.tree.borrow();
            let open = state.is_open(state.selected());
            if has_kids {
                let what = match &sel {
                    Some(Selected::Site(_)) => "its statuses",
                    Some(Selected::Group(..)) => "its tickets",
                    _ => "its children",
                };
                let words = if open { format!("hide {what}") } else { format!("show {what}") };
                // A site or a status has no page: ⏎ folds it, as space does.
                if matches!(sel, Some(Selected::Site(_) | Selected::Group(..))) {
                    b.push(Binding::new("⏎", words.clone()));
                }
                b.push(Binding::new("space", words));
            }
            drop(state);
            links(&mut b);
            view_rows(&mut b);
            b
        }
        Pane::Detail => {
            let mut b = vec![Binding::heading(name_of(p, sel.as_ref())).icon(sel_icon(sel.as_ref()))];
            if matches!(sel, Some(Selected::Ticket(_))) {
                b.push(Binding::new("⏎", "open its page"));
            }
            links(&mut b);
            b.push(Binding::new("pgdn", "scroll down a page"));
            b.push(Binding::new("pgup", "scroll up a page"));
            b.push(Binding::new("esc", "back to the middle pane"));
            b
        }
    }
}

#[cfg(test)]
mod detail_tests;

#[cfg(test)]
mod tree_tests;
