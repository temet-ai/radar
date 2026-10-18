//! The ticket page: `⏎` on a ticket opens it over the whole screen body,
//! with a breadcrumb (`Jira › acme › ABC › ABC-12`), the summary, and three
//! tabs: Overview (description, acceptance criteria and the details),
//! Comments (newest first, older ones paged in as you scroll) and Links
//! (parent, children, issue links, web links; `⏎` opens a linked ticket's
//! page). After them, tabs other parts add, and Code: the merge requests
//! Jira's development panel links to it, joined with another part's Code
//! tab when one adds it (`⏎` shows a row's merge request). Pages stack;
//! `esc` goes back one, and from the last to the tree. Described as view
//! data ([`view`]) and driven by intents ([`intent`]).

use std::cell::{Cell, RefCell};

use radar_core::plugin::{Binding, Effect, Msg, Outcome, Pane};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent};
use radar_core::ui::icon::Icon;
use radar_core::ui;
use radar_core::view::{self as v, Block, ColumnSpec, Ink, Intent, Node, PageView, PaneId, PaneView, Seg, TabView, Text, kit};
use radar_core::{Parts, Row, Subject, Theme, log};
use serde_json::json;

use radar_core::ui::rich::{self, Doc};

use crate::client::{COMMENTS_PAGE, DetailRead};
use crate::config::Site;
use crate::model::{Detail, Target, Ticket};
use crate::screen::{self, Selected, ago, status_text};
use crate::{Fetched, JiraPlugin, detail_key};

pub(crate) const TABS: [&str; 3] = ["Overview", "Comments", "Links"];
const OVERVIEW: usize = 0;
const COMMENTS: usize = 1;
const LINKS: usize = 2;
/// The tab of merge requests, Jira's and another part's together.
const CODE: &str = "Code";

/// One dispatched page read, owned by a configured-connection incarnation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Read {
    pub id: u64,
    pub generation: u64,
}

/// An older-comment read also owns the installed full-detail revision and prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct More {
    pub read: Read,
    pub revision: u64,
    pub start: usize,
}

/// Called on every configured connection change, including removal and rename.
/// General SiteData and contributions reconstructed from it remain independent.
pub(crate) fn invalidate(p: &mut JiraPlugin, changed: &[String]) {
    p.details.retain(|id, f| {
        id.split_once('\0')
            .is_some_and(|(site, _)| p.connections.generation(site) == Some(f.generation))
    });
    for page in &mut p.pages {
        if changed.contains(&page.site) {
            page.extra.get_mut().clear();
            if page.tab >= TABS.len() {
                page.tab = TABS.len() - 1;
                page.row.set(0);
            }
        }
    }
}

fn next_read(p: &mut JiraPlugin, generation: u64) -> Read {
    p.next_page_read = p
        .next_page_read
        .checked_add(1)
        .expect("Jira page read sequence exhausted");
    Read {
        id: p.next_page_read,
        generation,
    }
}

/// A page on the stack.
#[derive(Debug)]
pub(crate) struct Page {
    pub site: String,
    pub key: String,
    pub tab: usize,
    /// A list tab's cursor.
    pub row: Cell<usize>,
    /// The tabs after the page's own (Code, other parts'), as last shown.
    pub extra: RefCell<Vec<(String, Vec<Row>)>>,
}

impl Page {
    /// The requested ticket, independent of loaded summary or selected links.
    pub(crate) fn identity(&self) -> String {
        serde_json::json!(["ticket", self.site, self.key]).to_string()
    }

    fn new(site: &str, key: &str) -> Self {
        Self {
            site: site.into(),
            key: key.into(),
            tab: 0,
            row: Cell::new(0),
            extra: RefCell::new(Vec::new()),
        }
    }

    /// How many tabs, its own and the others.
    fn tab_count(&self) -> usize {
        TABS.len() + self.extra.borrow().len()
    }

    /// The rows of the added tab showing, if one is.
    fn added(&self) -> Option<std::cell::Ref<'_, (String, Vec<Row>)>> {
        let i = self.tab.checked_sub(TABS.len())?;
        std::cell::Ref::filter_map(self.extra.borrow(), |e| e.get(i)).ok()
    }
}

// --- opening and fetching ----------------------------------------------------------

/// `⏎` in the tree: a ticket's page; on a site or status, fold it.
pub(crate) fn open_from_tree(p: &mut JiraPlugin) -> Vec<Effect> {
    match screen::selected(p) {
        Some(Selected::Ticket(it)) => {
            let (site, key) = (it.site.name.clone(), it.t.key.clone());
            p.pages.clear();
            let mut fx = vec![Effect::Focus(Pane::Middle)];
            fx.extend(open(p, &site, &key));
            fx
        }
        Some(_) => {
            p.tree.borrow_mut().toggle_selected();
            vec![]
        }
        None => vec![],
    }
}

/// Push a ticket's page and fetch what it shows.
pub(crate) fn open(p: &mut JiraPlugin, site: &str, key: &str) -> Vec<Effect> {
    p.pages.push(Page::new(site, key));
    let mut fx = vec![
        crate::page_history::opened(p.cfg.site(site), key),
        Effect::Focus(Pane::Middle),
    ];
    fx.extend(fetch(p, site, key, false));
    fx
}

