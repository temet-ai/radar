//! How Jira joins the other parts (radar-core's `compose`): it owns tickets,
//! recognises their keys in text, opens a ticket's page when another part
//! asks to show one, and offers places on its tickets for others to fill.
//!
//! Its subject, in the vocabulary the parts share (GUIDE.md):
//! - `ticket`: id the key (`ABC-12`); fields `title` (the summary),
//!   `status`, `type` (the issue type's name), `assignee` (display name,
//!   empty when nobody), `parent` (the parent's key, empty when none),
//!   `project`, and `site` (the Jira site's name, Jira's own). The ticket
//!   fields are set once Jira holds the ticket.
//!
//! It adds to merge requests (`merge-request`), from what it holds, for
//! each ticket the merge request's `title` or `branch` names, or that a
//! ticket's development panel links to it (by its link): a ticket column
//! in the middle pane (a click opens the ticket), a Ticket section on its page and
//! detail pane, a Ticket tab on its page (`⏎` on the row opens the ticket's
//! page), and the actions "open ABC-12" and "move ABC-12 to …".
//! Pull requests use the same cached places with unambiguous ticket scope;
//! their only contributed action opens the ticket.

mod recognition;
#[cfg(test)]
mod pull_request_tests;

use std::ops::Range;

use radar_core::plugin::{Effect, Pane, Tone};
use radar_core::view::Text;
use radar_core::ui::icon::Icon;
use radar_core::{Action, Adds, Cell, Contribution, Offer, Owned, Point, Showing, Subject};

use crate::model::{Category, Ticket};
use crate::screen::{self, Selected};
use crate::{JiraPlugin, page, write};

pub(crate) const TICKET: &str = "ticket";
/// How wide a ticket's description is laid out in another part's section.
const SECTION_WIDTH: u16 = 72;
pub(crate) const MERGE_REQUEST: &str = "merge-request";
const PULL_REQUEST: &str = "pull-request";

pub(crate) const OWNS: [Owned; 1] = [Owned::page(TICKET, "tickets")];

pub(crate) const OFFERS: [Offer; 5] = [
    Offer::new(Point::Chips, TICKET, "ticket rows and the detail pane"),
    Offer::new(Point::Column, TICKET, "the Jira tree"),
    Offer::new(Point::Section, TICKET, "the ticket detail pane and page"),
    Offer::new(Point::Tab, TICKET, "the ticket page"),
    Offer::new(Point::Actions, TICKET, "tickets"),
];

/// A local clone's branch, as the git part draws it (`branch` field).
pub(crate) const CHECKOUT: &str = "checkout";

pub(crate) const ADDS: [Adds; 3] = [
    Adds::new(
        MERGE_REQUEST,
        "a ticket column (a click opens it), a Ticket section and tab (⏎ opens it), and open or move the ticket",
    ),
    Adds::new(PULL_REQUEST, "a Ticket section, tab, and open action on pull requests"),
    Adds::new(CHECKOUT, "a ticket column on its branches (a click opens it)"),
];

/// The site a ticket's key belongs to: the one watching its project, else
/// the first holding it, else the first.
pub(crate) fn site_for(p: &JiraPlugin, key: &str) -> Option<String> {
    let project = key.split('-').next().unwrap_or_default();
    p.cfg
        .sites
        .iter()
        .find(|s| s.watches(project))
        .or_else(|| {
            p.cfg
                .sites
                .iter()
                .find(|s| p.data.get(&s.name).is_some_and(|d| d.tickets.contains_key(key)))
        })
        .or_else(|| p.cfg.sites.first())
        .map(|s| s.name.clone())
}

/// A ticket held, on a site: the page's fresh copy, else the list's.
pub(crate) fn held<'a>(p: &'a JiraPlugin, site: &str, key: &str) -> Option<&'a Ticket> {
    p.details
        .get(&crate::detail_key(site, key))
        .and_then(|f| f.value.as_ref())
        .map(|d| &d.ticket)
        .or_else(|| p.data.get(site).and_then(|d| d.tickets.get(key)))
}

/// The key a ticket's row goes by for motion: its subject's.
pub(crate) fn motion_key(key: &str) -> String {
    radar_core::motion::key(TICKET, key)
}

/// The chips other parts put on a ticket, each after a space, to follow
/// its name.
pub(crate) fn chip_text(parts: &radar_core::Parts<'_>, subject: &Subject) -> radar_core::view::Text {
    use radar_core::view::{Ink, Seg, Text};
    let mut text = Text::new();
    for c in parts.chips(subject) {
        text.add(Seg::new(" ", Ink::Plain));
        text.add(Seg::chip(&c));
    }
    text
}

/// A ticket as a subject, with what others match on.
pub(crate) fn ticket(p: &JiraPlugin, site: &str, key: &str) -> Subject {
    let mut s = cached_ticket(p, site, key);
    if let Some(url) = p.cfg.site(site).and_then(|s| s.browse(key)) {
        s = s.with_url(url);
    }
    s
}

/// Explicit local routing identity and held metadata, without resolving a site
/// URL from the environment. Show accepts this site-qualified subject directly.
pub(crate) fn cached_ticket(p: &JiraPlugin, site: &str, key: &str) -> Subject {
    let mut s = Subject::new(TICKET, key, key)
        .with("project", key.split('-').next().unwrap_or_default())
        .with("site", site);
    if let Some(t) = held(p, site, key) {
        s = s
            .with("title", t.summary.clone())
            .with("status", t.status.clone())
            .with("type", t.kind.clone())
            .with("assignee", t.assignee.clone())
            .with("parent", t.parent.as_ref().map(|p| p.key.clone()).unwrap_or_default());
    }
    s
}

/// What the focused pane is about: the page's ticket, or the tree's.
pub(crate) fn subject(p: &JiraPlugin, pane: Pane) -> Option<Subject> {
    if let Some(page) = p.pages.last() {
        return Some(ticket(p, &page.site, &page.key));
    }
    if pane == Pane::Nav {
        return None;
    }
    match screen::selected(p)? {
        Selected::Ticket(it) => Some(ticket(p, &it.site.name, &it.t.key)),
        _ => None,
    }
}

/// Projects whose keys count as tickets here: watched, or held.
fn known_project(p: &JiraPlugin, project: &str) -> bool {
    p.cfg.sites.iter().any(|s| s.watches(project))
        || p.data.values().any(|d| d.tickets.values().any(|t| t.project == project))
}

/// Ticket keys in `text`, as whole words: `ABC-12`, not `XABC-12` or
/// `ABC-123x`. With `any_case`, `abc-12` too (branch names).
pub(crate) fn keys_in(text: &str, any_case: bool) -> Vec<(Range<usize>, String)> {
    let b = text.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let starts = (i == 0 || !word(b[i - 1])) && b[i].is_ascii_alphabetic();
        if !starts {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
            j += 1;
        }
        let project = &text[i..j];
        if j < b.len() && b[j] == b'-' {
            let mut k = j + 1;
            while k < b.len() && b[k].is_ascii_digit() {
                k += 1;
            }
            let ends = k == b.len() || !word(b[k]);
            let upper = project.to_ascii_uppercase();
            let key = format!("{upper}-{}", &text[j + 1..k]);
            if k > j + 1 && ends && (any_case || project == upper) && crate::config::is_ticket_key(&key) {
                out.push((i..k, key));
                i = k;
                continue;
            }
        }
        i = j.max(i + 1);
    }
    out
}

/// Configured browse URLs and bare keys of projects known here, in byte order.
pub(crate) fn recognise(p: &JiraPlugin, text: &str) -> Vec<(Range<usize>, Subject)> {
    recognition::recognise(p, text)
}

/// Another part asked to show a ticket: its page, over whatever was open.
pub(crate) fn show(p: &mut JiraPlugin, subject: &Subject) -> Showing {
    if !subject.is(TICKET) || !crate::config::is_ticket_key(&subject.id) {
        return Showing::Cannot;
    }
    let Some(site) = crate::subject_scope::resolve(p, subject) else { return Showing::Cannot };
    let site = site.name.clone();
    let key = subject.id.clone();
    Showing::Page {
        screen: crate::SCREEN.id,
        effects: page::open(p, &site, &key),
    }
}

/// The tickets a merge request names (title, branch) or is linked from
/// (a development panel), on the site each is on, once each.
fn tickets_of(p: &JiraPlugin, s: &Subject) -> Vec<(String, String)> {
    if s.is(PULL_REQUEST) {
        return pull_tickets(p, s);
    }
    let mut keys: Vec<String> = Vec::new();
    for (field, any_case) in [("title", false), ("branch", true)] {
        for (_, k) in keys_in(s.field(field).unwrap_or_default(), any_case) {
            if known_project(p, k.split('-').next().unwrap_or_default()) && !keys.contains(&k) {
                keys.push(k);
            }
        }
    }
    let mut out: Vec<(String, String)> = keys
        .into_iter()
        .filter_map(|k| site_for(p, &k).map(|site| (site, k)))
        .collect();
    // Jira's own record: a ticket page's development panel links to it.
    if let Some(url) = s.url.as_deref().map(|u| u.trim_end_matches('/')) {
        let links = |c: &[crate::model::CodeLink]| c.iter().any(|c| c.url.trim_end_matches('/') == url);
        // Read in the background for the tickets in view (and by a page).
        for (site, data) in &p.data {
            for (key, e) in &data.code {
                if links(&e.links) && !out.iter().any(|(_, k)| k == key) {
                    out.push((site.clone(), key.clone()));
                }
            }
        }
        for (id, f) in &p.details {
            let Some(d) = f.value.as_ref() else { continue };
            if links(&d.code)
                && let Some((site, key)) = id.split_once('\0')
                && !out.iter().any(|(_, k)| k == key)
            {
                out.push((site.to_string(), key.to_string()));
            }
        }
    }
    out
}