pub(crate) fn fetch(p: &mut JiraPlugin, site_name: &str, key: &str, force: bool) -> Vec<Effect> {
    crate::write_context::reconcile(p);
    let Some(site) = p.cfg.site(site_name).cloned() else {
        return vec![];
    };
    let (now, every) = (p.now, p.cfg.refresh_secs as i64);
    let id = detail_key(site_name, key);
    let generation = p
        .connections
        .generation(site_name)
        .expect("configured site reconciled");
    // A page kept in the cache is drawn at once; what follows decides
    // whether it is still good.
    if !p.details.contains_key(&id)
        && let Some(kept) = p
            .data
            .get(site_name)
            .and_then(|d| d.details.get(key))
            .filter(|kept| kept.url == site.url)
    {
        let f = Fetched {
            value: Some(kept.detail.clone()),
            at: kept.at.min(now),
            generation,
            ..Default::default()
        };
        p.details.insert(id.clone(), f);
    }
    enum Next {
        Skip,
        Check,
        Read,
    }
    let next = match p.details.get(&id) {
        Some(f) if f.loading || (f.checking && !force) => Next::Skip,
        Some(f) if !force && f.value.is_some() => {
            let held = f.value.as_ref().expect("checked");
            // While the list keeps the ticket's `updated` current, the page
            // is read again only when that moved. Otherwise (a failed read
            // to try again, a ticket outside the view) on the part's cycle:
            // a failed read again, a ticket outside the view asked for its
            // `updated` alone first.
            match (p.tracked(site_name, key), f.error.is_some()) {
                (Some(t), false) if t.updated == held.ticket.updated => Next::Skip,
                (Some(_), false) => Next::Read,
                (_, true) if now - f.at < every => Next::Skip,
                (_, true) => Next::Read,
                (None, false) if now - f.at < every => Next::Skip,
                (None, false) => Next::Check,
            }
        }
        _ => Next::Read,
    };
    match next {
        Next::Skip => return vec![],
        Next::Check => return check(p, site, key),
        Next::Read => {}
    }
    let read = next_read(p, generation);
    let f = p.details.entry(id).or_insert_with(|| Fetched {
        generation,
        ..Default::default()
    });
    // A new full read retires any More owner even if the full read later fails.
    f.more = None;
    f.request = Some(read);
    f.loading = true;
    let key = key.to_string();
    let project = key.split('-').next().unwrap_or_default().to_string();
    let limits = p.limits.clone();
    let action = crate::page_history::action(&site, &key);
    let provider = p.provider.clone();
    vec![crate::page_history::spawn(
        action,
        site.name.clone(),
        key.clone(),
        read,
        async move {
            let result = async {
                let c = provider.tracked(&site, &limits)?;
                c.detail_observed(&key, site.ac_field(&project).map(str::to_string))
                    .await
            }
            .await
            .map_err(|e| format!("{e:#}"));
            if let Err(e) = &result {
                log::warn(format!("jira {} {key}: {e}", site.name));
            }
            result
        },
    )]
}

/// Ask for a held page's ticket `updated` alone (a ticket no view lists):
/// the answer comes back as [`JiraMsg::Stamp`].
fn check(p: &mut JiraPlugin, site: Site, key: &str) -> Vec<Effect> {
    let Some(f) = p.details.get_mut(&detail_key(&site.name, key)) else {
        return vec![];
    };
    f.checking = true;
    let (limits, key) = (p.limits.clone(), key.to_string());
    let provider = p.provider.clone();
    vec![Effect::Spawn(Box::pin(async move {
        let result = async { provider.tracked(&site, &limits)?.updated_of(&key).await }
            .await
            .map_err(|e| format!("{e:#}"));
        Box::new(crate::JiraMsg::Stamp { site: site.name, key, result }) as Msg
    }))]
}

/// The answer to [`check`]: the page is as good as read for this cycle; if
/// the ticket moved it is read now, what is drawn staying up meanwhile. A
/// failed check is no reason to doubt the page: the next cycle asks again.
pub(crate) fn on_stamp(p: &mut JiraPlugin, site: &str, key: &str, result: Result<String, String>) -> Vec<Effect> {
    let now = p.now;
    let Some(f) = p.details.get_mut(&detail_key(site, key)).filter(|f| f.checking) else {
        return vec![];
    };
    f.checking = false;
    f.at = now;
    match result {
        Ok(updated) if f.value.as_ref().is_some_and(|d| d.ticket.updated != updated) => fetch(p, site, key, true),
        Ok(_) => vec![],
        Err(e) => {
            log::warn(format!("jira {site} {key}: checking whether the page moved: {e}"));
            vec![]
        }
    }
}

/// Keep a read page in `data` against its ticket's `updated`: the first page
/// of comments only, the latest [`crate::MAX_STORED_PAGES`] pages. Whether
/// anything worth a cache write changed.
fn keep(data: &mut crate::SiteData, url: &str, key: &str, detail: &Detail, now: i64) -> bool {
    let mut stored = detail.clone();
    stored.comments.truncate(COMMENTS_PAGE);
    if let Some(held) = data.details.get_mut(key)
        && held.detail == stored
        && held.url == url
    {
        // As it was: only the time it was read moves, in memory.
        held.at = now;
        return false;
    }
    data.details.insert(
        key.to_string(),
        crate::DetailEntry { url: url.to_string(), at: now, updated: stored.ticket.updated.clone(), detail: stored },
    );
    while data.details.len() > crate::MAX_STORED_PAGES {
        let Some(oldest) = data.details.iter().min_by_key(|(_, e)| e.at).map(|(k, _)| k.clone()) else {
            break;
        };
        data.details.remove(&oldest);
    }
    true
}

/// Older comments, when the Comments tab reaches its end and there are more.
pub(crate) fn fetch_more(p: &mut JiraPlugin, site_name: &str, key: &str) -> Vec<Effect> {
    crate::write_context::reconcile(p);
    let id = detail_key(site_name, key);
    let Some(f) = p.details.get(&id) else {
        return vec![];
    };
    let Some(d) = f.value.as_ref() else {
        return vec![];
    };
    if f.loading || f.more.is_some() || d.comments.len() >= d.comments_total {
        return vec![];
    }
    let Some(site) = p.cfg.site(site_name).cloned() else {
        return vec![];
    };
    let (start, generation, revision) = (d.comments.len(), f.generation, f.revision);
    let read = More {
        read: next_read(p, generation),
        revision,
        start,
    };
    p.details.get_mut(&id).expect("held detail").more = Some(read);
    let key = key.to_string();
    let action = crate::comments_history::action(&site, &key, start);
    let provider = p.provider.clone();
    vec![crate::comments_history::spawn(
        action,
        site.name.clone(),
        key.clone(),
        read,
        async move {
            async {
                provider.client(&site)?
                    .comments_observed(&key, start, COMMENTS_PAGE)
                    .await
            }
            .await
            .map_err(|e| format!("{e:#}"))
        },
    )]
}