/// New read-only references do not inherit bare-key first-site precedence.
/// Keep site identity until all cached evidence is gathered; ambiguous keys
/// contribute nothing. Development links match their complete URL exactly.
fn pull_tickets(p: &JiraPlugin, s: &Subject) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    let mut add = |site: &str, key: &str| {
        let pair = (site.to_string(), key.to_string());
        if p.cfg.site(site).is_some() && !found.contains(&pair) {
            found.push(pair);
        }
    };
    for (field, any_case) in [("title", false), ("branch", true)] {
        for (_, key) in keys_in(s.field(field).unwrap_or_default(), any_case) {
            let project = key.split('-').next().unwrap_or_default();
            for site in &p.cfg.sites {
                if site.watches(project) || held(p, &site.name, &key).is_some() {
                    add(&site.name, &key);
                }
            }
        }
    }
    if let Some(url) = s.url.as_deref().filter(|url| !url.is_empty()) {
        for (site, data) in &p.data {
            for (key, entry) in &data.code {
                if entry.links.iter().any(|link| link.url == url) {
                    add(site, key);
                }
            }
        }
        for (id, detail) in &p.details {
            if let Some(detail) = &detail.value
                && detail.code.iter().any(|link| link.url == url)
                && let Some((site, key)) = id.split_once('\0')
            {
                add(site, key);
            }
        }
    }
    found
        .iter()
        .filter(|(site, key)| {
            let project = key.split('-').next().unwrap_or_default();
            found.iter().filter(|(_, other)| other == key).count() == 1
                && p.cfg.sites.iter().filter(|s| &s.name == site).count() == 1
                && p.cfg.sites.iter()
                    .filter(|s| &s.name == site || s.watches(project) || held(p, &s.name, key).is_some())
                    .count() == 1
                && ticket(p, site, key).url.is_some()
        })
        .cloned()
        .collect()
}

/// Binding the site and URL prevents an old menu row from choosing a new
/// site's same-key ticket after settings or cached relationships change.
fn pull_open_action(subject: &Subject) -> String {
    format!(
        "open-pull-ticket\0{}\0{}\0{}",
        subject.field("site").unwrap_or_default(),
        subject.id,
        subject.url.as_deref().unwrap_or_default(),
    )
}

/// Share the same ticket relationships used by the request's column and tabs.
pub(crate) fn related(p: &JiraPlugin, s: &Subject) -> Vec<Subject> {
    if !(s.is(MERGE_REQUEST) || s.is(PULL_REQUEST)) {
        return vec![];
    }
    tickets_of(p, s).iter().map(|(site, key)| ticket(p, site, key)).collect()
}

fn tone(c: Category) -> Tone {
    match c {
        Category::Todo => Tone::Plain,
        Category::Doing => Tone::Info,
        Category::Done => Tone::Good,
    }
}