pub(crate) fn on_detail(
    p: &mut JiraPlugin,
    site: &str,
    key: &str,
    read: Read,
    result: Result<DetailRead, String>,
) -> bool {
    crate::write_context::reconcile(p);
    if p.connections.generation(site) != Some(read.generation) {
        return false;
    }
    let Some(f) = p.details.get_mut(&detail_key(site, key)) else {
        return false;
    };
    if f.generation != read.generation || f.request != Some(read) {
        return false;
    }
    f.request = None;
    f.loading = false;
    match result {
        Ok(got) => {
            let d = got.detail;
            // The page's fresh copy replaces the list's.
            if let Some(held) = p
                .data
                .get_mut(site)
                .and_then(|data| data.tickets.get_mut(key))
            {
                *held = d.ticket.clone();
            }
            // An optional code-read failure leaves the prior cache/freshness
            // intact; other existing replacement policies remain unchanged.
            if !got.evidence.code_failed
                && let Some(data) = p.data.get_mut(site)
                && data.tickets.contains_key(key)
            {
                data.code.insert(
                    key.to_string(),
                    crate::CodeEntry {
                        at: p.now,
                        updated: d.ticket.updated.clone(),
                        links: d.code.clone(),
                    },
                );
            }
            if let (Some(data), Some(configured)) = (p.data.get_mut(site), p.cfg.site(site)) {
                p.sync.unsaved |= keep(data, &configured.url, key, &d, p.now);
            }
            f.revision = read.id;
            f.value = Some(d);
            f.error = None;
            f.at = p.now;
        }
        Err(e) => f.error = Some(e),
    }
    true
}

pub(crate) fn on_more(
    p: &mut JiraPlugin,
    site: &str,
    key: &str,
    read: More,
    result: Result<(Vec<crate::model::Comment>, usize), String>,
) -> bool {
    crate::write_context::reconcile(p);
    if p.connections.generation(site) != Some(read.read.generation) {
        return false;
    }
    let id = detail_key(site, key);
    let Some(f) = p.details.get_mut(&id) else {
        return false;
    };
    if f.generation != read.read.generation || f.more != Some(read) {
        return false;
    }
    // Retire only this exact owner, even if its local base unexpectedly drifted.
    // It cannot append or change the error, and a new paging attempt can proceed.
    f.more = None;
    if f.loading
        || f.revision != read.revision
        || f.value
            .as_ref()
            .is_none_or(|d| d.comments.len() != read.start)
    {
        return false;
    }
    match (result, f.value.as_mut()) {
        (Ok((comments, total)), Some(d)) => {
            d.comments.extend(comments);
            d.comments_total = total;
        }
        (Err(e), _) => f.error = Some(e),
        _ => {}
    }
    true
}

pub(crate) fn loading(p: &JiraPlugin) -> bool {
    p.details.values().any(|f| f.loading || f.more.is_some())
}

// --- what a page is about -------------------------------------------------------------

fn fetched<'a>(p: &'a JiraPlugin, page: &Page) -> Option<&'a Fetched<Detail>> {
    p.details.get(&detail_key(&page.site, &page.key))
}

/// The ticket: the page's own copy once loaded, else the list's.
fn ticket<'a>(p: &'a JiraPlugin, page: &Page) -> Option<&'a Ticket> {
    fetched(p, page)
        .and_then(|f| f.value.as_ref())
        .map(|d| &d.ticket)
        .or_else(|| p.data.get(&page.site).and_then(|d| d.tickets.get(&page.key)))
}

fn detail<'a>(p: &'a JiraPlugin, page: &Page) -> Option<&'a Detail> {
    fetched(p, page).and_then(|f| f.value.as_ref())
}

/// The tabs after the page's own: Code (the merge requests Jira's
/// development panel links, after those of another part's Code tab, each
/// once), then the other parts' tabs.
struct AddedTabs {
    tabs: Vec<(String, Vec<Row>)>,
    /// Render-local attribution: the exact tab to which Jira appended rows.
    jira_code: Option<usize>,
}

fn added_tabs(p: &JiraPlugin, page: &Page, parts: &Parts<'_>) -> AddedTabs {
    let mut tabs = parts.tabs(&crate::join::ticket(p, &page.site, &page.key));
    let mut jira_code = None;
    let code = detail(p, page).map(|d| d.code.as_slice()).unwrap_or_default();
    if code.is_empty() {
        return AddedTabs { tabs, jira_code };
    }
    let at = match tabs.iter().position(|(title, _)| title.eq_ignore_ascii_case(CODE)) {
        Some(i) => i,
        None => {
            tabs.insert(0, (CODE.to_string(), vec![]));
            0
        }
    };
    let shown: Vec<String> = tabs[at]
        .1
        .iter()
        .filter_map(|r| r.subject.as_ref()?.url.as_deref().map(|u| u.trim_end_matches('/').to_string()))
        .collect();
    for c in code {
        if shown.iter().any(|u| *u == c.url.trim_end_matches('/')) {
            continue;
        }
        let status = match c.status.as_str() {
            "merged" => Ink::Good,
            "declined" => Ink::Muted,
            _ => Ink::Info,
        };
        // In the columns another part's Code rows use: name, state, then
        // the title.
        let name = format!("{} {}", c.repo, c.id);
        let mark = match c.status.as_str() {
            "merged" => Icon::Merged,
            "declined" => Icon::Closed,
            _ => Icon::Change,
        };
        let line = Text::from(vec![
            Seg::icon(mark),
            Seg::new(format!("{} ", ui::pad(&name, 16)), Ink::AccentSecondary),
            Seg::new(format!("{} ", ui::pad(&c.status, 8)), status),
            Seg::new(c.title.clone(), Ink::Primary),
            Seg::new(format!("  {} · from Jira", c.branch), Ink::Muted),
        ]);
        tabs[at].1.push(Row {
            line,
            subject: Some(Subject::link(c.url.clone())),
        });
        jira_code = Some(at);
    }
    AddedTabs { tabs, jira_code }
}

// --- text shared with the detail pane ---------------------------------------------------

/// A ticket's labels, each in the `label` look.
pub(crate) fn labels_text(p: &JiraPlugin, labels: &[String]) -> Text {
    let look = p.cfg.style_for("label");
    let mut text = Text::new();
    for (i, l) in labels.iter().enumerate() {
        if i > 0 {
            text.add(Seg::new(" · ", Ink::Muted));
        }
        text.add(Seg::styled(format!("#{l}"), look.clone()));
    }
    text
}

/// A ticket's rich text (from ADF) laid out by the kit for `width`, links
/// followed by where they go, its pictures named.
#[cfg(test)]
pub(crate) fn rich(t: &Theme, doc: &Doc, width: u16) -> Vec<radar_core::ratatui::text::Line<'static>> {
    let mut doc = doc.clone();
    for b in &mut doc.blocks {
        if let rich::Block::Media(m) = b {
            m.src.clear();
        }
    }
    rich::layout(&doc, &look(t), width).lines
}

/// How the kit fetches a ticket's picture: with the site's account and
/// token, sent only to the site itself.
pub(crate) fn pictures(p: &JiraPlugin, site: &str) -> ui::image::Fetch {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(fixture) = &p.provider.fixture {
        let fixture = fixture.clone();
        return ui::image::fetch(move |_src: String| {
            let fixture = fixture.clone();
            async move {
                fixture.blocked("pictures");
                anyhow::bail!("Jira hosted fixture does not support pictures")
            }
        });
    }
    let creds = p.cfg.site(site).and_then(|s| {
        Some((
            s.base_url().ok()?,
            crate::config::resolve(&s.email, "email").ok()?,
            crate::config::resolve(&s.token, "API token").ok()?,
        ))
    });
    ui::image::fetch(move |src: String| {
        let creds = creds.clone();
        async move { crate::client::picture(&src, creds).await }
    })
}

/// The kit's look for rich text, with a link's address after its words
/// unless the terminal opens the words themselves.
fn look(t: &Theme) -> rich::Look {
    let mut look = rich::Look::new(t);
    look.urls = !ui::link::on();
    look
}

/// A document's words, links with their addresses, laid out for `width`:
/// for a menu's notes, and (wide) for the command line.
pub(crate) fn text_at(doc: &Doc, width: u16) -> String {
    let mono = Theme::new(radar_core::design::theme::Tier::Mono);
    rich::text_of(&rich::layout(doc, &look(&mono), width).lines)
}

fn text(doc: &Doc) -> String {
    text_at(doc, 1000)
}

/// `Jira › acme › ABC › ABC-12`
fn crumbs(p: &JiraPlugin, page: &Page) -> Vec<Seg> {
    let colour = p
        .cfg
        .site(&page.site)
        .map_or_else(|| "blue".to_string(), |s| s.color.clone());
    let project = page.key.split('-').next().unwrap_or_default().to_string();
    vec![
        Seg::new("Jira", Ink::Secondary),
        Seg::styled(page.site.clone(), format!("jira.{colour} {}", p.cfg.style_for("site"))),
        Seg::new(project, Ink::Secondary),
        Seg::styled(page.key.clone(), p.cfg.style_for("key")),
    ]
}

/// What went wrong fetching the page, or `loading…` while there is nothing.
fn fetch_state(f: Option<&Fetched<Detail>>) -> Option<Text> {
    let f = f?;
    kit::fetching(f.loading, f.value.is_some(), f.error.as_deref())
}

// --- the page as view data -------------------------------------------------------------

fn tabs(page: &Page) -> Vec<String> {
    let mut labels = TABS.iter().map(|name| (*name).to_string()).collect::<Vec<_>>();
    labels.extend(page.extra.borrow().iter().map(|(title, _)| title.clone()));
    labels
}

/// The tabs with their icons: Overview, Comments, Links, then Code for the
/// merge requests; another part's tab has none.
fn tab_views(labels: Vec<String>) -> Vec<TabView> {
    labels
        .into_iter()
        .enumerate()
        .map(|(i, l)| {
            let icon = match i {
                OVERVIEW => Some(Icon::Info),
                COMMENTS => Some(Icon::Thread),
                LINKS => Some(Icon::Link),
                _ if l.eq_ignore_ascii_case(CODE) => Some(Icon::Change),
                _ => None,
            };
            TabView::new(l, icon)
        })
        .collect()
}

/// The page on top of the stack as view data.
pub(crate) fn view(p: &JiraPlugin, cx: &v::ViewCx<'_>) -> Option<v::ScreenView> {
    let page = p.pages.last()?;
    let extra = added_tabs(p, page, &cx.parts);
    *page.extra.borrow_mut() = extra.tabs;
    let mut view = PageView {
        crumbs: crumbs(p, page),
        title: Text::new().push(Seg::new(page.key.clone(), Ink::Emphasis).strong()),
        details: Text::new(),
        // Before the ticket is known its line stays, blank, so the tabs
        // do not move when it comes.
        under: Some(Text::new()),
        tabs: tab_views(tabs(page)),
        tab: page.tab.min(page.tab_count() - 1),
        notice: None,
        panes: vec![],
        layout: None,
    };
    // The summary on the left; status, type and assignee on the right; the
    // parent, project and when it changed under it.
    if let Some(tk) = ticket(p, page) {
        // Pages give the full name; rows are the short ones.
        let who = if tk.assignee.is_empty() { "unassigned".into() } else { tk.assignee.clone() };
        let [who_icon, who_name] = screen::assignee_segs(p, &page.site, tk, who, Ink::Secondary);
        view.title = Text::new()
            .push(Seg::icon(tk.icon()))
            .push(Seg::new(tk.summary.clone(), Ink::Emphasis).strong());
        let mut details = status_text(p, &tk.status, tk.category);
        details.extend(Text::from(vec![
            Seg::new(" · ", Ink::Muted),
            Seg::styled(tk.kind.clone(), p.cfg.style_for(tk.type_word())),
            Seg::new(" · ", Ink::Muted),
            who_icon,
            who_name,
        ]));
        view.details = details;
        let mut under = Text::new();
        if let Some(par) = &tk.parent {
            under.add(Seg::new("parent ", Ink::Muted));
            under.add(Seg::styled(par.key.clone(), p.cfg.style_for("key")));
            under.add(Seg::new(format!(" {} · ", par.summary), Ink::Secondary));
        }
        if !tk.project_name.is_empty() {
            under.add(Seg::new(format!("{} · ", tk.project_name), Ink::Secondary));
        }
        under.add(Seg::icon(Icon::Updated));
        under.add(Seg::new(format!("updated {} ago", ago(&tk.updated, cx.now)), Ink::Muted));
        view.under = Some(under);
    }
    view.panes = if let Some(tab) = page.added() {
        // A whole-page read can fail while its Jira Code rows remain. Name the
        // source, and never attach that failure to independently supplied rows.
        let notice = if extra.jira_code.is_some_and(|i| page.tab == TABS.len() + i) {
            fetched(p, page)
                .and_then(|f| f.error.as_deref())
                .map(|error| kit::sync_failed("Jira page", error))
        } else {
            None
        };
        vec![added(page, &tab.0, &tab.1, notice)]
    } else {
        match page.tab {
            OVERVIEW => overview(p, page, cx),
            COMMENTS => vec![comments(p, page, cx.now)],
            _ => vec![links(p, page)],
        }
    };
    Some(v::ScreenView::page(SCREEN_ID, view))
}