/// What Jira adds to a merge request, and the ticket column on a clone's
/// branch, from what it holds.
pub(crate) fn contribute(p: &JiraPlugin, point: Point, s: &Subject) -> Option<Contribution> {
    if !(s.is(MERGE_REQUEST) || s.is(PULL_REQUEST))
        && !(s.is(CHECKOUT) && point == Point::Column)
    {
        return None;
    }
    let found = tickets_of(p, s);
    if found.is_empty() {
        return None;
    }
    Some(match point {
        // A column, not chips after the title: its first ticket, which a
        // click opens, and how many more.
        Point::Column => {
            let (site, key) = &found[0];
            let more = match found.len() {
                1 => String::new(),
                n => format!(" +{}", n - 1),
            };
            let (text, feel, icon) = match held(p, site, key) {
                Some(t) => (format!("{key} · {}{more}", t.status), tone(t.category), Some(t.icon())),
                None => (format!("{key}{more}"), Tone::Muted, None),
            };
            Contribution::Cell(Cell::new(TICKET, text).tone(feel).icon(icon).show(ticket(p, site, key)))
        }
        Point::Section => {
            let mut lines = Vec::new();
            for (site, key) in &found {
                if !lines.is_empty() {
                    lines.push(Text::new());
                }
                match held(p, site, key) {
                    Some(t) => {
                        lines.push(Text::plain(format!("{key} · {}{} · {}", crate::model::mark(t.category), t.status, t.kind)));
                        lines.push(Text::plain(t.summary.clone()));
                        lines.push(Text::plain(if t.assignee.is_empty() {
                            "unassigned".to_string()
                        } else if p.is_mine(site, t) {
                            radar_core::ui::icon::plain_with(radar_core::ui::icon::Icon::You, "assigned to you")
                        } else {
                            format!("assigned to {}", t.assignee)
                        }));
                        let said = radar_core::ui::rich::plain(&t.description, SECTION_WIDTH);
                        let said: Vec<&str> = said.lines().collect();
                        for l in said.iter().take(3) {
                            lines.push(Text::plain(l.to_string()));
                        }
                        if said.len() > 3 {
                            lines.push(Text::plain("…"));
                        }
                    }
                    None => lines.push(Text::plain(format!("{key} · not fetched yet: its page has it"))),
                }
            }
            Contribution::Section {
                title: if found.len() == 1 { "Ticket".into() } else { "Tickets".into() },
                lines,
            }
        }
        Point::Actions => {
            let mut actions = Vec::new();
            for (n, (site, key)) in found.iter().enumerate() {
                if s.is(PULL_REQUEST) {
                    let subject = ticket(p, site, key);
                    let mut open = Action::new('v', format!("open {key}"), pull_open_action(&subject))
                        .icon(Icon::Issue);
                    if n > 0 {
                        open.key = None;
                    }
                    actions.push(open);
                    continue;
                }
                let mut open = Action::new('v', format!("open {key}"), format!("open\0{key}")).icon(Icon::Issue);
                let mut mv = Action::new('>', format!("move {key} to …"), format!("move\0{key}")).icon(Icon::Navigate).edits();
                if n > 0 {
                    open.key = None;
                    mv.key = None;
                }
                actions.extend([open, mv]);
            }
            Contribution::Actions(actions)
        }
        // A tab of the tickets, one row each: ⏎ opens the ticket's page.
        Point::Tab => {
            let rows = found
                .iter()
                .map(|(site, key)| radar_core::Row {
                    line: Text::plain(match held(p, site, key) {
                        Some(t) => format!(
                            "{key} · {}{} · {} · {}",
                            crate::model::mark(t.category),
                            t.status,
                            t.kind,
                            t.summary
                        ),
                        None => format!("{key} · not fetched yet: its page has it"),
                    }),
                    subject: Some(ticket(p, site, key)),
                })
                .collect();
            Contribution::Tab {
                title: if found.len() == 1 { "Ticket".into() } else { "Tickets".into() },
                rows,
            }
        }
        Point::Chips | Point::Line | Point::Mark => return None,
    })
}

/// One of the actions Jira added to a merge request was chosen.
pub(crate) fn act(p: &mut JiraPlugin, s: &Subject, action: &str) -> Vec<Effect> {
    if s.is(PULL_REQUEST) {
        return pull_tickets(p, s)
            .into_iter()
            .map(|(site, key)| ticket(p, &site, &key))
            .find(|subject| pull_open_action(subject) == action)
            .map_or_else(Vec::new, |subject| vec![Effect::Show(subject)]);
    }
    let Some((what, key)) = action.split_once('\0') else { return vec![] };
    let Some(site) = tickets_of(p, s).into_iter().find(|(_, k)| k == key).map(|(site, _)| site) else {
        return vec![];
    };
    match what {
        "open" => vec![Effect::Show(ticket(p, &site, key))],
        "move" => write::start_move(p, &site, key),
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_whole_words() {
        let got: Vec<String> = keys_in("ABC-12: fix (DEF-3), not aABC-1, ABC-123x or UTF-8x; ABC-0", false)
            .into_iter()
            .map(|(_, k)| k)
            .collect();
        assert_eq!(got, ["ABC-12", "DEF-3"]);
        let got: Vec<String> = keys_in("feature/abc-12-add-stats", true).into_iter().map(|(_, k)| k).collect();
        assert_eq!(got, ["ABC-12"]);
        assert!(keys_in("feature/abc-12-add-stats", false).is_empty(), "lower case only in branches");
        let (r, _) = &keys_in("see ABC-12.", false)[0];
        assert_eq!(&"see ABC-12."[r.clone()], "ABC-12");
    }

    #[test]
    fn only_keys_of_projects_known_here_are_tickets() {
        let p = crate::tests::sample();
        let found: Vec<String> = recognise(&p, "ABC-12 and XYZ-7 and UTF-8 and NOPE-1")
            .into_iter()
            .map(|(_, s)| format!("{} {}", s.id, s.field("site").unwrap_or("")))
            .collect();
        assert_eq!(found, ["ABC-12 acme", "XYZ-7 beta"]);
    }
}