const SCREEN_ID: &str = crate::SCREEN.id;

/// A tab of rows (Code, or another part's), with a cursor.
fn added(page: &Page, title: &str, rows: &[Row], notice: Option<Text>) -> PaneView {
    let body = if rows.is_empty() {
        Node::State {
            state: v::State::Empty,
            text: "Nothing here yet.".into(),
        }
    } else {
        let row = page.row.get().min(rows.len() - 1);
        page.row.set(row);
        Node::List {
            // A row about something with a link opens it on a click.
            rows: rows
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    v::ListRow::new(format!("row:{i}"), r.line.clone())
                        .linked(r.subject.as_ref().and_then(|s| s.url.clone()))
                })
                .collect(),
            selected: Some(row),
            editing: None,
        }
    };
    let mut pane = PaneView::new(PaneId::Middle, title.to_lowercase(), body);
    pane.head = notice;
    pane.inset = true;
    pane
}

/// Lines at the top of a tab: the last change and what Jira said.
fn change_lines(p: &JiraPlugin, page: &Page) -> Vec<Text> {
    match crate::write::outcome(p, &page.site, &page.key) {
        Some(l) => vec![l, Text::new()],
        None => vec![],
    }
}

/// What other parts add to the ticket's overview, each under its title.
pub(crate) fn section_lines(p: &JiraPlugin, site: &str, key: &str, parts: &Parts<'_>) -> Vec<Text> {
    let mut out = Vec::new();
    for (title, lines) in parts.sections(&crate::join::ticket(p, site, key)) {
        out.push(Text::new());
        out.push(kit::heading(&title));
        out.extend(lines);
    }
    out
}

/// The details beside the description: who, where, when, what it is in.
fn facts(p: &JiraPlugin, site: &str, tk: &Ticket, d: Option<&Detail>, now: i64) -> Vec<Text> {
    let kv = |label: &str, value: Text| kit::labelled(label, 14, value);
    let plain = |s: &str| {
        if s.is_empty() {
            Text::of("—", Ink::Muted)
        } else {
            Text::of(s, Ink::Primary)
        }
    };
    let list = |v: &[String]| plain(&v.join(", "));
    // A value with its icon in front (nothing when icons are off, or when
    // there is no value to mark).
    let led = |i: Icon, mut v: Text| {
        if v.0.first().is_some_and(|s| s.text != "—") {
            v.0.insert(0, Seg::icon(i));
        }
        v
    };
    let mut lines = vec![
        kv("status", status_text(p, &tk.status, tk.category)),
        kv("type", Text::from(vec![Seg::styled(tk.kind.clone(), p.cfg.style_for(tk.type_word()))])),
        kv("priority", {
            let mut v = plain(&tk.priority);
            if !tk.priority.is_empty() {
                v.0.insert(0, screen::priority_seg(&tk.priority));
            }
            v
        }),
        kv("assignee", {
            let name = if tk.assignee.is_empty() { "unassigned" } else { &tk.assignee };
            let [i, n] = screen::assignee_segs(p, site, tk, name.to_string(), Ink::Primary);
            if tk.assignee.is_empty() { Text::from(vec![n]) } else { Text::from(vec![i, n]) }
        }),
        kv("reporter", led(Icon::Person, plain(&tk.reporter))),
        kv(
            "sprint",
            match d {
                Some(d) => led(Icon::Calendar, plain(d.sprint.as_deref().unwrap_or(""))),
                None => Text::of("…", Ink::Muted),
            },
        ),
    ];
    if let Some(par) = &tk.parent {
        lines.push(kv(
            "parent",
            Text::from(vec![
                Seg::styled(format!("{} ", par.key), p.cfg.style_for("key")),
                Seg::new(par.summary.clone(), Ink::Primary),
            ]),
        ));
    }
    if let Some(d) = d {
        let kids: Vec<&crate::model::Link> = d.links.iter().filter(|l| l.relation == "child").collect();
        if !kids.is_empty() {
            lines.push(kv("children", Text::of(format!("{} (Links tab)", kids.len()), Ink::Primary)));
        }
    }
    lines.extend([
        kv(
            "labels",
            if tk.labels.is_empty() { plain("") } else { led(Icon::Label, labels_text(p, &tk.labels)) },
        ),
        kv("components", list(&tk.components)),
        kv("fix versions", list(&tk.fix_versions)),
        kv("created", led(Icon::Calendar, plain(&format!("{} ago", ago(&tk.created, now))))),
        kv("updated", led(Icon::Updated, plain(&format!("{} ago", ago(&tk.updated, now))))),
    ]);
    lines
}

fn overview(p: &JiraPlugin, page: &Page, cx: &v::ViewCx<'_>) -> Vec<PaneView> {
    let f = fetched(p, page);
    let Some(tk) = ticket(p, page) else {
        let body = match fetch_state(f).filter(|_| f.is_some_and(|f| f.error.is_some())) {
            Some(line) => Node::Rich {
                blocks: vec![Block::line(line)],
                pictures: None,
            },
            None => Node::State {
                state: v::State::Loading,
                text: "loading…".into(),
            },
        };
        return vec![PaneView::new(PaneId::Middle, "overview", body)];
    };
    let d = detail(p, page);
    let mut facts = facts(p, &page.site, tk, d, cx.now);
    // Wide enough: the details beside the text.
    let side = (cx.width >= 100).then(|| {
        let mut side = PaneView::new(PaneId::Detail, "details", Node::Document { lines: std::mem::take(&mut facts), cut: true });
        side.width = Some(40);
        side
    });
    let mut blocks: Vec<Block> = change_lines(p, page).into_iter().map(Block::line).collect();
    if let Some(l) = fetch_state(f) {
        blocks.extend([Block::line(l), Block::line(Text::new())]);
    }
    if !facts.is_empty() {
        blocks.extend(facts.into_iter().map(Block::line));
        blocks.push(Block::line(Text::new()));
    }
    blocks.push(Block::line(kit::heading("description")));
    if tk.description.is_empty() {
        blocks.push(Block::line(Text::of("no description", Ink::Muted)));
    } else {
        blocks.push(Block::doc(tk.description.clone()));
    }
    blocks.push(Block::line(Text::new()));
    blocks.push(Block::line(kit::heading("acceptance criteria")));
    let set = p.cfg.site(&page.site).and_then(|s| s.ac_field(&tk.project));
    match (set, tk.ac.is_empty()) {
        (None, _) => blocks.push(Block::line(Text::of(
            format!(
                "No field is set to hold {}'s acceptance criteria: settings, projects picks one.",
                tk.project
            ),
            Ink::Muted,
        ))),
        (Some(_), true) => blocks.push(Block::line(Text::of("none written", Ink::Muted))),
        (Some(_), false) => blocks.push(Block::doc(tk.ac.clone())),
    }
    blocks.extend(section_lines(p, &page.site, &page.key, &cx.parts).into_iter().map(Block::line));
    let main = PaneView::new(
        PaneId::Middle,
        "overview",
        Node::Rich {
            blocks,
            pictures: Some(page.site.clone()),
        },
    );
    std::iter::once(main).chain(side).collect()
}

fn comments(p: &JiraPlugin, page: &Page, now: i64) -> PaneView {
    let f = fetched(p, page);
    let d = detail(p, page);
    let title = match d {
        Some(d) if d.comments_total > 0 => format!("comments · newest first · {} of {}", d.comments.len(), d.comments_total),
        _ => "comments".into(),
    };
    let mut blocks: Vec<Block> = change_lines(p, page).into_iter().map(Block::line).collect();
    blocks.extend(fetch_state(f).map(Block::line));
    if let Some(d) = d {
        if d.comments.is_empty() {
            blocks.push(Block::line(Text::of("No comments yet.", Ink::Muted)));
        }
        for (i, c) in d.comments.iter().enumerate() {
            if i > 0 {
                blocks.push(Block::line(Text::new()));
            }
            blocks.push(Block::line(Text::from(vec![
                Seg::icon(Icon::Comment),
                Seg::new(c.author.clone(), Ink::Secondary).strong(),
                Seg::new(format!(" · {} ago", ago(&c.created, now)), Ink::Muted),
            ])));
            blocks.push(Block::Doc {
                doc: c.body.clone(),
                bar: Some(Seg::new("│ ", Ink::AccentMuted)),
                most: None,
                urls: None,
            });
        }
        let left = d.comments_total.saturating_sub(d.comments.len());
        if left > 0 {
            blocks.push(Block::line(Text::new()));
            blocks.push(Block::line(Text::of(
                if f.is_some_and(|f| f.more.is_some()) {
                    "loading older comments…".to_string()
                } else {
                    format!("{left} older: they load as you scroll down")
                },
                Ink::Muted,
            )));
        }
    }
    PaneView::new(
        PaneId::Middle,
        title,
        Node::Rich {
            blocks,
            pictures: Some(page.site.clone()),
        },
    )
}

fn links(p: &JiraPlugin, page: &Page) -> PaneView {
    let f = fetched(p, page);
    let Some(d) = detail(p, page) else {
        let line = fetch_state(f).unwrap_or_else(kit::loading);
        return PaneView::new(PaneId::Middle, "links", Node::Document { lines: vec![line], cut: true });
    };
    let body = if d.links.is_empty() {
        Node::State {
            state: v::State::Empty,
            text: "No parent, children or links.".into(),
        }
    } else {
        let columns = vec![
            ColumnSpec::new("relation", 16, 0),
            ColumnSpec::new("ticket", 12, 0),
            ColumnSpec::new("status", 18, 0),
            ColumnSpec::flex("title", 10),
        ];
        // A cell's words, cut with `…` to its column.
        let cut = |s: &str, w: usize| ui::fit(s, w - 1).trim_end().to_string();
        let rows = d
            .links
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let (who, status) = match &l.target {
                    Target::Ticket { key, status, category } => {
                        (
                            Text::new().push(Seg::icon(Icon::Issue)).push(Seg::styled(cut(key, 12), p.cfg.style_for("key"))),
                            if status.is_empty() {
                                Text::new()
                            } else {
                                status_text(p, status, *category)
                            },
                        )
                    }
                    Target::Web { .. } => (
                        Text::new().push(Seg::icon(Icon::Link)).push(Seg::new("web", Ink::Muted)),
                        Text::new(),
                    ),
                };
                // The title opens what the row is, in the terminal's way.
                let url = match &l.target {
                    Target::Ticket { key, .. } => crate::join::ticket(p, &page.site, key).url.unwrap_or_default(),
                    Target::Web { url } => url.clone(),
                };
                v::TableRow::new(format!("row:{i}"))
                    .cell("relation", Text::of(cut(&l.relation, 16), Ink::Secondary))
                    .cell("ticket", who)
                    .cell("status", status)
                    .cell("title", Text::new().push(Seg::new(l.title.clone(), Ink::Primary).linked(url)))
            })
            .collect();
        let row = page.row.get().min(d.links.len() - 1);
        page.row.set(row);
        Node::Table {
            columns,
            rows,
            selected: Some(row),
            empty: None,
        }
    };
    let mut pane = PaneView::new(PaneId::Middle, "links", body);
    // A failed refresh keeps the links, but its error must stay visible above them.
    pane.head = fetch_state(f);
    pane
}

// --- keys and intents -------------------------------------------------------------------

/// What `⏎` does on a list tab's row: a ticket's page, a web page, or
/// (on an added tab) showing the row's subject.
enum Enter {
    Ticket(String),
    Web(String),
    Show(Subject),
}

fn enter(p: &JiraPlugin, page: &Page) -> Option<Enter> {
    if let Some(tab) = page.added() {
        return tab.1.get(page.row.get())?.subject.clone().map(Enter::Show);
    }
    if page.tab != LINKS {
        return None;
    }
    let l = detail(p, page)?.links.get(page.row.get())?;
    Some(match &l.target {
        Target::Ticket { key, .. } => Enter::Ticket(key.clone()),
        Target::Web { url } => Enter::Web(url.clone()),
    })
}

fn browse(p: &JiraPlugin, page: &Page) -> Option<String> {
    p.cfg.site(&page.site)?.browse(&page.key)
}

fn row_at(target: &v::Target) -> Option<usize> {
    target.as_str().strip_prefix("row:")?.parse().ok()
}

/// What the user did on the page, from any frontend: a tab or a row
/// chosen, `⏎` on a row, the comments scrolled to their end, or a key.
pub(crate) fn intent(p: &mut JiraPlugin, intent: &Intent) -> Outcome {
    match intent {
        Intent::Select { target } if TabView::index(target).is_some() => {
            let i = TabView::index(target).unwrap_or(0);
            Outcome::Handled(show_tab(p, i))
        }
        Intent::Select { target } | Intent::Context { target } if row_at(target).is_some() => {
            if let Some(page) = p.pages.last() {
                page.row.set(row_at(target).unwrap_or(0));
            }
            Outcome::done()
        }
        Intent::Activate { target } if row_at(target).is_some() => {
            if let Some(page) = p.pages.last() {
                page.row.set(row_at(target).unwrap_or(0));
            }
            key(p, radar_core::view_tui::key_of(v::Chord::new(v::Key::Enter)))
        }
        // At the end of the comments: bring the older ones.
        Intent::End { .. } => {
            let Some(page) = p.pages.last() else {
                return Outcome::Ignored;
            };
            if page.tab != COMMENTS {
                return Outcome::done();
            }
            let (site, tkey) = (page.site.clone(), page.key.clone());
            Outcome::Handled(fetch_more(p, &site, &tkey))
        }
        Intent::Key { .. } | Intent::Invoke { .. } => match intent.chord() {
            Some(chord) => key(p, radar_core::view_tui::key_of(chord)),
            None => Outcome::Ignored,
        },
        _ => Outcome::Ignored,
    }
}

/// A key the frontend did not use for moving or scrolling.
pub(crate) fn key(p: &mut JiraPlugin, key: KeyEvent) -> Outcome {
    crate::write_context::reconcile(p);
    let Some(page) = p.pages.last() else {
        return Outcome::Ignored;
    };
    let (site, tkey, tab) = (page.site.clone(), page.key.clone(), page.tab);
    match key.code {
        KeyCode::Esc => {
            p.pages.pop();
            Outcome::Handled(if p.pages.is_empty() {
                vec![Effect::Focus(Pane::Middle)]
            } else {
                vec![]
            })
        }
        KeyCode::Char(c @ ('[' | ']')) => {
            let n = page.tab_count();
            let to = if c == ']' { (tab + 1) % n } else { (tab + n - 1) % n };
            Outcome::Handled(show_tab(p, to))
        }
        KeyCode::Char('r') => {
            let fx = fetch(p, &site, &tkey, true);
            Outcome::Handled(if p.cfg.site(&site).is_none() {
                vec![Effect::Notify("Jira site is no longer configured".into())]
            } else if fx.is_empty() {
                vec![Effect::Notify("already asking Jira".into())]
            } else {
                fx
            })
        }
        KeyCode::Char('o') => Outcome::Handled(browse(p, page).map(Effect::Open).into_iter().collect()),
        KeyCode::Char('y') => Outcome::Handled(match browse(p, page) {
            Some(url) => vec![Effect::Copy(url), Effect::Notify("link copied".into())],
            None => vec![],
        }),
        KeyCode::Char('Y') => Outcome::Handled(vec![
            Effect::Copy(tkey.clone()),
            Effect::Notify(format!("{tkey} copied")),
        ]),
        KeyCode::Enter => match enter(p, page) {
            Some(Enter::Ticket(k)) => Outcome::Handled(open(p, &site, &k)),
            Some(Enter::Web(url)) => Outcome::Handled(vec![Effect::Open(url)]),
            Some(Enter::Show(s)) => Outcome::Handled(vec![Effect::Show(s)]),
            None => Outcome::done(),
        },
        _ => Outcome::Ignored,
    }
}

/// The page's tabs, for the host to move across (GUIDE 1.5).
pub(crate) fn page_tabs(p: &JiraPlugin) -> Option<radar_core::plugin::PageTabs> {
    let page = p.pages.last()?;
    let names = tabs(page);
    let on = page.tab.min(names.len().saturating_sub(1));
    Some(radar_core::plugin::PageTabs { names, on })
}

/// Show tab `i` from its top. The host asks on `← → h l`, the page on
/// `[` `]`.
pub(crate) fn show_tab(p: &mut JiraPlugin, i: usize) -> Vec<Effect> {
    if let Some(page) = p.pages.last_mut() {
        let i = i.min(page.tab_count().saturating_sub(1));
        if i != page.tab {
            page.tab = i;
            page.row.set(0);
        }
    }
    vec![Effect::Focus(Pane::Middle)]
}

/// The page's actions menu (`x`): titled with the ticket, only its actions.
pub(crate) fn keys(p: &JiraPlugin) -> Vec<Binding> {
    let Some(page) = p.pages.last() else {
        return vec![];
    };
    let mut b = vec![Binding::heading(page.key.clone())];
    match enter(p, page) {
        Some(Enter::Ticket(k)) => b.push(Binding::new("⏎", format!("open {k}'s page"))),
        Some(Enter::Web(_)) => b.push(Binding::new("⏎", "open the link in your browser")),
        Some(Enter::Show(s)) => b.push(Binding::new("⏎", format!("show {}", s.label))),
        None => {}
    }
    if browse(p, page).is_some() {
        b.push(Binding::new("o", "open it in Jira"));
        b.push(Binding::new("y", "copy its link"));
    }
    b.push(Binding::new("Y", format!("copy {}", page.key)));
    b.push(Binding::new("r", "refresh this page"));
    b.push(Binding::heading("page"));
    let labels = tabs(page);
    let n = labels.len();
    let word = |i: usize| labels[i % n].split(" · ").next().unwrap_or_default().to_string();
    b.push(Binding::new("]", format!("next tab: {}", word(page.tab + 1))));
    b.push(Binding::new("[", format!("previous tab: {}", word(page.tab + n - 1))));
    let back = match p.pages.len() {
        0 | 1 => "the middle pane".to_string(),
        n => p.pages[n - 2].key.clone(),
    };
    b.push(Binding::new("esc", format!("back to {back}")));
    b
}

// --- the CLI's `show` -------------------------------------------------------------------

/// A ticket for `radar jira show`: as text, or JSON.
pub(crate) fn as_text(site: &Site, d: &Detail, json_out: bool) -> String {
    let tk = &d.ticket;
    let url = site.browse(&tk.key);
    if json_out {
        let links: Vec<serde_json::Value> = d
            .links
            .iter()
            .map(|l| match &l.target {
                Target::Ticket { key, status, .. } => {
                    json!({ "relation": l.relation, "key": key, "status": status, "title": l.title })
                }
                Target::Web { url } => json!({ "relation": l.relation, "url": url, "title": l.title }),
            })
            .collect();
        let comments: Vec<serde_json::Value> = d
            .comments
            .iter()
            .map(|c| {
                json!({
                    "author": screen::person(&c.author, ""),
                    "created": screen::rfc3339(&c.created),
                    "body": lines_text(&c.body),
                })
            })
            .collect();
        let v = json!({
            "site": site.name, "key": tk.key, "summary": tk.summary, "type": tk.kind,
            "status": tk.status, "category": tk.category.word(), "priority": tk.priority,
            "assignee": screen::person(&tk.assignee, &tk.assignee_id),
            "reporter": screen::person(&tk.reporter, ""), "sprint": d.sprint,
            "parent": tk.parent.as_ref().map(|p| json!({ "key": p.key, "summary": p.summary })),
            "labels": tk.labels, "components": tk.components, "fix_versions": tk.fix_versions,
            "created": screen::rfc3339(&tk.created), "updated": screen::rfc3339(&tk.updated), "url": url,
            "description": lines_text(&tk.description),
            "acceptance_criteria": lines_text(&tk.ac),
            "links": links, "comments": comments, "comments_total": d.comments_total,
        });
        return serde_json::to_string_pretty(&v).unwrap_or_default() + "\n";
    }
    let mut out = format!("{}  {}\n", tk.key, tk.summary);
    let mut kv = |k: &str, v: &str| {
        if !v.is_empty() {
            out.push_str(&format!("  {k:<14}{v}\n"));
        }
    };
    kv("status", &format!("{} {}", tk.category.symbol(), tk.status));
    kv("type", &tk.kind);
    kv("priority", &tk.priority);
    kv("assignee", if tk.assignee.is_empty() { "unassigned" } else { &tk.assignee });
    kv("reporter", &tk.reporter);
    kv("sprint", d.sprint.as_deref().unwrap_or(""));
    kv(
        "parent",
        &tk.parent.as_ref().map(|p| format!("{} {}", p.key, p.summary)).unwrap_or_default(),
    );
    kv("labels", &tk.labels.join(", "));
    kv("components", &tk.components.join(", "));
    kv("fix versions", &tk.fix_versions.join(", "));
    kv("link", url.as_deref().unwrap_or(""));
    let mut block = |title: &str, text: String| {
        out.push_str(&format!("\n{title}\n"));
        if text.is_empty() {
            out.push_str("  (none)\n");
        }
        for l in text.lines() {
            out.push_str(&format!("  {l}\n"));
        }
    };
    block("DESCRIPTION", lines_text(&tk.description));
    block("ACCEPTANCE CRITERIA", lines_text(&tk.ac));
    let links: Vec<String> = d
        .links
        .iter()
        .map(|l| match &l.target {
            Target::Ticket { key, status, .. } => format!("{:<16}{key:<12}{status:<16}{}", l.relation, l.title),
            Target::Web { url } => format!("{:<16}{} ({url})", l.relation, l.title),
        })
        .collect();
    block("LINKS", links.join("\n"));
    let comments: Vec<String> = d
        .comments
        .iter()
        .take(5)
        .map(|c| format!("{} · {}\n{}", c.author, c.created, lines_text(&c.body)))
        .collect();
    block(
        &format!("COMMENTS ({} in all, newest first)", d.comments_total),
        comments.join("\n\n"),
    );
    out
}

fn lines_text(doc: &Doc) -> String {
    text(doc)
}

#[cfg(test)]
mod rich_tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn no_line_is_wider_than_asked() {
        let t = Theme::new(radar_core::design::theme::Tier::Mono);
        let long = format!("https://example.com/{}", "x".repeat(150));
        for text in [format!("- {long}"), long.clone(), format!("> {long}")] {
            let doc = crate::adf::doc(&crate::adf::from_text(&text));
            let lines = rich(&t, &doc, 40);
            for l in &lines {
                assert!(l.width() <= 40, "{text}: {} wide: {:?}", l.width(), l);
            }
            let joined: String = lines.iter().flat_map(|l| l.spans.iter().map(|s| s.content.to_string())).collect::<String>().replace([' ', '│'], "");
            assert!(joined.contains(&"x".repeat(150)), "nothing lost");
        }
        // A link after a list marker runs on from the marker's line, with
        // no space put into it.
        let doc = crate::adf::doc(&crate::adf::from_text(&format!("- https://example.com/{}", "y".repeat(60))));
        let lines = rich(&t, &doc, 30);
        let text: Vec<String> = lines.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect()).collect();
        assert!(text[0].starts_with("• https://example.com/yyy"), "{text:?}");
        for l in &text[1..] {
            assert!(l.starts_with("  y") && !l.trim().contains(' '), "{text:?}");
        }
        // Words that link show where they go.
        let doc = crate::adf::doc(&crate::adf::from_text("see [notes](https://example.com/n)"));
        assert_eq!(lines_text(&doc), "see notes (https://example.com/n)");
        assert_eq!(lines_text(&crate::adf::doc(&Value::Null)), "");
    }
}

#[cfg(test)]
#[path = "page_ownership_tests.rs"]
mod ownership_tests;

#[cfg(test)]
#[path = "page_code_cache_tests.rs"]
mod code_cache_tests;

#[cfg(test)]
#[path = "page_cached_refresh_tests.rs"]
mod cached_refresh_tests;

#[cfg(test)]
#[path = "page_code_refresh_tests.rs"]
mod code_refresh_tests;

#[cfg(test)]
mod page_identity_tests {
    use super::*;

    #[test]
    fn page_identity_distinguishes_site_and_ticket() {
        let page = Page::new("ankh-morpork", "WATCH-12");
        assert_eq!(page.identity(), r#"["ticket","ankh-morpork","WATCH-12"]"#);
        assert_ne!(page.identity(), Page::new("lancre", "WATCH-12").identity());
        assert_ne!(page.identity(), Page::new("ankh-morpork", "WATCH-13").identity());
        assert_ne!(Page::new("ankh:morpork", "WATCH-12").identity(),
            Page::new("ankh", "morpork:WATCH-12").identity());
    }

    #[test]
    fn page_identity_ignores_tab_row_and_contributed_labels() {
        let mut page = Page::new("ankh-morpork", "WATCH-12");
        let identity = page.identity();
        page.tab = LINKS;
        page.row.set(3);
        page.extra.borrow_mut().push(("Watch notes".into(), vec![]));
        assert_eq!(page.identity(), identity);
    }
}
